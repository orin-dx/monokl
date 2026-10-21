# Cache Architecture — Semantic Model

Source: `docs/spec/01-core-architecture.md` §12 (`analysis/content_hash.rs`), §15 (`analysis/cache.rs`),
§16 (`analysis/persist.rs`); `docs/spec/03-multi-language-platform.md` §2a (`analysis/cache_tiers.rs`,
the 0.5.0+ extraction of the lookup logic shared across analyzers). Cross-referenced against
`docs/spec/07-edge-cases-and-failure-modes.md` Part 1 #4/#5/#18, Part 3 #1.
No code exists yet — monokl is spec-stage. Re-point at source once the crate exists; sync via `canon/drift`.

## Two-Layer Storage

1. **In-memory** — process-global `DashMap<String, Arc<FileAnalysis>>`, keyed by path string
   (§15). Lock-free reads. `insert` unconditionally replaces; no CAS/merge.
2. **On-disk** — `<root>/.monokl/cache.json`: `CacheFile { version: u32, config_hash: String,
   entries: BTreeMap<Utf8PathBuf, PersistedFileAnalysis> }` (§16). Gated on `version ==
   CACHE_VERSION` (`2`) and `config_hash == "monokl-{CARGO_PKG_VERSION}-oxc-0.128-lang-ts-{tsconfig_blake3_or_'no-tsconfig'}"`
   — an upgrade or tsconfig change invalidates the file. Mismatch → `MonoklError::StaleDiskCache`,
   caught in `persist::init()` and treated as cold start, never propagated to the caller.

## Intended Invariant: ContentHash Is Authoritative

Stated in §29 tenet 8 and `ARCHITECTURE.md`: "ContentHash (blake3) is authoritative; mtime+size
is the cheap fast-path dirty bit." The four-tier lookup (`cache_tiers::lookup_or_parse`, shared
by every `LanguageAnalyzer` since the 0.5.0 extraction — duplicated per-analyzer before that):

1. **Tier 1** — `stat()`; mtime+size match persisted entry → return cached, no file read.
2. **Tier 2** — read source, compute `ContentHash`, check in-memory `DashMap`; hash match → return.
3. **Tier 3** — `persist::lookup_by_hash`: mtime changed, content hash still matches disk entry →
   refresh mtime, return cached.
4. **Tier 4** — full parse, `cache::insert` + `persist::queue_write`.

Intent: mtime+size is a cheap pre-filter; ContentHash is what certifies validity. Tier 1 existing
should never let stale data escape that a hash check would have rejected.

## Known Defect, Not Yet Fixed in the Spec Text

**Tier 1 currently returns on mtime+size alone with no hash confirmation**, in both the v0.1.0
inline form (§17 `TsAnalyzer::analyze`) and the 0.5.0+ shared helper (§2a
`cache_tiers::lookup_or_parse`):

```rust
// Tier 1: mtime/size fast path
if let Some(persisted) = persist::lookup(path, mtime_ns, size_bytes)? {
    if let Some(analysis) = from_persisted(persisted, profile) {
        return Ok(analysis); // ContentHash never read or compared here
    }
}
```

`persist::lookup` (§16) compares only `entry.mtime_ns == mtime_ns && entry.size_bytes ==
size_bytes` — it never touches `entry.content_hash`. The "ContentHash is authoritative" claim is
**not implemented anywhere in the current spec**: the only path that reaches a hash comparison
(Tier 2/3) is unreachable whenever Tier 1 hits — the common case on a warm cache. A same-second
edit on a coarse-mtime filesystem, or clock skew, can serve stale analysis with zero diagnostic.

Pre-existing from v0.1.0's inline code — the 0.5.0+ extraction ported the bug, not introduced it
(`07-edge-cases-and-failure-modes.md` Part 3 #1, traced back to the original in the cross-cutting
section).

**Any spec@1 for this subsystem MUST decide:**

- **(a)** Add a content-hash confirmation step to Tier 1, closing the staleness window (changes
  Tier 1's cost profile — no longer a no-read path), **or**
- **(b)** Formally weaken the claimed guarantee to match what mtime+size actually provides, and
  document the residual staleness risk explicitly (tenet 8, `ARCHITECTURE.md`, this file).

**Do not silently port the current code — it does not deliver the guarantee the architecture
asserts.** Update `ARCHITECTURE.md` tenet 8 and this file together with whichever choice is made.

## `ContentHash` Type

```rust
// analysis/content_hash.rs, §12
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn of(content: &[u8]) -> Self { Self(blake3::hash(content).to_hex().to_string()) }
    pub fn as_str(&self) -> &str { &self.0 }
}
```

Hex-encoded blake3 digest, newtype-wrapped. No streaming/incremental API — `of()` always hashes
the full byte slice. `PartialEq`/`Eq` are derived, so hash comparisons elsewhere are plain string
equality on the hex form.

## Disk Persistence: Atomic Write, Three Open Gaps

`persist::flush` (§16): merge `write_queue` into the `CacheFile` snapshot, serialize, LRU-evict
in chunks of `(len/10).max(1)` if over `MAX_CACHE_BYTES` (100 MB), write to
`.monokl/cache.json.<pid>.tmp`, then `rename` onto `.monokl/cache.json`. Per-process temp
filename prevents same-machine nextest races on the temp file. Rename atomicity guarantees no
reader ever observes a torn/partial `cache.json` — it guarantees nothing else. Three gaps remain
open questions for the spec, not resolved:

1. **Concurrent-write race, last-writer-wins** (Part 1 #4). Two concurrent invocations each load
   their own snapshot at `init()`, mutate independently, `rename()` over the same path — no
   cross-process merge. Last to finish wins outright; the other's newly-cached entries vanish
   silently. Self-healing in effect (a lost entry just costs a re-parse next run, once Tier 2/3
   is actually reached), but the race itself is real and currently undocumented.
2. **Corrupted-vs-stale asymmetry** (Part 1 #5). `load_cache` catches `StaleDiskCache` (version
   or config_hash mismatch) and rebuilds from empty. It does **not** catch a genuinely malformed
   `cache.json` (e.g. a disk-full truncated write) — `serde_json::from_str` failure surfaces as
   `MonoklError::Json` via `?`, uncaught, and hard-fails the whole workspace build. Staleness
   self-heals; corruption of comparable severity does not, with no stated rationale.
3. **Orphaned temp files** (Part 1 #18). `.monokl/cache.json.<pid>.tmp` left behind by a process
   killed between `write` and `rename` is never cleaned up — a slow disk-space leak with no
   documented GC or startup sweep.

## Caller Contract

- `persist::init(root, config_hash)` must run before any `lookup`/`lookup_by_hash` call — both
  are pure reads against the `state.cache_file` snapshot taken at `init()`; they never re-read
  `cache.json` mid-process, so the read path has no torn-read hazard (only the write race above).
- `queue_write` buffers into `state.write_queue`; only `flush()` (once, at the end of
  `WorkspaceIndex::build`) performs the atomic write.
- `root` must be absolute — a relative path is rejected with `MonoklError::Io` before any cache
  file is touched.
- Cache writes happen only for `AnalysisProfile::Full` results (§2a) — a cheap
  `Dependencies`/`Structural` request must never overwrite a fuller cached entry with a thinner
  one. Any new analyzer plugging into `cache_tiers` must preserve this.

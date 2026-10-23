# Daemon Lifetimes — Semantic Model

> **FUTURE DESIGN — not implemented, not required for v0.1.0.** This file exists so that
> v0.1.0 work (especially the `LanguageAnalyzer` trait and cache architecture) doesn't
> foreclose this direction. See `docs/spec/06-daemon-architecture.md` for full context.

Source: `docs/spec/06-daemon-architecture.md`

## `PrecisionUpgrader`

Separate trait from `LanguageAnalyzer`, alongside `AnalyzerRegistry`. Runs *after* it, never
instead of it, only when available (§2).

```rust
pub trait PrecisionUpgrader: Send + Sync {
    fn languages(&self) -> &[LanguageId];
    /// Cheap, synchronous check — toolchain present, session reachable. Never blocks on a
    /// full backend init.
    fn is_available(&self) -> bool;
    fn upgrade(&self, request: UpgradeRequest) -> Result<UpgradeOutcome>;
}

pub struct UpgradeOutcome {
    pub precision: CapabilityPrecision,      // may still degrade to Heuristic/Structural
    pub provenance: &'static str,             // backend+operation id, never erased (§6 rule 5)
    pub candidates: Vec<UpgradeCandidate>,    // 0/1/N; ambiguity is len() > 1, none dropped
    pub extension: Option<DaemonExtension>,   // per-language typed data, never flattened (§4)
}
```

Returns an outcome, not a bare `CapabilityPrecision`: backends (Kotlin `KaErrorCallInfo`, Go's
dual local/global satisfaction, Scala's resolved-value-vs-provenance split, Rust trait-solver
facts) hand back non-unifiable shapes. One enum bump would manufacture confidence the upstream
tool never claimed.

## `ResourceLifetime` / `SessionOwnership`

```rust
pub enum ResourceLifetime {
    PerCall,                    // spawn fresh, use once, discard
    PerInvocation,               // constructed once, reused across one CLI run
    Session(SessionOwnership),  // held warm across many calls
}

pub enum SessionOwnership {
    Owned,     // monokl's host process spawns and fully owns this resource
    Attached,  // discovers/attaches to a resource it doesn't control — must detect
               // absence/staleness/version mismatch, never assume presence
}
```

**Lifetimes nest** — an upgrader declares typed sub-resources, each with its own lifetime, not
one flat field per impl. Canonical case: rust-analyzer's `proc-macro-srv-cli`, a subprocess
loaded once and reused with crash-supervision, living *inside* an otherwise-`PerInvocation`
Salsa build. C++ splits the other way: `ClangAstUpgrader{PerCall}` and `ClangdUpgrader{Session}`
are two separate impls for one language, since one impl can't honestly carry two lifetimes.

**Concurrency contract (invariant):** every `Session` resource is primed single-threaded before
parallel readers (`par_iter` over files) touch it; LSP backends run behind a connection-actor
(one task owns stdio, demultiplexes by request ID); `upgrade()` is read-only against the
resource during the parallel phase. Covers in-process thread safety only, not cross-process
(gap #2).

## `PrecisionLedger`

Keyed by `ContentHash` (same key `FileAnalysis`'s disk cache uses) but a separate store with its
own invalidation schedule, since confidence can change with no file change.

```rust
pub struct PrecisionAnnotation {
    pub target: AnnotationTarget,       // pointer, not a copy
    pub precision: CapabilityPrecision,
    pub provenance: &'static str,       // same discipline as UpgradeOutcome.provenance
    pub sentinel: Option<SentinelKind>, // which sentinel forced a downgrade, if any — §6
    pub for_content: ContentHash,       // mismatch = "stale, discard", no further comparison
}

pub enum AnnotationTarget { Span { start: usize, end: usize }, Symbol(SymbolId), Occurrence(OccurrenceId) }

pub struct PrecisionLedger {
    by_content: FxHashMap<ContentHash, Vec<PrecisionAnnotation>>,
}
```

Three write paths accumulate into it: **eager** (inline during `analyze_with_profile`), **lazy**
(query needs more precision than cached, backend now available), **retroactive** (future
`monokl enrich` sweep). `compute_precision()` (§7) is pure and downgrade-only per annotation; it
does not say what happens once several sit in the same `Vec` — gap #1.

## Known gaps — genuinely unresolved, not merely undocumented

Independently re-verified (`docs/spec/07-edge-cases-and-failure-modes.md`, "Parts 5 & 6" §,
findings #1–#4, #6 — no refutations recorded). Open questions for whoever writes spec@1:

1. **No ledger reconciliation rule.** Writes accumulate as a `Vec<PrecisionAnnotation>` per
   `ContentHash`, but nothing says how a reader resolves multiple entries on the same fact —
   last-write-wins, min-precision-wins, dedup-and-replace, append-and-let-caller-choose are all
   equally consistent with the struct as given.
2. **No cross-process story for `Session(Owned)`.** The concurrency contract above covers
   in-process thread safety only — nothing addresses an owned child process orphaned by `kill -9`
   of the parent (no pidfile/lockfile/reaping), or two monokl invocations racing to own the same
   backend for the same workspace.
3. **No degrade-back-to-baseline contract for mid-session `upgrade()` failure.** Java (§8)
   documents jdtls's hard-crash mode but mitigates it only at session *start* (a size gate) —
   nothing covers the workspace crossing that threshold *after* the gate passes and crashing
   mid-session. No fallback-to-cached-baseline, no re-establishment, no circuit-breaker.
4. **No fallback for Kotlin/Scala with no JVM present.** C/C++ falls back to the tree-sitter tier
   when Clang is missing; Kotlin/Scala were deliberately excluded from that same fallback tier, so
   the JVM-subprocess analyzer is each language's only rung below the deferred `Exact` tier, with
   nothing under it.
5. **No eviction or size-bound story for the ledger.** Combined with gap #1, growth is unbounded
   on two axes (content hashes × entries per hash) for a long-running session or repeated
   retroactive sweeps — no TTL, LRU, or max-size policy.

## Why this file exists before implementation

Two v0.1.0 decision points are shaped by this direction and should be checked against it now:

- **`LangData` extensibility** (`language-analyzer-contract.md`) — `FileAnalysis`'s
  shared-envelope/per-language-variant pattern is what §4 reuses for `DaemonExtension`;
  `FileAnalysis` needs a `daemon_extension: Option<DaemonExtension>` slot that stays `None` and
  inert until a `PrecisionUpgrader` exists, never merged into `language_data`.
- **`ContentHash` as a first-class, reusable type** (`cache-architecture.md`) — the ledger
  indexes by the same `ContentHash` the disk cache computes, as a second, independently
  invalidated store. If hash derivation stays buried inside the cache module instead of exposed
  as its own type, the ledger can't reuse it without duplicating the hashing logic.

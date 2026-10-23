# Part 6: Daemon / Session-Mode Architecture

> Design only, not yet implemented, not required for v0.1.0. Written so a future daemon/session-mode tier — `ra_ap_hir`, Salsa, `jdtls`, Kotlin/Scala full semantic resolution, or the real-compiler-frontend tiers in [05-research-and-decisions.md §12](./05-research-and-decisions.md) — has a shape to build into rather than forcing a rewrite of `LanguageAnalyzer`/`AnalyzerRegistry`/the cache layer later. Research and citations backing this design live in [05-research-and-decisions.md §14](./05-research-and-decisions.md).

Part of the [monokl spec](./README.md).

## 1. Why this exists

monokl has no daemon by default — every invocation is a fresh process reading from an on-disk cache. Real semantic resolution (name/type resolution, cross-file reference correctness) for several languages needs a real compiler or language server holding live project state, which doesn't fit "fresh process per invocation." That's `CapabilityPrecision::Exact`, and building it is deferred. Deferring the decision isn't the same as having no opinion on its shape: if `LanguageAnalyzer`/`AnalyzerRegistry` don't already accommodate it, "add the daemon later" becomes "rewrite the core traits later." This file is that shape.

## 2. The `PrecisionUpgrader` trait

A second trait, separate from `LanguageAnalyzer`, sitting alongside `AnalyzerRegistry`: it takes an already-produced result from the base per-file analysis and optionally enriches it using an external resource (a real compiler or language server). It runs after `LanguageAnalyzer`, never instead of it, and only when available.

```rust
pub trait PrecisionUpgrader: Send + Sync {
    fn languages(&self) -> &[LanguageId];
    /// Cheap, synchronous check — toolchain present, session reachable. Never blocks on a
    /// full backend init.
    fn is_available(&self) -> bool;
    fn upgrade(&self, request: UpgradeRequest) -> Result<UpgradeOutcome>;
}

pub struct UpgradeOutcome {
    /// What this specific upgrade attempt achieved — may still be `Heuristic`/`Structural` if a
    /// sentinel (§6) indicated the backend degraded internally.
    pub precision: CapabilityPrecision,
    /// Runtime backend identity, including version/configuration, plus the normalized operation.
    /// These are the `AnalyzerId` and `OperationId` shapes defined in Part 8; a static label cannot
    /// identify the server binary or configuration that actually produced the answer.
    pub backend: AnalyzerId,
    pub operation: OperationId,
    /// A typed outcome. Empty candidate lists never conflate an external symbol, an unresolved
    /// query, an unsupported operation, and a backend that declined to answer.
    pub resolution: UpgradeResolution,
    /// Per-language-typed data this backend exposed that has no generic shape — never flattened.
    /// See §4.
    pub extension: Option<DaemonExtension>,
}

pub enum UpgradeResolution {
    Resolved(UpgradeCandidate),
    Ambiguous { candidates: AmbiguousUpgradeCandidates, diagnostic: Diagnostic },
    Unresolved { diagnostic: Diagnostic },
    External { symbol: SymbolId },
    Declined { reason: BackendDeclineReason, diagnostic: Diagnostic },
}

/// Construction requires two candidates; additional candidates retain backend order.
pub struct AmbiguousUpgradeCandidates {
    first: UpgradeCandidate,
    second: UpgradeCandidate,
    additional: Vec<UpgradeCandidate>,
}

impl AmbiguousUpgradeCandidates {
    pub fn new(
        first: UpgradeCandidate,
        second: UpgradeCandidate,
        additional: Vec<UpgradeCandidate>,
    ) -> Self {
        Self { first, second, additional }
    }

    pub fn iter(&self) -> impl Iterator<Item = &UpgradeCandidate> {
        [&self.first, &self.second].into_iter().chain(self.additional.iter())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackendDeclineReason {
    ExecutableMissing,
    StartupFailed,
    InitializationFailed,
    UnsupportedCapability,
    Timeout,
    MalformedFrame,
    OversizedFrame,
    TransportClosed,
    BackendCrashed,
    StaleResponse,
    CircuitOpen,
}
```

`upgrade()` returns an outcome, not a bare `CapabilityPrecision`, because per-language backends hand back richer and more varied facts than one enum value can carry:

- Kotlin's `KaCallInfo` is a sealed type — ambiguous resolution is `KaErrorCallInfo { candidateCalls: Vec<KaCall>, diagnostic }`, a real set of candidates plus a reason, not a lower-confidence single answer.
- Go's `gopls` runs two distinct, never-unified algorithms for interface satisfaction — a local, real-`go/types`-assignability check and a global, method-set-fingerprint approximation. It never collapses these into one verdict, and neither should monokl.
- Scala's Metals can hand back a resolved implicit value without correctly explaining why it won — resolved-value precision and reasoning/provenance precision are two independent axes, not one.
- Rust's `ra_ap_hir` and C++'s Clang `RecoveryExpr` surface structured facts (trait predicates and dyn-compatibility violations; preserved partial subexpressions after a semantic error) that are new data, not a confidence signal layered on existing data.

Collapsing any of this into one enum bump would manufacture confidence the upstream tool never claimed.

## 3. Lifetime: a property of each resource, not one field per implementation

`ResourceLifetime` is the axis that decides how expensive a call is allowed to be: `PerCall` (spawn fresh, use once, discard), `PerInvocation` (constructed once, reused across one CLI run), `Session` (held warm across many calls within whatever process embeds monokl for a while).

```rust
pub enum ResourceLifetime {
    PerCall,
    PerInvocation,
    Session(SessionOwnership),
}

pub enum SessionOwnership {
    /// monokl's host process spawns and fully owns this resource.
    Owned,
    /// monokl discovers and attaches to a resource it doesn't control the lifecycle of — must
    /// detect absence, staleness, and version mismatch rather than assume presence.
    Attached,
}
```

Two refinements on top of the three-way split:

**Lifetimes nest.** rust-analyzer's own `proc-macro-srv-cli` is a subprocess loaded once and reused for many expansions with crash-supervision — living inside an otherwise-`PerInvocation` Salsa database build. A flat `lifetime: PerInvocation` on `RustUpgrader` can't express "this upgrader also owns a nested, supervised sub-resource with its own restart lifecycle." C++ needs the same nesting from a different angle: `ClangAstUpgrader{PerCall}` and `ClangdUpgrader{Session}` are two separate trait impls for one language, because a single impl can't honestly carry two lifetimes. An upgrader declares a set of typed sub-resources, each with its own lifetime, rather than being forced into one bucket.

**`Session` distinguishes owned vs. attached.** `gopls`'s real deployment model is `-remote=auto` — discovering and attaching to a daemon that may already be running, spawned by an entirely different process, with its own restart/versioning lifecycle monokl doesn't control. `kotlin-lsp`'s RocksDB-backed on-disk index persistence exists for the same reason: surviving across processes that don't share ownership. "Spawn and own" and "discover, attach, and version-check a resource owned elsewhere" are different failure surfaces — stale daemon, version mismatch, no daemon present, socket permissions — and `SessionOwnership` names both.

**Concurrency contract.** `WorkspaceIndex::build` already parallelizes per-file analysis (`par_iter`). A `Session` resource shared across worker threads via a naive `Arc<Mutex<ChildProcess>>` over one stdio pipe is not just slow but wrong — concurrent writes/reads on one pipe cross-wire which response belongs to which request. rust-analyzer's `GlobalState`/Salsa snapshot model and gopls's Session→View→Snapshot hierarchy (in `golang/tools/gopls/internal/cache` — not in `gopls/doc/design/design.md`, which argues only for a long-lived process with in-memory caching) both solve this the same way: clone `Arc`s into an immutable snapshot per request, never expose raw mutable shared state. Part 8 adopts the same split for monokl's own library API, without cancellation. Contract: every `Session` resource is a `Send + Sync` handle, primed in a single-threaded phase before parallel readers touch it; LSP-speaking backends are wrapped in a connection-actor (one task owns stdio, demultiplexes by request ID — the standard pattern `async-lsp`/`tower-lsp`-style clients already implement). For an in-process Rust `Session` built directly on `ra_ap_ide`'s Salsa `RootDatabase`: any lazy `set_file_text`-style write during parallel analysis bumps Salsa's revision counter and cancels every other in-flight query against that database, so all Salsa inputs load in a single-threaded prime phase before the database is handed to parallel readers, and `PrecisionUpgrader::upgrade` is read-only against the resource.

## 4. Ingestion model: `DaemonExtension`, additive and per-language

`FileAnalysis`'s existing shared-envelope/per-language-variant pattern (`FileAnalysis` → `LangData::{TypeScript, Rust, ...}`) is the right template for daemon-tier data too — SCIP's `SymbolInformation.kind` enumerates dozens of language-tailored variants rather than collapsing to one generic "callable" (a Java method and a Go function get different `kind`s "even if the symbols for these use the same syntax for the descriptor," per SCIP's own spec), LSP's extension model reserves vendor-namespaced methods for anything a specific server needs beyond its generic baseline with an explicit "clients ignore capabilities they don't understand" rule, and CodeQL ships a `shared/` tree of language-agnostic frameworks that each per-language library implements concretely rather than one universal schema. All three reject lowest-common-denominator collapse in favor of typed per-language leaves inside a shared envelope.

Daemon-tier data is still structurally different from what `LangData` already models: it's an optional, resource-gated enrichment pass over an already-complete base result, produced independently of and later than the base analyzer, not a peer data source. The mechanism that fits is LSP's capability-negotiation pattern:

```rust
pub struct FileAnalysis {
    // ...existing fields unchanged...
    pub language_data: LangData,
    /// Present only when a PrecisionUpgrader actually ran and produced a result. Absence is safe
    /// and silent — no upgrader ran, Session wasn't available, or the project was gated by size
    /// (see Java, §8). Never merged into `language_data`.
    pub daemon_extension: Option<DaemonExtension>,
}

pub enum DaemonExtension {
    Rust(RustTraitFacts),
    Go(GoSatisfactionFacts),
    Java(JavaClasspathFacts),
    Kotlin(KotlinSmartCastFacts),
    Scala(ScalaImplicitFacts),
    Cpp(CppRecoveryFacts),
    // one variant per language that ships a PrecisionUpgrader, added as each lands
}

/// gopls never unifies its local (real go/types assignability) and global
/// (method-set-fingerprint) interface-satisfaction algorithms into one verdict — neither should
/// monokl.
pub struct GoSatisfactionFacts {
    pub confirmed: Vec<SymbolRef>,
    pub candidate: Vec<SymbolRef>,
}

/// Metals can supply the resolved implicit value without reliably explaining why it won — these
/// are two independently-scored axes.
pub struct ScalaImplicitFacts {
    pub resolved_value: CapabilityPrecision,
    pub provenance: CapabilityPrecision,
}
```

`CapabilityPrecision` stays exactly what it already is — the base analyzer's own honest self-report — untouched by whether a `daemon_extension` is present. The extension is additive provenance, not a replacement value. `TraitImplIndex`-style per-language indexes remain the natural home for language-specific facts inside each extension variant.

## 5. Attaching confidence after the fact: the `PrecisionLedger`

`UpgradeOutcome` is a runtime return value — what a `PrecisionUpgrader` hands back the moment it's called. Confidence and enrichment shouldn't be tied only to that moment, or to the base analysis's own cache lifecycle. Three cases need support:

- **Eager**: a `PrecisionUpgrader` runs inline during `analyze_with_profile`, as §2 describes.
- **Lazy**: a query (`refs`/`definition`) needs higher precision than what's cached, and a backend is available now even though it wasn't at initial analysis time (a `Session` daemon started since, Clang got installed since) — enrichment happens at query time, without re-running or invalidating the base `FileAnalysis`.
- **Retroactive**: a separate pass (a future `monokl enrich` step, or an agent choosing to spend a turn on it) sweeps already-cached files and attaches confidence from whatever backends are available, decoupled from when those files were originally analyzed.

All three need confidence to be addressable and separately cacheable from `FileAnalysis` itself. Baking it into `FileAnalysis` would mean every new confidence signal — a `Session` becoming available, a new sentinel rule, a different upgrader version — forces a rebuild of the base analysis blob, and conflates two different invalidation lifecycles: `FileAnalysis` invalidates on content change; confidence annotations invalidate on backend availability or version change, which can happen without the file ever changing.

The `PrecisionLedger`, keyed by `ContentHash`, stores small, addressable annotations — never a copy of the AST — mirroring LSP's diagnostics model (`{range, severity, message, source}` tied to a document, decoupled from the document's own content) and SCIP's `Occurrence` model (`{range, symbol, symbol_roles}`, a thin annotation layer over source):

```rust
pub struct PrecisionAnnotation {
    /// Addressable pointer, not a copy — a span, a symbol identity, or an occurrence identity.
    pub target: AnnotationTarget,
    pub precision: CapabilityPrecision,
    /// "Exact via JDT full resolution" / "Structural via clangd, no compile_commands.json" — same
    /// provenance discipline as UpgradeOutcome (§2).
    pub backend: AnalyzerId,
    pub operation: OperationId,
    /// Which sentinel (if any) triggered a downgrade instead of a silent Exact claim — §6.
    pub sentinel: Option<SentinelKind>,
    /// The exact content this annotation applies to. A ContentHash mismatch means "stale,
    /// discard" with no further comparison needed — same invalidation key FileAnalysis's own disk
    /// cache already uses, just a separate store.
    pub for_content: ContentHash,
}

pub enum AnnotationTarget {
    Span { start: usize, end: usize },
    Symbol(SymbolId),
    Occurrence(OccurrenceId),
}

/// Separately persisted from FileAnalysis's own cache entry (its own section of
/// `.monokl/cache.json`, or its own file) — populated eagerly, lazily, or retroactively, on its
/// own invalidation schedule.
pub struct PrecisionLedger {
    by_content: FxHashMap<ContentHash, Vec<PrecisionAnnotation>>,
}
```

An annotation is a handful of small fields keyed by a pointer, never the subtree or source text it applies to — a consumer resolves `target` against data it already has (the base `FileAnalysis` it already fetched), rather than the annotation carrying a redundant copy. This matters for the existing token-budget-aware output stage (Part 1, `budget.rs`, tiktoken o200k_base, 2MB cap): a compact `provenance` tag travels with a result by default; the full reasoning behind it (which candidates were considered, why a sentinel fired) is available on demand through the `explain` command Part 2 already specifies, not inlined into every response. Default output stays terse; depth is opt-in.

This requires no change to §2's `PrecisionUpgrader` trait or §4's `DaemonExtension` — `UpgradeOutcome` is what gets turned into one or more `PrecisionAnnotation` entries when a ledger write happens, whether that write is eager, lazy, or retroactive. The ledger is the addressing/storage layer underneath, not a competing design.

## 6. Confidence-signaling rules

Neither LSP nor SCIP has a confidence field: `Location`/`LocationLink` carry only position; `ResponseError` carries only `code`/`message`/`data`; SCIP's `Relationship`/`Occurrence` messages carry none either. A server that resolved unambiguously and a server that guessed produce identical response shapes. The one incidental signal LSP carries: `textDocument/definition`/`textDocument/references` return `Location | Location[] | LocationLink[] | null` — cardinality greater than one for a query implying a single binding is a real ambiguity signal, though most clients discard it by taking `result[0]`.

The compilers themselves often know more than the protocol carries. Three real, queryable internal signals never reach a plain LSP response:

- **Clang**: `RecoveryExpr` is an AST node Clang emits when it can't resolve an overload, call, or type — a placeholder with no language semantics attached. `textDocument/hover` doesn't say so; clangd emits whatever type string it has. The signal survives only through clangd's non-standard `textDocument/ast` extension (`ASTNode.kind`/`role`/`arcana`), not plain LSP.
- **Eclipse JDT**: `IBinding.isRecovered()` is set whenever classpath gaps or unresolved types force JDT to fabricate a placeholder binding. It exists on JDT's Core DOM API; `jdtls`'s LSP surface doesn't expose it per-symbol — the only user-facing signal is a project-level "classpath is incomplete" warning, too coarse to say which specific reference degraded.
- **TypeScript**: the checker internally distinguishes a genuine `any` from an unresolved/error type, but the language service collapses both to the literal string `"any"` in hover output. The one honest signal — implicit-any suggestion diagnostics — lives in a separate, opt-in diagnostic category, requiring deliberate correlation by span.

Each resolver has an internal escape-hatch representation for "I couldn't resolve this" — a dependent/recovery type, a recovered-binding flag, a platform-type marker, a silently-first-picked candidate — and in every case that escape hatch is a small, enumerable, tool-specific sentinel, not a diagnostic that propagates through LSP on its own. Detection doesn't need per-language semantic understanding; it needs a per-backend denylist of sentinel shapes.

Rules, required before any `PrecisionUpgrader` promotes a result toward `Exact`:

1. Protocol silence proves nothing. A missing `ResponseError` or absent diagnostic doesn't mean the result is trustworthy — LSP has no wire-level field for "resolved to a degraded fallback."
2. Prefer native/extended APIs that expose the backend's own confidence bit over plain LSP: JDT `IBinding.isRecovered()`, Kotlin's `KaErrorCallInfo` sealed type, clangd's `textDocument/ast` extension, TS's separate opt-in suggestion-diagnostics channel.
3. Maintain a per-backend sentinel denylist, checked on every promotion attempt: JDT `isRecovered() == true`; Clang dependent/recovery type strings or diagnostics at or upstream of the queried span; TS hover text of bare `any` not matching an explicit source annotation; Kotlin `T!` platform-type markers or `SMARTCAST_IMPOSSIBLE`; Scala ambiguous-implicit compiler notices; Go's global-algorithm-only matches (cap those at `candidate`, never `confirmed` — see `GoSatisfactionFacts`, §4).
4. When only generic LSP is available and no sentinel exists, cap at `Structural` if the file has any diagnostic at or upstream of the queried location, and emit a monokl `Diagnostic` explaining the downgrade rather than guessing.
5. Tag every upgraded result with structured backend + operation identity — analyzer name, runtime version, configuration digest, operation name, and normalized-parameter digest — never an undifferentiated `Exact` bucket or display string. An analysis tier is carried as metadata and never erased, so a consumer can reason differently about a result from one backend/operation than another. `UpgradeOutcome.backend` and `.operation` (§2) are this identity.

## 7. Computing precision: sourcing, the downgrade lattice, and long-term feedback

§5 and §6 establish where confidence lives and how to detect degradation. This section covers what actually produces a `PrecisionAnnotation`'s value.

**Sourcing.** From the AST/parse itself, at analysis time: the base categorical tier (`Heuristic`/`Structural`/`Exact`) a `LanguageAnalyzer` or `PrecisionUpgrader` reports, and candidate-set size — one resolved candidate vs. N ambiguous ones is a structural fact out of the symbol table, not an inference. After the fact, at query time or later: sentinel-detected degradation (§6), cross-tier agreement (comparing two independently-derived answers for the same fact — a native parser's heuristic guess and a daemon's real resolution), and, optionally, a slow-moving reliability signal per `(backend, operation)` pair gathered across many runs (see below). This is why §5 designed the ledger as separately addressable and invalidated from `FileAnalysis` — none of these after-the-fact signals should require re-running the base analysis to attach.

**Not a numeric score.** Every ML-based type/fact-inference system worth citing here — DeepTyper, TypeWriter, Type4Py, HiTyper, OptTyper — either skips calibration analysis or, against its own published numbers, shows a gap between stated confidence and actual accuracy (DeepTyper's own results: a 90%-confidence threshold yields only ~80% true precision). Every one of them still pairs its statistical signal with an independent symbolic validator rather than trusting the model's raw confidence — TypeWriter round-trips candidates through a real type checker, HiTyper uses static rejection rules to veto neural predictions. That validator-gates-model pattern is what §6's sentinel design already is. Separately, agreement between independent static analyzers is empirically rare even among analyzers that are each individually correct (Habib & Pradel, "How Many of All Bugs Do We Find?", ASE 2018; Lenarduzzi et al., "A Critical Comparison on Six Static Analysis Tools," JSS 2023) — cross-tier agreement is real corroboration when observed, but its absence is the normal case, not a red flag.

Shipped production tools converge on the same categorical answer. CodeQL's `@precision` tag takes four values — `low`/`medium`/`high`/`very-high` — set once by the query author, feeding display only, never a per-finding computation. Semgrep's rule metadata has a `confidence` field (`HIGH`/`MEDIUM`/`LOW`) used to curate which rules run in CI-blocking vs. audit-only mode — rule-level, not per-instance. Google's Tricorder is the fully-documented production feedback mechanism (Sadowski et al., "Lessons from Building Static Analysis Tools at Google," CACM 2018): every finding carries a "Please fix"/"Not useful" button, the ratio is tracked per analyzer, and an analyzer crossing a 10% not-useful rate is disabled until its authors improve it — categorical, slow-moving, tuned at the analyzer/rule level, never a live per-instance score.

`PrecisionAnnotation.precision` is computed by starting from the categorical tier the producing analyzer/upgrader already reports, then applying secondary signals that can only move it down, never up:

```rust
fn compute_precision(
    reported: CapabilityPrecision,
    resolution: &UpgradeResolution,
    sentinel: Option<SentinelKind>,
    cross_tier_agreement: bool,
) -> CapabilityPrecision {
    let mut precision = reported;
    if matches!(resolution, UpgradeResolution::Ambiguous { .. }) {
        precision = precision.saturating_downgrade();
    }
    if sentinel.is_some() { precision = precision.saturating_downgrade(); }
    // Real corroboration when present, per the agreement-is-rare finding above this counteracts
    // at most one downgrade step — never grounds to exceed `reported`.
    if cross_tier_agreement && precision < reported {
        precision = precision.saturating_upgrade().min(reported);
    }
    precision
}
```

No numeric probability anywhere — the output stays one of the same four `CapabilityPrecision` values already shown to agents, so nothing about existing output formats or the token-budget discipline changes. This is a small, deterministic, unit-testable function; no training data, no model, no new dependency (no viable Rust crate exists for combining uncertain signals generally — checked directly). The closest precedent for this class of combination, done for program analysis specifically rather than borrowed from an unrelated ML domain, is Kremenek & Engler's Z-Ranking (SAS 2003) and its follow-up "Correlation Exploitation in Error Ranking" (Kremenek, Ashcraft, Yang, Engler, FSE 2004): statistical combination of correlated analysis signals into a ranking, over twenty years old, no deep learning required.

**Optional, not required for v1.** N-gram "naturalness" scoring (Hindle, Barr, Gabel, Su, Devanbu, "On the Naturalness of Software," ICSE 2012/CACM 2016) is a lightweight additional signal — corpus frequency counting, no training pipeline — and its lineage (Ray et al., "On the 'Naturalness' of Buggy Code," ICSE 2016) shows buggy code measurably scores less "natural" than clean code. If wanted later, this feeds `compute_precision` as one more downgrade signal, computed over data monokl already indexes, with no dependency on any daemon.

**Optional long-term feedback loop**, modeled on Tricorder: track a reliability ratio per `(backend, operation)` pair across many runs, and let a combination that proves chronically unreliable get systematically down-weighted. This requires a feedback signal monokl doesn't have designed yet — plausibly an agent or user later confirming or correcting a specific annotation, with no code-review UI to attach a button to the way Tricorder does — and isn't part of the v1 design. It's recorded because the `(backend, operation)`-keyed granularity already matches `PrecisionAnnotation.provenance` (§5) exactly, so no new identifier scheme would be needed if it's ever built.

## 8. Per-language lifetime table

Real backend, real lifetime fit, and what's language-specific enough that it must not be flattened into the shared `DaemonExtension` shape.

| Language | Real backend | Lifetime | What must not be flattened |
|---|---|---|---|
| TypeScript/JS | tsserver / `typescript-go` (TS 7.0's LSP server, GA July 8, 2026) | `PerCall` unusable — cost is whole-program graph construction, not per-query; `PerInvocation` a viable but costly stopgap; `Session` is the tool's native operating mode (VS Code never runs it any other way) | Genuine `any` vs. gave-up-and-defaulted-to-`any` — invisible unless the separate opt-in suggestion-diagnostics channel is correlated by span |
| Rust | `ra_ap_hir` / Salsa | `PerInvocation` (a fresh Salsa DB still memoizes across one run — rust-analyzer's own `analysis-stats` subcommand confirms this) as near-term default; `Session` matches the real `GlobalState`/VFS/snapshot design; a nested `PerCall`/`Session` proc-macro-server sub-resource lives inside either | Trait predicates, dyn-compatibility violations (`PredicateEvaluationResult`/`DynCompatibilityViolation`), generic monomorphization — trait-solver output, not string/AST matching |
| Python | Pyrefly / `ty` | Both are Rust-*implemented* but subprocess/LSP-*integrated* — neither ships an embeddable Rust library API today, so "Rust-native" doesn't mean "in-process." `PerInvocation` works; `Session` is where their Salsa/incremental advantage pays off | The categorical `Any`/`Unknown` distinction (gradual typing's own uncertainty marker) — map directly, don't invent a scalar confidence score no upstream tool provides |
| Go | `gopls` | `PerCall` untenable (cold start documented at 10s–3m26s+); `PerInvocation` only marginal; `Session, Attached` — a thin forwarder to gopls's own `-remote=auto` daemon, not monokl-owned | Confirmed (local, real `go/types` assignability) vs. candidate (global method-set-fingerprint) interface satisfaction — gopls itself never unifies these, see `GoSatisfactionFacts` (§4) |
| Java | `jdtls` / JDT Core | `PerCall` non-viable; `PerInvocation` fragile — batched Maven import OOM-crashes past ~3000 files, a hard crash, not a graceful slowdown; `Session` is the only lifetime that amortizes both cold-start and import cost, and needs an explicit project-size gate before attempting promotion at all | `IBinding.isRecovered()` — a queryable "this is a fabricated placeholder" flag on JDT's Core API, not exposed through `jdtls`'s LSP surface |
| Kotlin | `kotlin-lsp` (full IntelliJ, Alpha) or standalone Analysis API (unstable per JetBrains' own docs) | `PerCall` ruled out (GB-scale allocation, seconds-to-minutes even for a single init); `Session` with a recycling/health contract — bounded by staleness or memory budget, since unbounded warm sessions leak per JetBrains' own tracker | `KaErrorCallInfo{candidateCalls, diagnostic}` ambiguity, `SMARTCAST_IMPOSSIBLE` diagnostics, flexible/platform types (`T!`) — sealed/typed signals, map to `candidates`/extension fields, never a binary |
| Scala | Metals (`mtags` + Build Server Protocol + presentation compiler) | `PerCall`/`PerInvocation` both repay a 10s–15min build import; `Session`-only in practice. BSP is fully internal to Metals — not a second protocol monokl needs to speak | Implicit argument/conversion resolution as two independently-scored axes (resolved value vs. provenance) — see `ScalaImplicitFacts` (§4); Metals can be confidently wrong about why, not just silent |
| C/C++ | Clang subprocess (`clang-ast`) and clangd, as two separate `PrecisionUpgrader` impls | `PerCall`: `clang-ast`, cheap per translation unit; `Session`: clangd's background index, hours-scale cold start on large projects, only viable warm | `RecoveryExpr`'s preserved partial subexpressions/dependent types — carry as `CppRecoveryFacts` extension, never collapsed into a precision bump |
| C# | Roslyn (OmniSharp / `csharp-ls` / Microsoft's own server) | `PerInvocation`/`Session`, standard workspace-load-then-reuse | Verify Microsoft's official server's redistribution/licensing terms outside VS Code before depending on it |
| Ruby | Sorbet (near-zero fixed cold start via a compiled-in stdlib blob) or Solargraph | `PerCall` is viable for Sorbet — the one backend where that's true | The fixed-cost floor is uniquely cheap; project-size-proportional check time still applies on top of it |
| PHP | Intelephense | `PerInvocation`/`Session`, standard shape | Licensing (proprietary premium tier), not architecture, is the real constraint |

## 9. LSP backend lifecycle contract

An LSP-speaking upgrader is optional infrastructure, so its failure must reduce precision rather than take the base analyzer down. Every implementation must satisfy this lifecycle:

1. **Initialize and negotiate.** Record the backend executable identity and version, send `initialize`/`initialized`, and retain the negotiated capability set. An operation is available only when the server advertised the corresponding capability; a process accepting bytes is not proof that it can answer the request.
2. **Bind answers to content.** Document versions arrive through `didOpen`/`didChange`; ordinary definition/reference/hover requests carry a URI and position, not a version. When dispatching a request, the connection actor records the locally synchronized document version and source `ContentHash` in its pending-request context. Before publishing the response, it compares that context with the snapshot that will receive it. A response for older content is discarded as `Declined { reason: BackendDeclineReason::StaleResponse, diagnostic }`, never attached to newer content.
3. **Bound both protocol and work.** `LspBackendLimits` configures initialization deadline, per-request deadline, maximum frame bytes, restart backoff sequence, maximum restarts per window, and circuit-open interval. Missing executable, startup/initialization failure, unsupported capability, timeout, malformed/oversized framing, transport closure, crash, stale response, and an open restart circuit return `Declined` with the corresponding stable `BackendDeclineReason` plus a human-readable diagnostic, preserving the base structural result. A valid backend response of `null`/no location returns `Unresolved`; a location outside the indexed workspace returns `External`; one in-scope location returns `Resolved`; multiple viable locations return `Ambiguous` with all candidates.
4. **Supervise owned processes.** Graceful shutdown sends `shutdown`, then `exit`, then reaps the child. Unexpected exit may restart with bounded exponential backoff. Repeated failure opens a circuit for that `(workspace, backend)` so a query cannot create a restart storm; an explicit health probe or backoff expiry is required before retry.
5. **Isolate polyglot failure.** One language backend failing initialization, timing out, or crashing does not prevent other backends or the base analyzers from serving the snapshot.
6. **Publish health without source.** Lifecycle telemetry may include backend/version, language, capability names, cold/warm state, latency, restart count, requested/served precision, and degradation reason. It must not include source text or absolute paths.

```rust
use std::time::Duration;

pub struct LspBackendLimits {
    pub initialize_timeout: Duration,
    pub request_timeout: Duration,
    pub max_frame_bytes: usize,
    pub restart_backoff: Vec<Duration>,
    pub max_restarts: u32,
    pub restart_window: Duration,
    pub circuit_open_for: Duration,
}
```

The connection actor owns these limits and the pending-request table. Each
pending request stores the synchronized document version and `ContentHash`
captured at dispatch. The server does not echo document versions on ordinary
definition/reference/hover responses, so freshness is established by comparing
that stored request context with the receiving snapshot, not by reading a
version from the response.

Acceptance tests inject small `LspBackendLimits` values and cover unsupported capability, startup failure, initialization timeout, request timeout, malformed/oversized frame, response dispatched against a stale synchronized document version/hash, `null` response, external location, multiple candidates, crash/restart, restart-circuit opening, create/delete/rename notifications, partial polyglot failure, and graceful shutdown/reaping. Each decline fixture asserts its exact `BackendDeclineReason`, not diagnostic message text. Transport-crate selection remains deferred until a spike verifies these behaviors, MSRV, runtime coupling, and cancellation semantics; `lsp-types` supplies protocol DTOs but not lifecycle management.

## 10. Known gaps

- No published data exists on what fraction of Java symbol references are external-JAR-only; a bytecode-LOC statistic found during research measures a different thing and shouldn't be repurposed as a reference-density estimate.
- Microsoft's official C# language server's redistribution/licensing terms outside VS Code are unconfirmed.
- `ra_ap_hir`'s public API surfaces no explicit "multiple equally-valid candidates" variant for trait-method ambiguity, unlike Kotlin's `KaErrorCallInfo` — possibly a real gap in `ra_ap_hir` itself.
- TS 7.0's promised embeddable programmatic API (targeted for 7.1) doesn't exist yet — integration must go through LSP JSON-RPC only until it does.
- Whether `SessionOwnership`'s owned-vs-attached split needs a full enum variant or could stay a lighter metadata flag on `Session` hasn't been stress-tested against an actual implementation.
- Facebook/Meta's Infer/Zoncolan deployment is widely reported to use an engineer accept/dismiss feedback loop analogous to Google Tricorder's; the paper that likely describes it (Distefano, Fähndrich, Logozzo, O'Hearn, "Scaling Static Analyses at Facebook," CACM 2019) is real, but its full text wasn't accessible during research, so the specific mechanism is unconfirmed. Only Tricorder's mechanism (§7) is verified against full text.
- §7's optional long-term feedback loop has no defined input signal — it names the mechanism (Tricorder-style, keyed on `(backend, operation)`) but not where a "this was wrong" signal would come from for a CLI tool with no code-review UI to attach a button to.

# Code-intelligence ecosystem: techniques, crates, and project fit

This report records what to borrow from ProjectAtlas, OpenWiki, Repowise, Serena, sem, and related code-intelligence projects. It distinguishes reusable techniques from product overlap and gives every dependency an explicit disposition. The machine-readable evidence summary is [`CODE-INTELLIGENCE-ECOSYSTEM-001.json`](./CODE-INTELLIGENCE-ECOSYSTEM-001.json).

## The useful layer model

| Layer | Question | Appropriate mechanism | Owning project |
| :--- | :--- | :--- | :--- |
| Syntax capture | What declarations, imports, and candidate occurrences are present? | OXC or recovery-oriented parser ASTs; tree-sitter only for fallback languages | Monokl |
| Semantic resolution | What does this occurrence bind to? | Native semantic APIs, compiler/type-checker oracles, optional SCIP, or supervised LSP sessions | Monokl |
| Derived graph | What depends on what, with which scope and precision? | Typed edges, retained candidates, immutable snapshots, complete publication | Monokl; Wisp stores only derived joins |
| Context compilation | Which verified facts and artifacts belong in a bounded briefing? | Explicit links, current code facts, typed weak signals, explainable ranking and truncation | Wisp |
| Durable knowledge | Which claims and documents remain valid as evidence changes? | Canonical Git artifacts, content digests, provenance, stale-claim reconciliation | Wisp; OpenWiki is the closest external analogue |
| Evaluation | Is capture, resolution, ranking, or agent outcome actually better? | Independent oracles, metamorphic tests, paired trials, basis-labelled measurements | Prism and Lumen |
| Rendering | How is typed output made compact without changing its meaning? | Michi sections, budget/provenance blocks, differential format tests | Michi |

This split prevents two recurring errors: treating a parser match as a resolved fact, and turning a rebuildable retrieval store into project truth.

## Project and technique comparison

| Source | Useful pattern | Strengths | Costs or traps | Disposition |
| :--- | :--- | :--- | :--- | :--- |
| [ProjectAtlas](https://github.com/styler-ai/ProjectAtlas) | Purpose metadata, typed graph resolution, complete publication generations, measured-vs-modeled telemetry | Strong graph completeness and provenance discipline; explicit ambiguity; useful context-product ergonomics | Persistent SQLite/FTS architecture overlaps Wisp and conflicts with Monokl's rejected persistent search index; internal workspace crates are not a stable published dependency surface | Borrow contracts and tests, not storage architecture or internal crates |
| [OpenWiki](https://github.com/langchain-ai/openwiki) | Grounded Claims, evidence-version invalidation, durable page jobs, deterministic finalization, agent-authored linked docs | Closest analogue to Wisp's durable knowledge lifecycle; good stale-claim and partial-progress semantics | Generates a wiki rather than compiling task context; model-authored prose is heavier and less deterministic than Wisp's typed artifacts | Study for Wisp claim lifecycle and future documentation projection; do not merge it into Monokl |
| [Repowise](https://github.com/repowise-dev/repowise) | Capture/resolution separation, per-edge origin, compiler-oracle benchmarks, incremental reconciliation, Git analytics | Best evaluation methodology of the surveyed projects; changelog exposes real incremental failure classes | Broad product surface; history-derived signals can be mistaken for dependency truth; Python implementation is not a Rust reuse path | Adopt methodology and invariants; do not adopt as a library |
| [Serena](https://github.com/oraios/serena) | Polyglot LSP manager, capability-based tools, backend health, file notification, semantic edits | Concrete operational lessons for warm semantic backends and graceful feature exposure | Python/solidlsp stack is not a Monokl dependency; startup currently couples requested servers; restart policy is not a complete circuit breaker | Borrow lifecycle fixtures and tool taxonomy; implement a narrow Rust boundary only after evaluation |
| [sem](https://github.com/Ataraxy-Labs/sem) / [`sem-core`](https://docs.rs/sem-core/0.24.0/sem_core/) | Multi-language structural analysis and incremental parity probe | Useful independent implementation for differential testing; published Rust crate | Defaults enable a broad parser/storage stack; OXC version can diverge from Monokl; linking it would duplicate architecture | Dev-only oracle with minimal features, never production core |
| Codanna and similar all-in-one indexes | Integrated parse, search, embeddings, and index | Fast way to prototype an end-to-end comparison arm | Pulls in tree-sitter, Tantivy/embedding and persistent-index choices Monokl explicitly rejected | External benchmark arm only; reject as a production dependency |

## Reusable techniques

| Technique | Benefit | Failure mode it prevents | Cost | Where to apply |
| :--- | :--- | :--- | :--- | :--- |
| Separate capture from resolution | Honest recall and precision accounting | Regex/AST matches reported as semantic references | One extra origin/status field and two-stage evaluation | Monokl facts; Wisp projection; Prism metrics |
| Preserve `resolved`, `ambiguous`, `unresolved`, and `external` | Keeps uncertainty actionable and debuggable | First-candidate wins and missing-edge ambiguity | Larger DTOs and explicit consumer handling | Monokl and Wisp typed results |
| Structured backend provenance | Makes runtime server/version/config differences visible | A display label such as “exact via LSP” hiding incompatible backends | Digest/version bookkeeping | Monokl, Wisp, Lumen |
| Complete publication boundary | Readers see old-complete or new-complete state | Mixed generations across caches/tables | Off-side build and atomic swap/transaction; generations only for staged publication | Monokl snapshots; Wisp and Lumen stores; oxc-react-codegen watch state |
| Incremental equals clean rebuild | Tests correctness rather than speed alone | Create/delete/rename or parser-version paths learning less than full indexing | Generated sequences and canonical comparators | Monokl, Wisp, Lumen adapters, oxc-react-codegen |
| Independent compiler oracle | Distinguishes analyzer error from benchmark self-agreement | Using the system under test as ground truth | Toolchain pinning; some facts remain unjudged | Prism with TypeScript checker and Go RTA lanes |
| LSP capability/lifecycle contract | Makes optional precision reliable | Stale replies, unsupported calls, orphaned children, restart storms | Warm process, timeouts, supervision, telemetry | Monokl optional precision backends |
| Weak-signal isolation | Allows history signals without corrupting dependency semantics | Co-change becoming an import edge or PageRank input | Separate feature provenance and ablation | Wisp ranking; Prism evaluation |
| Measurement basis | Keeps savings and confidence claims honest | Modeled avoidance presented as observed billing reduction | Extra enum, baseline fingerprint, denominator metadata | Lumen first; Michi only after cross-tool need is proven |
| Post-edit diagnostics at coherent boundaries | Fast feedback after semantic edits | Shipping edits that leave compiler/LSP errors | Diagnostics can be transient between related edits | Agent workflows at subsystem/batch boundaries, not after every mutation |

## Rust crates and dependency dispositions

Published-version checks below are point-in-time observations from 2026-09-11, not permissive version requirements. Crates.io searches found no `projectatlas`/`projectatlas-core` release; Atlas's in-repo crates are versioned together and referenced through workspace paths, so they are source precedent rather than a stable registry dependency. Repowise and Serena are Python applications rather than Rust crate libraries; neither supplies a Rust crate to link into this suite.

| Crate | Role | Pros | Cons / risks | Disposition |
| :--- | :--- | :--- | :--- | :--- |
| `oxc_parser`, `oxc_ast`, `oxc_semantic`, `oxc_resolver` | TS/JS parse, local binding, module resolution | Fast native Rust stack; matches Monokl's selected architecture; `oxc_semantic` closes local shadowing/hoisting gaps | Versions must remain lockstep; AST nodes are arena-borrowed and must never escape analysis | Adopt in Monokl; keep owned output boundary |
| `ra_ap_syntax` | Rust recovery-oriented syntax | Parses broken/WIP Rust where `syn` fails fast | Larger compiler-derived dependency and no full HIR semantics | Keep for Monokl structural Rust tier |
| tree-sitter language crates | Generic fallback capture | Broad language coverage and good error recovery | Grammar queries are not name resolution; TS/JS path was explicitly rejected; incremental benefit exists only if old trees are retained | Use only for named fallback languages and benchmark cold cost honestly |
| [`lsp-types`](https://docs.rs/lsp-types/0.97.0/lsp_types/) 0.97.0 | LSP wire DTOs | Mature protocol types; no need to hand-roll messages | No transport, process ownership, framing, timeouts, or restart policy | Adopt when real methods ship, outside `monokl-core` |
| [`async-lsp`](https://docs.rs/async-lsp/0.2.4/async_lsp/) 0.2.4 | Async LSP client/server transport | Tower middleware, client support, concurrency and lifecycle building blocks | Runtime/cancellation/MSRV/frame-limit fit is unverified | Time-boxed spike; no adoption until fault fixtures pass |
| [`scip`](https://docs.rs/scip/0.9.0/scip/) 0.9.0 | Optional index interchange | Standard symbol/occurrence format and external precomputed precision source | Protobuf weight; no per-edge confidence; not designed as a query database | Optional import/export feature, never authority |
| [`sem-core`](https://docs.rs/sem-core/0.24.0/sem_core/) 0.24.0 | Differential structural oracle | Published, multi-language, ready-made independent behavior | Default features are broad; duplicates parsers/index; OXC skew | Dev dependency or external process with defaults disabled |
| [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) 3.x | MCP transport and schemas | Official SDK, client/server coverage, structured tools | Response caching can serve stale data if configured permissively; framework churn | Use at Wisp/agent transport boundary only; disable stale-on-error for authoritative reads |
| `notify` 8.2.x | Filesystem change hints | Cross-platform watcher abstraction | Events coalesce, duplicate, reorder, and can be lost; never a correctness log | Hint only; reconcile against filesystem and content digests |
| `arc-swap` 1.9.x | Lock-light snapshot publication | Cheap atomic pointer publication to immutable state | `rcu` returns the pre-swap value; publishing related maps separately creates mixed revisions | Publish one aggregate revision; test returned value and content |
| `rusqlite` | Rebuildable derived storage | Transactions, WAL, mature SQLite boundary | Multiple transactions need explicit complete-generation visibility; caches can be mistaken for authority | Wisp/Lumen derived stores only; prefer one transaction |
| `gix` | Git evidence without subprocess fan-out | Rich in-process Git access and typed object model | Larger API/dependency surface and licensing/reuse review needed | Evaluate in Wisp against measured subprocess costs |
| `toon-format` 0.5.x | External TOON compatibility oracle | Independent decoder/encoder and adversarial corpus source | Production dependency would violate Michi's zero-dependency default and may expose dialect differences | Dev-only differential test in Michi or Prism |
| `petgraph` | General graph algorithms | Mature SCC/path algorithms | Extra indirection/dependencies; unnecessary for adjacency-list queries without those algorithms | Use in Prism/Lumen where SCC is real; reject for Monokl's current import graph |
| Tantivy / embedding crates | Persistent lexical/vector retrieval | Powerful search and semantic retrieval | Duplicate index, memory/build cost, privacy/model versioning; conflicts with explicit Monokl non-goals | Optional Wisp prose retrieval only after evaluation; reject in Monokl |

## Project-specific adoption map

| Project | Adopt now in docs/specs | Spike or evaluate | Explicitly avoid |
| :--- | :--- | :--- | :--- |
| Monokl | Capture/resolution split, typed outcomes, structured backend identity, LSP lifecycle, atomic snapshot publication, incremental parity, compiler-oracle metrics | `async-lsp`, external `sem-core`, optional SCIP ingestion | Persistent search index, Atlas internals, Codanna stack, LSP inside core |
| Wisp | Preserve Monokl outcomes/candidates, complete derived-state publication, incremental parity, weak-signal provenance | Git co-change/ownership with Prism ablation | Second parser/index, history as dependency truth |
| Lumen | Observed/derived/modeled taxonomy, event invocation identity, backend lifecycle telemetry, atomic reingestion | Per-backend reliability metrics after enough samples | Treating modeled savings or transcript findings as project truth |
| Prism | Code-fact oracle artifacts, capture/resolution metrics, metamorphic incremental suites, LSP fault fixtures | TypeScript checker and Go RTA generators, manual gold corpus | Self-produced ground truth or pooled metrics that hide unjudged cases |
| Michi | External TOON interoperability plan and an open measurement-basis decision | Dev-only `toon-format` differential tests | Domain resolution logic or an unproven measurement API |
| oxc-react-codegen | One-revision watch publication, incremental/full equivalence, direct compiler-oracle lane | Programmatic Corsa versus optional warm semantic backend | Process ownership in core, conflating outward LSP server with inward client |
| Agent plugins | Freshness-aware LSP evidence guidance at coherent verification boundaries | A future capability-schema version only when a real workflow needs it | Diagnostics after every mutation; vendor-specific tool names |
| Callisto | No change | None from this research | Persistent incremental machinery for its fresh-per-command graph |

## Decision gates

1. A new parser or resolver must improve a measured fact class and state its ceiling; language count alone is not value.
2. A language-server transport must pass startup, capability, stale-document, timeout, crash/restart, restart-storm, polyglot-partial-failure, and shutdown fixtures before becoming a dependency.
3. Every incremental implementation must compare canonical output with a clean rebuild, not merely exercise an update code path.
4. Every benchmark must name its oracle, input digest, toolchain/version, unjudged bucket, and measurement basis.
5. A cross-project primitive enters Michi or `wisp-contracts` only when the consumers must agree on meaning; computation stays with the owning domain.
6. A derived store is disposable and rebuildable. If one transaction cannot publish a refresh, readers need an explicit last-complete generation.

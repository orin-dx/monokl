# Active Work — monokl

## Completed

### SPEC-004 correction: `rust-version`/`license` fixed for michi adoption

Post-gate, post-implementation correction via `canon/correct`, triggered by the michi
integration research below. Two of AC-020's nine locked `[workspace.package]` values were
wrong: `license = "MIT"` (the repo's actual `LICENSE` file was already AGPL-3.0 — an
internal inconsistency in the source doc, unrelated to michi) and `rust-version = "1.85"`
(michi, now a confirmed dependency, needs 1.96). Corrected to `AGPL-3.0-or-later` / `1.96`,
an intentional, explicitly-flagged divergence from `docs/spec/01-core-architecture.md`'s
own literal text — canon-verifier confirmed the source document genuinely still says the
old values, so this isn't silently drifting from spec, it's a stated, gated exception.
canon-auditor caught the correction's own blast radius: the `scope` field's blanket "all
verbatim per Section 28" claim wasn't updated alongside AC-020, so it briefly contradicted
the criterion it was supposed to summarize — fixed. `vector-planner` amended `PLAN-004`
(only T-001 owns AC-020) and, checking for incidental damage, found two more:
`T-006`/`T-023` had hardcoded `rust_version=='1.85'`/`license=='MIT'` assertions baked into
otherwise-unrelated `cargo metadata` checks — not their own criteria, but would have
produced false failures on any future re-run. Fixed both. `vector-challenger`'s review of
the amended T-001 found real shell bugs of its own: the primary "nothing else moved" diff
was guaranteed to fail (comparing against a 14-line expectation when the real file already
had 67 lines of later tasks' content committed), its offered fallback was an elided `...`
wrapped around a `head | diff -` construct that starves `diff` of a second operand, and a
dangling reference told the implementer to fall back to "T-001's original pre-amendment
steps" — which no longer existed in the plan at all. All fixed, each fix independently
executed against the real repo before trusting it. `lambda-implementer` re-ran the
corrected T-001 clean — commit `414225b`.

### SPEC-004: IMPLEMENTED — the Rust crate now actually exists

First track to move past the spec@1 stage into real code. `vector-planner` produced a 23-task
plan@1 (`PLAN-004.json`); two independent `vector-challenger` rounds found and fixed 13 issues
total (three tasks whose "red" phase could never actually be red, which would have halted
`lambda-implementer` outright; a verification gap where the plan's biggest blocks of transcribed
content — the 10 later dependency version pins and all of `lib.rs` — were never diffed against
the source document, only counted; a BSD `wc -l` padding bug in a length guard my own fix
introduced; a broken backtick-in-markdown assertion; a missing cwd anchor). Third round: clean
pass.

`lambda-recon` → 23 `lambda-implementer` task dispatches (one per task, strict no-batching) →
`lambda-mutator` → `lambda-exit-gate`, all in sequence. Root `Cargo.toml`, `crates/monokl/Cargo.toml`,
and `crates/monokl/src/lib.rs` now exist and are byte-exact against `docs/spec/01-core-architecture.md`,
confirmed via pattern-anchored diffs (not hardcoded line ranges) with working length guards.
23 commits (16 file-changing, 7 deliberate `--allow-empty` for behavioral-only verification
tasks), all 28 acceptance criteria evidenced, `lambda-exit-gate` independently re-ran the
load-bearing checks against live `cargo 1.96.0` rather than trusting task reports. `lambda-mutator`
confirmed zero mutable code paths (expected — `lib.rs` is bare module declarations, no function
bodies; both `Cargo.toml`s are non-code TOML) by isolating `lib.rs` into a synthetic crate and
running `cargo mutants --list` against it (0 candidates), not by assuming the tool had nothing to do.

**Known, deliberate, already-locked gap (AC-008B): the real workspace cannot `cargo build`/`cargo test` yet.**
`io-errors` (a dependency `MonoklError` needs) is pinned to a private registry (`registry =
"orin-cargo"`) with no `.cargo/config.toml` anywhere in the corpus configuring where that registry
actually is — `cargo metadata` fails at manifest-parse time with `registry index was not found in
any configuration: \`orin-cargo\``, confirmed and locked as expected behavior by SPEC-004's own
AC-008B and re-confirmed independently by `lambda-exit-gate`. This was fine for SPEC-004 itself
(a pure manifest/module-declaration lock, verified via content-exactness diffs and isolated `/tmp`
fixtures rather than real builds) — it stops being fine the moment real Rust logic needs to
actually compile and run its own tests, which is every track from here on.

### SPEC-008: Workspace/extract config types (Track 5c) — GATED after the longest correction sequence of any track so far

Spec: `.claude/specs/SPEC-008.json` (13 acceptance criteria — `TsconfigMode`, `WorkspaceOptions`
(with its 2-method `impl` block), `ExtractRequest`). Smallest type-count of any track (3 types)
but the most exit-gate rounds: grounding and audit passes were clean (one accurate prose
correction — a citation-history overstatement claiming finding #23 had been cited elsewhere when
it hadn't), but canon-exit-gate failed **5 times** across two distinct defect classes:

**Technical precision** (2 failures) — AC-012 initially quoted serde_json's generic "expected a
string" for a wrong-type `file` value, when `camino`'s own custom `Deserialize` impl for
`Utf8PathBuf` produces "expected a UTF-8 path string" instead; and AC-009 asserted a two-class
negative/non-integral numeric-rejection rule when the real rule (established for the structurally
identical `SearchLimits` fields in SPEC-007 AC-006) is three classes, including a parse-time
"number out of range" case for magnitudes exceeding finite f64 range.

**False comparative claims** (3 failures, escalated to the human) — after retry 3 formally
exhausted canon's retry budget on an AC-010 claim that CodeBlock's own analogous
line_start/line_end gap has "no documented downstream consequence" (false: SPEC-006 AC-004
documents one, a defended-against underflow panic), the user authorized continuing past the
budget. The fix held, but two more rounds surfaced further false "unlike"/"first" claims: AC-001's
"unlike every other enum this project's specs have locked so far" (false — SPEC-005 already locks
mixed-variant enums), and after a first correction attempt replaced it with an equally false
narrower claim (citing DependencyTarget and LangData as mixed enums when neither is, and claiming
TsconfigMode is "this project's first data-carrying enum with no serde derive" when SPEC-001's
`MonoklError` already is one) — resolved per the gate's own explicit recommendation ("delete it
rather than correcting it a third time") by removing the comparative claim entirely rather than
attempting a third correction. A final, independent full-document sweep for the same pattern
found nothing further wrong, plus one purely mechanical defect (`api_surface` needed the object
shape with name/signature/description keys, not plain strings — SPEC-005's precedent).

axiom (recon → verifier → exit-gate) then surfaced one more, genuinely interesting case: 
axiom-verifier read camino/serde_core/serde_json's actual cached crate source line-by-line and
concluded AC-012's `null`-value case was wrong (predicting "unit value" instead of "null"). Before
accepting that, an actual compiled-and-run reproduction was built and checked directly — the real
output was "invalid type: null, expected a UTF-8 path string", exactly matching AC-012 as
written. axiom-verifier's static source trace had followed a code path camino's `Utf8PathBuf`
Deserialize impl doesn't actually take for a null input. axiom-exit-gate independently rebuilt the
same reproduction before issuing its verdict and confirmed: pass, high confidence, zero blockers.

Net: a good illustration of two distinct failure modes this project's rigor standard exists to
catch — the technical-precision failures are exactly why the empirical-toolchain-compilation
standard exists (a plausible-sounding claim about exact error text or class count that only a
real compile-and-run can settle); the false-comparative-claim failures are a different lesson
entirely — unverified "first"/"unlike" framing language carries zero implementable content and
was, in this track, wrong three separate times before being deleted rather than re-corrected. And
the final axiom round is the sharpest illustration yet that even *reading real crate source
line-by-line* is not a substitute for actually compiling and running it — a careful static trace
through the right crate's real code reached a confident, wrong conclusion.

### SPEC-007: Search-command request/response types (Track 5b) — GATED after an escalation to the human

Spec: `.claude/specs/SPEC-007.json` (16 acceptance criteria — `SearchOptions`, `SearchLimits`,
`Language`, `SearchResponse`, `SymbolsResult`, `DependentsResult`). Grounding pass (canon-verifier)
came back 16/16 supported, zero issues — cleanest first pass of any track so far. canon-auditor
found 5 real issues (a dangling forward-reference, two missing error cases, a missing scope
disclaimer, an untestable hypothetical-variant claim) and canon-exit-gate then failed **3
consecutive times**, every failure landing on the same criterion, AC-003 (what happens when
`SearchOptions.limits` gets a malformed JSON value): first for a false "any array length other
than 4 fails with a length error" claim (actually: >4 elements produce a `trailing characters`
error, since serde's derived `visit_seq` never drains surplus elements); then for a "for the same
reason" claim about negative-vs-non-integral rejection (actually two different serde_json error
constructors); then for a rewrite that fixed both of those but introduced a new false claim about
which array lengths hit which error class. canon-exit-gate's own retry-2 verdict formally
recommended escalating to a human rather than attempting a 4th auto-patch, correctly diagnosing
that array-form deserialize is a **short-circuiting ordered procedure** (element-type check on
positions 0..min(len,4) always runs first and can preempt the length check, regardless of the
array's actual length) — not the flat length-keyed classification every rewrite kept re-encoding.

Surfaced this to the user directly via `AskUserQuestion` rather than continuing to auto-patch.
User chose: rewrite AC-003 as the exit-gate's own confirmed ordered procedure. That rewrite
passed the ordered-procedure claim outright (survived an adversarial boundary probe: position-3
elements, length-5 arrays with only the 5th element bad, two-bad-elements-at-once), but the same
gate pass then found the numeric-literal side of the rule was only half-corrected — a genuine
**three-way** discriminator (integer within i64/u64 range; non-integer-lexical-form or
one-past-the-integer-boundary reinterpreted as floating-point; magnitude exceeding finite f64
range fails at parse time with "number out of range", naming no type at all) where the rewrite
had assumed a two-way rule "regardless of numeric value" — falsified by boundary literals
(`18446744073709551615`/`616` for u64, `-9223372036854775808`/`809` for i64, `1e308`/`1e309`).
Fixed AC-003's whole-value contract, AC-003's array-form STEP 1, and AC-006 (which had never been
updated to match and still asserted the old two-way rule, creating an intra-document
contradiction) — all three worded identically per the gate's request so they can't drift apart
again. One more round after that: STEP 1's own out-of-range clause claimed the parse-time failure
always precedes positional dispatch, refuted by two counter-examples the gate constructed
(`["a", 1e309, 3, 4]` fails at position 0's type check, not position 1's parse failure;
`[1,2,3,4,1e309]` fails with `trailing characters`, not `number out of range`, since position 4
is surplus and never parsed) — fixed to state the real rule: first failure at the lowest position
wins, whether it's a type failure or a parse failure. Final gate pass: clean, high confidence,
including a battery of adversarial cases the gate constructed itself that aren't in the criterion
text (symmetric counter-examples, position-3 boundary, position-4-is-inside-the-window checks).
Added one small zero-risk completeness item the gate flagged but didn't block on (no criterion
stated `#[serde(deny_unknown_fields)]`'s absence for `SearchOptions`/`SearchLimits`, unlike the
SPEC-006 precedent) before the final confirmation pass.

axiom (recon → verifier → exit-gate) then passed clean on the first attempt, 16/16 independently
re-derived rather than trusting canon's extensive history — including re-running the full AC-003
boundary-literal matrix from scratch in its own compiled crate rather than accepting canon's
result on faith.

Net: this track is the clearest demonstration yet of why the empirical-toolchain-compilation
standard exists — every one of AC-003's failures was a *plausible-sounding, wrong* claim about
serde_json's exact runtime behavior that static reading alone would not have caught, and canon's
own 3-retry escalation protocol worked exactly as designed: it recognized a pattern of
narrowing-but-still-wrong patches and routed the decision to a human rather than continuing
indefinitely.

### SPEC-006: Result/diagnostic primitives (Track 5a) — GATED, first-attempt clean on both pipelines

Spec: `.claude/specs/SPEC-006.json` (15 acceptance criteria — `CodeBlock`, `RankedBlock`,
`ParentContext`, `Diagnostic`, `DiagnosticKind`, `LineHit`). First track drafted under the
corner-case-first methodology (enumerate missing/null/wrong-type/extra-field, boundary values,
cross-field invariants, and sibling-type interactions *before* writing each criterion, rather
than relying on the audit stage to find gaps after a bare-transcription first pass) — a direct
response to being asked whether the happy path and corner cases were genuinely both covered.

Two new serde mechanisms locked for the first time in this project: `#[serde(flatten)]`
(`RankedBlock.block`, verified by construction to produce zero key collisions between
`CodeBlock`'s 8 keys and `RankedBlock`'s own 6) and `rename_all = "lowercase"`
(`DiagnosticKind`, explicitly distinguished from `camelCase` even though the two rules produce
identical output for its 3 current single-word variants — a coincidence of naming, not
equivalence, stated precisely rather than glossed over). Also the project's first types with no
`Deserialize` (`RankedBlock`, `ParentContext` — Serialize only) and first type with no serde
derive at all (`LineHit` — using it where `Serialize`/`Deserialize` is required is a compile-time
`E0277`, not a runtime failure).

canon-verifier caught two real overfitted citations on the first grounding pass: AC-004 claimed
the `line_start <= line_end` gap was "genuinely new," overlooking that `01-core-architecture.md`'s
own `overlaps_significantly` comment (1217-1219) already flags it in prose; AC-011 claimed
`Diagnostic` was "the sole type instantiating" the *entire* cross-cutting silent-data-drop
pattern, overreaching past what Part 2 High item 3 (the actual matching finding) supports —
Part 1 finding #1 is a different phenomenon (no diagnostics field exists at all, vs. one
existing unpopulated). Both fixed by narrowing the claims to what the cited passages actually
say. canon-auditor then found seven issues on the same draft — a dangling forward-reference in
AC-005, two missing-error-case gaps (unknown JSON keys, negative-into-usize) that this spec's
own sibling SPEC-003 already established as required coverage but this draft initially dropped,
a missing SymbolKind-scope disclaimer on AC-009, an untestable hypothetical-variant claim in
AC-013 (split into a testable criterion plus a non-binding note moved to `reasoning`), and
imprecise phrasing in AC-008/AC-015 tightened into concrete, mechanically-checkable examples —
all fixed in one round. canon-exit-gate then passed clean on the first attempt, high confidence,
with every serde-mechanism claim empirically compiled against a real `serde`/`serde_json`
crate (cargo/rustc 1.96.0). axiom (recon → verifier → exit-gate) also passed clean, high
confidence, independently re-deriving all 15 criteria against its own fresh compile rather than
trusting canon's result — axiom-exit-gate caught one trivial citation-range imprecision
(1216-1218 vs the actual 1217-1219) as a non-blocking note, fixed directly afterward.

Net: two real grounding overreaches and seven real audit gaps caught and fixed before either
exit gate, zero found by either exit gate itself — the corner-case-first drafting approach
raised the floor as intended, but the review pipeline still earned its keep catching what
drafting missed regardless.

### Confidence equalization pass — SPEC-003 and SPEC-004 brought up to SPEC-005's level

Requested explicitly: before starting Track 5, make sure all three completed tracks sit at
the same confidence level rather than three different ones.

**SPEC-003** had never actually passed canon (its last verdict was ESCALATE, not pass) and
had never been through axiom at all. Ran a fresh canon-exit-gate budget: it failed twice more
before passing clean — first on a fabricated "TOON" citation (the term appears nowhere in
this project's actual spec corpus; it was pulled from a secondary reference and a
forward-looking CLAUDE.md note about a future dependency, not the primary source), then on a
wrong intermediary type in the fix for that (`FileAnalysis`, which has no `Deserialize` derive
at all — the real path is `CacheFile.entries → PersistedFileAnalysis.symbols`). Same exact
sentence, wrong three consecutive times before landing correctly. Then ran axiom fresh
(verifier + exit-gate) — both passed clean, with 12/12 criteria empirically re-confirmed by
compiling the verbatim types against real `serde_json`.

**SPEC-004** just needed axiom re-run after the registry-config fix from last session (applied
but never re-confirmed). Both axiom-verifier and axiom-exit-gate passed clean this time —
28/28 criteria independently re-derived, with every behavioral claim (`cargo publish`'s
registry restriction, the three-way `required-features` split, `duplicate key` vs. legal
cross-table promotion, the workspace-inheritance failure chain) reproduced against a live
cargo/rustc 1.96.0 toolchain.

Net: five real defects caught across the two specs during this pass alone (three in SPEC-003,
none new in SPEC-004 — its fix from last session held). All three specs (SPEC-003, SPEC-004,
SPEC-005) are now at the same level: passed canon, passed axiom, final-round claims
empirically verified against a real toolchain, not just read against source text.

### SPEC-005: Dependency/export/JSX/LangData type cluster — GATED

Spec: `.claude/specs/SPEC-005.json`
Status: passed both canon (3 rounds — a genuinely wrong claim caught and fixed: enum-level
`rename_all` does NOT reach struct-variant field names without a separate
`rename_all_fields` attribute, so `DependencyTarget`'s fields stay snake_case on the wire,
not camelCase as first drafted) and axiom (1 round — caught a misattributed citation, fixed).
Axiom's final pass empirically confirmed 38 separate observable behaviors by compiling the
verbatim type cluster against a real `serde` toolchain, including the once-wrong AC-005 claim.
Drafted directly this time (not via a separate canon-drafter dispatch) applying SPEC-001/003/
004's lessons proactively — correctly avoided over-generalizing the already-verified
Option&lt;T&gt; leniency finding to Vec&lt;T&gt; (a genuine, different mechanism), but still initially
over-generalized a *different* already-verified finding (struct field-level camelCase) to an
enum's struct-variant fields (a different mechanism again) — worth remembering: each serde
attribute's behavior needs re-deriving per type-shape, verified findings don't transfer by
analogy even when they look similar.

### Direct review pass (SPEC-003/SPEC-004, no subagent) — one real finding, fixed in both specs

Read SPEC-003.json and SPEC-004.json in full, end to end, as a genuinely fresh set of eyes
rather than another automated round. Found one real, substantive issue that no canon or axiom
round had caught, because none of them were checking for it: SPEC-003's AC-007 and AC-008
criterion text carried a blow-by-blow tally of `07-edge-cases-and-failure-modes.md`'s own
adversarial-verification history ("weakened 5 findings... struck through 1... out of 61
total... one of the 55 that survived") baked directly into the testable proposition, and
SPEC-004's AC-016 rationale did something similar ("was added after canon-exit-gate's retry-1
review found..."). This violates this session's own established rule — document the spec,
not the historical account of how it changed — everywhere except `05-research-and-decisions.md`
and each spec's own `revision_note`/`reasoning` fields, which exist specifically to carry that
history. Fixed by trimming both criteria and rationales down to the citation and the
substantive fact, moving nothing lost since `revision_note` already has the full account.
Cross-checked `core-types.md` against SPEC-003's now-corrected deserialize-behavior claims
(the TypeAlias camelCase fix, the missing-key/explicit-null semantics) — no drift found, the
semantic-model file never made the specific claims that needed correcting.

### SPEC-004: Cargo.toml + lib.rs + workspace root Cargo.toml — GATED, both pipelines confirmed

Spec: `.claude/specs/SPEC-004.json`
Status: canon-exit-gate passed cleanly on its 3rd (final) attempt, high confidence, 27 criteria.
axiom's independent cross-check (a second, differently-built pipeline) then found one more
real, material gap even after that clean pass: the `orin-cargo` Cargo registry's actual
configuration (a `.cargo/config.toml` entry) doesn't exist anywhere in this spec's corpus,
which means Section 1 + Section 28 together do NOT make the manifest independently
resolvable end-to-end the way an earlier draft implied — a real precondition, not a nitpick.
Fixed directly (new AC-008B states the gap explicitly; AC-001B, AC-025, purpose/scope
corrected to stop overclaiming). Scope also expanded significantly mid-review: the track
started as "Cargo.toml + lib.rs" and grew to include the workspace root Cargo.toml (Section
28, ~58 lines) after canon's own gate found a false "no workspace table exists in the corpus"
claim in an early draft — Section 28 turned out to be real, necessary, and already in the
corpus, just far from Section 1. Total: canon x4 rounds (3 fail, 1 pass) + axiom x1 round
(fail, now fixed but not re-confirmed). Given the volume of genuine defects found at every
single stage of review for this one track, recommend a fresh look before treating this as
fully closed, same as SPEC-003.

### SPEC-003: SymbolKind/Visibility/SymbolEntry — ESCALATED, needs human review

Spec: `.claude/specs/SPEC-003.json`
Semantic model: `docs/architecture/core-types.md`
Status: NOT gated. `canon-exit-gate`'s third attempt (its own stated retry maximum) returned
`escalate`, not pass or fail — it found one more real defect (AC-011 falsely claimed
`signature` carries `#[serde(default)]`, contradicting AC-004/AC-005) but flagged a pattern
worth a human's attention: each of the first two correction rounds fixed everything it was
asked to fix while independently introducing one new, small, source-contradicting claim into
the text it added. The final fix was applied directly (mechanical, verified against source),
but no automated re-confirmation was run afterward, deliberately — that would defeat the
point of an escalation. Recommend a human (or a fresh, independent read) confirm this before
treating it as gated.

### SPEC-001: MonoklError enum + Result<T> alias — GATED

Spec: `.claude/specs/SPEC-001.json`
Semantic model: `docs/architecture/error-taxonomy.md`
Implementation: not started — this is spec-stage only, no crate exists yet.
Exit-gate: PASS (4th attempt, high confidence). 24 acceptance criteria, all 14 `MonoklError`
variants covered, zero blockers on the final pass.

History: round 1 (10 blockers — false claims, unfalsifiable criteria, fabricated citations,
elided source facts presented as closed lists) → round 2 correction introduced a new problem
(bundled a genuine new design decision — cache.json corruption-vs-staleness recovery policy —
into what should have been a faithful-correction track, violating this project's own
CLAUDE.md process) → round 3 split the corruption-policy question out entirely, deferred to
SPEC-002, gated on `superpowers:brainstorming` → round 4 fixed two narrow residual defects
(undercounted `Walk` construction sites; a reachability claim for `SymlinkRejected`/
`FileTooLarge` refuted by `docs/spec/07-edge-cases-and-failure-modes.md` Part 3) → canon's own
gate passed with zero blockers.

Then run through a second, independently-built pipeline (`axiom`'s `verify-spec`) as an extra,
differently-postured check: `axiom-verifier` re-derived and confirmed all 24 criteria clean;
`axiom-exit-gate` still found two real, substantive gaps neither canon round had caught — (1)
AC-001's "no 15th variant" silently contradicted an already-adopted decision in
`docs/spec/05-research-and-decisions.md` §2 (a future `MonoklError::TraversalCapExceeded`
variant), and (2) AC-020's "no init subcommand in either enum" missed a real third `Subcmd`
enum revision in `docs/spec/02-inspection-and-analysis.md` that all four canon rounds had also
missed. Both fixed directly (mechanical, narrow edits — rescoped AC-001's claim rather than
weakening it, corrected the enum count to three throughout, including in
`error-taxonomy.md`). This is worth remembering: two structurally different review pipelines
catch different classes of defect — canon's rounds were excellent at spec-internal
consistency and grounding against the two files a track cites, but never cross-checked
`05-research-and-decisions.md`'s adopted-but-unimplemented decisions register, which is
exactly the kind of corpus-wide check a second, independently-designed tool is good for.

Not committed to git yet — canon's own orchestration convention calls for an automatic commit
on gate pass, but this session's standing rule is never to commit without explicit
instruction. Awaiting that instruction.

---

## Pipeline Protocol (follow for every track, in order)

1. `canon:canon-drafter` — write spec@1 (fed by a `requirement@1` and, where the answer isn't
   already fully researched elsewhere in `docs/spec/`, a `trace`-produced `research-report@1`)
2. `canon:canon-verifier` — grounding check against source artifacts
3. `canon:canon-auditor` — adversarial quality review — must pass before continuing
4. `canon:canon-exit-gate` — binding pass/fail verdict — must pass before continuing (max 3
   retries before escalating to a human reviewer)
5. Write `.claude/specs/<id>.json`, set `spec_file_path`, commit — **hold this step for
   explicit user instruction on this project**
6. `vector:vector-planner` — produce plan@1 with exact red tests
7. `vector:vector-challenger` — must pass before continuing
8. `lambda:lambda-recon` — confirm baseline tests pass
9. `lambda:lambda-implementer` — one task at a time, red phase mandatory
10. `lambda:lambda-reviewer` — review each commit before next task
11. `lambda:lambda-exit-gate` — adversarial final check
12. `just ci` (once a `justfile` exists) — must exit 0

Constraints:
- NEVER commit without explicit user instruction (overrides canon's own auto-commit
  convention for this project specifically)
- A design decision not already covered by `docs/spec/05-research-and-decisions.md` goes
  through `superpowers:brainstorming` before it becomes a spec@1 criterion — do not resolve
  it as a `judgment_call` inside an unrelated track (see SPEC-001's history above for what
  happens when this gets skipped)
- Prefer tight, single-subsystem tracks over broad ones — SPEC-001 covers one enum; splitting
  further when a track's `judgment_call` count climbs is a correct exit-gate response, not a
  failure

---

## Track decomposition — 01-core-architecture.md (the "original doc," v0.1.0 baseline)

Full breakdown, sized to pass canon-exit-gate's single-planning-cycle rule. Every track below
gets the SPEC-001 treatment: canon (drafter→verifier→auditor→exit-gate, retry to pass) +
axiom (verifier→exit-gate) as an independent second check. This is a large undertaking —
SPEC-001 alone took 4 canon rounds plus an axiom pass to reach a clean gate; expect similar or
lighter per track now that the scope-discipline lesson (defer real design decisions, don't
resolve them inline) is established, but this is realistically dozens of sessions of work at
this rigor level, not one sitting.

| # | Track | Source section(s) | Status |
|---|---|---|---|
| 1 | `MonoklError` + `Result<T>` | §3 (error.rs) | **GATED** — `SPEC-001.json` |
| 2 | Cargo.toml + lib.rs + workspace root Cargo.toml | §1-2, §28 | **GATED** — `SPEC-004.json` |
| 3 | Core symbol/visibility types (`SymbolKind`, `Visibility`, `SymbolEntry`) | §4 (subset) | **GATED** — `SPEC-003.json` |
| 4 | Dependency/export/JSX/`LangData` types | §4 (subset) | **GATED** — `SPEC-005.json` |
| 5a | Result/diagnostic primitives (`CodeBlock`, `RankedBlock`, `ParentContext`, `Diagnostic`, `DiagnosticKind`, `LineHit`) | §4 (subset) | GATED — `SPEC-006.json` |
| 5b | Search-command request/response types (`SearchOptions`, `SearchLimits`, `Language`, `SearchResponse`, `SymbolsResult`, `DependentsResult`) | §4 (subset) | GATED — `SPEC-007.json` |
| 5c | Workspace/extract config types (`TsconfigMode`, `WorkspaceOptions`, `ExtractRequest`) | §4 (subset) | GATED — `SPEC-008.json` |
| 6 | Query language (lexer/parser/plan) | §5 | **IMPLEMENTED** — `SPEC-009.json`; resolves query-and-ranking.md's regex/`+`/`-` gaps |
| 7 | Token counting | §6 | Not started |
| 8 | Text search | §7 | Not started — resolves the non-deterministic candidate-selection gap |
| 9 | Ranking (BM25/boost/tokenize) | §8 | Not started — resolves the unimplemented tie-break claim |
| 10 | Dedup | §9 | Not started |
| 11 | Budget | §10 | Not started — resolves the uncapped `max_results`/`max_tokens` gap |
| 12 | `LanguageAnalyzer` trait + capability model | §11 | Not started |
| 13 | `Digest` (was `ContentHash`) | §12, Part 8 §8.4 | **IMPLEMENTED** — `SPEC-011.json` (corrected to `Digest`), `crates/monokl/src/digest.rs`, ADR 0002 |
| 14 | `FileAnalysis` | §13 | Not started |
| 15 | `NodeKind` | §14 | Not started |
| 16 | In-memory cache | §15 | Not started — resolves cache-architecture.md's Tier-1 content-hash gap |
| 17 | Disk persistence | §16 | Not started — heavily overlaps SPEC-001/SPEC-002, reconcile when drafted |
| 18 | `TsAnalyzer` | §17 | Not started — "most important" module per the spec's own framing; likely needs further splitting |
| 19 | `WorkspaceEnricher` | §18 | Not started |
| 20 | Indices (`FileIdx`, `ImportGraph`, `SymbolIndex`, `WorkspaceIndex`) | §19 | Not started |
| 21 | CLI / pipeline / main.rs | §20-22 | Not started |
| 22 | napi FFI boundary | §26 | Not started |

Design tenets/lints (§29) is project-wide policy, not really spec@1-shaped — handle as
workspace `Cargo.toml` lint config directly when the crate is scaffolded, not as a track.

## Backlog (named, not yet spec'd)

- **SPEC-002 — cache.json corruption-vs-staleness recovery policy.** Deferred out of SPEC-001.
  Must start with `superpowers:brainstorming`, not `canon:canon-drafter` directly — this is a
  new design decision, not a faithful correction. See SPEC-001's non_goals and
  `docs/architecture/error-taxonomy.md` gap 1 for full context.
- **Core types** (`types.rs` — `SymbolEntry`, `Visibility`, `CodeBlock`, `LangData`, etc.).
  Semantic model already written: `docs/architecture/core-types.md`, which documents two
  real, would-not-compile naming defects in `docs/spec/04-analysis-fidelity.md` (`impl_owner`
  vs. the canonical `owner`; a nonexistent `Visibility::Super` variant) that any spec@1 for
  this track must resolve, not port.
- **git_scope.rs's three elided `Git`-variant operation-tag sites** inside `collect_changes`
  (`03-multi-language-platform.md:1428, 1430, 1470`) — deferred in SPEC-001 AC-015. Needs the
  git_scope.rs implementation track to decide the literal values (and, for two of the three,
  even confirm the constructed variant).
- **io_safety.rs wiring** — `read_to_string_capped` has zero call sites anywhere in the current
  spec corpus, so `SymlinkRejected`/`FileTooLarge` are currently unconstructible via any shown
  command path (SPEC-001 AC-016/AC-017). Wiring it into `extract`/`refs`/`definition` as the
  existing prose already claims is real, scoped implementation work.
- **Query language + ranking determinism.** Semantic model written:
  `docs/architecture/query-and-ranking.md`. Six open questions (regex hard-fail vs.
  silent-drop, `+regex:` degradation, the unimplemented `(file, line_start)` tie-break claim,
  non-deterministic candidate selection above `max_candidates`, uncapped query length,
  exclusion-only-query silent-empty behavior).
- **Cache Tier-1 content-hash gap.** Semantic model written:
  `docs/architecture/cache-architecture.md`. The architecture's own "ContentHash is
  authoritative" claim is not backed by the Tier-1 lookup code as specified — needs a decision,
  not a silent port.
- **LanguageAnalyzer diagnostic-signal gaps.** Semantic model written:
  `docs/architecture/language-analyzer-contract.md`. Macro/`#[cfg(...)]`-gated Rust code is
  invisible with no capability signal; `.d.ts` files get a misleading "language unsupported"
  message when the language is fully supported and this one file is intentionally excluded.
- **Daemon/session-mode design** (`PrecisionUpgrader`, `PrecisionLedger`, etc.) — explicitly
  future work, not required for v0.1.0. Semantic model written:
  `docs/architecture/daemon-lifetimes.md`, five independently-reverified open gaps.

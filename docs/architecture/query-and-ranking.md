# Query & Ranking — Semantic Model

Source: `docs/spec/01-core-architecture.md` §5 (`query/` — lexer, parser, plan, ~L641-897),
§8 (`rank/` — bm25, boost, tokenize_doc, ~L1058-1191). Cross-referenced against the `search`
pipeline stages (§20, ~L2315-2326) and design tenet #4 (§29, L3093).

`query/` is implemented (SPEC-009); `rank/`, `dedup.rs` and `text_search.rs` follow SPEC-012 and SPEC-013. Where
this doc and the code disagree, the code and its tests win.

**Resolved (05 §18):** #7 (one fail-fast regex policy), #9 (total-order tie-break), #10 (sorted walk), #11 (`max_files`
memory ceiling that keeps counting). **Still open:** #16 (query language). **Resolved since:** #21, as an empty result with one explanatory diagnostic (05 §21), and #8, where `+regex:`/`-regex:` keep their modifier (05 §24).

## Lexer (`query/lexer.rs`)

```rust
pub enum Token { Plus, Minus, RegexPrefix, QuotedString(String), Word(String), Eof }
```

- `+`/`-` become `Plus`/`Minus` only when they begin a token — `read_word` otherwise consumes
  greedily to whitespace, so `foo+bar` is one `Word`, not three tokens.
- `regex:` (case-sensitive) emits `RegexPrefix`; the *next* token is the pattern.
- `"..."` → `QuotedString`; `\"` is the only recognized escape.

## Parser (`query/parser.rs`)

```rust
pub enum Modifier { Required, Excluded, Optional }
pub struct ParsedTerm { pub modifier: Modifier, pub pattern: String, pub is_regex: bool }
pub fn parse(input: &str) -> Result<Vec<ParsedTerm>>
```

- `const LIMIT: usize = 64` — caps **term count**, checked post-loop via
  `MonoklError::TooManyTerms { count, limit }`. Whitespace-only input short-circuits to `Ok(vec![])`
  before the check runs.
- Bare `Word`/`QuotedString` → `Optional`, `pattern = regex::escape(...)`, `is_regex: false`.
- `regex:<word|quoted>` (unprefixed) → `Optional`, `is_regex: true`, pattern used **raw**.
- `+<word|quoted>` / `-<word|quoted>` → `Required`/`Excluded`, `is_regex: false`, escaped.
- `+regex:foo` / `-regex:foo` → `Required`/`Excluded`, `is_regex: true`, pattern used raw (finding #8, closed by 05 §24).

## `QueryPlan` (`query/plan.rs`)

```rust
pub struct QueryPlan { pub terms: Vec<ParsedTerm>, pub required: Vec<usize>,
                        pub excluded: Vec<usize>, pub scored: Vec<usize> /* == Optional */ }
impl QueryPlan {
    pub fn from_terms(terms: Vec<ParsedTerm>) -> Self;
    pub fn is_empty(&self) -> bool;               // terms.is_empty()
    pub fn search_patterns(&self) -> Vec<&str>;    // required ++ scored, in that order
}
```

`search_patterns()` deliberately omits `excluded` — exclusion terms never drive text search; they
apply as a post-match filter in the pipeline's "boolean eval" stage (§20 step 6).

**Exclusion-only queries (finding #21 — open question).** §20 step 2: `if
plan.search_patterns().is_empty() → empty response`. A query of only `Excluded` terms
(`-foo -bar`) leaves `required` and `scored` both empty, so `search_patterns()` is empty, so
`search` returns an empty response — silently, no `Diagnostic`. This *reads* like an intentional
guard but is undocumented as such, and is indistinguishable from "matched nothing." spec@1 must
pick one explicitly: (a) keep silent-empty but document/cite it here as by-design, or (b) emit
`Diagnostic { kind: Skipped }` naming the no-op before returning. Do not port current behavior
without deciding.

## Ranking pipeline (`rank/`)

```rust
pub fn rank_blocks(blocks: Vec<(CodeBlock, Vec<String>)>, query_terms: &[&str], df: &HashMap<...>)
    -> Vec<RankedBlock>
```

Per block: `bm25 = bm25::score(...)`,
`ntype = boost::node_type_boost(block.node_kind)`, `name = boost::name_match_boost(&block, query_terms)`
(1 + 2 * share of query terms in the block's declared name, so 1.0 to 3.0),
**`final_score = round6(bm25 * name * ntype)`**.
Sorted descending by `final_score` only; 1-based `rank` assigned post-sort.

BM25 (`rank/bm25.rs`): `K1 = 1.5`, `B = 0.5`, standard Okapi formula, `round6` to 6dp for
cross-platform f64 stability (`DP = 1_000_000.0`) — keep this rounding.

`node_type_boost` (`rank/boost.rs`): Function 1.5; Class/Method/Constructor 1.4; Interface/Struct
1.3; TypeAlias/Enum 1.2; Variable/Impl 1.1; Property/Field/Macro 1.0; Module 0.9; Other 0.7.

**Tie-break claim vs. reality (finding #9).** Tenet #4 (§29 L3093) states: **"tie-break on
`(file, line_start)`."** The actual sort:

```rust
ranked.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap_or(Ordering::Equal));
```

is single-key on `final_score` — no secondary comparator. Ties are plausible (`node_type_boost`
takes a small discrete value set); their order then falls out of Rust's
stable-sort preserving whatever pre-sort order the candidate walk produced — itself
non-deterministic per finding #10. **The architecture doc's determinism claim is not backed by the
ranking code as specified.** spec@1 must either implement `(file, line_start)` as an explicit
secondary sort key or correct tenet #4's prose — "probably stable in practice" does not satisfy it.

## Determinism gaps — must be resolved, not ported as-is

Each is an open question for spec@1, not a resolved design; record the decision explicitly rather
than silently carrying spec-stage behavior into implementation.

1. **#7 — bad `regex:` term hard-fails in one stage, silently drops in the next.** §20 step 4
   (`text_search::search_files`) joins all patterns into one `combined` regex and hard-fails the
   whole query via `?` on any invalid pattern. §20 step 5 (`term_regexes`, per-term, for
   highlighting) silently drops compile failures instead. Same malformed input, two contradictory
   outcomes depending on which of two independently-written regex-compile paths trips. Decide one
   shared compile path with one failure policy (fail-fast or fail-soft), not both.
2. **#8 — `+regex:foo` lost both modifier and regex semantics. Closed (05 §24):** the parser has a real
   `Required`/`Excluded` + `is_regex: true` arm.
3. **#9 — the `(file, line_start)` tie-break tenet #4 claims does not exist in `rank_blocks`.** See
   ranking section above. Decide: implement it, or strike the claim.
4. **#10 — candidate selection above `max_candidates` has no deterministic ordering.** The
   `ignore::WalkBuilder` walk in `text_search::search_files` isn't sorted
   (no `.sort_by_file_name()`) and `break`s once `candidate_count >= max_candidates` (default
   1000). Which files land inside the cap — and are thus even eligible for ranking — is
   filesystem/OS walk-order-dependent, not reproducible across runs or machines. This sits
   upstream of #9: a stable tie-break is moot over a non-reproducible candidate set. Decide: sort
   the walk, or explicitly document non-reproducibility above the cap.
5. **#11 — only `max_bytes` is a hard ceiling.** `max_results`, `max_tokens`, `max_candidates` are
   unbounded `Option<usize>` CLI args — opt-out by construction for 3 of the system's 4 resource
   knobs. Decide per-knob caps.
6. **#16 — no cap on a single query term's or the raw query string's character length.**
   `LIMIT = 64` bounds term *count* only; a single arbitrarily-long `Word` token is legal input
   today. Decide whether a length cap is needed alongside the count cap, and what error variant
   reports it (a sibling to `TooManyTerms`, not a repurposing of it).

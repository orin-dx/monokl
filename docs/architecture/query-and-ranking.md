# Query & Ranking — Semantic Model

Source: `docs/spec/01-core-architecture.md` §5 (`query/` — lexer, parser, plan), §8 (`rank/` — bm25, boost, tokenize_doc) and the `search` pipeline stages (§20). `query/`, `rank/`, `dedup.rs` and `text_search.rs` are implemented. Where this doc and the code disagree, the code and its tests win.

Edge-case findings #7, #8, #9, #10, #11 and #21 are resolved (05 §18, §21, §24). Open: #16 (no cap on one term's length).

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
    pub fn search_patterns(&self) -> Vec<&str>;    // required ++ scored, in that order
}
```

`search_patterns()` deliberately omits `excluded` — exclusion terms never drive text search; they
apply as a post-match filter in the pipeline's boolean evaluation stage.

**Exclusion-only queries (finding #21, resolved by 05 §21).** A query with no positive term (`-foo -bar`, or an empty
query) returns no results and one `Skipped` diagnostic saying the query has no positive terms, so it is not mistaken for "matched nothing."

## Ranking (`rank/`)

```rust
pub fn rank_blocks(blocks: Vec<(CodeBlock, Vec<String>)>, query_terms: &[&str], df: &HashMap<...>)
    -> Vec<RankedBlock>
```

Per block: `bm25 = bm25::score(...)`,
`ntype = boost::node_type_boost(block.node_kind)`, `name = boost::name_match_boost(&block, query_terms)`
(1 + 2 * share of query terms in the block's declared name, so 1.0 to 3.0),
**`final_score = round6(bm25 * name * ntype)`**.
Sorted by `final_score` descending, then file, `line_start`, `line_end`, `node_kind` and `code`; 1-based `rank` is
assigned after the sort.

BM25 (`rank/bm25.rs`): `K1 = 1.5`, `B = 0.5`, standard Okapi formula, `round6` to 6dp for
cross-platform f64 stability (`DP = 1_000_000.0`) — keep this rounding.

`node_type_boost` (`rank/boost.rs`): Function 1.5; Class/Method/Constructor 1.4; Interface/Struct
1.3; TypeAlias/Enum 1.2; Variable/Impl 1.1; Property/Field/Macro 1.0; Module 0.9; Other 0.7.

## Determinism and bounds

Resolved findings, kept so they are not reopened:

1. **#7 — a bad `regex:` term fails the whole search.** Every term, excluded ones too, compiles before any file is
   read (05 §18).
2. **#9 — total-order tie-break.** The six sort keys above leave no ties, so the order never depends on input order.
3. **#10 — the file walk is sorted by file name**, so the set of files reached under the file ceiling is reproducible.
4. **#11 — resource bounds.** The text search examines at most 100,000 files and keeps counting past it, reads at most
   4 MiB per file and keeps 4096 bytes per line; `--max-results` and `--max-tokens` cut the output; `--timeout` stops
   the work. See [ADR 0004](../adr/0004-search-stays-stateless-and-every-stage-has-a-stated-bound.md).

Open:

- **#16 — no cap on one term's or the query's length.** `LIMIT = 64` bounds term *count* only. Decide whether a length
  cap is needed, and which error reports it (a sibling to `TooManyTerms`, not a repurposing of it).

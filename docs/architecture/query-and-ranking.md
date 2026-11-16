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
(1.0 to 7.0, below), **`final_score = round6(bm25 * name * ntype)`**.
Sorted by `final_score` descending, then file, `line_start`, `line_end`, `node_kind` and `code`; 1-based `rank` is
assigned after the sort.

`name_match_boost` (`rank/boost.rs`) compares the query terms `Q` (repeats counted) with `N`, the distinct tokens of the
block's declared name (`CodeBlock.name`, tokenized like a document, so stop words such as the `use` of `useWatch` and the
`from` of `FromRequestParts` are not tokens): `R` is the share of `Q` matched by a token of `N`, `P` the share of `N`
that matches a term of `Q`. A token matches a term when they are equal, or when the token has at least 3 characters and
the term starts with it (`ext` matches `extension`, `req` matches `request`). It never matches by being longer than the
term (`parser` does not match `parse`), and a token of 1 or 2 characters has to equal the term. A prefix match counts at
full weight in `R`, in `P` and in `exact`.
`name = round6(1 + 2 * (R + P / 2) / 1.5 + 4 * exact)`, where `exact` is 1 when `R == 1` and `P == 1`. So a name that
holds none of the terms gets 1.0, a name that holds all of them and nothing else gets 7.0, and the name that says more than
the query (`validatePrice` for `price`) or less (`parse` for `parse other`) lands between. 1.0 for an empty query and for a
block with no name or a name of only stop words. 05 §42 records why this shape and the variants it beat, and §47 the
prefix rule and the variants it beat.

BM25 (`rank/bm25.rs`): `K1 = 1.5`, `B = 0.5`, standard Okapi formula, `round6` to 6dp for
cross-platform f64 stability (`DP = 1_000_000.0`) — keep this rounding.

`node_type_boost` (`rank/boost.rs`): Function 1.5; Class/Method/Constructor 1.4; Interface/Struct
1.3; TypeAlias/Enum 1.2; Variable 1.1; Property/Field/Macro 1.0; Module/Impl 0.9; Other 0.7 (§47 moved Impl from 1.1).

## Dedup and snippets

Both read the byte spans the analyzers fill ([ADR 0005](../adr/0005-hits-and-blocks-are-identified-by-byte-spans-and-blocks-form-a-tree.md)); neither reads line numbers or compares text.

`dedup_blocks` (`dedup.rs`) walks the ranked blocks best first and drops a block that shares more than half of the smaller span's bytes with a block already kept in the same file, then renumbers `rank`. The spans of one analyzed file nest or are disjoint, so this drops a block inside a kept block, equal to it, or holding it, and keeps two blocks that share a line but no byte (`const a = 1; const b = 2;`). A member (a block with a `parent`) never duplicates the top-level block that strictly encloses it, in either rank order. A block with an empty span overlaps nothing. A segment tree over the kept spans' ends keeps the walk near-linear (`tests/scaling.rs`).

The snippet (`output/agent.rs`) shows the matched lines of a block, at most `--snippet-lines` (default 1, up to 3), each cut to 160 characters, with the rest counted (`...and 2 more matching lines`). A matched line is left out when its text lies wholly inside the block's `signature_span`, because the signature column already shows it. With `--snippet-lines 0` the column is `null`, so a variable or type alias row, which has no signature, shows no declaration text.

## Which rows are printed

`budget.rs` cuts the ranked, deduplicated list in three steps, in this order. Core holds none of them; it returns every ranked block.

1. **Relevance cut** (`--min-score`, default 0.3). Keep the leading rows whose `final_score` is at least `min_score * top`, where `top` is the first row's score. The scores fall down the list, so the kept rows are a prefix. A score equal to the threshold is kept, so rows that tie with the top survive `--min-score 1`. A top score of 0 keeps every row (the threshold is 0). `--min-score 0` turns the cut off.
2. **Floor.** The first eight rows are kept whatever their score, or all of them when there are fewer. The floor lifts the relevance cut only.
3. **Row cap** (`--max-results`, default 20). Applies to what is left and wins over the floor: `--max-results 1` prints one row.
4. **Token cap** (`--max-tokens`, default 20,000). The longest prefix whose printed rows fit; a row larger than the whole cap leaves no rows. It counts the printed rows, so it sees the snippet length.

When any rule dropped rows the response is truncated: `totalCount` is the number of ranked blocks and the help line names the flag for each rule that cut: `showing 4 of 37; --min-score 0 or --max-results N for more, or refine with +term/-term`, or `--max-tokens N` when the tokens ran out before the rows. The wording is the output layer's (`truncation_guidance`); core carries `total_blocks_before_truncation` and `blocks_cut_by_relevance` only.

Why these defaults: [05 §46](../spec/05-research-and-decisions.md). To get the output of versions before the change, run `--min-score 0 --max-results 50 --snippet-lines 3`.

Known gap: there is no `--offset`, so rows past the cap or the cut are reached by `--min-score 0` and `--max-results N`, or by a narrower query (`+term`, `-term`, a path), not by paging.

## Determinism and bounds

Resolved findings, kept so they are not reopened:

1. **#7 — a bad `regex:` term fails the whole search.** Every term, excluded ones too, compiles before any file is
   read (05 §18).
2. **#9 — total-order tie-break.** The six sort keys above leave no ties, so the order never depends on input order.
3. **#10 — the file walk is sorted by file name**, so the set of files reached under the file ceiling is reproducible.
4. **#11 — resource bounds.** The text search examines at most 100,000 files and keeps counting past it, reads at most
   4 MiB per file and keeps 4096 bytes per line; `--min-score`, `--max-results` and `--max-tokens` cut the output; `--timeout` stops
   the work. See [ADR 0004](../adr/0004-search-stays-stateless-and-every-stage-has-a-stated-bound.md).

Open:

- **#16 — no cap on one term's or the query's length.** `LIMIT = 64` bounds term *count* only. Decide whether a length
  cap is needed, and which error reports it (a sibling to `TooManyTerms`, not a repurposing of it).

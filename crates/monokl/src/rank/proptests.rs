use camino::{Utf8Path, Utf8PathBuf};
use proptest::prelude::*;

use super::*;

const KINDS: [SymbolKind; 15] = [
    SymbolKind::Function,
    SymbolKind::Method,
    SymbolKind::Constructor,
    SymbolKind::Class,
    SymbolKind::Struct,
    SymbolKind::Enum,
    SymbolKind::Interface,
    SymbolKind::TypeAlias,
    SymbolKind::Property,
    SymbolKind::Field,
    SymbolKind::Variable,
    SymbolKind::Module,
    SymbolKind::Impl,
    SymbolKind::Macro,
    SymbolKind::Other,
];

const TERMS: [&str; 3] = ["user", "name", "foo"];

fn input_block() -> impl Strategy<Value = (CodeBlock, Vec<String>)> {
    (
        prop::sample::select(vec!["a.ts", "a.b.ts", "a/z.ts", "b.ts"]),
        0usize..4,
        0usize..4,
        prop::sample::select(KINDS.to_vec()),
        prop::sample::select(vec!["", "a()", "b()"]),
        prop::collection::vec(0usize..6, 0..3),
        prop::collection::vec(prop::sample::select(vec!["user", "name", "foo", "other"]), 0..5),
    )
        .prop_map(|(file, line_start, line_end, node_kind, code, matched_lines, tokens)| {
            let block = CodeBlock {
                file: Utf8PathBuf::from(file),
                line_start,
                line_end,
                node_kind,
                code: code.to_string(),
                symbol_signature: None,
                matched_lines,
                matched_keywords: Vec::new(),
            };
            (block, tokens.into_iter().map(String::from).collect())
        })
}

fn df_strategy() -> impl Strategy<Value = HashMap<String, usize>> {
    prop::collection::vec(0usize..8, TERMS.len())
        .prop_map(|counts| TERMS.iter().map(ToString::to_string).zip(counts).collect())
}

fn terms_strategy() -> impl Strategy<Value = Vec<&'static str>> {
    prop::collection::vec(prop::sample::select(TERMS.to_vec()), 0..4)
}

fn components(path: &Utf8Path) -> Vec<&str> {
    path.components().map(|c| c.as_str()).collect()
}

/// Whether `a` sorts strictly before `b`, written out key by key rather than as a comparator chain.
fn precedes(a: &RankedBlock, b: &RankedBlock) -> bool {
    // Adding 0.0 turns -0.0 into 0.0.
    let (score_a, score_b) = (a.final_score + 0.0, b.final_score + 0.0);
    if score_a.total_cmp(&score_b).is_ne() {
        return score_a.total_cmp(&score_b).is_gt();
    }
    sort_keys(a) < sort_keys(b)
}

fn sort_keys(r: &RankedBlock) -> (Vec<&str>, usize, usize, Option<usize>, &str) {
    let kind = KINDS.iter().position(|k| *k == r.block.node_kind);
    (components(&r.block.file), r.block.line_start, r.block.line_end, kind, &r.block.code)
}

/// Selection sort that always takes the earliest minimal element, so equal blocks keep their relative order.
fn reference_order(mut pool: Vec<RankedBlock>) -> Vec<RankedBlock> {
    let mut out = Vec::new();
    while !pool.is_empty() {
        let mut best = 0;
        for idx in 1..pool.len() {
            if precedes(&pool[idx], &pool[best]) {
                best = idx;
            }
        }
        out.push(pool.remove(best));
    }
    for (rank, block) in (1..).zip(&mut out) {
        block.rank = rank;
    }
    out
}

/// Identity of an input block, ignoring everything the ranker computes.
fn identity(b: &CodeBlock) -> (String, usize, usize, usize, String, Vec<usize>) {
    let kind = KINDS.iter().position(|k| *k == b.node_kind).unwrap_or(usize::MAX);
    (b.file.to_string(), b.line_start, b.line_end, kind, b.code.clone(), b.matched_lines.clone())
}

/// The six sort keys plus rank. Blocks equal on all six are indistinguishable here, so their relative order is free.
fn full_key(r: &RankedBlock) -> (String, usize, usize, usize, String, u64, usize) {
    let b = &r.block;
    let kind = KINDS.iter().position(|k| *k == b.node_kind).unwrap_or(usize::MAX);
    (b.file.to_string(), b.line_start, b.line_end, kind, b.code.clone(), r.final_score.to_bits(), r.rank)
}

proptest! {
    #[test]
    fn rank_blocks_matches_the_naive_reference(
        blocks in prop::collection::vec(input_block(), 0..12),
        terms in terms_strategy(),
        df in df_strategy(),
    ) {
        let ranked = rank_blocks(blocks.clone(), &terms, &df);
        prop_assert_eq!(ranked.len(), blocks.len());

        // The reference sorts the ranker's own scored blocks; scoring itself is covered by score tests.
        let mut input_ids: Vec<_> = blocks.iter().map(|(b, _)| identity(b)).collect();
        let mut output_ids: Vec<_> = ranked.iter().map(|r| identity(&r.block)).collect();
        input_ids.sort();
        output_ids.sort();
        prop_assert_eq!(output_ids, input_ids);

        let mut scrambled: Vec<RankedBlock> = ranked.iter().rev().cloned().collect();
        for r in &mut scrambled {
            r.rank = 0;
        }
        let expected = reference_order(scrambled);
        prop_assert_eq!(ranked.iter().map(full_key).collect::<Vec<_>>(), expected.iter().map(full_key).collect::<Vec<_>>());
    }

    #[test]
    fn rank_blocks_ignores_input_order_when_no_blocks_tie(
        blocks in prop::collection::vec(input_block(), 0..12),
        terms in terms_strategy(),
        df in df_strategy(),
        seed in any::<u64>(),
    ) {
        let key = |b: &CodeBlock| (b.file.clone(), b.line_start, b.line_end, KINDS.iter().position(|k| *k == b.node_kind), b.code.clone());
        let mut keys: Vec<_> = blocks.iter().map(|(b, _)| key(b)).collect();
        keys.sort();
        keys.dedup();
        prop_assume!(keys.len() == blocks.len());

        let mut shuffled = blocks.clone();
        // Deterministic Fisher-Yates driven by a splitmix-style generator, so failures replay from the seed alone.
        let mut state = seed;
        for idx in (1..shuffled.len()).rev() {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            let pick = usize::try_from((z ^ (z >> 31)) % (idx as u64 + 1)).unwrap_or(0);
            shuffled.swap(idx, pick);
        }

        let original = rank_blocks(blocks, &terms, &df);
        let permuted = rank_blocks(shuffled, &terms, &df);
        prop_assert_eq!(original.iter().map(full_key).collect::<Vec<_>>(), permuted.iter().map(full_key).collect::<Vec<_>>());
    }
}

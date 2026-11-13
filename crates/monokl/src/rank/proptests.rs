use camino::Utf8PathBuf;
use proptest::prelude::*;

use super::*;
use crate::types::SymbolKind;

const KINDS: [SymbolKind; 4] = [SymbolKind::Function, SymbolKind::Class, SymbolKind::Variable, SymbolKind::Other];

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
                name: None,
                matched_lines,
                parent: None,
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

/// The tie-break keys that follow `final_score`.
fn tie_keys(r: &RankedBlock) -> (&Utf8PathBuf, usize, usize, SymbolKind, &str) {
    (&r.block.file, r.block.line_start, r.block.line_end, r.block.node_kind, &r.block.code)
}

fn full_key(r: &RankedBlock) -> (Utf8PathBuf, usize, usize, SymbolKind, String, u64, usize) {
    let b = &r.block;
    (b.file.clone(), b.line_start, b.line_end, b.node_kind, b.code.clone(), r.final_score.to_bits(), r.rank)
}

proptest! {
    #[test]
    fn equal_scores_break_ties_by_file_line_kind_then_code(
        blocks in prop::collection::vec(input_block(), 0..12),
        terms in terms_strategy(),
        df in df_strategy(),
    ) {
        let ranked = rank_blocks(blocks, &terms, &df);
        for pair in ranked.windows(2) {
            prop_assert!(pair[0].final_score >= pair[1].final_score);
            if pair[0].final_score.total_cmp(&pair[1].final_score).is_eq() {
                prop_assert!(tie_keys(&pair[0]) <= tie_keys(&pair[1]));
            }
        }
    }

    #[test]
    fn rank_blocks_ignores_input_order(
        (blocks, shuffled) in prop::collection::vec(input_block(), 0..12)
            .prop_flat_map(|blocks| (Just(blocks.clone()), Just(blocks).prop_shuffle())),
        terms in terms_strategy(),
        df in df_strategy(),
    ) {
        let original = rank_blocks(blocks, &terms, &df);
        let permuted = rank_blocks(shuffled, &terms, &df);
        prop_assert_eq!(original.iter().map(full_key).collect::<Vec<_>>(), permuted.iter().map(full_key).collect::<Vec<_>>());
    }
}

use proptest::prelude::*;
use rstest::rstest;

use super::*;

fn ts(source: &str) -> usize {
    peak_cost(source, Syntax::TypeScript { jsx: false }, usize::MAX)
}

fn tsx(source: &str) -> usize {
    peak_cost(source, Syntax::TypeScript { jsx: true }, usize::MAX)
}

fn rust(source: &str) -> usize {
    peak_cost(source, Syntax::Rust, usize::MAX)
}

#[rstest]
#[case::empty("", 0)]
#[case::one_word("a", 1)]
#[case::words_add_up("a b c", 3)]
#[case::one_frame("()", 4)]
#[case::frames_nest("[[[]]]", 12)]
#[case::calls("f(g(h()))", 15)]
#[case::objects("{a:{b:{c:1}}}", 19)]
#[case::a_closed_frame_leaves_one("a(b)(c)(d)", 8)]
#[case::a_semicolon_ends_the_chain("a(b); c(d);", 6)]
#[case::a_comma_ends_the_chain("f(a, b, c)", 6)]
#[case::angle_brackets_survive_commas("A<B, C<D, E<F>>>", 11)]
#[case::unbalanced_closers_cost_one_each(")))]]]}}}", 9)]
#[case::unclosed_openers_stay_open("(((", 12)]
#[case::a_closer_of_the_wrong_kind_pops_nothing("([)]", 9)]
#[case::a_closer_with_no_opener("(]", 5)]
#[case::brackets_in_a_string_are_free("\"(((((((\"", 0)]
#[case::brackets_in_a_single_quoted_string_are_free("'[[[['", 0)]
#[case::brackets_in_a_line_comment_are_free("// (((((\n", 0)]
#[case::brackets_in_a_block_comment_are_free("/* ((( \n ((( */", 0)]
#[case::a_template_hides_its_text_not_its_substitutions("`((((${a}))))`", 5)]
#[case::templates_nest_through_substitutions("`${`${`${1}`}`}`", 13)]
#[case::a_regex_hides_its_brackets("x = /(((/", 2)]
#[case::a_regex_class_may_hold_a_slash("x = /[/(]/", 2)]
#[case::a_division_hides_nothing("x / ((( y", 15)]
#[case::an_escaped_quote_does_not_end_a_string("'\\'(((' ", 0)]
#[case::a_string_with_no_close_on_its_line_is_an_operator("x = 'it\n(((", 16)]
#[case::an_increment_ends_an_operand("a++ / b / c", 7)]
#[case::a_lines_end_ends_the_statement("a = 1\nb = 2\nc = 3", 3)]
#[case::a_continuation_word_does_not("x\ninstanceof y\nin z", 5)]
#[case::an_if_head_does_not_end_the_statement("if (a)\nif (b)\nif (c) x", 10)]
#[case::a_closing_brace_ends_the_statement("function a(){}function b(){}", 7)]
#[case::a_regex_after_a_block_hides_no_closers("{{{ x } /}}}}/ {{{{", 31)]
#[case::a_slash_after_a_tag_is_not_a_regex("a </b>(((", 18)]
#[case::a_carriage_return_ends_a_line_comment("// x\r(((", 12)]
#[case::a_line_separator_ends_a_line_comment("// x\u{2028}(((", 13)]
#[case::a_slash_after_a_contextual_word_may_divide("of / (((( / x", 20)]
#[case::a_slash_after_await_may_start_a_regex_that_hides_closers("await /)))))/ (((", 20)]
fn typescript_costs(#[case] source: &str, #[case] expected: usize) {
    assert_eq!(ts(source), expected);
}

#[rstest]
#[case::a_string_with_braces_is_code("const a = 'x {{{{{{{{{{'", 44, 3)]
#[case::a_string_with_tags_charges_each("const a = 'x <b><b><b>'", 6, 3)]
#[case::a_comment_with_braces_is_code("a // {{{{\n", 17, 1)]
#[case::closers_in_a_string_pop_nothing("((( ')' ')' ')' ", 12, 12)]
#[case::a_regex_with_braces_is_code("x = /{{{{/", 18, 2)]
#[case::template_text_with_braces_is_code("x = `{{{{`", 18, 2)]
#[case::a_closing_brace_does_not_end_the_statement_in_jsx("function a(){}function b(){}", 11, 7)]
fn jsx_text_can_hide_what_a_string_or_comment_cannot(#[case] source: &str, #[case] jsx: usize, #[case] plain: usize) {
    assert_eq!((tsx(source), ts(source)), (jsx, plain));
}

#[rstest]
#[case::empty("", 0)]
#[case::a_frame("fn a() {}", 7)]
#[case::angle_brackets_cost_four("A<A<A<u8>>>", 19)]
#[case::a_closure_pipe_survives_commas("|a, b| |c, d| x", 6)]
#[case::a_char_hides_its_bracket("'(' '{' ')'", 0)]
#[case::an_escaped_char_hides_its_quote("'\\'' '\\u{1F600}' '\\\\'", 0)]
#[case::a_lifetime_is_not_a_char("<'a, 'b>", 7)]
#[case::a_string_hides_its_brackets("\"((((\\\"((\"", 0)]
#[case::a_string_may_span_lines("\"(((\n(((\" x", 1)]
#[case::a_raw_string_hides_its_brackets("r#\"(((\" ((\"#", 0)]
#[case::a_raw_string_with_no_hashes("r\"(((\" x", 1)]
#[case::a_byte_char("b'(' x", 2)]
#[case::a_block_comment_nests("/* /* ( */ ( */ x", 1)]
#[case::a_line_comment_hides_brackets("// ((((\nx", 1)]
#[case::an_unterminated_string_hides_the_rest("\"(((", 0)]
#[case::a_closing_brace_ends_the_statement("fn a() {} fn b() {}", 7)]
#[case::match_arms("match x { A => { 1 } B => { 2 } }", 14)]
fn rust_costs(#[case] source: &str, #[case] expected: usize) {
    assert_eq!(rust(source), expected);
}

#[test]
fn the_scan_stops_at_the_limit_and_reports_the_cost_that_passed_it() {
    let source = "(".repeat(1_000_000);
    assert_eq!(peak_cost(&source, Syntax::TypeScript { jsx: false }, 100), 104);
    assert_eq!(peak_cost(&source, Syntax::Rust, 100), 104);
}

#[test]
fn a_source_over_the_cap_is_reported_with_its_depth_and_the_cap() {
    let source = "[".repeat(600);
    assert_eq!(check(&source, Syntax::Rust, 512), Err(Degradation::TooDeep { depth: 513, cap: 512 }));
    assert_eq!(check(&source[..512], Syntax::Rust, 512), Ok(()));
}

/// What a stack of matching brackets reaches, which no reading of the source can be below.
fn matched_depth(source: &str) -> usize {
    let (mut open, mut deepest) = (Vec::new(), 0);
    for byte in source.bytes() {
        match byte {
            b'(' | b'[' | b'{' => {
                open.push(byte);
                deepest = deepest.max(open.len());
            }
            b')' | b']' | b'}' => {
                let opener = match byte {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                if open.last() == Some(&opener) {
                    open.pop();
                }
            }
            _ => {}
        }
    }
    deepest
}

proptest! {
    #[test]
    fn bracket_soup_costs_at_least_its_matched_depth(source in "[()\\[\\]{}]{0,200}") {
        let floor = matched_depth(&source) * FRAME_COST;
        prop_assert!(ts(&source) >= floor);
        prop_assert!(tsx(&source) >= floor);
        prop_assert!(rust(&source) >= floor);
    }

    #[test]
    fn text_around_brackets_never_lowers_the_cost(
        before in "[a-z ,;.]{0,20}",
        brackets in "[(\\[{]{1,40}",
        after in "[a-z ,;.\\n]{0,20}",
    ) {
        let around = format!("{before}{brackets}{after}");
        prop_assert!(ts(&around) >= ts(&brackets));
        prop_assert!(rust(&around) >= rust(&brackets));
    }

    #[test]
    fn the_scan_ends_on_any_input(source in "\\PC{0,200}") {
        // No byte costs more than a frame, so the cost is at most that of every byte being one.
        let most = source.len() * FRAME_COST;
        prop_assert!(ts(&source) <= most && tsx(&source) <= most && rust(&source) <= most);
    }
}

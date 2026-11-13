//! Pure extraction for one Rust file.
//!
//! The parse owns its syntax tree. Everything leaving this module is owned data, so no tree node escapes.

use camino::Utf8Path;
use ra_ap_syntax::ast::{self, AstNode, HasAttrs, HasModuleItem, HasName};
use ra_ap_syntax::{Edition, NodeOrToken, SourceFile, SyntaxKind, SyntaxNode, TextRange};

use crate::analysis::Degradation;
use crate::analysis::nesting::{self, Syntax};
use crate::analysis::parse_stack::on_parse_stack;
use crate::types::{CodeBlock, ParentContext, SymbolKind};

/// The fewest lines an `impl` or trait method needs to get a block of its own. A shorter one is a stub or accessor
/// that the block holding it already shows whole.
const MIN_MEMBER_LINES: usize = 3;

/// The longest source that is parsed. Parse time and memory grow with the length, not with the stack, so this is chosen
/// from measurements of a whole `monokl search` over one generated file: at 4 MB it takes 181 ms and peaks at 151 MB of
/// resident memory, about 37 bytes per source byte.
pub(super) const MAX_PARSED_SOURCE_LEN: usize = 4 << 20;

/// The deepest nesting parsed, in levels of brackets or the equivalent in other nesting (see [`nesting::peak_cost`]).
///
/// `ra_ap_syntax` hands a finished tree to a shared background thread to drop, one with the default 2 MiB stack, and a
/// drop recurses once per level of the tree. That overflows at 4,320 levels in a build without optimizations (485 bytes
/// a level) and 26,240 optimized, and overflowing aborts the process. A level of the tree costs at least one unit, and
/// a unit is a quarter of a level here, so the 1,024 units of the cap bound the tree. Generic arguments are the deepest
/// per unit, at 1.3 tree levels; the tests build 32 nestings at the cap and a fuzzer builds thousands more, and none is
/// over 2,150 levels, half of what the unoptimized drop holds. The parser's time is quadratic in
/// the depth, so the cap also bounds a worst-case parse to milliseconds.
pub(crate) const MAX_NESTING_LEVELS: usize = 256;

/// Everything extracted from one file, before it is wrapped in a `FileAnalysis`.
#[derive(Debug, Default)]
pub(super) struct Extraction {
    pub(super) blocks: Vec<CodeBlock>,
    pub(super) test_blocks: Vec<CodeBlock>,
    pub(super) had_parse_errors: bool,
    pub(super) degradation: Option<Degradation>,
}

impl Extraction {
    /// The result for a file that could not be analyzed: no data, flagged as having errors and the reason.
    fn degraded(degradation: Degradation) -> Self {
        Self { had_parse_errors: true, degradation: Some(degradation), ..Self::default() }
    }
}

/// Extracts the blocks of `source`.
///
/// A file too long or nested too
/// deeply for the parse stack and the tree drop is [`Extraction::degraded`] without being parsed, and so is one whose
/// parse or extraction panics.
pub(super) fn extract(path: &Utf8Path, source: &str) -> Extraction {
    if source.len() > MAX_PARSED_SOURCE_LEN {
        return Extraction::degraded(Degradation::TooLarge { bytes: source.len(), cap: MAX_PARSED_SOURCE_LEN });
    }
    if let Err(degradation) = nesting::check(source, Syntax::Rust, MAX_NESTING_LEVELS) {
        return Extraction::degraded(degradation);
    }
    on_parse_stack(|| parse_and_extract(path, source)).unwrap_or_else(Extraction::degraded)
}

fn parse_and_extract(path: &Utf8Path, source: &str) -> Extraction {
    let parse = SourceFile::parse(source, Edition::CURRENT);
    let had_parse_errors = !parse.errors().is_empty();
    // The root of a parse is always a source file, so the cast cannot fail; the let-else keeps that out of a panic.
    let Some(file) = SourceFile::cast(parse.syntax_node()) else {
        return Extraction::degraded(Degradation::Panicked);
    };
    let mut walker = Walker { path, source, lines: LineIndex::new(source), out: Extraction::default() };
    let file_is_test = file.attrs().any(|attr| attr.excl_token().is_some() && is_test_attr(&attr));
    walker.items(file.items(), None, file_is_test);
    Extraction { had_parse_errors, ..walker.out }
}

/// Parses and extracts on the calling thread, for a test that supplies its own stack.
#[cfg(all(test, feature = "lang-ts"))]
pub(super) fn parse_here(source: &str) {
    drop(parse_and_extract(Utf8Path::new("a.rs"), source));
}

/// Maps byte offsets to 1-based line numbers.
struct LineIndex {
    newlines: Vec<u32>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        Self {
            newlines: source
                .bytes()
                .enumerate()
                .filter(|&(_, byte)| byte == b'\n')
                .filter_map(|(offset, _)| u32::try_from(offset).ok())
                .collect(),
        }
    }

    fn line(&self, offset: u32) -> usize {
        self.newlines.partition_point(|&newline| newline < offset) + 1
    }
}

struct Walker<'s> {
    path: &'s Utf8Path,
    source: &'s str,
    lines: LineIndex,
    out: Extraction,
}

impl Walker<'_> {
    fn text(&self, range: TextRange) -> &str {
        self.source.get(usize::from(range.start())..usize::from(range.end())).unwrap_or_default()
    }

    fn line_of(&self, offset: ra_ap_syntax::TextSize) -> usize {
        self.lines.line(u32::from(offset))
    }

    /// Walks the items of a file or an inline module. Each becomes a block, then its own members do. `in_test` marks
    /// the items as test code when an enclosing item or file is.
    fn items(&mut self, items: impl Iterator<Item = ast::Item>, holder: Option<&ParentContext>, in_test: bool) {
        for item in items {
            self.item(&item, holder, in_test);
        }
    }

    fn item(&mut self, item: &ast::Item, holder: Option<&ParentContext>, in_test: bool) {
        let Some((kind, name)) = describe(item) else { return };
        let in_test = in_test || item.attrs().any(|attr| is_test_attr(&attr));
        self.record(item.syntax(), kind, name.clone(), holder, in_test);

        let Some(name) = name else { return };
        let holder = ParentContext { kind, name };
        match item {
            ast::Item::Module(module) => {
                let Some(list) = module.item_list() else { return };
                self.items(list.items(), Some(&holder), in_test);
            }
            ast::Item::Impl(imp) => self.methods(imp.assoc_item_list(), &holder, in_test),
            ast::Item::Trait(item_trait) => self.methods(item_trait.assoc_item_list(), &holder, in_test),
            ast::Item::AsmExpr(_)
            | ast::Item::Const(_)
            | ast::Item::Enum(_)
            | ast::Item::ExternBlock(_)
            | ast::Item::ExternCrate(_)
            | ast::Item::Fn(_)
            | ast::Item::MacroCall(_)
            | ast::Item::MacroDef(_)
            | ast::Item::MacroRules(_)
            | ast::Item::Static(_)
            | ast::Item::Struct(_)
            | ast::Item::TypeAlias(_)
            | ast::Item::Union(_)
            | ast::Item::Use(_) => {}
        }
    }

    /// A block for each method with a body in an `impl` or trait, at least [`MIN_MEMBER_LINES`] long unless it is test
    /// code. Any other member (a bodiless method, a constant, a type, a macro call) gets one only when it is itself
    /// marked as test code, so its matches do not go to the holder.
    fn methods(&mut self, list: Option<ast::AssocItemList>, holder: &ParentContext, in_test: bool) {
        let Some(list) = list else { return };
        for member in list.assoc_items() {
            let marked = member.attrs().any(|attr| is_test_attr(&attr));
            let (kind, name, node) = match &member {
                ast::AssocItem::Fn(function) if function.body().is_some() => {
                    if function.name().is_none() {
                        continue;
                    }
                    let in_test = in_test || marked;
                    let range = function.syntax().text_range();
                    // A short test method still gets a block, so a search can tell its matches from the holder's.
                    if !in_test && self.line_of(range.end()) - self.line_of(range.start()) + 1 < MIN_MEMBER_LINES {
                        continue;
                    }
                    self.record(
                        function.syntax(),
                        SymbolKind::Method,
                        declared(function.name()),
                        Some(holder),
                        in_test,
                    );
                    continue;
                }
                ast::AssocItem::Fn(function) => (SymbolKind::Method, declared(function.name()), function.syntax()),
                ast::AssocItem::Const(item) => (SymbolKind::Variable, declared(item.name()), item.syntax()),
                ast::AssocItem::TypeAlias(item) => (SymbolKind::TypeAlias, declared(item.name()), item.syntax()),
                ast::AssocItem::MacroCall(call) => (SymbolKind::Macro, None, call.syntax()),
            };
            if marked {
                self.record(node, kind, name, Some(holder), true);
            }
        }
    }

    /// Records one item's block, with attributes and doc comments, in `test_blocks` when it is test code.
    fn record(
        &mut self,
        node: &SyntaxNode,
        kind: SymbolKind,
        name: Option<String>,
        holder: Option<&ParentContext>,
        in_test: bool,
    ) {
        let signature = self.signature(node);
        let range = node.text_range();
        let block = CodeBlock {
            file: self.path.to_owned(),
            line_start: self.line_of(range.start()),
            line_end: self.line_of(range.end()),
            node_kind: kind,

            name,
            code: self.text(range).to_owned(),
            symbol_signature: Some(signature),
            matched_lines: Vec::new(),
            parent: holder.cloned(),
        };
        if in_test {
            self.out.test_blocks.push(block);
        } else {
            self.out.blocks.push(block);
        }
    }

    /// The item's header on one line: everything from its visibility or first keyword up to its body or initializer,
    /// without attributes and doc comments. `pub fn f<T>(a: &str) -> T where T: Clone`.
    fn signature(&self, node: &SyntaxNode) -> String {
        let mut start = None;
        let mut end = node.text_range().end();
        for element in node.children_with_tokens() {
            let kind = element.kind();
            if start.is_none() {
                if matches!(kind, SyntaxKind::ATTR | SyntaxKind::COMMENT | SyntaxKind::WHITESPACE) {
                    continue;
                }
                start = Some(element.text_range().start());
            }
            let is_body = match &element {
                NodeOrToken::Node(_) => matches!(
                    kind,
                    SyntaxKind::BLOCK_EXPR
                        | SyntaxKind::RECORD_FIELD_LIST
                        | SyntaxKind::VARIANT_LIST
                        | SyntaxKind::ASSOC_ITEM_LIST
                        | SyntaxKind::ITEM_LIST
                        | SyntaxKind::EXTERN_ITEM_LIST
                        | SyntaxKind::TOKEN_TREE
                ),
                NodeOrToken::Token(_) => kind == SyntaxKind::EQ,
            };
            if is_body {
                end = element.text_range().start();
                break;
            }
        }
        let Some(start) = start else { return String::new() };
        collapse(self.text(TextRange::new(start, end.max(start)))).trim_end_matches([';', ',']).trim_end().to_owned()
    }
}

/// The kind and the name of an item that gets a block, `None` for one that does not (a `use`, an `extern crate`, an
/// out-of-line module). The name is `None` for an item that has none (a macro call, an `extern` block) or that the parse
/// recovered without one.
fn describe(item: &ast::Item) -> Option<(SymbolKind, Option<String>)> {
    Some(match item {
        ast::Item::Fn(it) => (SymbolKind::Function, declared(it.name())),
        ast::Item::Struct(it) => (SymbolKind::Struct, declared(it.name())),
        ast::Item::Union(it) => (SymbolKind::Struct, declared(it.name())),
        ast::Item::Enum(it) => (SymbolKind::Enum, declared(it.name())),
        ast::Item::Trait(it) => (SymbolKind::Interface, declared(it.name())),
        ast::Item::TypeAlias(it) => (SymbolKind::TypeAlias, declared(it.name())),
        ast::Item::Const(it) => (SymbolKind::Variable, declared(it.name())),
        ast::Item::Static(it) => (SymbolKind::Variable, declared(it.name())),
        ast::Item::MacroRules(it) => (SymbolKind::Macro, declared(it.name())),
        ast::Item::MacroDef(it) => (SymbolKind::Macro, declared(it.name())),
        // A module declared in another file has no body to split.
        ast::Item::Module(it) => (it.item_list().is_some()).then(|| (SymbolKind::Module, declared(it.name())))?,
        ast::Item::Impl(it) => {
            let target = it.self_ty().map(|ty| collapse(&ty.syntax().text().to_string()));
            let name = match (it.trait_(), target) {
                (Some(implemented), Some(target)) => {
                    Some(format!("{} for {target}", collapse(&implemented.syntax().text().to_string())))
                }
                (None, target) => target,
                (Some(_), None) => None,
            };
            (SymbolKind::Impl, name)
        }
        // These declare no name of their own, but what they hold (`thread_local!`'s statics, an `extern` block's
        // functions) is searchable only through a block.
        ast::Item::MacroCall(_) => (SymbolKind::Macro, None),
        ast::Item::ExternBlock(_) => (SymbolKind::Other, None),
        ast::Item::AsmExpr(_) | ast::Item::ExternCrate(_) | ast::Item::Use(_) => return None,
    })
}

/// The text of a declared name, `None` when the parse recovered the item without one.
fn declared(name: Option<ast::Name>) -> Option<String> {
    name.map(|name| name.text().to_owned())
}

/// `text` on one line: every run of whitespace becomes one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether an attribute marks the item it decorates as test-only: `#[test]`, `#[tokio::test]`, or a `#[cfg(...)]`
/// that holds only when `test` does.
fn is_test_attr(attr: &ast::Attr) -> bool {
    match attr.meta() {
        Some(ast::Meta::CfgMeta(cfg)) => cfg.cfg_predicate().is_some_and(|predicate| implies_test(&predicate)),
        Some(ast::Meta::PathMeta(meta)) => meta.path().is_some_and(|path| last_segment_is_test(&path)),
        Some(ast::Meta::TokenTreeMeta(meta)) => meta.path().is_some_and(|path| last_segment_is_test(&path)),
        Some(ast::Meta::CfgAttrMeta(_) | ast::Meta::KeyValueMeta(_) | ast::Meta::UnsafeMeta(_)) | None => false,
    }
}

fn last_segment_is_test(path: &ast::Path) -> bool {
    path.segment().and_then(|segment| segment.name_ref()).is_some_and(|name| name.text() == "test")
}

/// Whether a `cfg` predicate can only hold in a test build: `test`, an `all` with such a part, or an `any` of only such
/// parts. A `not` never does.
fn implies_test(predicate: &ast::CfgPredicate) -> bool {
    match predicate {
        ast::CfgPredicate::CfgAtom(atom) => {
            atom.eq_token().is_none() && atom.ident_token().is_some_and(|ident| ident.text() == "test")
        }
        ast::CfgPredicate::CfgComposite(composite) => {
            let keyword = composite.keyword();
            let mut parts = composite.cfg_predicates();
            match keyword.as_ref().map(ra_ap_syntax::SyntaxToken::text) {
                Some("all") => parts.any(|part| implies_test(&part)),
                Some("any") => {
                    let parts: Vec<_> = parts.collect();
                    !parts.is_empty() && parts.iter().all(implies_test)
                }
                _ => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_are_one_based_and_count_preceding_newlines() {
        let index = LineIndex::new("a\nbb\n\nc");
        assert_eq!([0, 1, 2, 4, 5, 6].map(|offset| index.line(offset)), [1, 1, 2, 2, 3, 4]);
    }
}

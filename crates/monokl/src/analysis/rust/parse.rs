//! Pure extraction for one Rust file.
//!
//! The parse owns its syntax tree. Everything leaving this module is owned data, so no tree node escapes.

use std::thread;

use camino::Utf8Path;
use ra_ap_syntax::ast::{self, AstNode, HasAttrs, HasModuleItem, HasName, HasVisibility};
use ra_ap_syntax::{Edition, NodeOrToken, SourceFile, SyntaxKind, SyntaxNode, TextRange, WalkEvent};

use crate::analysis::{AnalysisProfile, Degradation};
use crate::types::{CodeBlock, ParentContext, SymbolEntry, SymbolKind, Visibility};

/// The fewest lines an `impl` or trait method needs to get a block of its own. A shorter one is a stub or accessor
/// that the block holding it already shows whole.
const MIN_MEMBER_LINES: usize = 3;

/// Stack reserved per byte of source. A stack overflow aborts the process, so the stack must be big enough before the
/// parse starts. Each level of the parser's recursion consumes at least one byte of source, and the costliest level
/// measured (a nested `(`, `{` or unary `-`) uses under 1.6 KB of stack optimized and under 4 KB unoptimized.
///
/// The parser's time grows with the square of the nesting depth (20,000 nested parentheses take a second), so a file of
/// the longest parsed length made only of them takes minutes. Only a pathological file reaches that.
///
/// `debug_assertions` stands in for "unoptimized", which `cfg!` cannot ask for directly.
const STACK_PER_SOURCE_BYTE: usize = if cfg!(debug_assertions) { 12 << 10 } else { 4 << 10 };

/// Stack for what does not grow with the source.
const BASE_STACK: usize = 8 << 20;

/// The most stack one parse reserves. Only touched pages are committed; a parse whose stack cannot be reserved is
/// reported as degraded.
const MAX_STACK: usize = 4 << 30;

/// The longest source that is parsed; a longer one could need more than [`MAX_STACK`].
pub(super) const MAX_PARSED_SOURCE_LEN: usize = (MAX_STACK - BASE_STACK) / STACK_PER_SOURCE_BYTE;

/// The deepest syntax tree that is kept. `ra_ap_syntax` hands a finished tree to a shared background thread to drop, one
/// with the default 2 MiB stack, and a drop recurses once per level of the tree: about 350 bytes a level unoptimized
/// and 130 optimized. Overflowing that stack aborts the process, so this stays at half of what fits.
pub(super) const MAX_TREE_DEPTH: usize = if cfg!(debug_assertions) { 2_000 } else { 8_000 };

/// Everything extracted from one file, before it is wrapped in a `FileAnalysis`.
#[derive(Debug, Default)]
pub(super) struct Extraction {
    pub(super) symbols: Vec<SymbolEntry>,
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

/// Extracts what `profile` asks for from `source`.
///
/// Rust files have no dependency records, so the dependencies profile reads nothing. A file too long to be sure of its
/// stack, or one whose parse or extraction panics, yields [`Extraction::degraded`] with the reason.
pub(super) fn extract(path: &Utf8Path, source: &str, profile: AnalysisProfile) -> Extraction {
    if profile == AnalysisProfile::Dependencies {
        return Extraction::default();
    }
    if source.len() > MAX_PARSED_SOURCE_LEN {
        return Extraction::degraded(Degradation::TooLarge { bytes: source.len(), cap: MAX_PARSED_SOURCE_LEN });
    }
    let stack_size = BASE_STACK + source.len() * STACK_PER_SOURCE_BYTE;
    guarded(stack_size, || parse_and_extract(path, source, profile))
}

/// Runs `run` on its own thread with `stack_size` bytes of stack. A panic, or a thread that cannot be started, yields
/// [`Extraction::degraded`] with the reason.
fn guarded(stack_size: usize, run: impl FnOnce() -> Extraction + Send) -> Extraction {
    thread::scope(|scope| {
        let builder = thread::Builder::new().name("monokl-parse".to_owned()).stack_size(stack_size);
        let Ok(handle) = builder.spawn_scoped(scope, run) else {
            return Extraction::degraded(Degradation::ThreadUnavailable);
        };
        // The closure's output is discarded on a panic, so no torn state is observable afterwards.
        handle.join().unwrap_or_else(|_| Extraction::degraded(Degradation::Panicked))
    })
}

fn parse_and_extract(path: &Utf8Path, source: &str, profile: AnalysisProfile) -> Extraction {
    let parse = SourceFile::parse(source, Edition::CURRENT);
    let depth = tree_depth(&parse.syntax_node());
    if depth > MAX_TREE_DEPTH {
        // The tree is leaked rather than dropped: its drop would run on the shared thread, where it would overflow.
        std::mem::forget(parse);
        return Extraction::degraded(Degradation::TooDeep { depth, cap: MAX_TREE_DEPTH });
    }
    let had_parse_errors = !parse.errors().is_empty();
    // The root of a parse is always a source file, so the cast cannot fail; the let-else keeps that out of a panic.
    let Some(file) = SourceFile::cast(parse.syntax_node()) else {
        return Extraction::degraded(Degradation::Panicked);
    };
    let mut walker = Walker { path, source, profile, lines: LineIndex::new(source), out: Extraction::default() };
    let file_is_test = file.attrs().any(|attr| attr.excl_token().is_some() && is_test_attr(&attr));
    walker.items(file.items(), None, file_is_test);
    Extraction { had_parse_errors, ..walker.out }
}

/// The number of levels of `root`'s tree, the root being the first.
fn tree_depth(root: &SyntaxNode) -> usize {
    let (mut depth, mut deepest) = (0usize, 0usize);
    for event in root.preorder() {
        match event {
            WalkEvent::Enter(_) => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            WalkEvent::Leave(_) => depth = depth.saturating_sub(1),
        }
    }
    deepest
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

/// The item whose members are being walked: what each member block takes as its parent.
struct Holder {
    parent: ParentContext,
    /// The type that owns the members: an `impl`'s target or a trait's name; `None` for a module.
    owner: Option<String>,
    /// The trait an `impl` implements.
    trait_impl: Option<String>,
}

/// An item or method as the walk found it.
struct Declared<'a> {
    node: &'a SyntaxNode,
    kind: SymbolKind,
    /// `None` for an item that has no name (a macro call, an `extern` block) or that the parse recovered without one.
    name: Option<&'a str>,
    visibility: Option<ast::Visibility>,
}

struct Walker<'s> {
    path: &'s Utf8Path,
    source: &'s str,
    profile: AnalysisProfile,
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
    fn items(&mut self, items: impl Iterator<Item = ast::Item>, holder: Option<&Holder>, in_test: bool) {
        for item in items {
            self.item(&item, holder, in_test);
        }
    }

    fn item(&mut self, item: &ast::Item, holder: Option<&Holder>, in_test: bool) {
        let Some((kind, name)) = describe(item) else { return };
        let in_test = in_test || item.attrs().any(|attr| is_test_attr(&attr));
        let declared = Declared { node: item.syntax(), kind, name: name.as_deref(), visibility: item_visibility(item) };
        self.record(&declared, holder, in_test);

        let Some(name) = name else { return };
        let line = self.line_of(item.syntax().text_range().start());
        match item {
            ast::Item::Module(module) => {
                let Some(list) = module.item_list() else { return };
                let holder = Holder { parent: ParentContext { kind, name, line }, owner: None, trait_impl: None };
                self.items(list.items(), Some(&holder), in_test);
            }
            ast::Item::Impl(imp) => {
                let trait_impl = imp.trait_().map(|ty| collapse(&ty.syntax().text().to_string()));
                let owner = imp.self_ty().map(|ty| collapse(&ty.syntax().text().to_string()));
                let holder = Holder { parent: ParentContext { kind, name, line }, owner, trait_impl };
                self.methods(imp.assoc_item_list(), &holder, in_test);
            }
            ast::Item::Trait(item_trait) => {
                let holder =
                    Holder { owner: Some(name.clone()), parent: ParentContext { kind, name, line }, trait_impl: None };
                self.methods(item_trait.assoc_item_list(), &holder, in_test);
            }
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
    fn methods(&mut self, list: Option<ast::AssocItemList>, holder: &Holder, in_test: bool) {
        let Some(list) = list else { return };
        for member in list.assoc_items() {
            let marked = member.attrs().any(|attr| is_test_attr(&attr));
            let (kind, name, visibility, node) = match &member {
                ast::AssocItem::Fn(function) if function.body().is_some() => {
                    let Some(name) = function.name() else { continue };
                    let in_test = in_test || marked;
                    let range = function.syntax().text_range();
                    // A short test method still gets a block, so a search can tell its matches from the holder's.
                    if !in_test && self.line_of(range.end()) - self.line_of(range.start()) + 1 < MIN_MEMBER_LINES {
                        continue;
                    }
                    let declared = Declared {
                        node: function.syntax(),
                        kind: SymbolKind::Method,
                        name: Some(name.text()),
                        visibility: function.visibility(),
                    };
                    self.record(&declared, Some(holder), in_test);
                    continue;
                }
                ast::AssocItem::Fn(function) => {
                    (SymbolKind::Method, function.name(), function.visibility(), function.syntax())
                }
                ast::AssocItem::Const(item) => (SymbolKind::Variable, item.name(), item.visibility(), item.syntax()),
                ast::AssocItem::TypeAlias(item) => {
                    (SymbolKind::TypeAlias, item.name(), item.visibility(), item.syntax())
                }
                ast::AssocItem::MacroCall(call) => (SymbolKind::Macro, None, None, call.syntax()),
            };
            if marked {
                let declared = Declared { node, kind, name: name.as_ref().map(ast::Name::text), visibility };
                self.record(&declared, Some(holder), true);
            }
        }
    }

    /// Records one item: its symbol unless it is test code, and its block, with attributes and doc comments, under the
    /// full profile. `name` is `None` for an item without one, which leaves it without a symbol.
    fn record(&mut self, declared: &Declared<'_>, holder: Option<&Holder>, in_test: bool) {
        let Declared { node, kind, name, ref visibility } = *declared;
        let signature = self.signature(node);
        let range = node.text_range();
        let (line_start, line_end) = (self.line_of(range.start()), self.line_of(range.end()));
        // A trait's methods and the methods implementing one are as visible as the trait, which they do not say.
        let inherits_visibility =
            holder.is_some_and(|h| h.trait_impl.is_some() || h.parent.kind == SymbolKind::Interface);
        if let (Some(name), false) = (name, in_test) {
            self.out.symbols.push(SymbolEntry {
                name: name.to_owned(),
                kind,
                line: line_start,
                signature: Some(signature.clone()),
                owner: holder.and_then(|holder| holder.owner.clone()),
                trait_impl: holder.and_then(|holder| holder.trait_impl.clone()),
                visibility: (!inherits_visibility).then(|| self.visibility(visibility.as_ref())),
                kind_detail: None,
            });
        }
        if self.profile != AnalysisProfile::Full {
            return;
        }
        let block = CodeBlock {
            file: self.path.to_owned(),
            line_start,
            line_end,
            node_kind: kind,
            code: self.text(range).to_owned(),
            symbol_signature: Some(signature),
            matched_lines: Vec::new(),
            matched_keywords: Vec::new(),
            parent: holder.map(|holder| holder.parent.clone()),
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

    fn visibility(&self, visibility: Option<&ast::Visibility>) -> Visibility {
        let Some(visibility) = visibility else { return Visibility::Private };
        let text = collapse(self.text(visibility.syntax().text_range()));
        let text = text.replace(' ', "");
        match text.as_str() {
            "pub" => Visibility::Public,
            "pub(crate)" => Visibility::Crate,
            _ => Visibility::Module,
        }
    }
}

/// The kind and the name of an item that gets a block, `None` for one that does not (a `use`, an `extern crate`, an
/// out-of-line module). The name is `None` for an item that has none (a macro call, an `extern` block) or that the parse
/// recovered without one.
fn describe(item: &ast::Item) -> Option<(SymbolKind, Option<String>)> {
    let named = |name: Option<ast::Name>| name.map(|name| name.text().to_owned());
    Some(match item {
        ast::Item::Fn(it) => (SymbolKind::Function, named(it.name())),
        ast::Item::Struct(it) => (SymbolKind::Struct, named(it.name())),
        ast::Item::Union(it) => (SymbolKind::Struct, named(it.name())),
        ast::Item::Enum(it) => (SymbolKind::Enum, named(it.name())),
        ast::Item::Trait(it) => (SymbolKind::Interface, named(it.name())),
        ast::Item::TypeAlias(it) => (SymbolKind::TypeAlias, named(it.name())),
        ast::Item::Const(it) => (SymbolKind::Variable, named(it.name())),
        ast::Item::Static(it) => (SymbolKind::Variable, named(it.name())),
        ast::Item::MacroRules(it) => (SymbolKind::Macro, named(it.name())),
        ast::Item::MacroDef(it) => (SymbolKind::Macro, named(it.name())),
        // A module declared in another file has no body to split.
        ast::Item::Module(it) => (it.item_list().is_some()).then(|| (SymbolKind::Module, named(it.name())))?,
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

fn item_visibility(item: &ast::Item) -> Option<ast::Visibility> {
    match item {
        ast::Item::Fn(it) => it.visibility(),
        ast::Item::Struct(it) => it.visibility(),
        ast::Item::Union(it) => it.visibility(),
        ast::Item::Enum(it) => it.visibility(),
        ast::Item::Trait(it) => it.visibility(),
        ast::Item::TypeAlias(it) => it.visibility(),
        ast::Item::Const(it) => it.visibility(),
        ast::Item::Static(it) => it.visibility(),
        ast::Item::Module(it) => it.visibility(),
        ast::Item::MacroDef(it) => it.visibility(),
        ast::Item::Impl(it) => it.visibility(),
        ast::Item::MacroRules(_)
        | ast::Item::AsmExpr(_)
        | ast::Item::ExternBlock(_)
        | ast::Item::ExternCrate(_)
        | ast::Item::MacroCall(_)
        | ast::Item::Use(_) => None,
    }
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
    fn a_panic_becomes_the_degraded_extraction() {
        let extraction = guarded(BASE_STACK, || panic!("boom"));
        assert!(extraction.had_parse_errors);
        assert_eq!(extraction.degradation, Some(Degradation::Panicked));
        assert!(extraction.symbols.is_empty());
        assert!(extraction.blocks.is_empty());
        assert!(extraction.test_blocks.is_empty());
    }

    #[test]
    fn a_run_that_does_not_panic_is_returned_untouched() {
        let extraction = guarded(BASE_STACK, || Extraction { had_parse_errors: true, ..Extraction::default() });
        assert!(extraction.had_parse_errors);
        assert_eq!(extraction.degradation, None);
    }

    #[test]
    fn line_numbers_are_one_based_and_count_preceding_newlines() {
        let index = LineIndex::new("a\nbb\n\nc");
        assert_eq!([0, 1, 2, 4, 5, 6].map(|offset| index.line(offset)), [1, 1, 2, 2, 3, 4]);
    }
}

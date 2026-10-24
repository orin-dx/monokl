//! Pure extraction for one TypeScript or JavaScript file.
//!
//! The parse owns its arena. Everything leaving this module is owned data, so no arena-borrowed node escapes.

use std::panic::{AssertUnwindSafe, catch_unwind};

use camino::Utf8Path;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Class, Declaration, ExportAllDeclaration, ExportDeclaration, ExportDefaultDeclaration,
    ExportDefaultDeclarationKind, ExportFromDeclaration, ExportNamedDeclaration, Function, ImportDeclaration,
    ImportDeclarationSpecifier, JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXElement, JSXElementName,
    JSXMemberExpression, JSXMemberExpressionObject, ModuleExportName, Program, Statement, VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType, Span};

use crate::analysis::AnalysisProfile;
use crate::types::{
    BindingKind, CodeBlock, DependencyBinding, DependencyRecord, DependencyTarget, ExportRecord, JsxAttribute,
    JsxElementEntry, SymbolEntry, SymbolKind,
};

/// Files nested deeper than this are not parsed: the parser recurses once per level and overflowing the stack aborts
/// the process. A 2 MiB debug-build stack, the tightest a worker thread is likely to have, overflows between 700 and
/// 850 nested arrays; real code stays far below this.
const MAX_SOURCE_NESTING_DEPTH: usize = 256;

/// Everything extracted from one file, before it is wrapped in a `FileAnalysis`.
#[derive(Debug, Default)]
pub(super) struct Extraction {
    pub(super) symbols: Vec<SymbolEntry>,
    pub(super) dependencies: Vec<DependencyRecord>,
    pub(super) exports: Vec<ExportRecord>,
    pub(super) blocks: Vec<CodeBlock>,
    pub(super) jsx_elements: Vec<JsxElementEntry>,
    pub(super) type_only_imports: Vec<String>,
    pub(super) had_parse_errors: bool,
}

impl Extraction {
    /// The result for a file that could not be analyzed: no data, flagged as having errors.
    fn degraded() -> Self {
        Self { had_parse_errors: true, ..Self::default() }
    }
}

/// Extracts what `profile` asks for from `source`.
///
/// A file nested too deeply to parse safely, or one whose parse or extraction panics, yields
/// [`Extraction::degraded`].
pub(super) fn extract(path: &Utf8Path, source: &str, profile: AnalysisProfile) -> Extraction {
    if max_bracket_nesting_depth(source) > MAX_SOURCE_NESTING_DEPTH {
        return Extraction::degraded();
    }
    guarded(|| parse_and_extract(path, source, profile))
}

fn guarded(run: impl FnOnce() -> Extraction) -> Extraction {
    // The closure's output is discarded on a panic, so no torn state is observable afterwards.
    catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| Extraction::degraded())
}

/// The deepest bracket nesting in `source`, skipping comments and string literals.
///
/// A cheap linear scan used as a proxy for how deep the parser will recurse. It undercounts around apostrophes in
/// JSX text and regex literals, which only ever lets a file through, never rejects a valid one.
fn max_bracket_nesting_depth(source: &str) -> usize {
    let bytes = source.as_bytes();
    let (mut depth, mut max_depth, mut i) = (0usize, 0usize, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            quote @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'(' | b'{' | b'[' => {
                depth += 1;
                max_depth = max_depth.max(depth);
                i += 1;
            }
            b')' | b'}' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    max_depth
}

/// JavaScript files may hold JSX whatever their extension; TypeScript files only in `.tsx`, since `<T>x` is a cast
/// elsewhere.
fn source_type_for(path: &Utf8Path) -> SourceType {
    let source_type = SourceType::from_path(path.as_std_path()).unwrap_or_else(|_| SourceType::ts());
    if source_type.is_javascript() { source_type.with_jsx(true) } else { source_type }
}

fn parse_and_extract(path: &Utf8Path, source: &str, profile: AnalysisProfile) -> Extraction {
    let allocator = Allocator::default();
    // Parentheses carry no meaning here and a `ParenthesizedExpression` node would only add a level to every walk.
    let options = ParseOptions { preserve_parens: false, ..ParseOptions::default() };
    let parsed = Parser::new(&allocator, source, source_type_for(path)).with_options(options).parse();
    // The recovered program is still used below, so record the diagnostics rather than discarding the parse.
    let had_parse_errors = parsed.fatal_error || !parsed.diagnostics.is_empty();

    let mut extractor = Extractor { path, source, profile, lines: LineIndex::new(source), out: Extraction::default() };
    for statement in &parsed.program.body {
        extractor.statement(statement);
    }
    if profile == AnalysisProfile::Full {
        extractor.jsx(&parsed.program);
    }
    Extraction { had_parse_errors, ..extractor.out }
}

/// Maps byte offsets to 1-based line numbers.
struct LineIndex {
    newlines: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        Self { newlines: source.bytes().enumerate().filter(|&(_, byte)| byte == b'\n').map(|(i, _)| i).collect() }
    }

    fn line(&self, offset: u32) -> usize {
        self.newlines.partition_point(|&newline| newline < offset as usize) + 1
    }
}

struct Extractor<'s> {
    path: &'s Utf8Path,
    source: &'s str,
    profile: AnalysisProfile,
    lines: LineIndex,
    out: Extraction,
}

impl Extractor<'_> {
    fn structural(&self) -> bool {
        self.profile != AnalysisProfile::Dependencies
    }

    fn text(&self, span: Span) -> &str {
        self.source.get(span.start as usize..span.end as usize).unwrap_or_default()
    }

    fn statement(&mut self, statement: &Statement<'_>) {
        match statement {
            Statement::ImportDeclaration(decl) => self.import(decl),
            Statement::ExportDeclaration(decl) => self.export_declaration(decl),
            Statement::ExportNamedDeclaration(decl) => self.export_named(decl),
            Statement::ExportFromDeclaration(decl) => self.export_from(decl),
            Statement::ExportDefaultDeclaration(decl) => self.export_default(decl),
            Statement::ExportAllDeclaration(decl) => self.export_all(decl),
            Statement::FunctionDeclaration(function) => {
                self.function(function);
            }
            Statement::ClassDeclaration(class) => {
                self.class(class);
            }
            Statement::TSInterfaceDeclaration(decl) => {
                self.symbol(decl.id.name.as_str(), SymbolKind::Interface, decl.span, None);
            }
            Statement::TSTypeAliasDeclaration(decl) => {
                self.symbol(decl.id.name.as_str(), SymbolKind::TypeAlias, decl.span, None);
            }
            Statement::TSEnumDeclaration(decl) => {
                self.symbol(decl.id.name.as_str(), SymbolKind::Enum, decl.span, None);
            }
            Statement::VariableDeclaration(decl) => {
                self.variables(decl);
            }
            _ => {}
        }
        if self.profile == AnalysisProfile::Full {
            self.block(statement);
        }
    }

    fn symbol(&mut self, name: &str, kind: SymbolKind, span: Span, signature: Option<String>) {
        if !self.structural() {
            return;
        }
        self.out.symbols.push(SymbolEntry {
            name: name.to_owned(),
            kind,
            line: self.lines.line(span.start),
            signature,
            owner: None,
            trait_impl: None,
            visibility: None,
            kind_detail: None,
        });
    }

    fn export(&mut self, name: String, span: Span, re_export: bool) {
        if !self.structural() {
            return;
        }
        self.out.exports.push(ExportRecord { name, line: self.lines.line(span.start), re_export });
    }

    /// Records a function symbol and returns its name; anonymous functions have none.
    fn function(&mut self, function: &Function<'_>) -> Option<String> {
        let name = function.id.as_ref()?.name.as_str();
        let signature = self.function_signature(function);
        self.symbol(name, SymbolKind::Function, function.span, Some(signature));
        Some(name.to_owned())
    }

    /// The function's text up to its body, on one line: `function f(a: A): B`.
    fn function_signature(&self, function: &Function<'_>) -> String {
        let end = function.body.as_ref().map_or(function.span.end, |body| body.span.start);
        let head = self.text(Span::new(function.span.start, end));
        head.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(';').trim_end().to_owned()
    }

    fn class(&mut self, class: &Class<'_>) -> Option<String> {
        let name = class.id.as_ref()?.name.as_str();
        self.symbol(name, SymbolKind::Class, class.span, None);
        Some(name.to_owned())
    }

    fn variables(&mut self, decl: &VariableDeclaration<'_>) -> Vec<String> {
        let mut names = Vec::new();
        for declarator in &decl.declarations {
            if let BindingPattern::BindingIdentifier(id) = &declarator.id {
                self.symbol(id.name.as_str(), SymbolKind::Variable, declarator.span, None);
                names.push(id.name.as_str().to_owned());
            }
        }
        names
    }

    /// Records the symbols a declaration introduces and returns their names.
    fn declaration(&mut self, decl: &Declaration<'_>) -> Vec<String> {
        match decl {
            Declaration::FunctionDeclaration(function) => self.function(function).into_iter().collect(),
            Declaration::ClassDeclaration(class) => self.class(class).into_iter().collect(),
            Declaration::TSInterfaceDeclaration(decl) => {
                self.symbol(decl.id.name.as_str(), SymbolKind::Interface, decl.span, None);
                vec![decl.id.name.as_str().to_owned()]
            }
            Declaration::TSTypeAliasDeclaration(decl) => {
                self.symbol(decl.id.name.as_str(), SymbolKind::TypeAlias, decl.span, None);
                vec![decl.id.name.as_str().to_owned()]
            }
            Declaration::TSEnumDeclaration(decl) => {
                self.symbol(decl.id.name.as_str(), SymbolKind::Enum, decl.span, None);
                vec![decl.id.name.as_str().to_owned()]
            }
            Declaration::VariableDeclaration(decl) => self.variables(decl),
            Declaration::TSExternalModuleDeclaration(_)
            | Declaration::TSNamespaceDeclaration(_)
            | Declaration::TSGlobalDeclaration(_)
            | Declaration::TSImportEqualsDeclaration(_) => Vec::new(),
        }
    }

    fn import(&mut self, decl: &ImportDeclaration<'_>) {
        let specifier = decl.source.value.as_str().to_owned();
        if decl.import_kind.is_type() {
            self.out.type_only_imports.push(specifier);
            return;
        }
        let mut bindings = Vec::new();
        for specifier in decl.specifiers.iter().flatten() {
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(named) => {
                    if !named.import_kind.is_type() {
                        bindings.push(DependencyBinding {
                            imported: module_export_name(&named.imported),
                            local: named.local.name.as_str().to_owned(),
                            kind: BindingKind::Named,
                        });
                    }
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => bindings.push(DependencyBinding {
                    imported: "default".to_owned(),
                    local: default.local.name.as_str().to_owned(),
                    kind: BindingKind::Default,
                }),
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => bindings.push(DependencyBinding {
                    imported: "*".to_owned(),
                    local: namespace.local.name.as_str().to_owned(),
                    kind: BindingKind::Namespace,
                }),
            }
        }
        self.dependency(decl.source.value.as_str(), decl.span, bindings);
    }

    fn dependency(&mut self, specifier: &str, span: Span, bindings: Vec<DependencyBinding>) {
        self.out.dependencies.push(DependencyRecord {
            line: self.lines.line(span.start),
            bindings,
            target: DependencyTarget::File {
                specifier: specifier.to_owned(),
                resolved: None,
                is_relative: specifier.starts_with('.'),
            },
        });
    }

    /// `export function f() {}`, `export const x = 1`.
    fn export_declaration(&mut self, decl: &ExportDeclaration<'_>) {
        for name in self.declaration(&decl.declaration) {
            self.export(name, decl.span, false);
        }
    }

    /// `export { a, b as c }`.
    fn export_named(&mut self, decl: &ExportNamedDeclaration<'_>) {
        for specifier in &decl.specifiers {
            self.export(module_export_name(&specifier.exported), decl.span, false);
        }
    }

    /// `export { a } from './m'`. A type-only re-export adds no runtime dependency.
    fn export_from(&mut self, decl: &ExportFromDeclaration<'_>) {
        for specifier in &decl.specifiers {
            self.export(module_export_name(&specifier.exported), decl.span, true);
        }
        if decl.export_kind.is_type() {
            return;
        }
        let bindings = decl
            .specifiers
            .iter()
            .filter(|specifier| !specifier.export_kind.is_type())
            .map(|specifier| DependencyBinding {
                imported: module_export_name(&specifier.local),
                local: module_export_name(&specifier.exported),
                kind: BindingKind::Named,
            })
            .collect();
        self.dependency(decl.source.value.as_str(), decl.span, bindings);
    }

    fn export_default(&mut self, decl: &ExportDefaultDeclaration<'_>) {
        self.export("default".to_owned(), decl.span, false);
        match &decl.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                self.function(function);
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                self.class(class);
            }
            _ => {}
        }
    }

    fn export_all(&mut self, decl: &ExportAllDeclaration<'_>) {
        let (name, kind) = match &decl.exported {
            Some(exported) => (module_export_name(exported), BindingKind::Namespace),
            None => ("*".to_owned(), BindingKind::Glob),
        };
        self.export(name.clone(), decl.span, true);
        if !decl.export_kind.is_type() {
            let binding = DependencyBinding { imported: "*".to_owned(), local: name, kind };
            self.dependency(decl.source.value.as_str(), decl.span, vec![binding]);
        }
    }

    fn block(&mut self, statement: &Statement<'_>) {
        let Some(node_kind) = statement_kind(statement) else { return };
        if matches!(statement, Statement::ImportDeclaration(_)) {
            return;
        }
        let span = statement.span();
        let code = self.text(span);
        if code.trim().is_empty() {
            return;
        }
        let symbol_signature = match (node_kind, statement_function(statement)) {
            (SymbolKind::Function, Some(function)) => Some(self.function_signature(function)),
            _ => None,
        };
        let block = CodeBlock {
            file: self.path.to_owned(),
            line_start: self.lines.line(span.start),
            line_end: self.lines.line(span.end),
            node_kind,
            code: code.to_owned(),
            symbol_signature,
            matched_lines: Vec::new(),
            matched_keywords: Vec::new(),
        };
        self.out.blocks.push(block);
    }

    fn jsx(&mut self, program: &Program<'_>) {
        let mut collector = JsxCollector { lines: &self.lines, found: Vec::new() };
        collector.visit_program(program);
        self.out.jsx_elements = collector.found;
    }
}

/// The kind of block a top-level statement makes, or `None` when it is not one.
fn statement_kind(statement: &Statement<'_>) -> Option<SymbolKind> {
    match statement {
        Statement::FunctionDeclaration(_) => Some(SymbolKind::Function),
        Statement::ClassDeclaration(_) => Some(SymbolKind::Class),
        Statement::TSInterfaceDeclaration(_) => Some(SymbolKind::Interface),
        Statement::TSTypeAliasDeclaration(_) => Some(SymbolKind::TypeAlias),
        Statement::TSEnumDeclaration(_) => Some(SymbolKind::Enum),
        Statement::VariableDeclaration(_) => Some(SymbolKind::Variable),
        Statement::ExportDeclaration(export) => {
            Some(declaration_kind(&export.declaration).unwrap_or(SymbolKind::Other))
        }
        Statement::ImportDeclaration(_)
        | Statement::ExportNamedDeclaration(_)
        | Statement::ExportFromDeclaration(_)
        | Statement::ExportDefaultDeclaration(_)
        | Statement::ExportAllDeclaration(_) => Some(SymbolKind::Other),
        _ => None,
    }
}

fn declaration_kind(decl: &Declaration<'_>) -> Option<SymbolKind> {
    match decl {
        Declaration::FunctionDeclaration(_) => Some(SymbolKind::Function),
        Declaration::ClassDeclaration(_) => Some(SymbolKind::Class),
        Declaration::TSInterfaceDeclaration(_) => Some(SymbolKind::Interface),
        Declaration::TSTypeAliasDeclaration(_) => Some(SymbolKind::TypeAlias),
        Declaration::TSEnumDeclaration(_) => Some(SymbolKind::Enum),
        Declaration::VariableDeclaration(_) => Some(SymbolKind::Variable),
        Declaration::TSExternalModuleDeclaration(_)
        | Declaration::TSNamespaceDeclaration(_)
        | Declaration::TSGlobalDeclaration(_)
        | Declaration::TSImportEqualsDeclaration(_) => None,
    }
}

/// The function a statement declares, directly or through `export`.
fn statement_function<'a, 'b>(statement: &'b Statement<'a>) -> Option<&'b Function<'a>> {
    match statement {
        Statement::FunctionDeclaration(function) => Some(function),
        Statement::ExportDeclaration(export) => match &export.declaration {
            Declaration::FunctionDeclaration(function) => Some(function),
            _ => None,
        },
        _ => None,
    }
}

fn module_export_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.as_str().to_owned(),
        ModuleExportName::IdentifierReference(id) => id.name.as_str().to_owned(),
        ModuleExportName::StringLiteral(literal) => literal.value.as_str().to_owned(),
    }
}

/// Collects every JSX element in document order.
struct JsxCollector<'l> {
    lines: &'l LineIndex,
    found: Vec<JsxElementEntry>,
}

impl<'a> Visit<'a> for JsxCollector<'_> {
    fn visit_jsx_element(&mut self, element: &JSXElement<'a>) {
        let opening = &element.opening_element;
        let name = jsx_element_name(&opening.name);
        let attributes = opening.attributes.iter().map(jsx_attribute).collect();
        self.found.push(JsxElementEntry {
            is_html: name.chars().next().is_some_and(char::is_lowercase),
            name,
            line: self.lines.line(opening.span.start),
            attributes,
        });
        walk::walk_jsx_element(self, element);
    }
}

fn jsx_attribute(item: &JSXAttributeItem<'_>) -> JsxAttribute {
    match item {
        JSXAttributeItem::Attribute(attribute) => {
            let name = match &attribute.name {
                JSXAttributeName::Identifier(id) => id.name.as_str().to_owned(),
                JSXAttributeName::NamespacedName(ns) => format!("{}:{}", ns.namespace.name, ns.name.name),
            };
            let (string_value, is_expression) = match &attribute.value {
                Some(JSXAttributeValue::StringLiteral(literal)) => (Some(literal.value.as_str().to_owned()), false),
                Some(
                    JSXAttributeValue::ExpressionContainer(_)
                    | JSXAttributeValue::Element(_)
                    | JSXAttributeValue::Fragment(_),
                ) => (None, true),
                None => (None, false),
            };
            JsxAttribute { name, string_value, is_expression, is_spread: false }
        }
        JSXAttributeItem::SpreadAttribute(_) => {
            JsxAttribute { name: "...".to_owned(), string_value: None, is_expression: false, is_spread: true }
        }
    }
}

fn jsx_element_name(name: &JSXElementName<'_>) -> String {
    match name {
        JSXElementName::Identifier(id) => id.name.as_str().to_owned(),
        JSXElementName::IdentifierReference(id) => id.name.as_str().to_owned(),
        JSXElementName::NamespacedName(ns) => format!("{}:{}", ns.namespace.name, ns.name.name),
        JSXElementName::MemberExpression(member) => jsx_member(member),
        JSXElementName::ThisExpression(_) => "this".to_owned(),
    }
}

fn jsx_member(member: &JSXMemberExpression<'_>) -> String {
    let object = match &member.object {
        JSXMemberExpressionObject::IdentifierReference(id) => id.name.as_str().to_owned(),
        JSXMemberExpressionObject::MemberExpression(inner) => jsx_member(inner),
        JSXMemberExpressionObject::ThisExpression(_) => "this".to_owned(),
    };
    format!("{object}.{}", member.property.name)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn a_panic_becomes_the_degraded_extraction() {
        let extraction = guarded(|| panic!("boom"));
        assert!(extraction.had_parse_errors);
        assert!(extraction.symbols.is_empty());
        assert!(extraction.dependencies.is_empty());
        assert!(extraction.exports.is_empty());
        assert!(extraction.blocks.is_empty());
        assert!(extraction.jsx_elements.is_empty());
        assert!(extraction.type_only_imports.is_empty());
    }

    #[test]
    fn a_run_that_does_not_panic_is_returned_untouched() {
        let extraction = guarded(|| Extraction { type_only_imports: vec!["x".to_owned()], ..Extraction::default() });
        assert!(!extraction.had_parse_errors);
        assert_eq!(extraction.type_only_imports, ["x"]);
    }

    #[rstest]
    #[case("", 0)]
    #[case("a(b[c{}])", 3)]
    #[case("(((", 3)]
    #[case(")))(", 1)]
    #[case("// (((\n[", 1)]
    #[case("/* ((( */ [", 1)]
    #[case("'(((' + [[", 2)]
    #[case("\"\\\"(((\" + [", 1)]
    #[case("`(((` + {", 1)]
    fn nesting_depth_ignores_comments_and_strings(#[case] source: &str, #[case] depth: usize) {
        assert_eq!(max_bracket_nesting_depth(source), depth);
    }

    #[test]
    fn line_numbers_are_one_based_and_count_preceding_newlines() {
        let index = LineIndex::new("a\nbb\n\nc");
        assert_eq!([0, 1, 2, 4, 5, 6].map(|offset| index.line(offset)), [1, 1, 2, 2, 3, 4]);
    }

    #[rstest]
    #[case("a.ts", true, false)]
    #[case("a.tsx", true, true)]
    #[case("a.mts", true, false)]
    #[case("a.js", false, true)]
    #[case("a.cjs", false, true)]
    #[case("a.unknown", true, false)]
    fn source_types_follow_the_extension(#[case] path: &str, #[case] typescript: bool, #[case] jsx: bool) {
        let source_type = source_type_for(Utf8Path::new(path));
        assert_eq!((source_type.is_typescript(), source_type.is_jsx()), (typescript, jsx));
    }
}

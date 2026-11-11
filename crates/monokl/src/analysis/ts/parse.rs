//! Pure extraction for one TypeScript or JavaScript file.
//!
//! The parse owns its arena. Everything leaving this module is owned data, so no arena-borrowed node escapes.

use camino::Utf8Path;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Class, ClassElement, Declaration, ExportDefaultDeclarationKind, Expression, Function,
    MethodDefinition, MethodDefinitionKind, ObjectExpression, ObjectProperty, ObjectPropertyKind, PropertyDefinition,
    PropertyKind, Statement,
};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType, Span};

use crate::analysis::Degradation;
use crate::analysis::nesting::{self, Syntax};
use crate::analysis::parse_stack::on_parse_stack;
use crate::types::{CodeBlock, ParentContext, SymbolKind};

/// The fewest lines a class or object member needs to get a block of its own. A shorter one is a stub or accessor that
/// the block holding it already shows whole.
const MIN_MEMBER_LINES: usize = 3;

/// The longest source that is parsed. Parse time and memory grow with the length, not with the stack, so this is chosen
/// from measurements of a whole `monokl search` over one generated file: at 3.4 MB it takes 40 ms and peaks at 94 MB of
/// resident memory, about 28 bytes per source byte. A file twice that long is generated data, not code a search is for,
/// and what it would cost is no longer small.
pub(super) const MAX_PARSED_SOURCE_LEN: usize = 4 << 20;

/// The deepest nesting parsed, in levels of brackets or the equivalent in other nesting (see [`nesting::peak_cost`]).
///
/// 65,536 units (four per level) is what the fixed parse stack holds with a factor of two and a half to spare in the
/// worst case measured; see [`PARSE_STACK_BYTES`](crate::analysis::parse_stack::PARSE_STACK_BYTES). Ordinary code nests
/// a few dozen levels. A file over this is reported as too deep instead of parsed.
pub(super) const MAX_NESTING_LEVELS: usize = 16_384;

/// Everything extracted from one file, before it is wrapped in a `FileAnalysis`.
#[derive(Debug, Default)]
pub(super) struct Extraction {
    pub(super) blocks: Vec<CodeBlock>,
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
/// A file too long or nested too deeply for the parse stack is [`Extraction::degraded`] without being parsed, and so is
/// one whose parse or extraction panics. The scan that rejects the nesting is linear in the length, and the parse runs
/// on a stack of one fixed size, so what a file can cost is bounded by these two limits and not by its contents.
pub(super) fn extract(path: &Utf8Path, source: &str) -> Extraction {
    if source.len() > MAX_PARSED_SOURCE_LEN {
        return Extraction::degraded(Degradation::TooLarge { bytes: source.len(), cap: MAX_PARSED_SOURCE_LEN });
    }
    let syntax = Syntax::TypeScript { jsx: source_type_for(path).is_jsx() };
    if let Err(degradation) = nesting::check(source, syntax, MAX_NESTING_LEVELS) {
        return Extraction::degraded(degradation);
    }
    on_parse_stack(|| parse_and_extract(path, source)).unwrap_or_else(Extraction::degraded)
}

/// Parses and extracts on the calling thread, for a test that supplies its own stack.
#[cfg(all(test, feature = "lang-rust"))]
pub(super) fn parse_here(path: &Utf8Path, source: &str) {
    drop(parse_and_extract(path, source));
}

/// JavaScript files may hold JSX whatever their extension; TypeScript files only in `.tsx`, since `<T>x` is a cast
/// elsewhere.
fn source_type_for(path: &Utf8Path) -> SourceType {
    let source_type = SourceType::from_path(path.as_std_path()).unwrap_or_else(|_| SourceType::ts());
    if source_type.is_javascript() { source_type.with_jsx(true) } else { source_type }
}

fn parse_and_extract(path: &Utf8Path, source: &str) -> Extraction {
    let allocator = Allocator::default();
    // Parentheses carry no meaning here and a `ParenthesizedExpression` node would only add a level to every walk.
    let options = ParseOptions { preserve_parens: false, ..ParseOptions::default() };
    let parsed = Parser::new(&allocator, source, source_type_for(path)).with_options(options).parse();
    // The recovered program is still used below, so record the diagnostics rather than discarding the parse.
    let had_parse_errors = parsed.fatal_error || !parsed.diagnostics.is_empty();

    let mut extractor = Extractor { path, source, lines: LineIndex::new(source), out: Extraction::default() };
    for statement in &parsed.program.body {
        extractor.statement(statement);
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
    lines: LineIndex,
    out: Extraction,
}

impl Extractor<'_> {
    fn text(&self, span: Span) -> &str {
        self.source.get(span.start as usize..span.end as usize).unwrap_or_default()
    }

    fn statement(&mut self, statement: &Statement<'_>) {
        self.block(statement);
        self.member_blocks(statement);
    }

    /// The function's text up to its body, on one line: `function f(a: A): B`.
    fn function_signature(&self, function: &Function<'_>) -> String {
        let end = function.body.as_ref().map_or(function.span.end, |body| body.span.start);
        self.signature_text(Span::new(function.span.start, end))
    }

    /// The text of `span` on one line, without a trailing `;`.
    fn signature_text(&self, span: Span) -> String {
        self.text(span).split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(';').trim_end().to_owned()
    }

    fn block(&mut self, statement: &Statement<'_>) {
        let Some(node_kind) = statement_kind(statement) else { return };
        if matches!(statement, Statement::ImportDeclaration(_)) {
            return;
        }
        let symbol_signature = match (node_kind, statement_function(statement)) {
            (SymbolKind::Function, Some(function)) => Some(self.function_signature(function)),
            _ => None,
        };
        self.push_block(statement.span(), node_kind, symbol_signature, None);
    }

    /// Pushes a block over `span`, unless it holds nothing but whitespace.
    fn push_block(
        &mut self,
        span: Span,
        node_kind: SymbolKind,
        symbol_signature: Option<String>,
        parent: Option<ParentContext>,
    ) {
        let code = self.text(span);
        if code.trim().is_empty() {
            return;
        }
        let block = CodeBlock {
            file: self.path.to_owned(),
            line_start: self.lines.line(span.start),
            line_end: self.lines.line(span.end),
            node_kind,
            code: code.to_owned(),
            symbol_signature,
            matched_lines: Vec::new(),
            parent,
        };
        self.out.blocks.push(block);
    }

    /// Pushes a block for each long member of the named class or object literal that a top-level or exported statement
    /// holds, after the statement's own block. An anonymous class has no name to give its members as a parent, so it
    /// gets none.
    fn member_blocks(&mut self, statement: &Statement<'_>) {
        if let Some(class) = statement_class(statement) {
            self.class_member_blocks(class);
        }
        for (parent, object) in Self::statement_objects(statement) {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => self.object_property_block(property, &parent),
                    ObjectPropertyKind::SpreadProperty(_) => {}
                }
            }
        }
    }

    /// The object literals a top-level or exported statement binds to a name, each with the parent its members get: the
    /// declarator's name for `const api = { ... }`, `default` for `export default { ... }`.
    fn statement_objects<'a, 'b>(statement: &'b Statement<'a>) -> Vec<(ParentContext, &'b ObjectExpression<'a>)> {
        let declaration = match statement {
            Statement::VariableDeclaration(declaration) => Some(&**declaration),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(declaration) => Some(&**declaration),
                _ => None,
            },
            _ => None,
        };
        if let Some(declaration) = declaration {
            return declaration
                .declarations
                .iter()
                .filter_map(|declarator| {
                    let BindingPattern::BindingIdentifier(id) = &declarator.id else { return None };
                    let object = object_literal(declarator.init.as_ref()?)?;
                    let parent = ParentContext { kind: SymbolKind::Variable, name: id.name.as_str().to_owned() };
                    Some((parent, object))
                })
                .collect();
        }
        let Statement::ExportDefaultDeclaration(export) = statement else { return Vec::new() };
        let Some(object) = export.declaration.as_expression().and_then(object_literal) else { return Vec::new() };
        let parent = ParentContext { kind: SymbolKind::Other, name: "default".to_owned() };
        vec![(parent, object)]
    }

    /// A method, constructor or function-valued property of a named class.
    fn class_member_blocks(&mut self, class: &Class<'_>) {
        let Some(id) = &class.id else { return };
        let parent = ParentContext { kind: SymbolKind::Class, name: id.name.as_str().to_owned() };
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(method) => self.method_block(method, &parent),
                ClassElement::PropertyDefinition(property) => self.property_block(property, &parent),
                ClassElement::StaticBlock(_)
                | ClassElement::AccessorProperty(_)
                | ClassElement::TSIndexSignature(_) => {}
            }
        }
    }

    /// A method with a body: `async f(a: A): B { ... }`. Getters and setters are methods; a signature without a body
    /// (an overload or an abstract method) is not.
    fn method_block(&mut self, method: &MethodDefinition<'_>, parent: &ParentContext) {
        let Some(body) = &method.value.body else { return };
        let node_kind = match method.kind {
            MethodDefinitionKind::Constructor => SymbolKind::Constructor,
            MethodDefinitionKind::Method | MethodDefinitionKind::Get | MethodDefinitionKind::Set => SymbolKind::Method,
        };
        let head = Span::new(method.decorators.last().map_or(method.span.start, |d| d.span.end), body.span.start);
        let signature = self.signature_text(head);
        self.member_block(method.span, node_kind, signature, parent);
    }

    /// A property holding an arrow or function expression: `handle = async (req) => { ... }`. Any other value is data,
    /// which the class block shows well enough.
    fn property_block(&mut self, property: &PropertyDefinition<'_>, parent: &ParentContext) {
        let Some(body_start) = property.value.as_ref().and_then(function_body_start) else { return };
        let head = Span::new(property.decorators.last().map_or(property.span.start, |d| d.span.end), body_start);
        let signature = self.signature_text(head);
        self.member_block(property.span, SymbolKind::Property, signature, parent);
    }

    /// An object property that is a method (`async f() { ... }`, a getter or a setter) or holds an arrow or function
    /// expression (`f: () => { ... }`). Any other value is data, which the object's block shows well enough.
    fn object_property_block(&mut self, property: &ObjectProperty<'_>, parent: &ParentContext) {
        let Some(body_start) = function_body_start(&property.value) else { return };
        let node_kind = if property.method || property.kind != PropertyKind::Init {
            SymbolKind::Method
        } else {
            SymbolKind::Property
        };
        let signature = self.signature_text(Span::new(property.span.start, body_start));
        self.member_block(property.span, node_kind, signature, parent);
    }

    fn member_block(&mut self, span: Span, node_kind: SymbolKind, signature: String, parent: &ParentContext) {
        let lines = self.lines.line(span.end) - self.lines.line(span.start) + 1;
        if lines >= MIN_MEMBER_LINES {
            self.push_block(span, node_kind, Some(signature), Some(parent.clone()));
        }
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
        Statement::ExportDefaultDeclaration(export) => Some(match &export.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(_) => SymbolKind::Function,
            ExportDefaultDeclarationKind::ClassDeclaration(_) => SymbolKind::Class,
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => SymbolKind::Interface,
            _ => SymbolKind::Other,
        }),
        Statement::ImportDeclaration(_)
        | Statement::ExportNamedDeclaration(_)
        | Statement::ExportFromDeclaration(_)
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

/// Where the body of an arrow function or function expression starts; `None` for any other expression and for a
/// function without a body.
fn function_body_start(expression: &Expression<'_>) -> Option<u32> {
    match expression {
        Expression::ArrowFunctionExpression(arrow) => Some(arrow.body.span().start),
        Expression::FunctionExpression(function) => function.body.as_ref().map(|body| body.span.start),
        _ => None,
    }
}

/// The object literal an expression is, looking through parentheses, `as`, `satisfies` and `!`.
fn object_literal<'a, 'b>(expression: &'b Expression<'a>) -> Option<&'b ObjectExpression<'a>> {
    match expression {
        Expression::ObjectExpression(object) => Some(object),
        Expression::ParenthesizedExpression(inner) => object_literal(&inner.expression),
        Expression::TSAsExpression(inner) => object_literal(&inner.expression),
        Expression::TSSatisfiesExpression(inner) => object_literal(&inner.expression),
        Expression::TSNonNullExpression(inner) => object_literal(&inner.expression),
        _ => None,
    }
}

/// The class a statement declares, directly or through `export` or `export default`.
fn statement_class<'a, 'b>(statement: &'b Statement<'a>) -> Option<&'b Class<'a>> {
    match statement {
        Statement::ClassDeclaration(class) => Some(class),
        Statement::ExportDeclaration(export) => match &export.declaration {
            Declaration::ClassDeclaration(class) => Some(class),
            _ => None,
        },
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class),
            _ => None,
        },
        _ => None,
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn a_parenthesized_default_export_has_no_function_block() {
        let extraction = extract(Utf8Path::new("a.ts"), "export default (function g() {});\n");
        assert!(!extraction.had_parse_errors);
        let blocks: Vec<_> = extraction.blocks.iter().map(|b| (b.line_start, b.line_end, b.node_kind)).collect();
        assert_eq!(blocks, [(1, 1, SymbolKind::Other)]);
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

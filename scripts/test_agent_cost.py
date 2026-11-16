#!/usr/bin/env python3
"""Checks how agent-cost.py sizes the file read the rg-only policy pays for: the lines of the block monokl's analyzers
report for the expected declaration, with exact bytes (each line plus its newline).

`declared_block` is checked on literal spans. The synthetic sources and the pinned zod and date-fns files are searched
with the release binary (`cargo build --release`), whose rows carry each block's first and last line. Run as
`python3 -I scripts/test_agent_cost.py`; `just agent-cost` runs it first. The pinned files are cloned on demand like the
harness's.
"""

import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = str(ROOT / "target" / "release" / "monokl")
_spec = importlib.util.spec_from_file_location("agent_cost", ROOT / "scripts" / "agent-cost.py")
AC = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(AC)
AC.RC.configure(BINARY)
TS, RUST = AC.RC.LANGS["ts"], AC.RC.LANGS["rust"]


def synthetic(source, name, symbol, lang=TS, member=False):
    """(first line, last line, bytes) of the block for `symbol` in a file holding `source`."""
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory)
        (path / name).write_text(source)
        declarations = AC.declaration_lines(path, lang, name, symbol, member)
        span = AC.declared_block(AC.monokl_spans(BINARY, symbol, path, name), declarations)
        return (*span, AC.span_bytes(source.splitlines(), span))


def pinned(repo, file, symbol, member=False):
    for name, url, sha, lang, _held in AC.RC.repositories([repo]):
        path = AC.RC.checkout(name, url, sha)
        declarations = AC.declaration_lines(path, lang, file, symbol, member)
        span = AC.declared_block(AC.monokl_spans(BINARY, symbol, path, file), declarations)
        return (*span, AC.span_bytes((path / file).read_text().splitlines(), span))


class DeclaredBlock(unittest.TestCase):
    def test_the_largest_innermost_block_stands_in_for_overload_signatures(self):
        spans = [(1, 3), (4, 6), (7, 40)]
        self.assertEqual(AC.declared_block(spans, [(1, ""), (4, ""), (7, "")]), (7, 40))

    def test_a_container_does_not_stand_in_for_the_member_it_holds(self):
        self.assertEqual(AC.declared_block([(1, 50), (10, 20)], [(10, "")]), (10, 20))

    def test_a_declaration_no_block_holds_has_no_block(self):
        self.assertIsNone(AC.declared_block([(1, 5)], [(9, "")]))

    def test_span_bytes_counts_each_line_and_its_newline(self):
        self.assertEqual(AC.span_bytes(["ab", "", "cde", "f"], (2, 3)), 1 + 4)


class RowLines(unittest.TestCase):
    OUT = (
        "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n"
        "  src/a.ts,1,2,function,null,null,null,1,1\n"
        "  src/b.ts,3,4,function,null,xxx…,null,2,0.9\n"
        "totalCount: 5\n"
        "truncated[1]{row,field,removed}:\n"
        "  1,symbolSignature,51\n"
        "help[1]: showing 2 of 5\n"
    )

    def test_a_truncated_block_has_no_rows(self):
        self.assertEqual(list(AC.RC.row_lines(self.OUT)), [
            "  src/a.ts,1,2,function,null,null,null,1,1",
            "  src/b.ts,3,4,function,null,xxx…,null,2,0.9",
        ])

    def test_an_empty_result_has_no_rows(self):
        self.assertEqual(list(AC.RC.row_lines("search: []\nhelp[1]: query has no positive terms\n")), [])


class PinnedFiles(unittest.TestCase):
    def test_add_business_days_with_an_unindented_generic_header(self):
        self.assertEqual(pinned("date-fns", "pkgs/core/src/addBusinessDays/index.ts", "addBusinessDays"),
                         (43, 87, 1476))

    def test_to_json_schema_is_the_implementation_not_an_overload(self):
        self.assertEqual(pinned("zod", "packages/zod/src/v4/core/json-schema-processors.ts", "toJSONSchema"),
                         (886, 934, 1518))

    def test_a_method_followed_by_a_comma_ends_at_its_closing_brace(self):
        self.assertEqual(pinned("zod", "packages/zod/src/v4/core/memoizer.ts", "guard", member=True),
                         (226, 238, 637))


class SyntheticSources(unittest.TestCase):
    def test_multiline_generic_header_with_unindented_close(self):
        source = "\n".join([
            "export function pick<",
            "  T extends { a: string },",
            "  R = T,",
            ">(",
            "  value: T,",
            "  options?: Options<R> | undefined,",
            "): R {",
            "  const x = value.a;",
            "",
            "  return x as R;",
            "}",
            "",
            "export const next = 1;",
        ])
        self.assertEqual(synthetic(source, "a.ts", "pick"), (1, 11, 157))

    def test_arrow_function_const(self):
        source = "export const add = (a: number, b: number) => {\n  return a + b\n}\n\nexport const other = 1\n"
        self.assertEqual(synthetic(source, "a.ts", "add"), (1, 3, 64))

    def test_type_alias(self):
        source = "\n".join([
            "export type Shape =",
            "  | { kind: 'circle'; r: number }",
            "  | { kind: 'square'; side: number };",
            "",
            "export type Next = string;",
        ])
        self.assertEqual(synthetic(source, "a.ts", "Shape"), (1, 3, 92))

    def test_class_method_member(self):
        source = "\n".join([
            "export class Store {",
            "  getState(): State {",
            "    return this.state;",
            "  }",
            "",
            "  setState(next: State): void {",
            "    this.state = next;",
            "  }",
            "}",
        ])
        self.assertEqual(synthetic(source, "a.ts", "getState", member=True), (2, 4, 49))

    def test_braces_in_strings_comments_templates_and_regex(self):
        source = "\n".join([
            "export function tricky(a: string) {",
            "  const s = '}' + \"}\" + `${a}}{`;",
            "  const r = /}/.test(a); // }",
            "  /* } */",
            "  return `x ${ { y: '}' }.y } }`;",
            "}",
            "",
            "export const next = 1;",
        ])
        self.assertEqual(synthetic(source, "a.ts", "tricky"), (1, 6, 146))

    def test_rust_impl_method_with_a_where_clause_and_an_attribute(self):
        source = "\n".join([
            "impl<T> Foo<T> {",
            "    #[inline]",
            "    pub fn bar<U>(&self, u: U) -> Result<U, Error>",
            "    where",
            "        U: Clone + Into<T>,",
            "    {",
            '        let s = "}";',
            "        // }",
            "        let c = '}';",
            "        Ok(u)",
            "    }",
            "",
            "    pub fn baz(&self) {}",
            "}",
        ])
        self.assertEqual(synthetic(source, "a.rs", "bar", RUST, member=True), (2, 11, 184))

    def test_rust_struct_with_lifetime_and_generics(self):
        source = "\n".join([
            "/// A span.",
            "pub struct Span<'a, T: Into<Vec<u8>>> {",
            "    text: &'a str,",
            "    items: Vec<T>,",
            "}",
            "",
            "pub struct Wrapper(u8);",
            "pub fn next() {}",
        ])
        self.assertEqual(synthetic(source, "a.rs", "Span", RUST), (1, 5, 92))


if __name__ == "__main__":
    unittest.main(verbosity=1)

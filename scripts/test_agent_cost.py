#!/usr/bin/env python3
"""Checks `block_bytes`, the size of the file read the rg-only policy in agent-cost.py is charged for.

Each case gives the source, the 1-based line the declaration is on, the exact lines the block must be and the exact
bytes it costs (each line plus its newline). Run as `python3 -I scripts/test_agent_cost.py`; `just agent-cost` runs it
first. The addBusinessDays case reads the pinned date-fns checkout, cloned on demand like the harness's.
"""

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("agent_cost", ROOT / "scripts" / "agent-cost.py")
AC = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(AC)
TS, RUST = AC.RC.LANGS["ts"], AC.RC.LANGS["rust"]


def block(source, line, lang=TS):
    """(first line, last line, bytes, has a body) of the block of `source` that starts on 1-based `line`."""
    lines = source.splitlines()
    size, body = AC.block_bytes(lines, line - 1, lang)
    covered = 0
    count = 0
    while covered < size:
        covered += len(lines[line - 1 + count].encode()) + 1
        count += 1
    return line, line + count - 1, size, body


class BlockBytes(unittest.TestCase):
    def check(self, source, line, last, size, body, lang=TS):
        self.assertEqual(block(source, line, lang), (line, last, size, body))

    def test_pinned_add_business_days_reads_through_its_closing_brace(self):
        for name, url, sha, _lang, _held in AC.RC.repositories(["date-fns"]):
            path = AC.RC.checkout(name, url, sha)
            lines = (path / "pkgs/core/src/addBusinessDays/index.ts").read_text().splitlines()
            self.assertEqual(AC.block_bytes(lines, 43 - 1, TS), (1476, True))
            self.assertEqual(lines[43 - 1], "export function addBusinessDays<")
            self.assertEqual(lines[87 - 1], "}")

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
        self.check(source, 1, 11, 157, True)

    def test_arrow_function_const_with_a_body(self):
        source = "\n".join([
            "export const add = (a: number, b: number) => {",
            "  return a + b",
            "}",
            "",
            "export const other = 1",
        ])
        self.check(source, 1, 3, 64, True)

    def test_arrow_function_const_with_an_expression_body_and_no_semicolon(self):
        source = "\n".join([
            "export const double = (x: number) =>",
            "  x * 2",
            "export const triple = 3",
        ])
        self.check(source, 1, 2, 45, False)

    def test_single_line_const_without_a_semicolon(self):
        source = "export const createStore = ((s) => impl(s)) as CreateStore\n\nexport const next = 1\n"
        self.check(source, 1, 1, 59, False)

    def test_type_alias_union_ends_at_its_semicolon(self):
        source = "\n".join([
            "export type Shape =",
            "  | { kind: 'circle'; r: number }",
            "  | { kind: 'square'; side: number };",
            "",
            "export type Next = string;",
        ])
        self.check(source, 1, 3, 92, True)

    def test_type_alias_of_an_object_type(self):
        source = "type Options = {\n  a: string;\n  b?: number;\n} & Base\n\ntype Next = 1\n"
        self.check(source, 1, 4, 53, True)

    def test_interface_member_ends_with_its_line(self):
        source = "\n".join([
            "export interface Store {",
            "  getState(): State",
            "  setState(",
            "    partial: Partial<State>,",
            "    replace?: boolean,",
            "  ): void;",
            "  subscribe(listener: () => void): () => void",
            "}",
        ])
        self.check(source, 2, 2, 20, False)
        self.check(source, 3, 6, 75, False)
        self.check(source, 7, 7, 46, False)

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
        self.check(source, 3, 11, 170, True, RUST)
        self.check(source, 13, 13, 25, True, RUST)

    def test_rust_struct_with_lifetime_and_generics(self):
        source = "\n".join([
            "pub struct Span<'a, T: Into<Vec<u8>>> {",
            "    text: &'a str,",
            "    items: Vec<T>,",
            "}",
            "",
            "pub struct Wrapper(u8);",
            "pub struct Unit;",
            "pub fn next() {}",
        ])
        self.check(source, 1, 4, 80, True, RUST)
        self.check(source, 6, 6, 24, False, RUST)
        self.check(source, 7, 7, 17, False, RUST)

    def test_braces_in_strings_comments_templates_and_regex_do_not_count(self):
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
        self.check(source, 1, 6, 146, True)

    def test_a_declaration_that_never_ends_reads_to_the_end_of_the_file(self):
        source = "export function broken(a: number) {\n  return a;\n"
        self.check(source, 1, 2, 48, True)


if __name__ == "__main__":
    unittest.main(verbosity=1)

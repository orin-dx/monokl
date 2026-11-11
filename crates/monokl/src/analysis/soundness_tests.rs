//! A differential soundness check of the nesting scan: any input the scan accepts must parse without overflowing a stack
//! sized for the cap.
//!
//! The real cap and stack are 65,536 units (TypeScript) on 256 MiB. This runs the same ratio at 1/32: a cap of 512 levels
//! and an 8 MiB stack, the margin that `PARSE_STACK_BYTES` documents. Overflowing a stack aborts the process, so each
//! parse runs in a child process, which is this test binary run again on one test with a file of inputs. The child names
//! the input it is about to parse before it parses it, so the parent knows which one it died on.
//!
//! Inputs come from a seeded generator of nesting-heavy token soups: bracket and operator prefixes repeated to depths that
//! straddle the cap, with glue (comments, strings, regular expressions, JSX text, statement separators) between the
//! repeats, which is where a reset or a span read as the wrong kind of text makes the scan undercount.
//!
//! The run is a few seconds by default. `MONOKL_FUZZ_SECONDS` sets its length, half on the seed in `MONOKL_FUZZ_SEED`
//! (fixed by default) and half on a random one; a failure prints both and saves each counterexample under the system
//! temporary directory. `MONOKL_MINIMIZE=<file>` with `--ignored` shrinks one, and prints the stack it needs.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use camino::Utf8Path;

use super::nesting::{self, Syntax};

/// The scaled-down cap, in levels, and the stack that holds it with the same margin as the real pair.
pub(crate) const SMALL_LEVELS: usize = 512;
pub(crate) const SMALL_STACK: usize = 8 << 20;
/// The deepest syntax tree `ra_ap_syntax`'s 2 MiB drop thread holds with a factor of two.
const MAX_TREE_DEPTH: usize = 2_150;

const CHILD_ENV: &str = "MONOKL_SOUNDNESS_BATCH";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Ts,
    Tsx,
    Rust,
}

impl Kind {
    fn syntax(self) -> Syntax {
        match self {
            Self::Ts => Syntax::TypeScript { jsx: false },
            Self::Tsx => Syntax::TypeScript { jsx: true },
            Self::Rust => Syntax::Rust,
        }
    }

    fn byte(self) -> u8 {
        match self {
            Self::Ts => 0,
            Self::Tsx => 1,
            Self::Rust => 2,
        }
    }

    fn from_byte(byte: u8) -> Self {
        match byte {
            0 => Self::Ts,
            1 => Self::Tsx,
            _ => Self::Rust,
        }
    }
}

fn cap_units(kind: Kind) -> usize {
    let levels = if kind == Kind::Rust { crate::analysis::rust::MAX_NESTING_LEVELS } else { SMALL_LEVELS };
    levels * nesting::FRAME_COST
}

fn cost_of(kind: Kind, source: &str) -> usize {
    nesting::peak_cost(source, kind.syntax(), usize::MAX)
}

/// Whether the scan accepts `source` under the scaled-down cap.
pub(crate) fn scan_accepts(kind: Kind, source: &str) -> bool {
    // Rust's cap comes from the tree drop and not the stack, so it is the real one.
    let levels = if kind == Kind::Rust { crate::analysis::rust::MAX_NESTING_LEVELS } else { SMALL_LEVELS };
    nesting::check(source, kind.syntax(), levels).is_ok()
}

/// Parses and extracts `source` on a thread with `stack` bytes, as the analyzers do on the real one. For Rust it also
/// aborts when the tree is deeper than the drop thread holds.
fn parse_on_stack(kind: Kind, source: &str, stack: usize) {
    let run = || match kind {
        Kind::Ts => crate::analysis::ts::parse_on_stack(Utf8Path::new("a.ts"), source),
        Kind::Tsx => crate::analysis::ts::parse_on_stack(Utf8Path::new("a.tsx"), source),
        Kind::Rust => {
            crate::analysis::rust::parse_on_stack(source);
            if rust_tree_depth(source) > MAX_TREE_DEPTH {
                std::process::abort();
            }
        }
    };
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new().stack_size(stack).spawn_scoped(scope, run);
        // A panic in the parse is the analyzer's `Panicked` degradation, not an overflow.
        if let Ok(handle) = handle {
            let joined = handle.join();
            drop(joined);
        }
    });
}

fn rust_tree_depth(source: &str) -> usize {
    let root = ra_ap_syntax::SourceFile::parse(source, ra_ap_syntax::Edition::CURRENT).syntax_node();
    let (mut depth, mut deepest) = (0usize, 0usize);
    for event in root.preorder() {
        match event {
            ra_ap_syntax::WalkEvent::Enter(_) => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            ra_ap_syntax::WalkEvent::Leave(_) => depth = depth.saturating_sub(1),
        }
    }
    deepest
}

/// The child half: parses each input of the batch the parent wrote, naming it first. A test that does nothing unless
/// the parent set the variable.
#[test]
fn child() {
    let Ok(path) = std::env::var(CHILD_ENV) else { return };
    let bytes = std::fs::read(path).unwrap();
    let mut stdout = std::io::stdout().lock();
    let mut at = 0;
    let mut index = 0;
    while at < bytes.len() {
        let kind = Kind::from_byte(bytes[at]);
        let len = u32::from_le_bytes([bytes[at + 1], bytes[at + 2], bytes[at + 3], bytes[at + 4]]) as usize;
        let source = std::str::from_utf8(&bytes[at + 5..at + 5 + len]).unwrap();
        at += 5 + len;
        writeln!(stdout, "#{index}").unwrap();
        stdout.flush().unwrap();
        let stack = std::env::var("MONOKL_SOUNDNESS_STACK").ok().and_then(|v| v.parse().ok()).unwrap_or(SMALL_STACK);
        parse_on_stack(kind, source, stack);
        index += 1;
    }
}

fn encode(inputs: &[(Kind, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (kind, source) in inputs {
        out.push(kind.byte());
        out.extend_from_slice(&u32::try_from(source.len()).unwrap().to_le_bytes());
        out.extend_from_slice(source.as_bytes());
    }
    out
}

/// The indexes of the inputs whose parse aborted the process, found by running the batch in child processes.
fn crashing(inputs: &[(Kind, String)]) -> Vec<usize> {
    crashing_on(inputs, SMALL_STACK)
}

fn crashing_on(inputs: &[(Kind, String)], stack: usize) -> Vec<usize> {
    let dir = std::env::temp_dir().join(format!("monokl-soundness-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut crashed = Vec::new();
    let mut start = 0;
    while start < inputs.len() {
        let file = dir.join("batch.bin");
        std::fs::write(&file, encode(&inputs[start..])).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "analysis::soundness_tests::child", "--nocapture", "--test-threads", "1"])
            .env(CHILD_ENV, &file)
            .env("MONOKL_SOUNDNESS_STACK", stack.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut last = None;
        for line in BufReader::new(child.stdout.take().unwrap()).lines() {
            let line = line.unwrap();
            if let Some(number) = line.strip_prefix('#') {
                last = number.parse::<usize>().ok();
            }
        }
        let status = child.wait().unwrap();
        if status.success() {
            break;
        }
        // The child died on the input it last named; everything after it still needs running.
        let culprit = start + last.unwrap_or(0);
        crashed.push(culprit);
        start = culprit + 1;
    }
    std::fs::remove_dir_all(&dir).ok();
    crashed
}

/// Whether parsing `source` on the scaled-down stack aborts the process.
pub(crate) fn overflows(kind: Kind, source: &str) -> bool {
    !crashing(&[(kind, source.to_owned())]).is_empty()
}

/// Shrinks a counterexample by removing blocks of it while the scan still accepts it and the parse still overflows.
pub(crate) fn minimize(kind: Kind, source: &str) -> String {
    let mut current: Vec<char> = source.chars().collect();
    let mut block = current.len() / 2;
    while block >= 1 {
        let mut at = 0;
        while at < current.len() {
            let end = (at + block).min(current.len());
            let candidate: String = current[..at].iter().chain(current[end..].iter()).collect();
            if scan_accepts(kind, &candidate) && overflows(kind, &candidate) {
                current = candidate.chars().collect();
            } else {
                at += block;
            }
        }
        block /= 2;
    }
    current.into_iter().collect()
}

/// Prints the minimal form of the counterexample in the file named by `MONOKL_MINIMIZE`. For debugging the scan.
#[test]
#[ignore = "a debugging aid: set MONOKL_MINIMIZE to a counterexample file"]
fn minimize_the_counterexample_in_the_environment() {
    let Ok(path) = std::env::var("MONOKL_MINIMIZE") else { return };
    let kind = match path.rsplit('.').next() {
        Some("rs") => Kind::Rust,
        Some("tsx") => Kind::Tsx,
        _ => Kind::Ts,
    };
    let source = std::fs::read_to_string(&path).unwrap();
    let small = if std::env::var("MONOKL_NO_MINIMIZE").is_ok() { source } else { minimize(kind, &source) };
    let cost = nesting::peak_cost(&small, kind.syntax(), usize::MAX);
    let mut needed = SMALL_STACK;
    while needed < (4 << 30) && !crashing_on(&[(kind, small.clone())], needed).is_empty() {
        needed *= 2;
    }
    writeln!(std::io::stderr(), "needs a stack of about {} MiB", needed >> 20).ok();
    let depth = if kind == Kind::Rust {
        crate::analysis::parse_stack::on_parse_stack(|| rust_tree_depth(&small)).unwrap_or(0)
    } else {
        0
    };
    writeln!(std::io::stderr(), "{kind:?} {} bytes, cost {cost}, tree depth {depth}: {small:?}", small.len()).ok();
}

// -- the generator --------------------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).unwrap()
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    /// A repeat count between 1 and about 3,000, favoring the range that straddles the scaled cap.
    fn count(&mut self) -> usize {
        match self.below(10) {
            0..=1 => 1 + self.below(8),
            2..=3 => 1 + self.below(120),
            _ => 100 + self.below(2_900),
        }
    }
}

/// An opener, its closer, and what may follow each opener without ending the nest.
struct Piece {
    open: &'static str,
    close: &'static str,
    glue: &'static [&'static str],
}

const fn piece(open: &'static str, close: &'static str, glue: &'static [&'static str]) -> Piece {
    Piece { open, close, glue }
}

const NEUTRAL: &[&str] = &["", "", " ", "\n", "/* ( */", "// [\n", "/*{*/", "\n\n", "/* ) */"];
const EXPR_GLUE: &[&str] = &[
    "",
    "",
    " ",
    "\n",
    "/* ( */",
    "// (\n",
    "\"(\" + ",
    "'[' + ",
    "/(/.test(a) || ",
    "`${a}(` + ",
    "a ? b : ",
    "/* } */ ",
    "!",
    "await ",
    "x, ",
    "a / b + ",
    "/a/ + ",
    "\"'\" + ",
    "'\"' + ",
    "`'` + ",
    "a < b || ",
    "a > b || ",
];
const TEXT_GLUE: &[&str] = &[
    "",
    "",
    " ",
    "\n",
    ";",
    ",",
    "'",
    "\"",
    "//",
    "/*",
    ")",
    "]",
    ") ]",
    "x",
    " y\n",
    "&amp;",
    "{/*c*/}",
    "`",
    "(",
    "/",
    "// x",
    ";\n",
    "it's ",
    "a/b ",
    "</b>",
    "<b/>",
    "<b>x</b>",
    "{/* ) */}",
    "&lt;",
    ":",
    "=",
    "!",
    "\n  text;\n",
];
const STMT_GLUE: &[&str] = &["", "", " ", "\n", "/* ( */", "// (\n", "\n\n", "/*{*/"];

const TS_EXPR: &[Piece] = &[
    piece("(", ")", EXPR_GLUE),
    piece("[", "]", EXPR_GLUE),
    piece("{a:", "}", EXPR_GLUE),
    piece("f(", ")", EXPR_GLUE),
    piece("!", "", EXPR_GLUE),
    piece("await ", "", EXPR_GLUE),
    piece("new ", "", EXPR_GLUE),
    piece("typeof ", "", EXPR_GLUE),
    piece("a ? b : ", "", EXPR_GLUE),
    piece("() => ", "", EXPR_GLUE),
    piece("(a) => (", ")", EXPR_GLUE),
    piece("async () => ", "", EXPR_GLUE),
    piece("x = ", "", EXPR_GLUE),
    piece("1 + ", "", EXPR_GLUE),
    piece("-", "", EXPR_GLUE),
    piece("[...", "]", EXPR_GLUE),
    piece("`${", "}`", EXPR_GLUE),
    piece("class{x=", "}", EXPR_GLUE),
    piece("function(){return ", "}", EXPR_GLUE),
    piece("class extends ", " {}", EXPR_GLUE),
    piece("of / [", "]", EXPR_GLUE),
    piece("of / (", ")", EXPR_GLUE),
    piece("a / (", ")", EXPR_GLUE),
    piece("a++ / (", ")", EXPR_GLUE),
    piece("yield ", "", EXPR_GLUE),
    piece("import(", ")", EXPR_GLUE),
    piece("a as T as ", "", EXPR_GLUE),
    piece("a!.b[", "]", EXPR_GLUE),
    piece("/x/.test(", ")", EXPR_GLUE),
    piece("a.b(", ")", EXPR_GLUE),
    piece("{...", "}", EXPR_GLUE),
    piece("{[", "]: 1}", EXPR_GLUE),
    piece("{get a() {return ", "}}", EXPR_GLUE),
    piece("a?.(", ")", EXPR_GLUE),
    piece("a?.[", "]", EXPR_GLUE),
    piece("<T,>(x: T) => ", "", EXPR_GLUE),
    piece("async <T>(x: T) => (", ")", EXPR_GLUE),
    piece("@a(", ")", EXPR_GLUE),
    piece("void ", "", EXPR_GLUE),
    piece("delete ", "", EXPR_GLUE),
    piece("a satisfies ", "", EXPR_GLUE),
    piece("{a() {return ", "}}", EXPR_GLUE),
];
const TSX_EXPR: &[Piece] = &[
    piece("<a>", "</a>", TEXT_GLUE),
    piece("<a>", "</a>", TEXT_GLUE),
    piece("<>", "</>", TEXT_GLUE),
    piece("<a b={", "}/>", EXPR_GLUE),
    piece("<a>{", "}</a>", EXPR_GLUE),
    piece("<a b=\"x\" c={1}>", "</a>", TEXT_GLUE),
    piece("<a b=<c/>>", "</a>", TEXT_GLUE),
    piece("<a b=<c d=<e/>>>", "</a>", TEXT_GLUE),
    piece("<a.b>", "</a.b>", TEXT_GLUE),
    piece("<a {...p}>", "</a>", TEXT_GLUE),
    piece("<a>{(<a>", "</a>)}</a>", TEXT_GLUE),
    piece("<a>{[<a>", "</a>]}</a>", TEXT_GLUE),
    piece("<a>{f(<a>", "</a>)}</a>", TEXT_GLUE),
    piece("<a>{x ? <a>", "</a> : 1}</a>", TEXT_GLUE),
    piece("<a b={(<a>", "</a>)}/>", TEXT_GLUE),
    piece("<a>{[", "]}</a>", EXPR_GLUE),
    piece("<a>{f(", ")}</a>", EXPR_GLUE),
    piece("<a>{x ? ", " : 1}</a>", EXPR_GLUE),
    piece("(", ")", EXPR_GLUE),
    piece("[", "]", EXPR_GLUE),
    piece("f(", ")", EXPR_GLUE),
    piece("a ? ", " : 1", EXPR_GLUE),
    piece("() => ", "", EXPR_GLUE),
];
/// Type assertions, which only a `.ts` file has: in a `.tsx` file `<A>` starts an element.
const TS_ONLY_EXPR: &[Piece] = &[piece("<A>", "", EXPR_GLUE), piece("<A><B>(", ")", EXPR_GLUE)];
const TS_STMT: &[Piece] = &[
    piece("for await (const x of xs)\n", "", STMT_GLUE),
    piece("for await (const x of xs) ", "", STMT_GLUE),
    piece("for (const x of xs) ", "", STMT_GLUE),
    piece("for (;;)\n", "", STMT_GLUE),
    piece("for (const k in o)\n", "", STMT_GLUE),
    piece("if (1)\n", "", STMT_GLUE),
    piece("if (1) ", "", STMT_GLUE),
    piece("while (1) ", "", STMT_GLUE),
    piece("with (a)\n", "", STMT_GLUE),
    piece("do ", " while (1)", STMT_GLUE),
    piece("{", "}", STMT_GLUE),
    piece("{;", "}", STMT_GLUE),
    piece("a: ", "", STMT_GLUE),
    piece("try {", "} finally {}", STMT_GLUE),
    piece("switch (1) { case 1: ", "}", STMT_GLUE),
    piece("if (1) {} else ", "", STMT_GLUE),
    piece("if (1) {}\nelse ", "", STMT_GLUE),
    piece("const g = () => {", "}", STMT_GLUE),
    piece("x = function () {", "}", STMT_GLUE),
    piece("label: for (;;) ", "", STMT_GLUE),
    piece("await await ", "", STMT_GLUE),
    piece("if (a < b) ", "", STMT_GLUE),
    piece("while (a) /re/.test(b) && ", "", STMT_GLUE),
];
const TS_TYPE: &[Piece] = &[
    piece("A<", ">", NEUTRAL),
    piece("A<x,", ">", NEUTRAL),
    piece("[", "]", NEUTRAL),
    piece("(", ")", NEUTRAL),
    piece("() => ", "", NEUTRAL),
    piece("A extends B ? C : ", "", NEUTRAL),
    piece("keyof ", "", NEUTRAL),
    piece("{a:", "}", NEUTRAL),
    piece("readonly ", "", NEUTRAL),
    piece("typeof a[", "]", NEUTRAL),
    piece("Array<", ">", NEUTRAL),
    piece("infer U extends ", "", NEUTRAL),
];

const RS_ARG_GLUE: &[&str] = &[
    "",
    "",
    " ",
    "\n",
    "/* ( */",
    "// (\n",
    "\"(\", ",
    "'(', ",
    "r#\"(\"#, ",
    "b'(', ",
    "'\\'', ",
    "x, ",
    "'a, ",
    "|| ",
    "a < b, ",
    "a >> b, ",
    "/* /* ( */ ) */",
];
const RS_EXPR: &[Piece] = &[
    piece("f(", ")", RS_ARG_GLUE),
    piece("vec![", "]", RS_ARG_GLUE),
    piece("(1, ", ")", RS_ARG_GLUE),
    piece("m!(", ")", RS_ARG_GLUE),
    piece("m!{", "}", RS_ARG_GLUE),
    piece("m![", "]", RS_ARG_GLUE),
    piece("move || ", "", NEUTRAL),
    piece("async move { ", "}", &["", " ", ";"]),
    piece("a?.b(", ")?", NEUTRAL),
    piece("a.await.b(", ")", NEUTRAL),
    piece("&mut ", "", NEUTRAL),
    piece("..", "", NEUTRAL),
    piece("S { a: 1, ..", " }", NEUTRAL),
    piece("if let Some(x) = ", " { 1 } else { 2 }", NEUTRAL),
    piece("while let Some(x) = ", " {}", NEUTRAL),
    piece("for x in ", " {}", NEUTRAL),
    piece("[0; ", "]", NEUTRAL),
    piece("a[", "]", NEUTRAL),
    piece("(", ")", NEUTRAL),
    piece("[", "]", NEUTRAL),
    piece("{", "}", &["", " ", "\n", ";", "; ", "/* ( */", "// (\n", "'(';"]),
    piece("|a| ", "", NEUTRAL),
    piece("|a, b| ", "", NEUTRAL),
    piece("- ", "", NEUTRAL),
    piece("!", "", NEUTRAL),
    piece("&", "", NEUTRAL),
    piece("*", "", NEUTRAL),
    piece("f(", ")", NEUTRAL),
    piece("m!(", ")", NEUTRAL),
    piece("if a { 1 } else ", "", NEUTRAL),
    piece("match a { _ => ", "}", NEUTRAL),
    piece("loop { ", "}", &["", " ", ";", "\n"]),
    piece("unsafe { ", "}", &["", " ", ";"]),
    piece("Some(", ")", NEUTRAL),
    piece("x = ", "", NEUTRAL),
    piece("1 + ", "", NEUTRAL),
    piece("return ", "", NEUTRAL),
    piece("'a: { ", "}", &["", " ", ";"]),
    piece("a.b(", ")", NEUTRAL),
    piece("a as u8 as ", "", NEUTRAL),
    piece("(1, ", ")", NEUTRAL),
    piece("S { a: ", " }", NEUTRAL),
    piece("vec![", "]", NEUTRAL),
    piece("a::<", ">()", NEUTRAL),
    piece("Foo::<A<", ">>", NEUTRAL),
];
const RS_TYPE: &[Piece] = &[
    piece("A<", ">", NEUTRAL),
    piece("A<u8,", ">", NEUTRAL),
    piece("&", "", NEUTRAL),
    piece("(", ",)", NEUTRAL),
    piece("[", "; 1]", NEUTRAL),
    piece("fn() -> ", "", NEUTRAL),
    piece("Box<dyn ", ">", NEUTRAL),
    piece("&'a mut ", "", NEUTRAL),
];
const RS_ITEM: &[Piece] = &[
    piece("mod m { ", "}", &["", " ", "\n", "/* ( */"]),
    piece("fn a() { ", "}", &["", " ", ";", "\n"]),
    piece("impl A { fn b() { ", "} }", &["", " ", ";"]),
    piece("trait T { fn b() { ", "} }", &["", " "]),
];

/// The glue that confuses a lexer on its own, dropped between the pieces.
const NOISE: &[&str] = &[
    "/",
    " of ",
    "<",
    ">",
    "`",
    "'",
    "\"",
    "//",
    "/*",
    "*/",
    ")",
    "}",
    "]",
    ";",
    ",",
    "\n",
    "(",
    "[",
    "{",
    "=>",
    "?",
    ":",
    "r#\"",
    "\"#",
    "'(' ",
    "\\",
    "$",
    "${",
    "</",
    "/>",
    "for await (",
    "await ",
    " as ",
    "!",
    "++",
    "--",
];

fn repeat_pieces(rng: &mut Rng, pieces: &[Piece]) -> (String, String) {
    let (mut open, mut close) = (String::new(), String::new());
    let mut closes = Vec::new();
    for _ in 0..=rng.below(3) {
        let piece = rng.pick(pieces);
        let count = rng.count();
        let sparse = rng.chance(30);
        for _ in 0..count {
            open.push_str(piece.open);
            if !sparse || rng.chance(50) {
                open.push_str(rng.pick(piece.glue));
            }
        }
        closes.push((piece.close, count));
    }
    for (text, count) in closes.into_iter().rev() {
        for _ in 0..count {
            close.push_str(text);
        }
    }
    (open, close)
}

fn generate(rng: &mut Rng, kind: Kind) -> String {
    let (open, close) = match kind {
        Kind::Ts => {
            let pool = match rng.below(7) {
                0 => TS_STMT,
                1 => TS_TYPE,
                2 => TS_ONLY_EXPR,
                _ => TS_EXPR,
            };
            repeat_pieces(rng, pool)
        }
        Kind::Tsx => {
            let pool = if rng.chance(75) { TSX_EXPR } else { TS_EXPR };
            repeat_pieces(rng, pool)
        }
        Kind::Rust => {
            let pool = match rng.below(5) {
                0 => RS_TYPE,
                1 => RS_ITEM,
                _ => RS_EXPR,
            };
            repeat_pieces(rng, pool)
        }
    };
    let core = rng.pick(&["1", "x", "a", "", "null"]);
    let mut source = match (kind, open.as_str()) {
        (Kind::Rust, _)
            if open.starts_with("mod ")
                || open.starts_with("fn ")
                || open.starts_with("impl ")
                || open.starts_with("trait ") =>
        {
            format!("{open}fn leaf() {{}}{close}\n")
        }
        (Kind::Rust, _)
            if open.starts_with("A<")
                || open.starts_with('&')
                || open.starts_with("fn(")
                || open.starts_with("Box<") =>
        {
            format!("pub type T = {open}u8{close};\n")
        }
        (Kind::Rust, _) => format!("pub fn deep() {{ let x = {open}{core}{close}; }}\npub fn after() {{}}\n"),
        (_, _) if is_stmt(&open) => {
            format!("export async function f() {{\n{open}{core};{close}\n}}\nexport const after = 1;\n")
        }
        (_, _) if is_type(&open) => format!("export type T = {open}number{close};\nexport const after = 1;\n"),
        (_, _) => format!("export const x = {open}{core}{close};\nexport const after = 1;\n"),
    };
    // Noise dropped at random places, which mostly makes a syntax error and sometimes a different valid file.
    for _ in 0..(if rng.chance(25) { 1 + rng.below(3) } else { 0 }) {
        let mut at = rng.below(source.len() + 1);
        while !source.is_char_boundary(at) {
            at += 1;
        }
        source.insert_str(at, rng.pick(NOISE));
    }
    source
}

fn is_stmt(open: &str) -> bool {
    TS_STMT.iter().any(|piece| open.starts_with(piece.open))
}

fn is_type(open: &str) -> bool {
    TS_TYPE.iter().any(|piece| open.starts_with(piece.open))
        && !TS_EXPR.iter().any(|piece| open.starts_with(piece.open))
}

/// Stack bytes per unit of cost, measured for each piece nested alone up to the scaled cap. The unit is meant to be at most
/// 1.7 KB in a build without optimizations, so the real stack holds the cap twice over; this prints the worst offenders.
#[test]
#[ignore = "a measurement: run with --nocapture"]
fn measure_the_stack_each_piece_needs_per_unit() {
    type Wrap = fn(&str, &str) -> String;
    let pools: [(Kind, &[Piece], Wrap); 6] = [
        (Kind::Ts, TS_EXPR, |o, c| format!("export const x = {o}1{c};\n")),
        (Kind::Ts, TS_ONLY_EXPR, |o, c| format!("export const x = {o}1{c};\n")),
        (Kind::Ts, TS_STMT, |o, c| format!("export async function f() {{\n{o}x;{c}\n}}\n")),
        (Kind::Ts, TS_TYPE, |o, c| format!("export type T = {o}number{c};\n")),
        (Kind::Tsx, TSX_EXPR, |o, c| format!("export const x = {o}1{c};\n")),
        (Kind::Rust, RS_EXPR, |o, c| format!("pub fn deep() {{ let x = {o}1{c}; }}\n")),
    ];
    let mut rows = Vec::new();
    for (kind, pieces, wrap) in pools {
        for piece in pieces {
            let build = |depth: usize| wrap(&piece.open.repeat(depth), &piece.close.repeat(depth));
            let (mut low, mut high) = (1, 100_000);
            while high - low > 1 {
                let middle = usize::midpoint(low, high);
                if scan_accepts(kind, &build(middle)) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            let source = build(low);
            let cost = nesting::peak_cost(&source, kind.syntax(), usize::MAX);
            let mut stack = 256 << 10;
            while stack < (2 << 30) && !crashing_on(&[(kind, source.clone())], stack).is_empty() {
                stack *= 2;
            }
            rows.push((stack / cost.max(1), piece.open, low, cost, stack));
        }
    }
    rows.sort_unstable();
    for (per_unit, open, depth, cost, stack) in rows.iter().rev().take(25) {
        writeln!(std::io::stderr(), "{per_unit:>7} B/unit  {open:?} x{depth}: cost {cost}, stack {} KiB", stack >> 10)
            .ok();
    }
}

/// The outcome of one fuzzing run.
pub(crate) struct Report {
    pub(crate) inputs: usize,
    pub(crate) accepted: usize,
    /// How many of the accepted inputs cost at least half the cap, which is where an undercount can overflow.
    pub(crate) near_cap: usize,
    pub(crate) counterexamples: Vec<(Kind, String)>,
}

/// Generates inputs for `budget` from `seed`, runs the ones the scan accepts, and collects those that overflow.
pub(crate) fn fuzz(seed: u64, budget: Duration) -> Report {
    let mut rng = Rng(seed | 1);
    let started = Instant::now();
    let mut report = Report { inputs: 0, accepted: 0, near_cap: 0, counterexamples: Vec::new() };
    while started.elapsed() < budget {
        let mut batch = Vec::new();
        while batch.len() < 400 && started.elapsed() < budget {
            let kind = *rng.pick(&[Kind::Ts, Kind::Ts, Kind::Tsx, Kind::Tsx, Kind::Rust]);
            let source = generate(&mut rng, kind);
            report.inputs += 1;
            if scan_accepts(kind, &source) {
                batch.push((kind, source));
            }
        }
        report.accepted += batch.len();
        report.near_cap += batch.iter().filter(|(kind, source)| cost_of(*kind, source) >= cap_units(*kind) / 2).count();
        for index in crashing(&batch) {
            report.counterexamples.push(batch[index].clone());
        }
    }
    report
}

fn describe(report: &Report, seed: u64) -> String {
    let mut text = format!(
        "seed {seed}: {} inputs, {} accepted, {} of them over half the cap, {} overflowed\n",
        report.inputs,
        report.accepted,
        report.near_cap,
        report.counterexamples.len()
    );
    let dir = std::env::temp_dir().join("monokl-counterexamples");
    std::fs::create_dir_all(&dir).ok();
    for (number, (kind, source)) in report.counterexamples.iter().enumerate() {
        let path = dir.join(format!(
            "{seed}-{number}.{}",
            match kind {
                Kind::Rust => "rs",
                Kind::Tsx => "tsx",
                Kind::Ts => "ts",
            }
        ));
        std::fs::write(&path, source).ok();
        let head: String = source.chars().take(160).collect();
        writeln!(text, "  {kind:?} {} bytes at {}: {head:?}", source.len(), path.display()).ok();
    }
    text
}

/// Seconds to fuzz for, from `MONOKL_FUZZ_SECONDS`; the default is a few seconds so CI stays quick.
fn budget() -> Duration {
    let seconds = std::env::var("MONOKL_FUZZ_SECONDS").ok().and_then(|value| value.parse::<f64>().ok()).unwrap_or(3.0);
    Duration::from_secs_f64(seconds)
}

/// Asserts that `source` is a real hazard, which the parse overflows the scaled stack on, and that the scan rejects it.
fn assert_rejected_and_hazardous(kind: Kind, source: &str) {
    assert!(!scan_accepts(kind, source), "the scan accepts it");
    assert!(overflows(kind, source), "the parse does not overflow, so it is not a hazard");
}

/// `for await (` is a loop head: `await` used to replace that, so each loop's `)` looked like the end of a statement and
/// the estimate was reset at every loop.
#[test]
fn nested_for_await_loops_are_rejected_and_do_overflow() {
    let source = format!("export async function f() {{\n{}x;\n}}\n", "for await (const x of xs)\n".repeat(20_000));
    assert_rejected_and_hazardous(Kind::Ts, &source);
}

/// A `;` in JSX text used to reset what the open elements had charged.
#[test]
fn nested_jsx_elements_separated_by_semicolons_are_rejected_and_do_overflow() {
    let source = format!("export const x = {}x{};\n", "<a>;".repeat(20_000), "</a>;".repeat(20_000));
    assert_rejected_and_hazardous(Kind::Tsx, &source);
}

/// Found by the fuzzer: a `}` that closes a template's `${` inside what might be a regular expression (after `of /`) was
/// not allowed to pop, so the template's closing backtick opened a template of its own, which hid what followed.
#[test]
fn a_template_substitution_closed_inside_a_possible_regex_does_not_flip_the_backticks() {
    let source = format!("export const x = of / (`${{a}}`{}1{}`;\n", "+f/(".repeat(6_000), ")".repeat(6_000));
    assert_rejected_and_hazardous(Kind::Ts, &source);
}

/// Found by the fuzzer: Rust 2024 lexes `#"` as a prefix token and what follows as code, so the quote starts no string and
/// the nest between it and the next quote is real.
#[test]
fn a_guarded_string_prefix_starts_no_string() {
    let source = format!("pub fn a() {{ let x = #\"{}1{}\"; }}\n", "{".repeat(3_000), "}".repeat(3_000));
    assert_rejected_and_hazardous(Kind::Rust, &source);
}

#[test]
fn a_regex_lookahead_that_fails_is_paid_for_once_in_total() {
    // Every `of /` could start a regular expression, and none ends on its line, which is 3 MiB long.
    let source =
        format!("export const x = {}1{}{};\n", "of / [".repeat(10_000), "]".repeat(10_000), " ".repeat(3 << 20));
    let started = Instant::now();
    let accepted = scan_accepts(Kind::Ts, &source);
    assert!(started.elapsed() < Duration::from_secs(2), "the scan took {:?}", started.elapsed());
    assert!(!accepted);
}

#[test]
fn nothing_the_scan_accepts_overflows_a_stack_sized_for_the_cap() {
    // The check is only worth anything if the child can run: an ordinary file must not count as a crash.
    assert!(crashing(&[(Kind::Ts, "export const a = 1;\n".to_owned())]).is_empty(), "the child process does not run");
    assert!(!crashing(&[(Kind::Ts, format!("{}1{}", "(".repeat(1_000_000), ")".repeat(1_000_000)))]).is_empty());
    let fixed = std::env::var("MONOKL_FUZZ_SEED").ok().and_then(|value| value.parse().ok()).unwrap_or(0x5eed_u64);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let random = u64::try_from(nanos % u128::from(u64::MAX)).unwrap();
    let half = budget() / 2;
    let mut message = String::new();
    for seed in [fixed, random] {
        let report = fuzz(seed, half);
        let line = describe(&report, seed);
        writeln!(std::io::stderr(), "{line}").ok();
        if !report.counterexamples.is_empty() {
            message.push_str(&line);
        }
    }
    assert!(message.is_empty(), "the scan accepted inputs that overflow the scaled stack:\n{message}");
}

//! A bound on how deep a source file recurses, computed in one pass before any parser or tree walk sees it.
//!
//! The parsers and the extractors recurse once per nesting level, and a stack overflow aborts the process, so a file's
//! depth must be known to be small before it is parsed. [`peak_cost`] estimates the worst recursion the file could
//! cause as a number of units, in time proportional to the source length and memory proportional to the limit.
//!
//! # What a unit is
//!
//! | Token | Cost |
//! | --- | --- |
//! | `(` `[` `{` and the `${` of a template | [`FRAME_COST`] while it is open, then 1 once its closer has popped it, because `a()()()` and `a[0][0]` nest through the calls |
//! | `<` (generic arguments, JSX, a comparison) | the syntax's angle cost, which a `,` does not undo |
//! | a word, a number, any other punctuation byte | 1 |
//! | a string, a comment, a regular expression, the text of a template | nothing, with the exceptions below |
//!
//! The running cost inside a frame returns to the frame's start at a `;`, and to what its angle brackets added at a
//! `,`, because nothing that recurses survives either. Without semicolons it also returns there when a word starts a
//! new statement: after a closing `}`, and after a line that ends in an operand when the next word cannot continue it.
//!
//! # Never an underestimate
//!
//! An overestimate degrades a file that was not going to be useful. An underestimate lets a hostile file overflow the
//! parse stack and abort the process. So wherever the scan cannot tell what a byte is, it takes the reading that costs
//! more:
//!
//! - A closer pops a frame only if it is of the frame's kind. Any other closer is ignored, and costs 1.
//! - A quote with no closing quote on its line is an operator, not a string. A line ends at a line feed, a carriage
//!   return, or U+2028 or U+2029 in TypeScript, and at a line feed in Rust.
//! - In a file that may hold JSX, text that looks like a string, a comment, a regular expression or the text of a
//!   template may be JSX text, which can hold an expression in braces and nested tags. Such text that contains a `{` is
//!   scanned as code, and text that contains `<` charges each of them.
//! - A `/` after an operand is a division, and what follows it is code. After a closing `}` or a word that is only
//!   sometimes an operator (`await`, `of`, `as`) it may start a regular expression instead, so no closer up to the next
//!   `/` on the line pops anything.
//!
//! Nothing but the returned number shows which reading a scan took, so each has a test that a nest hidden behind it is
//! still counted, and a property test that a string of brackets costs at least what a stack of matching ones reaches.
//!
//! What it does not do is price a nest down: `</div>` costs as much as `<div>`, so a file with more than about 4,000
//! keyed JSX elements (`<li key={i}>text</li>`) or 13,000 empty ones in one expression, or about 32,000 operands and
//! operators in one statement, is over the cap whatever its depth.

use std::ops::Range;

use super::file_analysis::Degradation;

/// What a frame costs: a nested bracket is the deepest recursion per byte either parser has, at 1.6 KB of stack per
/// level in a build without optimizations. Everything else is priced in this unit; see `PARSE_STACK_BYTES`.
pub(crate) const FRAME_COST: usize = 4;

/// Which language's lexical rules the scan follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Syntax {
    /// `jsx` is whether the file may hold JSX, whose text can look like strings and comments.
    #[cfg(feature = "lang-ts")]
    TypeScript { jsx: bool },
    #[cfg(feature = "lang-rust")]
    Rust,
}

impl Syntax {
    /// The cost of `<`: each level of generic arguments is five tree levels deep in Rust and about three frames' worth
    /// of stack in TypeScript, and a comma inside does not end it.
    const fn angle_cost(self) -> usize {
        match self {
            #[cfg(feature = "lang-ts")]
            Self::TypeScript { .. } => 2,
            #[cfg(feature = "lang-rust")]
            Self::Rust => 4,
        }
    }

    /// Whether the source may hold text that is neither code nor a string or comment: JSX text.
    const fn text_may_hide_code(self) -> bool {
        match self {
            #[cfg(feature = "lang-ts")]
            Self::TypeScript { jsx } => jsx,
            #[cfg(feature = "lang-rust")]
            Self::Rust => false,
        }
    }

    /// Whether a word right after a closing `}` on the same line starts a new statement. In JSX it can be the next
    /// attribute.
    const fn brace_ends_statement(self) -> bool {
        !self.text_may_hide_code()
    }

    /// Whether a line ending in an operand ends the statement when the next line starts with a word. Rust has
    /// semicolons.
    const fn newline_ends_statement(self) -> bool {
        match self {
            #[cfg(feature = "lang-ts")]
            Self::TypeScript { .. } => true,
            #[cfg(feature = "lang-rust")]
            Self::Rust => false,
        }
    }

    #[cfg(feature = "lang-rust")]
    const fn is_rust(self) -> bool {
        match self {
            #[cfg(feature = "lang-ts")]
            Self::TypeScript { .. } => false,
            Self::Rust => true,
        }
    }
}

/// The largest cost the source can reach, or the first cost over `limit` if it passes it, which ends the scan early so
/// the frame stack stays within `limit / FRAME_COST` entries.
pub(crate) fn peak_cost(source: &str, syntax: Syntax, limit: usize) -> usize {
    let mut scan = Scan::new(source.as_bytes(), syntax);
    while scan.i < scan.bytes.len() && scan.peak <= limit {
        scan.step();
    }
    scan.peak
}

/// `Ok` if `source` nests no deeper than `max_levels` levels of brackets, or the equivalent in other nesting.
pub(crate) fn check(source: &str, syntax: Syntax, max_levels: usize) -> Result<(), Degradation> {
    let limit = max_levels * FRAME_COST;
    let cost = peak_cost(source, syntax, limit);
    if cost > limit { Err(Degradation::TooDeep { depth: cost.div_ceil(FRAME_COST), cap: max_levels }) } else { Ok(()) }
}

/// A bracket that is open.
struct Frame {
    kind: u8,
    /// Where it opened.
    pos: usize,
    /// The enclosing frame's cost, base and keep, restored when this one closes.
    cost: usize,
    base: usize,
    keep: usize,
    /// The parenthesis of an `if`, `for`, `while` or `with`, whose closer does not end an operand: a statement follows.
    head: bool,
    /// The `${` of a template, whose closer resumes the template's text.
    template: bool,
}

/// What follows skipped text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Resume {
    /// A string, a regular expression or the end of a template: an operand has ended.
    Operand,
    /// A comment: the scan is where it was.
    Trivia,
    /// The `${` of a template: a frame opens.
    Substitution,
}

/// Text scanned as code because it might be.
struct Region {
    start: usize,
    end: usize,
    close_len: usize,
    resume: Resume,
}

/// The last token, as far as the scan needs to know.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    Other,
    /// The `}` of a brace frame.
    ClosedBrace,
    /// `if`, `for`, `while` or `with`.
    HeadWord,
    /// A word that is an operator in some positions and a name in others, such as `of` or `await`.
    ContextualWord,
}

struct Scan<'s> {
    bytes: &'s [u8],
    syntax: Syntax,
    i: usize,
    /// The recursion the position is inside, in units.
    cost: usize,
    /// What `cost` returns to at a `;`: the cost when the innermost frame opened.
    base: usize,
    /// What `cost` returns to at a `,`: `base` plus the angle brackets since.
    keep: usize,
    peak: usize,
    frames: Vec<Frame>,
    /// The last token ends an operand, so a `/` after it divides.
    operand: bool,
    /// What the last token was, when that matters to the next.
    last: Last,
    /// A line break came since the last token.
    newline: bool,
    plain: Option<Region>,
    /// Closers before this offset pop nothing.
    no_pop_until: usize,
}

impl<'s> Scan<'s> {
    fn new(bytes: &'s [u8], syntax: Syntax) -> Self {
        Self {
            bytes,
            syntax,
            i: 0,
            cost: 0,
            base: 0,
            keep: 0,
            peak: 0,
            frames: Vec::new(),
            operand: false,
            last: Last::Other,
            newline: false,
            plain: None,
            no_pop_until: 0,
        }
    }

    fn at(&self, index: usize) -> u8 {
        self.bytes.get(index).copied().unwrap_or(0)
    }

    fn bump(&mut self, cost: usize) {
        self.cost += cost;
        self.peak = self.peak.max(self.cost);
    }

    /// A cost that a `,` does not undo.
    fn bump_kept(&mut self, cost: usize) {
        self.bump(cost);
        self.keep += cost;
    }

    fn reset(&mut self) {
        self.cost = self.base;
        self.keep = self.base;
    }

    /// The state after any token but a bracket or a word.
    fn token(&mut self, operand: bool) {
        self.operand = operand;
        self.last = Last::Other;
        self.newline = false;
    }

    fn step(&mut self) {
        if let Some(region) = &self.plain
            && self.i >= region.end
        {
            self.end_plain();
            return;
        }
        let byte = self.at(self.i);
        match byte {
            b'\n' => {
                self.newline = true;
                self.i += 1;
            }
            b if b.is_ascii_whitespace() => self.i += 1,
            b if is_word_byte(b) => self.word(),
            b'\\' => {
                // Outside a string only an identifier's escape has one, and a bracket cannot follow it.
                self.i += if self.at(self.i + 1) == b'\r' && self.at(self.i + 2) == b'\n' { 3 } else { 2 };
            }
            b'(' | b'[' | b'{' => self.open(byte),
            b')' | b']' | b'}' => self.close(byte),
            b',' => {
                self.cost = self.keep;
                self.token(false);
                self.i += 1;
            }
            b';' => {
                self.reset();
                self.token(false);
                self.i += 1;
            }
            b'/' | b'\'' | b'"' | b'`' if self.plain.is_none() => self.quote_or_slash(byte),
            _ => self.punctuation(byte),
        }
    }

    fn punctuation(&mut self, byte: u8) {
        let previous = self.at(self.i.wrapping_sub(1));
        let mut operand = false;
        match byte {
            b'<' => self.bump_kept(self.syntax.angle_cost()),
            #[cfg(feature = "lang-rust")]
            b'|' if self.syntax.is_rust() => self.bump_kept(1),
            _ => self.bump(1),
        }
        match byte {
            // `a++` and `a--` end an operand, and `a!` does when it follows one.
            b'+' | b'-' => operand = previous == byte && self.i > 0,
            b'!' => operand = self.operand,
            _ => {}
        }
        self.token(operand);
        self.i += 1;
    }

    fn word(&mut self) {
        let start = self.i;
        let mut end = start;
        while is_word_byte(self.at(end)) && end < self.bytes.len() {
            end += 1;
        }
        let word = self.bytes.get(start..end).unwrap_or_default();
        #[cfg(feature = "lang-rust")]
        if self.syntax.is_rust()
            && matches!(word, b"r" | b"br" | b"cr")
            && let Some((content, close_len)) = raw_string(self.bytes, end)
        {
            self.skip(content, close_len, Resume::Operand);
            return;
        }
        self.starts_operand(word);
        self.bump(1);
        self.token(!is_operator_word(word));
        if matches!(word, b"if" | b"for" | b"while" | b"with") {
            self.last = Last::HeadWord;
        } else if is_contextual_word(word) {
            self.last = Last::ContextualWord;
        }
        self.i = end;
    }

    /// A word or literal is about to start. After an operand that ends its statement it begins the next one, so nothing
    /// that came before it nests around it.
    fn starts_operand(&mut self, word: &[u8]) {
        if is_continuation_word(word) || !self.operand {
            return;
        }
        let same_line = self.last == Last::ClosedBrace && self.syntax.brace_ends_statement();
        if same_line || (self.newline && self.syntax.newline_ends_statement()) {
            self.reset();
        }
    }

    fn open(&mut self, kind: u8) {
        let head = kind == b'(' && self.last == Last::HeadWord;
        self.push(kind, head, false);
        self.i += 1;
        self.token(false);
    }

    fn push(&mut self, kind: u8, head: bool, template: bool) {
        self.frames.push(Frame {
            kind,
            pos: self.i,
            cost: self.cost,
            base: self.base,
            keep: self.keep,
            head,
            template,
        });
        self.bump(FRAME_COST);
        self.base = self.cost;
        self.keep = self.cost;
    }

    fn close(&mut self, byte: u8) {
        let kind = match byte {
            b')' => b'(',
            b']' => b'[',
            _ => b'{',
        };
        let floor = self.plain.as_ref().map_or(0, |region| region.start);
        let poppable = self.i >= self.no_pop_until
            && self.frames.last().is_some_and(|frame| frame.kind == kind && frame.pos >= floor);
        let Some(frame) = poppable.then(|| self.frames.pop()).flatten() else {
            self.bump(1);
            self.token(true);
            self.i += 1;
            return;
        };
        self.cost = frame.cost;
        self.base = frame.base;
        self.keep = frame.keep;
        self.bump(1);
        self.i += 1;
        if frame.template {
            self.template(self.i);
            return;
        }
        self.token(!frame.head);
        if kind == b'{' && !frame.head {
            self.last = Last::ClosedBrace;
        }
    }

    fn quote_or_slash(&mut self, byte: u8) {
        match self.syntax {
            #[cfg(feature = "lang-ts")]
            Syntax::TypeScript { .. } => self.typescript_quote_or_slash(byte),
            #[cfg(feature = "lang-rust")]
            Syntax::Rust => self.rust_quote_or_slash(byte),
        }
    }

    /// Skips the text `content`, closed by `close_len` bytes, or scans it as code if it might be code. Text that cannot
    /// be code is free; text that might be is [`Syntax::text_may_hide_code`]'s JSX text.
    fn skip(&mut self, content: Range<usize>, close_len: usize, resume: Resume) {
        let text = self.bytes.get(content.clone()).unwrap_or_default();
        if self.syntax.text_may_hide_code() {
            if text.contains(&b'{') {
                self.plain = Some(Region { start: content.start, end: content.end, close_len, resume });
                self.i = content.start;
                return;
            }
            let angles = text.split(|&byte| byte == b'<').count().saturating_sub(1);
            self.bump_kept(angles);
        }
        if resume == Resume::Trivia && text.contains(&b'\n') {
            self.newline = true;
        }
        self.finish(content.end + close_len, resume);
    }

    fn end_plain(&mut self) {
        if let Some(region) = self.plain.take() {
            self.finish(region.end + region.close_len, region.resume);
        }
    }

    /// Resumes after skipped text.
    fn finish(&mut self, next: usize, resume: Resume) {
        self.i = next;
        match resume {
            Resume::Trivia => {}
            Resume::Operand => {
                self.token(true);
            }
            Resume::Substitution => {
                // The frame sits at the `{` of the `${`.
                self.i = next.saturating_sub(1);
                self.push(b'{', false, true);
                self.i = next;
                self.token(false);
            }
        }
    }

    /// The text of a template starting at `from`, up to its closing backtick or its next `${`.
    fn template(&mut self, from: usize) {
        let mut end = from;
        loop {
            match self.at(end) {
                _ if end >= self.bytes.len() => return self.skip(from..self.bytes.len(), 0, Resume::Operand),
                b'\\' => end += 2,
                b'`' => return self.skip(from..end, 1, Resume::Operand),
                b'$' if self.at(end + 1) == b'{' => return self.skip(from..end, 2, Resume::Substitution),
                _ => end += 1,
            }
        }
    }

    #[cfg(feature = "lang-ts")]
    fn typescript_quote_or_slash(&mut self, byte: u8) {
        let start = self.i;
        match byte {
            b'`' => {
                self.starts_operand(b"`");
                self.template(start + 1);
            }
            b'/' => match self.at(start + 1) {
                b'/' => {
                    let end = self.line_end(start + 2);
                    self.skip(start + 2..end, 0, Resume::Trivia);
                }
                b'*' => {
                    let rest = self.bytes.get(start + 2..).unwrap_or_default();
                    match rest.windows(2).position(|pair| pair == b"*/") {
                        Some(offset) => self.skip(start + 2..start + 2 + offset, 2, Resume::Trivia),
                        None => self.skip(start + 2..self.bytes.len(), 0, Resume::Trivia),
                    }
                }
                _ => self.slash(),
            },
            _ => {
                let end = self.string_end(start, byte);
                match end {
                    Some(end) => {
                        self.starts_operand(b"\"");
                        self.skip(start + 1..end, 1, Resume::Operand);
                    }
                    None => self.punctuation(byte),
                }
            }
        }
    }

    /// Where the line that `from` is on ends. JavaScript ends one at a line feed, a carriage return, U+2028 or U+2029.
    #[cfg(feature = "lang-ts")]
    fn line_end(&self, from: usize) -> usize {
        let rest = self.bytes.get(from..).unwrap_or_default();
        let offset = rest.iter().enumerate().position(|(at, &byte)| {
            matches!(byte, b'\n' | b'\r')
                || (byte == 0xe2 && rest.get(at + 1) == Some(&0x80) && matches!(rest.get(at + 2), Some(0xa8 | 0xa9)))
        });
        offset.map_or(self.bytes.len(), |offset| from + offset)
    }

    /// Where the string opened by the quote at `start` closes on its line, if it does.
    #[cfg(feature = "lang-ts")]
    fn string_end(&self, start: usize, quote: u8) -> Option<usize> {
        let mut index = start + 1;
        while index < self.bytes.len() {
            match self.at(index) {
                b'\\' => index += if self.at(index + 1) == b'\r' && self.at(index + 2) == b'\n' { 3 } else { 2 },
                b'\n' | b'\r' => return None,
                byte if byte == quote => return Some(index),
                _ => index += 1,
            }
        }
        None
    }

    /// A `/` that is not a comment: a division after an operand, a regular expression otherwise.
    #[cfg(feature = "lang-ts")]
    fn slash(&mut self) {
        let start = self.i;
        let after_angle = start > 0 && self.at(start - 1) == b'<';
        let ambiguous = matches!(self.last, Last::ClosedBrace | Last::ContextualWord);
        if after_angle || (self.operand && !ambiguous) {
            self.punctuation(b'/');
            return;
        }
        let Some(end) = self.regex_end(start) else {
            self.punctuation(b'/');
            return;
        };
        if ambiguous {
            // After a `}` or a word that is only sometimes an operator the `/` may divide or start a regular
            // expression. As one it hides closers, so none of them up to its end may pop a frame.
            self.no_pop_until = end + 1;
            self.punctuation(b'/');
            return;
        }
        self.skip(start + 1..end, 1, Resume::Operand);
    }

    #[cfg(feature = "lang-ts")]
    fn regex_end(&self, start: usize) -> Option<usize> {
        let mut index = start + 1;
        let mut class = false;
        while index < self.bytes.len() {
            match self.at(index) {
                b'\\' => index += 1,
                b'\n' | b'\r' => return None,
                0xe2 if self.at(index + 1) == 0x80 && matches!(self.at(index + 2), 0xa8 | 0xa9) => return None,
                b'[' => class = true,
                b']' => class = false,
                b'/' if !class => return Some(index),
                _ => {}
            }
            index += 1;
        }
        None
    }

    #[cfg(feature = "lang-rust")]
    fn rust_quote_or_slash(&mut self, byte: u8) {
        let start = self.i;
        match byte {
            b'/' => match self.at(start + 1) {
                b'/' => {
                    let rest = self.bytes.get(start + 2..).unwrap_or_default();
                    let end = rest.iter().position(|&byte| byte == b'\n').map_or(self.bytes.len(), |at| start + 2 + at);
                    self.skip(start + 2..end, 0, Resume::Trivia);
                }
                b'*' => {
                    let (end, close_len) = rust_block_comment_end(self.bytes, start + 2);
                    self.skip(start + 2..end, close_len, Resume::Trivia);
                }
                _ => self.punctuation(byte),
            },
            b'"' => {
                let mut index = start + 1;
                while index < self.bytes.len() && self.at(index) != b'"' {
                    index += if self.at(index) == b'\\' { 2 } else { 1 };
                }
                let end = index.min(self.bytes.len());
                self.starts_operand(b"\"");
                self.skip(start + 1..end, usize::from(end < self.bytes.len()), Resume::Operand);
            }
            b'\'' => match self.char_literal_len(start) {
                Some(len) => {
                    self.starts_operand(b"'");
                    self.skip(start + 1..start + len - 1, 1, Resume::Operand);
                }
                None => self.punctuation(byte),
            },
            _ => self.punctuation(byte),
        }
    }

    /// The length of the character literal that starts at `start`, quotes included, or `None` for a lifetime or label.
    #[cfg(feature = "lang-rust")]
    fn char_literal_len(&self, start: usize) -> Option<usize> {
        let first = self.at(start + 1);
        if first == b'\\' {
            // The longest escape is `\u{10FFFF}`.
            return (start + 3..start + 13).find(|&index| self.at(index) == b'\'').map(|index| index - start + 1);
        }
        let width = match first {
            0..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        };
        (self.at(start + 1 + width) == b'\'').then_some(width + 2)
    }
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') || byte >= 0x80
}

/// Words after which an expression starts, so a `/` begins a regular expression and no operand has ended.
fn is_operator_word(word: &[u8]) -> bool {
    is_contextual_word(word)
        || matches!(
            word,
            b"case"
                | b"delete"
                | b"do"
                | b"else"
                | b"extends"
                | b"in"
                | b"instanceof"
                | b"new"
                | b"return"
                | b"throw"
                | b"typeof"
                | b"void"
        )
}

/// Operator words that can also be names, so a `/` after one may divide.
fn is_contextual_word(word: &[u8]) -> bool {
    matches!(
        word,
        b"as"
            | b"asserts"
            | b"await"
            | b"implements"
            | b"infer"
            | b"is"
            | b"keyof"
            | b"of"
            | b"readonly"
            | b"satisfies"
            | b"unique"
            | b"yield"
    )
}

/// Words that continue the statement before them even on a new line.
fn is_continuation_word(word: &[u8]) -> bool {
    matches!(
        word,
        b"as"
            | b"catch"
            | b"else"
            | b"extends"
            | b"finally"
            | b"implements"
            | b"in"
            | b"instanceof"
            | b"is"
            | b"of"
            | b"satisfies"
            | b"where"
            | b"while"
    )
}

/// The content of the raw string whose `#`s and opening quote start at `from`, if one does, and the length of its
/// closer. A raw string ends at a quote followed by as many `#`s as it opened with, or at the end of the source.
#[cfg(feature = "lang-rust")]
fn raw_string(bytes: &[u8], from: usize) -> Option<(Range<usize>, usize)> {
    let hashes = bytes.get(from..)?.iter().take_while(|&&byte| byte == b'#').count();
    if bytes.get(from + hashes) != Some(&b'"') {
        return None;
    }
    let content = from + hashes + 1;
    let closing: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
    let rest = bytes.get(content..)?;
    match rest.windows(closing.len()).position(|window| window == closing) {
        Some(at) => Some((content..content + at, closing.len())),
        None => Some((content..bytes.len(), 0)),
    }
}

/// Where the block comment whose text starts at `from` ends, and how long its closer is. Rust's nest.
#[cfg(feature = "lang-rust")]
fn rust_block_comment_end(bytes: &[u8], from: usize) -> (usize, usize) {
    let (mut depth, mut index) = (1usize, from);
    while index < bytes.len() {
        match bytes.get(index..index + 2) {
            Some(b"/*") => {
                depth += 1;
                index += 2;
            }
            Some(b"*/") => {
                depth -= 1;
                if depth == 0 {
                    return (index, 2);
                }
                index += 2;
            }
            _ => index += 1,
        }
    }
    (bytes.len(), 0)
}

#[cfg(all(test, feature = "lang-ts", feature = "lang-rust"))]
#[path = "nesting_tests.rs"]
mod tests;

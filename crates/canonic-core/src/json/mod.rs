//! Pruned borrowed JSON parser.
//!
//! Strings and numbers point into the input where they can. A [`Filter`] derived from the
//! compiled plan decides what to build: anything the plan never reads is skipped at scan
//! speed and left as [`Node::Skipped`], never materialized.
//!
//! Lenient where JSON.parse is strict, because `map` must never throw:
//! - content after the root value is ignored;
//! - skipped subtrees are only bracket-matched, not validated;
//! - strings are decoded by [`unescape`], which never fails.

mod unescape;

use std::borrow::Cow;

use memchr::memchr2;

pub use unescape::unescape;

pub const MAX_DEPTH: usize = 512;
pub const MAX_BYTES: usize = 1 << 31;

#[derive(Debug, Clone, PartialEq)]
pub enum Node<'a> {
    Null,
    Bool(bool),
    /// Raw number text, checked against the JSON number grammar.
    Num(&'a str),
    Str(Cow<'a, str>),
    Arr(Vec<Node<'a>>),
    /// Members in document order, duplicates included.
    Obj(Vec<(Cow<'a, str>, Node<'a>)>),
    /// A member or element the filter pruned. Array positions are kept.
    Skipped,
}

/// What the plan reads below one value.
///
/// `all` parses everything below. Otherwise an object is built only if `keys` is non-empty
/// (unlisted members become `Skipped`), an array only if `idxs` or `elems` is set (an
/// `idxs` entry wins over `elems` for its position), and any other container is `Skipped`
/// whole. Scalars are always read. Keys are the decoded UTF-8 bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub keys: Vec<(Box<[u8]>, Filter)>,
    pub idxs: Vec<(usize, Filter)>,
    pub elems: Option<Box<Filter>>,
    pub all: bool,
}

static ALL: Filter = Filter {
    keys: Vec::new(),
    idxs: Vec::new(),
    elems: None,
    all: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Nesting deeper than [`MAX_DEPTH`] in a part the filter builds.
    Depth,
    /// Input longer than [`MAX_BYTES`].
    Size,
    /// Malformed input at this byte offset.
    Syntax(usize),
}

/// Parses `buf`, building only what `filter` asks for. Never panics.
pub fn parse<'a>(buf: &'a [u8], filter: &Filter) -> Result<Node<'a>, ParseError> {
    if buf.len() > MAX_BYTES {
        return Err(ParseError::Size);
    }
    let pos = if buf.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    let mut p = Parser { buf, pos };
    p.ws();
    // Trailing content is ignored on purpose (JSON.parse would reject it; see module doc).
    p.value(filter, 0)
}

struct Parser<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while let Some(b' ' | b'\n' | b'\r' | b'\t') = self.buf.get(self.pos) {
            self.pos += 1;
        }
    }

    fn err<T>(&self) -> Result<T, ParseError> {
        Err(ParseError::Syntax(self.pos))
    }

    fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        if self.buf.get(self.pos) != Some(&c) {
            return self.err();
        }
        self.pos += 1;
        Ok(())
    }

    /// One value at `pos` (whitespace already skipped). `depth` counts enclosing containers.
    fn value(&mut self, f: &Filter, depth: usize) -> Result<Node<'a>, ParseError> {
        match self.buf.get(self.pos) {
            Some(b'{') if f.all || !f.keys.is_empty() => self.object(f, depth + 1),
            Some(b'[') if f.all || !f.idxs.is_empty() || f.elems.is_some() => {
                self.array(f, depth + 1)
            }
            Some(b'{' | b'[') => self.skip(),
            Some(b'"') => Ok(Node::Str(self.string()?)),
            Some(b't') => self.literal(b"true", Node::Bool(true)),
            Some(b'f') => self.literal(b"false", Node::Bool(false)),
            Some(b'n') => self.literal(b"null", Node::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.err(),
        }
    }

    fn object(&mut self, f: &Filter, depth: usize) -> Result<Node<'a>, ParseError> {
        if depth > MAX_DEPTH {
            return Err(ParseError::Depth);
        }
        self.pos += 1;
        let mut members: Vec<(Cow<'a, str>, Node<'a>)> = Vec::new();
        self.ws();
        if self.buf.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Node::Obj(members));
        }
        loop {
            self.ws();
            if self.buf.get(self.pos) != Some(&b'"') {
                return self.err();
            }
            let key = self.string()?;
            self.ws();
            self.expect(b':')?;
            self.ws();
            let child = if f.all {
                Some(&ALL)
            } else {
                // First occurrence wins in the pruned path: a repeat of a built key is skipped.
                f.keys
                    .iter()
                    .find(|(k, _)| **k == *key.as_bytes())
                    .filter(|_| {
                        !members
                            .iter()
                            .any(|(k, v)| *k == key && !matches!(v, Node::Skipped))
                    })
                    .map(|(_, c)| c)
            };
            let v = match child {
                Some(c) => self.value(c, depth)?,
                None => self.skip()?,
            };
            members.push((key, v));
            self.ws();
            match self.buf.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Node::Obj(members));
                }
                _ => return self.err(),
            }
        }
    }

    fn array(&mut self, f: &Filter, depth: usize) -> Result<Node<'a>, ParseError> {
        if depth > MAX_DEPTH {
            return Err(ParseError::Depth);
        }
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.buf.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Node::Arr(items));
        }
        loop {
            self.ws();
            let i = items.len();
            let child = if f.all {
                Some(&ALL)
            } else {
                f.idxs
                    .iter()
                    .find(|(k, _)| *k == i)
                    .map(|(_, c)| c)
                    .or(f.elems.as_deref())
            };
            items.push(match child {
                Some(c) => self.value(c, depth)?,
                None => self.skip()?,
            });
            self.ws();
            match self.buf.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Node::Arr(items));
                }
                _ => return self.err(),
            }
        }
    }

    fn literal(&mut self, word: &[u8], node: Node<'a>) -> Result<Node<'a>, ParseError> {
        if !self.buf[self.pos..].starts_with(word) {
            return self.err();
        }
        self.pos += word.len();
        Ok(node)
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`
    fn number(&mut self) -> Result<Node<'a>, ParseError> {
        let start = self.pos;
        if self.buf.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.buf.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return self.err(),
        }
        if self.buf.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            self.required_digits()?;
        }
        if let Some(b'e' | b'E') = self.buf.get(self.pos) {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.buf.get(self.pos) {
                self.pos += 1;
            }
            self.required_digits()?;
        }
        // Only ASCII was consumed, so this cannot fail.
        std::str::from_utf8(&self.buf[start..self.pos])
            .map(Node::Num)
            .or_else(|_| self.err())
    }

    fn digits(&mut self) {
        while self.buf.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
    }

    fn required_digits(&mut self) -> Result<(), ParseError> {
        if !self.buf.get(self.pos).is_some_and(u8::is_ascii_digit) {
            return self.err();
        }
        self.digits();
        Ok(())
    }

    /// Reads a string; `pos` is on the opening quote and ends after the closing one.
    fn string(&mut self) -> Result<Cow<'a, str>, ParseError> {
        let end = string_end(self.buf, self.pos + 1)?;
        let raw = &self.buf[self.pos + 1..end - 1];
        self.pos = end;
        Ok(unescape(raw))
    }

    /// Skips the value at `pos` without building it.
    fn skip(&mut self) -> Result<Node<'a>, ParseError> {
        match self.buf.get(self.pos) {
            Some(b'{' | b'[') => self.pos = skip_container(self.buf, self.pos)?,
            // Scalars are cheap: read and drop them so they are still checked.
            _ => {
                self.value(&ALL, 0)?;
            }
        }
        Ok(Node::Skipped)
    }
}

// The two scans below are the only places that walk raw bytes; Task 10 swaps them for the
// SIMD structural index.

/// Offset just past the closing quote of a string whose body starts at `from`.
fn string_end(buf: &[u8], from: usize) -> Result<usize, ParseError> {
    let mut i = from;
    loop {
        let k = memchr2(b'"', b'\\', buf.get(i..).unwrap_or_default())
            .ok_or(ParseError::Syntax(buf.len()))?;
        if buf[i + k] == b'"' {
            return Ok(i + k + 1);
        }
        i += k + 2;
    }
}

/// Offset just past the container opening at `start`. Brackets are counted, not matched
/// or validated, and the count is a plain integer, so any depth is fine here.
fn skip_container(buf: &[u8], start: usize) -> Result<usize, ParseError> {
    let mut depth = 0usize;
    let mut i = start;
    while let Some(&c) = buf.get(i) {
        match c {
            b'"' => {
                i = string_end(buf, i + 1)?;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(ParseError::Syntax(buf.len()))
}

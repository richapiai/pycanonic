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
//!
//! A skipped container is matched by [`simd::skip`], 64 bytes at a time; what the filter builds is
//! read byte by byte, strings with `memchr`.

pub mod simd;
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
    /// Members in document order, duplicates included. A member the filter doesn't list is left
    /// out; a listed one whose value it pruned is kept as `Skipped`.
    Obj(Vec<(Cow<'a, str>, Node<'a>)>),
    /// A value the filter pruned. Array positions are kept.
    Skipped,
}

/// What the plan reads below one value.
///
/// `all` parses everything below. Otherwise an object is built only if `keys` is non-empty
/// (unlisted members are left out), an array only if `idxs` or `elems` is set (an
/// `idxs` entry wins over `elems` for its position), and any other container is `Skipped`
/// whole. Scalars are always read. Keys are the decoded UTF-8 bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub keys: Vec<(Box<[u8]>, Filter)>,
    pub idxs: Vec<(usize, Filter)>,
    pub elems: Option<Box<Filter>>,
    pub all: bool,
}

/// Builds everything below.
pub(crate) static ALL: Filter = Filter {
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
    parse_in(buf, filter, true)
}

/// `simd: false` skips with the byte scan alone, the reference [`simd::skip`] must agree with.
fn parse_in<'a>(buf: &'a [u8], filter: &Filter, simd: bool) -> Result<Node<'a>, ParseError> {
    check_size(buf.len())?;
    let pos = if buf.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    let mut p = Parser { buf, pos, simd };
    p.ws();
    // Trailing content is ignored on purpose (JSON.parse would reject it; see module doc).
    p.value(filter, 0)
}

fn check_size(len: usize) -> Result<(), ParseError> {
    if len > MAX_BYTES {
        return Err(ParseError::Size);
    }
    Ok(())
}

struct Parser<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Skip containers with [`simd::skip`]; the byte scan otherwise.
    simd: bool,
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
        // Every occurrence of a wanted key is built; `Doc::get` reads the last, as `JSON.parse` does.
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
            let (raw, plain) = self.raw_string()?;
            self.ws();
            self.expect(b':')?;
            self.ws();
            let child = if f.all {
                Some(&ALL)
            } else {
                wanted(&f.keys, raw, plain)
            };
            // An unlisted member is skipped and not kept: no path reads it, so its key is never decoded.
            match child {
                Some(c) => {
                    let v = self.value(c, depth)?;
                    members.push((decode(raw, plain), v));
                }
                None => _ = self.skip()?,
            }
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
        // SAFETY: only the ASCII bytes matched above were consumed, and ASCII is UTF-8.
        Ok(Node::Num(unsafe {
            std::str::from_utf8_unchecked(&self.buf[start..self.pos])
        }))
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
        let (raw, plain) = self.raw_string()?;
        Ok(decode(raw, plain))
    }

    /// The bytes between the quotes of the string at `pos`, which ends past it, and whether they
    /// are plain: no backslash and all ASCII, so they are their own decoded text. Short plain
    /// strings, most keys and values, are settled in one byte loop.
    fn raw_string(&mut self) -> Result<(&'a [u8], bool), ParseError> {
        let start = self.pos + 1;
        let mut i = start;
        // Eight bytes at a time up to the first quote, backslash or non-ASCII byte, then by byte.
        while let Some(w) = self.buf.get(i..i + 8) {
            let x = u64::from_le_bytes(w.try_into().unwrap_or_default());
            let hit = zero_byte(x ^ (LO * u64::from(b'"')))
                | zero_byte(x ^ (LO * u64::from(b'\\')))
                | (x & HI);
            if hit != 0 {
                i += hit.trailing_zeros() as usize / 8;
                break;
            }
            i += 8;
        }
        while let Some(&c) = self.buf.get(i) {
            match c {
                b'"' => {
                    self.pos = i + 1;
                    return Ok((&self.buf[start..i], true));
                }
                b'\\' | 0x80.. => break,
                _ => i += 1,
            }
        }
        let end = scan_string_end(self.buf, i)?;
        self.pos = end;
        Ok((&self.buf[start..end - 1], false))
    }

    /// Skips the value at `pos` without building it.
    fn skip(&mut self) -> Result<Node<'a>, ParseError> {
        match self.buf.get(self.pos) {
            Some(b'{' | b'[') => self.pos = self.skip_container(self.pos)?,
            // Decoding never fails, so a skipped string only needs its end.
            Some(b'"') => self.pos = scan_string_end(self.buf, self.pos + 1)?,
            // Other scalars are cheap: read and drop them so they are still checked.
            _ => {
                self.value(&ALL, 0)?;
            }
        }
        Ok(Node::Skipped)
    }

    /// Offset just past the container opening at `start`. Where [`simd::skip`] gives up (a
    /// backslash outside a string, or no close) the byte scan decides, errors included.
    fn skip_container(&self, start: usize) -> Result<usize, ParseError> {
        match self.simd.then(|| simd::skip(self.buf, start)).flatten() {
            Some(end) => Ok(end),
            None => scan_container(self.buf, start),
        }
    }
}

const LO: u64 = 0x0101_0101_0101_0101;
const HI: u64 = 0x8080_8080_8080_8080;

/// The high bit of each zero byte of `x`. Exact up to the lowest zero byte; a byte above it may
/// be flagged falsely, which `trailing_zeros` never reads.
fn zero_byte(x: u64) -> u64 {
    x.wrapping_sub(LO) & !x & HI
}

/// `a == b` for short byte strings, the keys the parser and `Doc::get` compare all the time, by
/// overlapping word loads instead of a `memcmp` call; longer ones go to `==`.
#[inline]
pub(crate) fn same_key(a: &[u8], b: &[u8]) -> bool {
    let n = a.len();
    if n != b.len() {
        return false;
    }
    let w8 = |s: &[u8], at: usize| u64::from_le_bytes(s[at..at + 8].try_into().unwrap_or_default());
    let w4 = |s: &[u8], at: usize| u32::from_le_bytes(s[at..at + 4].try_into().unwrap_or_default());
    match n {
        0 => true,
        1..=3 => a[0] == b[0] && a[n / 2] == b[n / 2] && a[n - 1] == b[n - 1],
        4..=7 => w4(a, 0) == w4(b, 0) && w4(a, n - 4) == w4(b, n - 4),
        8..=16 => w8(a, 0) == w8(b, 0) && w8(a, n - 8) == w8(b, n - 8),
        _ => a == b,
    }
}

/// A string's text from its raw bytes; `plain` (see `Parser::raw_string`) bytes are it already.
fn decode(raw: &[u8], plain: bool) -> Cow<'_, str> {
    match plain {
        // SAFETY: `raw_string` saw every plain byte below 0x80, and ASCII is UTF-8.
        true => Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(raw) }),
        false => unescape(raw),
    }
}

/// The filter entry for the member whose key has the raw (still escaped) bytes `raw`. Filter keys
/// are decoded UTF-8: plain bytes are compared as they are, anything else is decoded first.
fn wanted<'f>(keys: &'f [(Box<[u8]>, Filter)], raw: &[u8], plain: bool) -> Option<&'f Filter> {
    let find = |key: &[u8]| keys.iter().find(|(k, _)| same_key(k, key)).map(|(_, c)| c);
    match plain {
        true => find(raw),
        false => find(unescape(raw).as_bytes()),
    }
}

// The byte scans: the reference for `simd::skip`, and the fallback where it gives up.

/// Offset just past the closing quote of a string whose body starts at `from`.
fn scan_string_end(buf: &[u8], from: usize) -> Result<usize, ParseError> {
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
fn scan_container(buf: &[u8], start: usize) -> Result<usize, ParseError> {
    let mut depth = 0usize;
    let mut i = start;
    while let Some(&c) = buf.get(i) {
        match c {
            b'"' => {
                i = scan_string_end(buf, i + 1)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // The size gate is tested on lengths alone: allocating 2 GiB aborts on hosts without overcommit.
    #[test]
    fn size_gate_rejects_only_past_max_bytes() {
        assert_eq!(check_size(MAX_BYTES), Ok(()));
        assert_eq!(check_size(MAX_BYTES + 1), Err(ParseError::Size));
        assert_eq!(check_size(usize::MAX), Err(ParseError::Size));
    }

    fn all() -> Filter {
        ALL.clone()
    }

    fn pruned() -> Filter {
        Filter {
            keys: vec![
                (b"a".as_slice().into(), all()),
                (
                    b"l".as_slice().into(),
                    Filter {
                        idxs: vec![(1, all())],
                        ..Filter::default()
                    },
                ),
            ],
            ..Filter::default()
        }
    }

    /// The SIMD skip and the byte scan agree, errors and offsets included.
    fn same(buf: &[u8]) {
        for f in [all(), pruned(), Filter::default()] {
            assert_eq!(parse_in(buf, &f, true), parse_in(buf, &f, false), "{buf:?}");
        }
    }

    #[test]
    fn a_backslash_outside_a_string_falls_back_to_the_byte_scan() {
        // In a skipped subtree `\"` opens a string for the parser but is an escaped quote to
        // stage 1; the SIMD skip must give up there.
        let doc = br#"{"x":[\"],"a":1}"#;
        assert_eq!(parse(doc, &pruned()), Err(ParseError::Syntax(doc.len())));
        same(doc);
        same(br#"{"x":[\\"],"a":1}"#);
        same(br#"{"x":{"y":[1,\],"z":"]"},"a":"\\"}"#);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4096))]

        #[test]
        fn same_key_is_byte_equality(
            a in prop::collection::vec(prop::sample::select(b"ab".to_vec()), 0..24),
            b in prop::collection::vec(prop::sample::select(b"ab".to_vec()), 0..24),
        ) {
            prop_assert_eq!(same_key(&a, &b), a == b);
            prop_assert!(same_key(&a, &a));
        }

        /// `raw_string`'s fast paths read a string as the plain byte scan plus `unescape` does.
        #[test]
        fn string_fast_path_matches_the_byte_scan(
            body in prop::collection::vec(
                prop::sample::select(b"ab\"\\u0Ae\xc3\xa9\xff ".to_vec()),
                0..40,
            )
        ) {
            let mut doc = b"\"".to_vec();
            doc.extend_from_slice(&body);
            let mut p = Parser {
                buf: &doc,
                pos: 0,
                simd: true,
            };
            let want = scan_string_end(&doc, 1).map(|end| (end, unescape(&doc[1..end - 1])));
            prop_assert_eq!(p.string().map(|s| (p.pos, s)), want);
        }

        #[test]
        fn index_and_byte_scan_agree(
            bytes in prop::collection::vec(
                prop::sample::select(b"{}[]:,\"\\ al1".to_vec()),
                0..300,
            )
        ) {
            same(&bytes);
            let mut doc = br#"{"l":[0,"#.to_vec();
            doc.extend_from_slice(&bytes);
            same(&doc);
        }
    }
}

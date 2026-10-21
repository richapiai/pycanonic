//! `Out` as JSON text, byte for byte what `JSON.stringify` writes for the same canonical value.

use crate::doc::Doc;
use crate::plan::Plan;
use crate::sink::{Out, Sink};
use crate::value::{Value, js_number_to_string};
use std::ops::Range;

pub fn write_json(plan: &Plan, out: &Out, buf: &mut Vec<u8>) {
    write_out(&plan.names, out, buf);
}

/// `write_json` with the plan's names; an `Out` with no `Out::Obj` needs none.
pub(crate) fn write_out(names: &[String], out: &Out, buf: &mut Vec<u8>) {
    match out {
        Out::Null => buf.extend_from_slice(b"null"),
        Out::Bool(true) => buf.extend_from_slice(b"true"),
        Out::Bool(false) => buf.extend_from_slice(b"false"),
        Out::Int(i) => write_int(*i, buf),
        Out::BigInt(s) => buf.extend_from_slice(s.as_bytes()),
        Out::Float(f) if f.is_finite() => buf.extend_from_slice(js_number_to_string(*f).as_bytes()),
        Out::Float(_) => buf.extend_from_slice(b"null"),
        Out::Str(s) => write_str(s, buf),
        Out::List(xs) => {
            buf.push(b'[');
            for (n, x) in xs.iter().enumerate() {
                if n > 0 {
                    buf.push(b',');
                }
                write_out(names, x, buf);
            }
            buf.push(b']');
        }
        Out::Obj(m) => {
            buf.push(b'{');
            for (n, (k, x)) in m.iter().enumerate() {
                if n > 0 {
                    buf.push(b',');
                }
                write_str(&names[*k], buf);
                buf.push(b':');
                write_out(names, x, buf);
            }
            buf.push(b'}');
        }
        Out::Map(m) => {
            buf.push(b'{');
            for (n, (k, x)) in m.iter().enumerate() {
                if n > 0 {
                    buf.push(b',');
                }
                write_str(k, buf);
                buf.push(b':');
                write_out(names, x, buf);
            }
            buf.push(b'}');
        }
    }
}

/// A `Sink` that writes JSON while the plan runs, so no `Out` tree is built: each value is written
/// to `buf` when it is made and stands for its byte range there, and a list or object copies its
/// children's ranges into place behind them. The root's range is `write_json` of what `RustSink`
/// builds for the same run, byte for byte.
pub(crate) struct JsonSink<'n> {
    pub buf: Vec<u8>,
    /// `"name":` for each of `Plan::names`, written once per artifact by `member_keys`.
    pub keys: &'n [Box<[u8]>],
}

/// Each canonical name as an object member's key, quotes and colon included.
pub(crate) fn member_keys(names: &[String]) -> Vec<Box<[u8]>> {
    let key = |name: &String| {
        let mut k = Vec::new();
        write_str(name, &mut k);
        k.push(b':');
        k.into_boxed_slice()
    };
    names.iter().map(key).collect()
}

impl JsonSink<'_> {
    /// The root's text, moved to the front of `buf` and the rest dropped.
    pub fn finish(mut self, root: Range<usize>) -> Vec<u8> {
        let n = root.len();
        self.buf.copy_within(root, 0);
        self.buf.truncate(n);
        self.buf
    }

    /// `d` as `RustSink::passthrough` builds it, then as `write_out` writes that.
    fn doc<D: Doc>(&mut self, d: &D) {
        if let Some(n) = d.len() {
            self.buf.push(b'[');
            for i in 0..n {
                if i > 0 {
                    self.buf.push(b',');
                }
                match d.item(i) {
                    Some(x) => self.doc(&x),
                    None => self.buf.extend_from_slice(b"null"),
                }
            }
            self.buf.push(b']');
        } else if let Some(m) = d.members() {
            self.buf.push(b'{');
            for (n, (k, v)) in m.into_iter().enumerate() {
                if n > 0 {
                    self.buf.push(b',');
                }
                write_str(&k, &mut self.buf);
                self.buf.push(b':');
                self.doc(&v);
            }
            self.buf.push(b'}');
        } else {
            write_value(&d.leaf(), &mut self.buf);
        }
    }
}

impl Sink for JsonSink<'_> {
    type Out = Range<usize>;

    fn scalar(&mut self, v: Value<'_>) -> Range<usize> {
        let start = self.buf.len();
        write_value(&v, &mut self.buf);
        start..self.buf.len()
    }

    fn passthrough<D: Doc>(&mut self, d: &D) -> Range<usize> {
        let start = self.buf.len();
        self.doc(d);
        start..self.buf.len()
    }

    fn list(&mut self, items: Vec<Range<usize>>) -> Range<usize> {
        let start = self.buf.len();
        self.buf.push(b'[');
        for (n, r) in items.into_iter().enumerate() {
            if n > 0 {
                self.buf.push(b',');
            }
            self.buf.extend_from_within(r);
        }
        self.buf.push(b']');
        start..self.buf.len()
    }

    fn object(&mut self, members: Vec<(usize, Range<usize>)>) -> Range<usize> {
        let start = self.buf.len();
        self.buf.push(b'{');
        for (n, (k, r)) in members.into_iter().enumerate() {
            if n > 0 {
                self.buf.push(b',');
            }
            self.buf.extend_from_slice(&self.keys[k]);
            self.buf.extend_from_within(r);
        }
        self.buf.push(b'}');
        start..self.buf.len()
    }
}

/// A scalar as `write_out` writes the `Out` that `RustSink::scalar` makes of it.
fn write_value(v: &Value<'_>, buf: &mut Vec<u8>) {
    match v {
        Value::Null | Value::Container => buf.extend_from_slice(b"null"),
        Value::Bool(true) => buf.extend_from_slice(b"true"),
        Value::Bool(false) => buf.extend_from_slice(b"false"),
        Value::Int(i) => write_int(*i, buf),
        Value::BigInt(s) => buf.extend_from_slice(s.as_bytes()),
        Value::Float(f) if f.is_finite() => {
            buf.extend_from_slice(js_number_to_string(*f).as_bytes())
        }
        Value::Float(_) => buf.extend_from_slice(b"null"),
        Value::Str(s) => write_str(s, buf),
    }
}

fn write_int(i: i64, buf: &mut Vec<u8>) {
    if i < 0 {
        buf.push(b'-');
    }
    let mut n = i.unsigned_abs();
    let mut tmp = [0u8; 20];
    let mut at = tmp.len();
    loop {
        at -= 1;
        tmp[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    buf.extend_from_slice(&tmp[at..]);
}

/// Per byte: 0 copies as is, `u` writes `\u00XX`, anything else writes `\` and that byte.
const ESCAPE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 0x20 {
        t[i] = b'u';
        i += 1;
    }
    t[0x08] = b'b';
    t[0x09] = b't';
    t[0x0a] = b'n';
    t[0x0c] = b'f';
    t[0x0d] = b'r';
    t[b'"' as usize] = b'"';
    t[b'\\' as usize] = b'\\';
    t
};

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Whether any byte of `b` is a control byte, `"` or a backslash: eight bytes at a time (a byte below
/// 0x20 borrows when 0x20 is taken from it, as a zero byte does from 1), the tail by table.
fn needs_escape(b: &[u8]) -> bool {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;
    let below = |x: u64, n: u64| x.wrapping_sub(LO * n) & !x & HI;
    let (words, tail) = b.as_chunks::<8>();
    words.iter().any(|w| {
        let x = u64::from_le_bytes(*w);
        below(x, 0x20) | below(x ^ (LO * 0x22), 1) | below(x ^ (LO * 0x5c), 1) != 0
    }) || tail.iter().any(|&c| ESCAPE[c as usize] != 0)
}

pub(crate) fn write_str(s: &str, buf: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    if !needs_escape(bytes) {
        buf.reserve(bytes.len() + 2);
        buf.push(b'"');
        buf.extend_from_slice(bytes);
        buf.push(b'"');
        return;
    }
    buf.push(b'"');
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let e = ESCAPE[b as usize];
        if e == 0 {
            continue;
        }
        buf.extend_from_slice(&bytes[start..i]);
        if e == b'u' {
            buf.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[(b >> 4) as usize],
                HEX[(b & 0xf) as usize],
            ]);
        } else {
            buf.extend_from_slice(&[b'\\', e]);
        }
        start = i + 1;
    }
    buf.extend_from_slice(&bytes[start..]);
    buf.push(b'"');
}

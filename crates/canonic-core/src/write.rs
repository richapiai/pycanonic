//! `Out` as JSON text, byte for byte what `JSON.stringify` writes for the same canonical value.

use crate::plan::Plan;
use crate::sink::Out;
use crate::value::js_number_to_string;

pub fn write_json(plan: &Plan, out: &Out, buf: &mut Vec<u8>) {
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
                write_json(plan, x, buf);
            }
            buf.push(b']');
        }
        Out::Obj(m) => {
            buf.push(b'{');
            for (n, (k, x)) in m.iter().enumerate() {
                if n > 0 {
                    buf.push(b',');
                }
                write_str(&plan.names[*k], buf);
                buf.push(b':');
                write_json(plan, x, buf);
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
                write_json(plan, x, buf);
            }
            buf.push(b'}');
        }
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

fn write_str(s: &str, buf: &mut Vec<u8>) {
    let bytes = s.as_bytes();
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

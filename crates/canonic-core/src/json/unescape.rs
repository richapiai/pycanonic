use std::borrow::Cow;

use memchr::memchr;

const REPLACEMENT: char = '\u{FFFD}';

/// Decodes a JSON string body (the bytes between the quotes).
///
/// Borrowed when the bytes carry no backslash and are valid UTF-8, owned otherwise.
/// Lenient by design, never fails: invalid UTF-8, lone surrogate escapes, unknown escapes
/// and truncated `\u` escapes all become U+FFFD. A valid surrogate pair decodes to one
/// scalar value.
pub fn unescape(raw: &[u8]) -> Cow<'_, str> {
    let Some(first) = memchr(b'\\', raw) else {
        return String::from_utf8_lossy(raw);
    };
    let mut out = String::with_capacity(raw.len());
    out.push_str(&String::from_utf8_lossy(&raw[..first]));
    let mut i = first;
    while i < raw.len() {
        if raw[i] != b'\\' {
            // Backslash is ASCII, so a run never splits a multi-byte sequence.
            let end = memchr(b'\\', &raw[i..]).map_or(raw.len(), |k| i + k);
            out.push_str(&String::from_utf8_lossy(&raw[i..end]));
            i = end;
            continue;
        }
        let Some(&e) = raw.get(i + 1) else {
            out.push(REPLACEMENT);
            break;
        };
        i += 2;
        out.push(match e {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => match hex4(raw, i) {
                None => REPLACEMENT,
                Some(hi) => {
                    i += 4;
                    if (0xD800..0xDC00).contains(&hi)
                        && raw.get(i..i + 2) == Some(b"\\u")
                        && let Some(lo @ 0xDC00..0xE000) = hex4(raw, i + 2)
                    {
                        i += 6;
                        let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                        char::from_u32(cp).unwrap_or(REPLACEMENT)
                    } else {
                        // Lone surrogates (high or low) have no char and fall out here.
                        char::from_u32(hi).unwrap_or(REPLACEMENT)
                    }
                }
            },
            _ => REPLACEMENT,
        });
    }
    Cow::Owned(out)
}

fn hex4(raw: &[u8], at: usize) -> Option<u32> {
    raw.get(at..at + 4)?
        .iter()
        .try_fold(0, |v, &c| Some(v * 16 + (c as char).to_digit(16)?))
}

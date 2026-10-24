// The comparison rule both conformance runners share (conformance/README.md). compare.py is the
// Python twin.

use serde_json::Value as J;

/// The first difference between `got` and `want` as `path: got != want`, or `None` when equal.
pub fn diff(path: &str, got: &J, want: &J) -> Option<String> {
    let differ = || Some(format!("{path}: got {got}, want {want}"));
    match (got, want) {
        (J::Number(a), J::Number(b)) => (!num_eq(a, b)).then(differ).flatten(),
        (J::Object(a), J::Object(b)) => {
            let (ka, kb): (Vec<_>, Vec<_>) = (a.keys().collect(), b.keys().collect());
            if ka != kb {
                return Some(format!("{path}: keys {ka:?}, want {kb:?}"));
            }
            a.iter()
                .zip(b.values())
                .find_map(|((k, x), y)| diff(&format!("{path}.{k}"), x, y))
        }
        (J::Array(a), J::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (x, y))| diff(&format!("{path}[{i}]"), x, y)),
        _ => (got != want).then(differ).flatten(),
    }
}

/// By value. When either side is at least 2^53 in magnitude, integer or not, within one
/// double step of `a` (TypeScript writes a rounded big integer, pycanonic keeps it exact).
fn num_eq(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
    if x.abs().max(y.abs()) >= 9007199254740992.0 {
        return (x - y).abs() <= x.abs().next_up() - x.abs();
    }
    match (a.as_i128(), b.as_i128()) {
        (Some(i), Some(j)) => i == j,
        _ => x == y,
    }
}

/// Case text with every lone surrogate escape (`\ud800`) rewritten to the U+FFFD escape, which is what
/// both runners compare against and what serde_json can hold. Surrogate pairs stay.
pub fn fffd(text: &str) -> String {
    let b = text.as_bytes();
    let hex = |i: usize| {
        b.get(i..i + 6)
            .filter(|s| s[0] == b'\\' && s[1] == b'u')
            .and_then(|s| u16::from_str_radix(std::str::from_utf8(&s[2..]).ok()?, 16).ok())
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            let n = text[i..].chars().next().unwrap().len_utf8();
            out.push_str(&text[i..i + n]);
            i += n;
            continue;
        }
        match hex(i) {
            Some(0xD800..=0xDBFF) if matches!(hex(i + 6), Some(0xDC00..=0xDFFF)) => {
                out.push_str(&text[i..i + 12]);
                i += 12;
            }
            Some(0xD800..=0xDFFF) => {
                out.push_str("\\ufffd");
                i += 6;
            }
            _ => {
                out.push_str(&text[i..i + 2]);
                i += 2;
            }
        }
    }
    out
}

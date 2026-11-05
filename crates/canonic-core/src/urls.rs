//! `url_part`: reads the host, path, a path segment or a query value of a `scheme://` URL.
//! Everything is one pass over the bytes, with its own percent-decoder (no URL crate).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UrlPart {
    Host,
    Path,
    /// The n-th non-empty path segment. Negative counts from the end.
    Segment(i64),
    /// The first value of a query parameter, by decoded name.
    Query(String),
}

/// `%XX` escapes to bytes, and `+` to a space when `plus`. `None` on a bad escape or invalid UTF-8.
fn decode(s: &str, plus: bool) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = |c: Option<&u8>| (*c? as char).to_digit(16);
                out.push((hex(b.get(i + 1))? * 16 + hex(b.get(i + 2))?) as u8);
                i += 3;
                continue;
            }
            b'+' if plus => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

pub fn url_part(s: &str, p: &UrlPart) -> Option<String> {
    let (scheme, rest) = s.split_once("://")?;
    let mut sc = scheme.chars();
    if !sc.next()?.is_ascii_alphabetic()
        || !sc.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let tail = tail.split('#').next().unwrap_or("");
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    match p {
        UrlPart::Host => {
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            // A port follows the last colon, unless that colon is inside an IPv6 literal.
            let host = match host.rfind(':') {
                Some(i) if !host[i..].contains(']') => &host[..i],
                _ => host,
            };
            (!host.is_empty()).then(|| host.to_string())
        }
        UrlPart::Path => (!path.is_empty()).then(|| decode(path, false)).flatten(),
        UrlPart::Segment(n) => {
            let segs: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
            let i = if *n < 0 {
                segs.len().checked_sub(n.unsigned_abs() as usize)?
            } else {
                usize::try_from(*n).ok()?
            };
            decode(segs.get(i)?, false)
        }
        UrlPart::Query(name) => {
            // A key that does not decode cannot be the name, so it is skipped.
            let (_, value) = query.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (decode(k, true).as_deref() == Some(name)).then_some((k, v))
            })?;
            decode(value, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "https://x.example/people/ada%20lovelace/?ref=a%2Fb&q=1+2#top";

    #[test]
    fn reads_each_part() {
        assert_eq!(url_part(U, &UrlPart::Host).unwrap(), "x.example");
        assert_eq!(
            url_part(U, &UrlPart::Path).unwrap(),
            "/people/ada lovelace/"
        );
        assert_eq!(url_part(U, &UrlPart::Segment(-1)).unwrap(), "ada lovelace");
        assert_eq!(url_part(U, &UrlPart::Query("q".into())).unwrap(), "1 2");
        assert_eq!(url_part("x.example/a", &UrlPart::Host), None);
    }

    #[test]
    fn hostile_input_is_linear_and_never_panics() {
        let long = format!("https://x.example/{}?k=1", "a".repeat(10_000));
        let escapes = format!("https://x.example/{}", "%41".repeat(3_333));
        let pairs = format!("https://x.example/?{}k=hit", "a=1&".repeat(2_500));
        let parts = [
            UrlPart::Host,
            UrlPart::Path,
            UrlPart::Segment(0),
            UrlPart::Segment(i64::MIN),
            UrlPart::Segment(i64::MAX),
            UrlPart::Query("k".into()),
        ];
        for u in [
            &long,
            &escapes,
            &pairs,
            &"%".repeat(10_000),
            &"://".repeat(3_000),
        ] {
            for p in &parts {
                let _ = url_part(u, p);
            }
        }
        assert_eq!(
            url_part(&long, &UrlPart::Segment(-1)).unwrap().len(),
            10_000
        );
        assert_eq!(
            url_part(&pairs, &UrlPart::Query("k".into())).unwrap(),
            "hit"
        );
        assert_eq!(url_part("https://x.example/%zz", &UrlPart::Path), None);
        assert_eq!(url_part("https://x.example/%", &UrlPart::Path), None);
        assert_eq!(url_part("https://x.example/%ff", &UrlPart::Path), None);
    }
}

// Stage 1: every backend of `structurals` against the portable one, and the portable one against
// a byte-at-a-time state machine.

use canonic_core::json::simd::{structurals, structurals_portable};
use proptest::prelude::*;

/// The definition, one byte at a time: quotes that are not escaped by an odd backslash run, and
/// `{ } [ ] : ,` and `\` outside strings. Backslashes escape the next byte everywhere, as in
/// simdjson, so `\` outside a string is reported for the parser to fall back on.
fn reference(buf: &[u8]) -> Vec<u32> {
    let (mut in_str, mut esc) = (false, false);
    let mut out = Vec::new();
    for (i, &b) in buf.iter().enumerate() {
        let i = i as u32;
        match b {
            b'\\' => {
                if !in_str {
                    out.push(i);
                }
                esc = !esc;
                continue;
            }
            b'"' if !esc => {
                out.push(i);
                in_str = !in_str;
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' if !in_str => out.push(i),
            _ => {}
        }
        esc = false;
    }
    out
}

fn check(buf: &[u8]) {
    let want = reference(buf);
    assert_eq!(structurals_portable(buf).idx, want, "portable on {buf:?}");
    assert_eq!(structurals(buf).idx, want, "dispatched on {buf:?}");
    let brackets: Vec<u32> = want
        .iter()
        .copied()
        .filter(|&i| matches!(buf[i as usize], b'{' | b'}' | b'[' | b']' | b'\\'))
        .collect();
    assert_eq!(
        structurals_portable(buf).brackets,
        brackets,
        "portable brackets on {buf:?}"
    );
    assert_eq!(
        structurals(buf).brackets,
        brackets,
        "dispatched brackets on {buf:?}"
    );
    #[cfg(target_arch = "x86_64")]
    assert_eq!(
        canonic_core::json::simd::structurals_sse2(buf).brackets,
        brackets,
        "sse2 brackets on {buf:?}"
    );
    #[cfg(target_arch = "x86_64")]
    assert_eq!(
        canonic_core::json::simd::structurals_sse2(buf).idx,
        want,
        "sse2 on {buf:?}"
    );
}

#[test]
fn simple_document() {
    let doc = br#"{"a\"]":[1,{"b":"\\"}],"c":"x"}"#;
    assert_eq!(
        structurals(doc).idx,
        vec![
            0, 1, 6, 7, 8, 10, 11, 12, 14, 15, 16, 19, 20, 21, 22, 23, 25, 26, 27, 29, 30
        ]
    );
    check(doc);
}

#[test]
fn empty_and_short_inputs() {
    for doc in [&b""[..], b"{", b"\"", b"\\", b"\\\"", b" \"\\\"\" "] {
        check(doc);
    }
}

/// Every 6-byte pattern over the bytes that matter, straddling the first 64-byte block boundary.
#[test]
fn every_pattern_across_the_block_boundary() {
    const A: &[u8] = b"\\\"{a";
    for off in 56..=66 {
        for n in 0..A.len().pow(6) {
            let mut buf = vec![b' '; 140];
            let mut k = n;
            for j in 0..6 {
                buf[off + j] = A[k % A.len()];
                k /= A.len();
            }
            check(&buf);
        }
    }
}

#[test]
fn long_backslash_runs() {
    for run in 0..200 {
        for lead in 0..64 {
            let mut buf = vec![b'"'; 1];
            buf.extend(std::iter::repeat_n(b' ', lead));
            buf.extend(std::iter::repeat_n(b'\\', run));
            buf.extend_from_slice(b"\"],\"");
            check(&buf);
        }
    }
}

fn string_body() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just("a".to_string()),
            Just("é€😀".to_string()),
            Just(r#"\""#.to_string()),
            Just(r"\\".to_string()),
            Just(r#"\\\""#.to_string()),
            Just(r"\né".to_string()),
            Just("{[:,]}".to_string()),
            (1usize..80).prop_map(|n| r"\\".repeat(n)),
            (0usize..70).prop_map(|n| " ".repeat(n)),
        ],
        0..10,
    )
    .prop_map(|v| v.concat())
}

fn json_value() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        string_body().prop_map(|s| format!("\"{s}\"")),
        Just("null".to_string()),
        Just("-1.5e3".to_string()),
        Just("true".to_string()),
    ];
    leaf.prop_recursive(6, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(|v| format!("[{}]", v.join(","))),
            prop::collection::vec((string_body(), inner), 0..8).prop_map(|v| {
                let m: Vec<String> = v.iter().map(|(k, x)| format!("\"{k}\": {x}")).collect();
                format!("{{{}}}", m.join(", "))
            }),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn json_documents_match(pad in 0usize..64, doc in json_value()) {
        let buf = format!("{}{doc}", " ".repeat(pad));
        check(buf.as_bytes());
    }

    #[test]
    fn hostile_bytes_match(
        bytes in prop::collection::vec(
            prop_oneof![
                prop::sample::select(b"{}[]:,\"\\ a".to_vec()),
                any::<u8>(),
            ],
            0..400,
        )
    ) {
        check(&bytes);
    }
}

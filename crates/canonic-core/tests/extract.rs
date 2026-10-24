// Port of canonic/packages/canonic/test/extract.test.ts, the examples of docs/streaming.md, the
// bench-body comparisons against the TypeScript extract outputs, and the generated-document
// property from fuzz/extract.test.ts (extract equals the routing fields of map, whole and chunked).

use canonic_core::{Artifact, Out, RustSink, Sink, Value, compile};
use proptest::prelude::*;
use serde_json::{Value as J, json};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> J {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

fn artifact(spec: &J, schema: &J) -> Artifact {
    Artifact::new(compile(spec, schema, &[]).unwrap(), HashMap::new()).unwrap()
}

/// The spec of extract.test.ts, over tests/fixtures/canonical.json.
fn acme_spec() -> J {
    json!({
        "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted",
        "anchors": { "phone": "\"mobile\":" },
        "fields": {
            "email": { "coalesce": ["work_email", "emails[0].address"], "transform": ["trim", "lower"] },
            "phone": { "from": "mobile", "transform": "normalize_phone" },
            "company.domain": { "from": "org.website", "transform": "to_domain" },
            "first_name": "name"
        }
    })
}

static ACME: LazyLock<Artifact> =
    LazyLock::new(|| artifact(&acme_spec(), &fixture("canonical.json")));

/// The same spec without anchors: scanner mode only.
static ACME_SCAN: LazyLock<Artifact> = LazyLock::new(|| {
    let mut s = acme_spec();
    s["anchors"] = json!({});
    artifact(&s, &fixture("canonical.json"))
});

static GLOBEX: LazyLock<Artifact> = LazyLock::new(|| {
    artifact(
        &fixture("specs/globex.json"),
        &fixture("specs/canonical/person.enrich.json"),
    )
});

fn bench_spec(anchored: bool) -> J {
    let mut s = fixture("specs/bench.json");
    if anchored {
        // canonic/bench/person.js `anchoredSpec`.
        s["anchors"] = json!({ "email": "\"workEmail\":", "phone": "\"mobile\":", "company.domain": "\"website\":" });
        s["fields"]["email"] =
            json!({ "from": "contact.workEmail", "transform": ["trim", "lower"] });
    }
    s
}

static BENCH: LazyLock<Artifact> =
    LazyLock::new(|| artifact(&bench_spec(false), &fixture("specs/canonical/bench.json")));
static BENCH_ANCHORED: LazyLock<Artifact> =
    LazyLock::new(|| artifact(&bench_spec(true), &fixture("specs/canonical/bench.json")));
static BODIES: LazyLock<J> = LazyLock::new(|| fixture("bench_bodies.json"));

fn raws() -> Vec<&'static [u8]> {
    BODIES["raws"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().as_bytes())
        .collect()
}

/// JavaScript has one number type: a whole Float compares equal to the integer JSON.stringify writes.
fn num(f: f64) -> J {
    if f.fract() == 0.0 && f.abs() < 9e15 {
        json!(f as i64)
    } else {
        serde_json::Number::from_f64(f).map_or(J::Null, J::Number)
    }
}

fn out_js(o: &Out) -> J {
    match o {
        Out::Bool(b) => json!(b),
        Out::Int(i) => json!(i),
        Out::BigInt(s) => num(s.parse().unwrap()),
        Out::Float(f) => num(*f),
        Out::Str(s) => json!(s),
        Out::Null | Out::List(_) | Out::Obj(_) | Out::Map(_) => J::Null,
    }
}

/// An extracted value as `extract` reports it to JavaScript: missing and rejected are both null.
fn js(v: &Option<Value<'static>>) -> J {
    v.clone().map_or(J::Null, |v| out_js(&RustSink.scalar(v)))
}

fn keyed(a: &Artifact, vals: &[Option<Value<'static>>]) -> J {
    assert_eq!(vals.len(), a.plan.routing.len());
    let m = a.plan.routing.iter().zip(vals);
    J::Object(m.map(|(r, v)| (r.name.clone(), js(v))).collect())
}

fn ex(a: &Artifact, raw: &[u8]) -> J {
    keyed(a, &a.extract(raw))
}

/// The routing fields of `map_bytes(raw)`, keyed like `extract`.
fn map_routing(a: &Artifact, raw: &[u8]) -> J {
    let out = a.map_bytes(raw);
    let at = |name: &str| {
        let mut cur = &out;
        for part in name.split('.') {
            let Out::Obj(m) = cur else { return J::Null };
            cur = &m.iter().find(|(i, _)| a.plan.names[*i] == part).unwrap().1;
        }
        out_js(cur)
    };
    J::Object(
        a.plan
            .routing
            .iter()
            .map(|r| (r.name.clone(), at(&r.name)))
            .collect(),
    )
}

fn streamed<'c>(a: &Artifact, chunks: impl IntoIterator<Item = &'c [u8]>) -> J {
    let mut s = a.stream();
    for c in chunks {
        s.push(c);
    }
    keyed(a, &s.result())
}

fn bytewise(a: &Artifact, raw: &[u8]) -> J {
    streamed(a, raw.chunks(1))
}

// extract.test.ts: artifact.extract

#[test]
fn reads_the_anchored_field_and_ignores_key_shaped_text_inside_strings() {
    let raw = br#"{"bio":"says \"mobile\": 999","name":"x","mobile":"+44 20 7946 0018","org":{"website":"acme.com"}}"#;
    assert_eq!(
        ex(&ACME, raw),
        json!({ "email": null, "phone": "+442079460018", "company.domain": "acme.com" })
    );
}

#[test]
fn a_token_after_an_escaped_quote_is_not_the_anchor() {
    // The first key is `"mobile`. Its text holds `"mobile":` behind a backslash.
    let raw = br#"{"\"mobile":"+44 20 7946 0018","mobile":"4155550100"}"#;
    assert_eq!(ex(&ACME, raw)["phone"], json!("4155550100"));
    // An even run is an escaped backslash: the quote after it is real.
    let raw = br#"{"a\\":1,"mobile":"4155550100"}"#;
    assert_eq!(ex(&ACME, raw)["phone"], json!("4155550100"));
}

#[test]
fn anchors_are_searched_in_the_order_the_spec_lists_them() {
    let mut spec = bench_spec(true);
    spec["anchors"] = json!({ "company.domain": "\"website\":", "email": "\"workEmail\":", "phone": "\"mobile\":" });
    let plan = compile(&spec, &fixture("specs/canonical/bench.json"), &[]).unwrap();
    let names: Vec<&str> = plan
        .anchors
        .iter()
        .map(|&i| plan.routing[i].name.as_str())
        .collect();
    assert_eq!(names, ["company.domain", "email", "phone"]);
}

#[test]
fn anchors_out_of_document_order_are_still_found() {
    let raw =
        br#"{"org":{"website":"acme.com"},"contact":{"mobile":"4155550100","workEmail":"A@b.co"}}"#;
    assert_eq!(
        ex(&BENCH_ANCHORED, raw),
        json!({ "email": "a@b.co", "phone": "4155550100", "company.domain": "acme.com" })
    );
}

#[test]
fn falls_back_to_the_scanner_when_the_anchor_token_is_not_there_verbatim() {
    assert_eq!(
        ex(&ACME, br#"{ "mobile" : "4155550100" }"#)["phone"],
        json!("4155550100")
    );
}

#[test]
fn an_empty_key_anchor_that_is_not_there_verbatim_falls_back_instead_of_looping() {
    let spec = json!({
        "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted",
        "anchors": { "phone": "\"\":" },
        "fields": { "phone": { "from": "[\"\"]", "transform": "normalize_phone" } }
    });
    let a = artifact(&spec, &fixture("canonical.json"));
    assert_eq!(
        ex(&a, br#"{"" : "4155550100"}"#)["phone"],
        json!("4155550100")
    );
    assert_eq!(
        ex(&a, br#"{"":"4155550100"}"#)["phone"],
        json!("4155550100")
    );
}

#[test]
fn never_panics_on_malformed_or_truncated_input() {
    let good = json!({ "work_email": "a@b.co", "mobile": "4155550100", "org": { "website": "acme.com" }, "pad": "x".repeat(50) });
    let good = serde_json::to_vec(&good).unwrap();
    for i in 0..=good.len() {
        ACME.extract(&good[..i]);
        ACME_SCAN.extract(&good[..i]);
        bytewise(&ACME, &good[..i]);
    }
    assert_eq!(
        ex(&ACME, b""),
        json!({ "email": null, "phone": null, "company.domain": null })
    );
}

#[test]
fn a_spec_without_routing_fields_extracts_nothing() {
    let mut s = acme_spec();
    s["anchors"] = json!({});
    s["fields"] = json!({ "first_name": "name" });
    let a = artifact(&s, &fixture("canonical.json"));
    assert!(a.extract(br#"{"name":"Ada"}"#).is_empty());
    let mut st = a.stream();
    assert!(st.push(b""));
    assert!(st.result().is_empty());
}

// extract.test.ts: artifact.stream

#[test]
fn push_reports_done_at_the_last_routing_field_before_the_body_ends() {
    let raw = r#"{"work_email":"A@b.co","mobile":"4155550100","org":{"website":"acme.com"},"emails":[],"rest":[1,2,3]}"#;
    let mut s = acme_spec();
    s["anchors"] = json!({});
    s["fields"] = json!({ "email": "work_email", "phone": "mobile" });
    let a = artifact(&s, &fixture("canonical.json"));
    let mut st = a.stream();
    let done_at = raw.as_bytes().chunks(1).position(|c| st.push(c));
    // The closing quote of the phone value.
    assert_eq!(done_at, Some(raw.find("4155550100").unwrap() + 10));
    // Pushing after done is ignored.
    assert!(st.push(b"garbage"));
    assert_eq!(
        keyed(&a, &st.result()),
        json!({ "email": "A@b.co", "phone": "4155550100" })
    );
}

#[test]
fn a_value_split_across_chunks_is_read_after_the_next_push() {
    let mut s = ACME.stream();
    assert!(!s.push(br#"{"mobile":"415555"#));
    assert!(s.push(br#"0100","work_email":"a@b.co","org":{"website":"acme.com"}}"#));
    assert_eq!(keyed(&ACME, &s.result())["phone"], json!("4155550100"));
}

#[test]
fn result_before_the_end_treats_unseen_fields_as_missing() {
    let mut s = ACME.stream();
    s.push(br#"{"mobile":"4155550100""#);
    let r = s.result();
    assert_eq!(
        keyed(&ACME, &r),
        json!({ "email": null, "phone": "4155550100", "company.domain": null })
    );
    assert_eq!(r[0], None);
}

// docs/streaming.md

#[test]
fn the_extract_example_of_the_streaming_doc() {
    let raw = br#"{"fname":"Ada","e_mail":" Ada@Globex.com ","tel":"+1 415 555 0100","org_url":"https://globex.com/x"}"#;
    let want = json!({ "email": "ada@globex.com", "phone": "+14155550100", "company.domain": "globex.com" });
    assert_eq!(ex(&GLOBEX, raw), want);
    assert_eq!(map_routing(&GLOBEX, raw), want);
}

#[test]
fn the_stream_example_of_the_streaming_doc() {
    let mut s = GLOBEX.stream();
    assert!(!s.push(br#"{"fname":"Ada","e_mail":" Ada@Glo"#));
    assert!(!s.push(br#"bex.com ","tel":"+1 415 555 0100","#));
    assert!(s.push(br#""org_url":"https://globex.com/x""#));
    assert_eq!(
        keyed(&GLOBEX, &s.result()),
        json!({ "email": "ada@globex.com", "phone": "+14155550100", "company.domain": "globex.com" })
    );
}

#[test]
fn a_scalar_ending_a_chunk_is_read_on_the_next_push() {
    let spec = json!({
        "provider": "globex", "endpoint": "person.enrich", "version": 1, "status": "promoted",
        "fields": { "phone": { "from": "tel", "transform": "normalize_phone" } }
    });
    let a = artifact(&spec, &fixture("specs/canonical/person.enrich.json"));
    let mut s = a.stream();
    assert!(!s.push(br#"{"tel":4155550100"#)); // phone not resolved yet
    assert!(s.push(br#","x":1"#)); // now it is
    assert_eq!(
        keyed(&a, &s.result()),
        ex(&a, br#"{"tel":4155550100,"x":1}"#)
    );
}

#[test]
fn malformed_input_returns_the_fields_read_before_the_damage() {
    let r = GLOBEX.extract(br#"{"e_mail":"a@b.com", oops"#);
    assert_eq!(keyed(&GLOBEX, &r)["email"], json!("a@b.com"));
    assert_eq!(r[1], None);
}

#[test]
fn a_duplicate_key_resolves_to_its_first_occurrence() {
    let raw = br#"{"e_mail":"first@a.com","e_mail":"last@a.com"}"#;
    assert_eq!(ex(&GLOBEX, raw)["email"], json!("first@a.com"));
    // Through the scanner too (the anchor token is spaced out of verbatim).
    let raw = br#"{"e_mail" :"first@a.com","e_mail" :"last@a.com"}"#;
    assert_eq!(ex(&GLOBEX, raw)["email"], json!("first@a.com"));
    assert_eq!(bytewise(&GLOBEX, raw)["email"], json!("first@a.com"));
}

#[test]
fn stream_always_scans_while_extract_uses_the_anchor() {
    // The anchor reads whichever `"e_mail":` comes first, wherever it sits (known-limits.md).
    let raw = br#"{"manager":{"e_mail":"boss@a.com"},"e_mail":"me@a.com"}"#;
    assert_eq!(ex(&GLOBEX, raw)["email"], json!("boss@a.com"));
    assert_eq!(bytewise(&GLOBEX, raw)["email"], json!("me@a.com"));
}

// Values, the pipeline and the None / Some(Null) split

#[test]
fn missing_is_none_and_a_rejected_value_is_some_null() {
    let r = ACME_SCAN.extract(br#"{"work_email":"not an email","org":{"website":{"a":1}}}"#);
    assert_eq!(r, vec![Some(Value::Null), None, Some(Value::Null)]);
}

#[test]
fn coalesce_skips_null_and_absent_paths_and_does_not_retry_after_a_rejection() {
    let a = &*ACME_SCAN;
    let r = ex(
        a,
        br#"{"work_email":null,"emails":[{"address":" X@Y.com "}]}"#,
    );
    assert_eq!(r["email"], json!("x@y.com"));
    let r = a.extract(br#"{"work_email":"nope","emails":[{"address":"x@y.com"}]}"#);
    assert_eq!(r[0], Some(Value::Null));
}

#[test]
fn a_from_many_routing_field_runs_its_list_transform() {
    let spec = json!({
        "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted",
        "fields": { "email": { "from": ["user", "host"], "transform": { "join": "@" } } }
    });
    let a = artifact(&spec, &fixture("canonical.json"));
    for raw in [
        &br#"{"user":"ada","host":"acme.com"}"#[..],
        br#"{"host":"acme.com"}"#,
        br#"{"x":1}"#,
    ] {
        assert_eq!(
            ex(&a, raw),
            map_routing(&a, raw),
            "{}",
            String::from_utf8_lossy(raw)
        );
    }
    assert_eq!(
        ex(&a, br#"{"user":"ada","host":"acme.com"}"#)["email"],
        json!("ada@acme.com")
    );
    assert_eq!(a.extract(br#"{"x":1}"#), vec![None]);
}

#[test]
fn a_path_below_a_captured_value_is_read_from_the_captured_text_first_key_wins() {
    let spec = json!({
        "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted",
        "fields": {
            "email": "contact",
            "phone": { "from": "contact.tel", "transform": "normalize_phone" }
        }
    });
    let a = artifact(&spec, &fixture("canonical.json"));
    let raw = br#"{"contact":{"tel":"+1 415 555 0100","tel":"+44 20 7946 0018"}}"#;
    let r = a.extract(raw);
    assert_eq!(r[0], Some(Value::Null)); // an object fails the email validator
    assert_eq!(keyed(&a, &r)["phone"], json!("+14155550100"));
    assert_eq!(bytewise(&a, raw), keyed(&a, &r));
    // `map` keeps the last, as JSON.parse does (known-limits.md, "Duplicate keys").
    assert_eq!(map_routing(&a, raw)["phone"], json!("+442079460018"));
}

#[test]
fn escapes_lone_surrogates_and_a_byte_order_mark() {
    let raw = "\u{FEFF}{\"work_email\":\"\\u0041da@b.co\",\"mobile\":\"\\ud83d4155550100\"}";
    assert_eq!(
        ex(&ACME_SCAN, raw.as_bytes()),
        map_routing(&ACME_SCAN, raw.as_bytes())
    );
    assert_eq!(ex(&ACME_SCAN, raw.as_bytes())["email"], json!("ada@b.co"));
    assert_eq!(
        bytewise(&ACME_SCAN, raw.as_bytes()),
        ex(&ACME_SCAN, raw.as_bytes())
    );
}

#[test]
fn numbers_and_literals_go_through_the_executor_rules() {
    let spec = json!({
        "provider": "globex", "endpoint": "person.enrich", "version": 1, "status": "promoted",
        "fields": { "phone": { "from": "tel", "transform": "normalize_phone" } }
    });
    let a = artifact(&spec, &fixture("specs/canonical/person.enrich.json"));
    for v in [
        "4155550100",
        "14155550100.0",
        "1e10",
        "-0",
        "12345678901234567890123",
        "true",
        "null",
        "[1]",
        "12x",
    ] {
        let raw = format!(r#"{{"tel":{v}}}"#);
        if v != "12x" {
            assert_eq!(
                ex(&a, raw.as_bytes()),
                map_routing(&a, raw.as_bytes()),
                "{v}"
            );
        }
        assert_eq!(bytewise(&a, raw.as_bytes()), ex(&a, raw.as_bytes()), "{v}");
    }
    // Not a JSON number: missing, as `Number("12x")` is in TypeScript.
    assert_eq!(a.extract(br#"{"tel":12x}"#), vec![None]);
}

// Bench bodies

#[test]
fn every_bench_body_extracts_the_typescript_routing_outputs() {
    for (i, raw) in raws().into_iter().enumerate() {
        assert_eq!(ex(&BENCH, raw), BODIES["routing"][i], "body {i}");
        assert_eq!(
            ex(&BENCH_ANCHORED, raw),
            BODIES["anchored"][i],
            "anchored body {i}"
        );
        assert_eq!(ex(&BENCH, raw), map_routing(&BENCH, raw), "map body {i}");
    }
}

/// One byte at a time, so routing values split across a `\u` escape.
#[test]
fn every_bench_body_streamed_one_byte_at_a_time_equals_extract() {
    for (i, raw) in raws().into_iter().enumerate() {
        assert_eq!(bytewise(&BENCH, raw), ex(&BENCH, raw), "body {i}");
        assert_eq!(
            bytewise(&BENCH_ANCHORED, raw),
            ex(&BENCH_ANCHORED, raw),
            "anchored body {i}"
        );
    }
}

/// The body with `\u00e9` written into the email and website values.
fn with_escape(raw: &[u8]) -> Vec<u8> {
    let text = std::str::from_utf8(raw).unwrap();
    let text = text.replacen("\"workEmail\":\"", "\"workEmail\":\"\\u00e9", 1);
    text.replacen("\"website\":\"", "\"website\":\"x\\u00e9", 1)
        .into_bytes()
}

#[test]
fn a_routing_value_split_inside_a_unicode_escape_streams_like_extract() {
    for raw in raws() {
        let raw = with_escape(raw);
        let whole = ex(&BENCH, &raw);
        assert_eq!(whole, map_routing(&BENCH, &raw));
        assert_eq!(bytewise(&BENCH, &raw), whole);
        let esc = raw.windows(6).position(|w| w == b"\\u00e9").unwrap();
        for cut in esc..=esc + 6 {
            assert_eq!(
                streamed(&BENCH, [&raw[..cut], &raw[cut..]]),
                whole,
                "cut {cut}"
            );
        }
    }
    // The decoded value reaches the transforms: an é makes the website a different domain.
    let raw = br#"{"contact":{"workEmail":"\u00e9@x.com"},"org":{"website":"caf\u00e9.com"}}"#;
    let mut s = BENCH.stream();
    for c in raw.chunks(1) {
        s.push(c);
    }
    let r = s.result();
    assert_eq!(r, BENCH.extract(raw));
    assert_eq!(keyed(&BENCH, &r), map_routing(&BENCH, raw));
}

/// Wall-clock ratios are only checked with `PYCANONIC_TIMING_TESTS=1` (CI's non-blocking timing
/// job): shared runners are too noisy to block on them.
fn timing() -> bool {
    std::env::var_os("PYCANONIC_TIMING_TESTS").is_some_and(|v| v == "1")
}

/// The fastest of `n` runs of `f`, so one slow run (a context switch) cannot decide a ratio.
fn fastest(n: usize, mut f: impl FnMut()) -> Duration {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

/// A bench body behind `scale` copies of every resumable state stretched: a long unwanted
/// container, a long key, a long value and a long captured email.
fn padded(scale: usize) -> Vec<u8> {
    let body = std::str::from_utf8(raws()[0]).unwrap();
    let pad = format!(
        r#"{{"pad":{{"s":"{}","n":[{}0]}},"{}":"{}","contact":{{"workEmail":"{}@x.com"}},"#,
        "x".repeat(2_000 * scale),
        "1,".repeat(500 * scale),
        "k".repeat(1_000 * scale),
        "v".repeat(1_000 * scale),
        "a".repeat(1_000 * scale),
    );
    format!("{pad}{}", &body[1..]).into_bytes()
}

#[test]
fn one_byte_pushes_stay_linear() {
    // Four times the input must cost about four times as much. A rescan per push costs 16.
    let (small, big) = (padded(4), padded(16));
    assert_eq!(bytewise(&BENCH, &big), ex(&BENCH, &big));
    if !timing() {
        return;
    }
    let t_small = fastest(3, || _ = bytewise(&BENCH, &small));
    let t_big = fastest(3, || _ = bytewise(&BENCH, &big));
    let ratio = t_big.as_secs_f64() / t_small.as_secs_f64();
    assert!(
        ratio < 12.0,
        "{ratio:.1}x for 4x the input ({t_small:?} -> {t_big:?})"
    );
}

#[test]
fn extract_stops_reading_at_the_last_routing_field() {
    // All three routing fields in the first 200 bytes, behind one small unwanted object.
    let prefix = br#"{"x":{"a":[1,2]},"contact":{"workEmail":"a@b.co","mobile":"4155550100"},"org":{"website":"acme.com"}"#;
    let mut big = prefix.to_vec();
    big.extend_from_slice(br#","pad":["#);
    while big.len() < 1 << 20 {
        big.extend_from_slice(br#"{"k":"[{\"x\":1}]","n":[1,{"m":null}]},"#);
    }
    big.extend_from_slice(b"0]}");
    for a in [&*BENCH, &*BENCH_ANCHORED] {
        assert_eq!(ex(a, &big), ex(a, prefix));
        assert_eq!(
            ex(a, prefix),
            json!({ "email": "a@b.co", "phone": "4155550100", "company.domain": "acme.com" })
        );
        if !timing() {
            continue;
        }
        let t_prefix = fastest(50, || _ = a.extract(prefix));
        let t_big = fastest(50, || _ = a.extract(&big));
        let ratio = t_big.as_secs_f64() / t_prefix.as_secs_f64();
        assert!(
            ratio < 50.0,
            "{ratio:.1}x the prefix's time ({t_prefix:?} -> {t_big:?})"
        );
    }
}

// Generated documents (fuzz/gen.ts, reduced to the keys the acme spec reads)

#[derive(Debug, Clone)]
enum G {
    Obj(Vec<(String, G)>),
    Arr(Vec<G>),
    Str(String),
    /// Raw JSON text: numbers, literals and strings with escapes a Rust String cannot hold.
    Raw(String),
}

const KEYS: &[&str] = &[
    "work_email",
    "emails",
    "address",
    "mobile",
    "org",
    "website",
    "name",
    "a",
    "x y",
    "q\"uote",
    "back\\slash",
    "é",
    "",
    " ",
];
const STRS: &[&str] = &[
    " ADA@Acme.com ",
    "a@b.co",
    "N/A",
    "",
    "x@y",
    "+1 (415) 555-0100",
    "4155550100",
    "n/a",
    "https://acme.com/x",
    "acme.com",
    "\"mobile\":",
    "\"work_email\": \"evil@x.com\"",
    "{\"a\":",
    "\\\"mobile\\\":",
    "\\",
    "\\\"",
    "}",
    "]",
    ",",
    ":",
    "😀",
    "line\nbreak",
    "/",
    "\u{1f}",
];
const RAWS: &[&str] = &[
    "0",
    "-0",
    "1e400",
    "-1e-400",
    "5E+3",
    "0.0",
    "4155550100",
    "12345678901234567890123",
    "-12.5",
    "true",
    "false",
    "null",
    r#""\ud83d""#,
    r#""\ude00x""#,
    r#""caf\u00e9.com""#,
    r#""a\u0040b.co""#,
    r#""\"mobile\": 1""#,
];

fn key() -> impl Strategy<Value = String> {
    prop_oneof![6 => prop::sample::select(KEYS).prop_map(String::from), 1 => ".{0,4}"]
}

fn doc() -> impl Strategy<Value = G> {
    let leaf = prop_oneof![
        prop::sample::select(STRS).prop_map(|s| G::Str(s.into())),
        ".{0,12}".prop_map(G::Str),
        prop::sample::select(RAWS).prop_map(|s| G::Raw(s.into())),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec((key(), inner.clone()), 0..6).prop_map(G::Obj),
            prop::collection::vec(inner, 0..6).prop_map(G::Arr),
        ]
    })
}

const EMAILS: &[&str] = &[" ADA@Acme.com ", "a@b.co", "x\"y@q.io"];
const PHONES: &[&str] = &["+1 (415) 555-0100", "4155550100", "+44 20 7946 0018"];
const SITES: &[&str] = &["https://acme.com/x", "acme.com", "www.Globex.io"];

/// A value at a routing path: mostly one the transforms and validator accept.
fn routed(ok: &'static [&'static str]) -> impl Strategy<Value = G> {
    prop_oneof![3 => prop::sample::select(ok).prop_map(|s| G::Str(s.into())), 1 => doc()]
}

/// A root member at a path the acme spec reads, or anything.
fn member() -> impl Strategy<Value = (String, G)> {
    let entry = |k: &'static str, v: BoxedStrategy<G>| v.prop_map(move |v| (k.to_string(), v));
    prop_oneof![
        1 => entry("work_email", routed(EMAILS).boxed()),
        1 => entry("mobile", routed(PHONES).boxed()),
        1 => entry(
            "emails",
            prop::collection::vec(
                prop::collection::vec(prop_oneof![
                    3 => routed(EMAILS).prop_map(|v| ("address".to_string(), v)),
                    1 => (key(), doc()),
                ], 0..3)
                .prop_map(G::Obj),
                0..3,
            )
            .prop_map(G::Arr)
            .boxed(),
        ),
        1 => entry(
            "org",
            prop::collection::vec(prop_oneof![
                3 => routed(SITES).prop_map(|v| ("website".to_string(), v)),
                1 => (key(), doc()),
            ], 0..3)
            .prop_map(G::Obj)
            .boxed(),
        ),
        4 => (key(), doc()),
    ]
}

fn root() -> impl Strategy<Value = G> {
    prop_oneof![
        8 => prop::collection::vec(member(), 0..8).prop_map(G::Obj),
        1 => doc(),
    ]
}

/// Whitespace between tokens and how often characters are written as `\uXXXX`.
struct Fmt {
    ws: &'static str,
    escape: u8,
}

fn quote(s: &str, f: &Fmt, out: &mut String) {
    out.push('"');
    for (i, ch) in s.chars().enumerate() {
        let esc = f.escape == 2 || (f.escape == 1 && i % 2 == 0);
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 || esc => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn serialize(g: &G, f: &Fmt, out: &mut String) {
    match g {
        G::Obj(e) => {
            out.push('{');
            for (i, (k, v)) in e.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(f.ws);
                quote(k, f, out);
                out.push_str(f.ws);
                out.push(':');
                out.push_str(f.ws);
                serialize(v, f, out);
            }
            out.push_str(f.ws);
            out.push('}');
        }
        G::Arr(v) => {
            out.push('[');
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(f.ws);
                serialize(x, f, out);
            }
            out.push_str(f.ws);
            out.push(']');
        }
        G::Str(s) => quote(s, f, out),
        G::Raw(r) => out.push_str(r),
    }
}

/// Times `key` occurs as an object key anywhere, and whether it is a root key.
/// `g` with only the first occurrence of each key in every object.
fn first_keys(g: &G) -> G {
    match g {
        G::Obj(e) => {
            let mut seen = std::collections::HashSet::new();
            let e = e.iter().filter(|(k, _)| seen.insert(k.as_str()));
            G::Obj(e.map(|(k, v)| (k.clone(), first_keys(v))).collect())
        }
        G::Arr(v) => G::Arr(v.iter().map(first_keys).collect()),
        other => other.clone(),
    }
}

fn count_key(g: &G, key: &str) -> usize {
    match g {
        G::Obj(e) => e
            .iter()
            .map(|(k, v)| usize::from(k == key) + count_key(v, key))
            .sum(),
        G::Arr(v) => v.iter().map(|x| count_key(x, key)).sum(),
        G::Str(_) | G::Raw(_) => 0,
    }
}

/// Splits `raw` into chunks of the given sizes, cycling, the last one taking what is left.
fn chunked<'r>(raw: &'r [u8], sizes: &[usize]) -> Vec<&'r [u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    for n in sizes.iter().cycle() {
        if i >= raw.len() {
            break;
        }
        let end = (i + n).min(raw.len());
        out.push(&raw[i..end]);
        i = end;
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// `extract` keeps the first of a duplicated key and `map` the last, as in TypeScript
    /// (known-limits.md), so `map` reads the document with only the first of each key.
    #[test]
    fn extract_equals_the_routing_fields_of_map_whole_and_chunked(
        g in root(),
        ws in prop::sample::select(&["", " ", "\n  ", "\t", " \r\n "][..]),
        escape in 0u8..3,
        sizes in prop::collection::vec(1usize..80, 1..8),
    ) {
        let mut raw = String::new();
        serialize(&g, &Fmt { ws, escape }, &mut raw);
        let raw = raw.as_bytes();
        let mut firsts = String::new();
        serialize(&first_keys(&g), &Fmt { ws, escape }, &mut firsts);
        let want = map_routing(&ACME_SCAN, firsts.as_bytes());
        prop_assert_eq!(&ex(&ACME_SCAN, raw), &want);
        prop_assert_eq!(&streamed(&ACME_SCAN, chunked(raw, &sizes)), &want);
        // Anchored: holds while the anchor key sits once, at the root.
        if let G::Obj(e) = &g
            && count_key(&g, "mobile") == 1
            && e.iter().any(|(k, _)| k == "mobile")
        {
            prop_assert_eq!(&ex(&ACME, raw), &want);
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic(
        bytes in prop_oneof![
            prop::collection::vec(any::<u8>(), 0..200),
            prop::collection::vec(prop::sample::select(b"{}[]:,\"\\ ae1u0".to_vec()), 0..200),
        ],
        sizes in prop::collection::vec(1usize..16, 1..4),
    ) {
        for a in [&*ACME, &*ACME_SCAN, &*GLOBEX] {
            a.extract(&bytes);
            streamed(a, chunked(&bytes, &sizes));
        }
    }
}

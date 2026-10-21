use canonic_core::json::{MAX_DEPTH, Node};
use canonic_core::{Filter, ParseError, parse, unescape};
use proptest::prelude::*;
use std::borrow::Cow;

fn all() -> Filter {
    Filter {
        all: true,
        ..Filter::default()
    }
}

fn keys(ks: Vec<(&str, Filter)>) -> Filter {
    Filter {
        keys: ks
            .into_iter()
            .map(|(k, f)| (k.as_bytes().into(), f))
            .collect(),
        ..Filter::default()
    }
}

fn members<'n, 'a>(n: &'n Node<'a>) -> &'n [(Cow<'a, str>, Node<'a>)] {
    match n {
        Node::Obj(m) => m,
        other => panic!("expected Obj, got {other:?}"),
    }
}

fn str_of<'n, 'a>(n: &'n Node<'a>) -> &'n Cow<'a, str> {
    match n {
        Node::Str(s) => s,
        other => panic!("expected Str, got {other:?}"),
    }
}

#[test]
fn plain_strings_are_borrowed() {
    let doc = parse(br#"{"name":"Ada"}"#, &all()).unwrap();
    let m = members(&doc);
    assert!(matches!(m[0].0, Cow::Borrowed("name")));
    assert!(matches!(str_of(&m[0].1), Cow::Borrowed("Ada")));
    assert!(matches!(unescape(b"plain"), Cow::Borrowed("plain")));
}

#[test]
fn escaped_strings_decode() {
    let doc = parse(br#"["a\"b\\c\/d\n\t\r\b\f\u00e9"]"#, &all()).unwrap();
    let Node::Arr(items) = doc else {
        panic!("expected Arr")
    };
    let s = str_of(&items[0]);
    assert!(matches!(s, Cow::Owned(_)));
    assert_eq!(s, "a\"b\\c/d\n\t\r\u{8}\u{c}\u{e9}");
}

#[test]
fn lone_surrogates_become_replacement() {
    assert_eq!(unescape(br"\ud800"), "\u{FFFD}");
    assert_eq!(unescape(br"\udfff"), "\u{FFFD}");
    assert_eq!(unescape(br"x\ud800y"), "x\u{FFFD}y");
    assert_eq!(unescape(br"\ud800\ud800"), "\u{FFFD}\u{FFFD}");
    assert_eq!(unescape(br"\ud800\u0041"), "\u{FFFD}A");
    let doc = parse(br#""\ud800""#, &all()).unwrap();
    assert_eq!(str_of(&doc), "\u{FFFD}");
}

#[test]
fn surrogate_pair_decodes() {
    assert_eq!(unescape(br"\ud83d\ude00"), "\u{1F600}");
    let doc = parse(br#"{"e":"\uD83D\uDE00!"}"#, &all()).unwrap();
    assert_eq!(str_of(&members(&doc)[0].1), "\u{1F600}!");
}

#[test]
fn invalid_utf8_is_lossy_and_owned() {
    let s = unescape(b"a\xffb");
    assert!(matches!(s, Cow::Owned(_)));
    assert_eq!(s, "a\u{FFFD}b");
    assert_eq!(unescape(b"\xff\\n"), "\u{FFFD}\n");
}

#[test]
fn bad_escapes_never_panic() {
    assert_eq!(unescape(br"\x"), "\u{FFFD}");
    assert_eq!(unescape(br"\u12"), "\u{FFFD}12");
    assert_eq!(unescape(br"\uZZZZ"), "\u{FFFD}ZZZZ");
    assert_eq!(unescape(b"\\"), "\u{FFFD}");
    assert_eq!(unescape("\\\u{e9}x".as_bytes()), "\u{FFFD}x");
    assert_eq!(unescape("\\\u{1F600}x".as_bytes()), "\u{FFFD}x");
}

#[test]
fn bom_and_whitespace_are_skipped() {
    let doc = parse(b"\xEF\xBB\xBF \n\t\r{\"a\":1}", &all()).unwrap();
    assert_eq!(members(&doc)[0].1, Node::Num("1"));
}

#[test]
fn bare_array_root_parses() {
    let doc = parse(b"[1, -2.5e3, true, false, null, \"s\"]", &all()).unwrap();
    assert_eq!(
        doc,
        Node::Arr(vec![
            Node::Num("1"),
            Node::Num("-2.5e3"),
            Node::Bool(true),
            Node::Bool(false),
            Node::Null,
            Node::Str(Cow::Borrowed("s")),
        ])
    );
}

#[test]
fn scalar_root_parses() {
    assert_eq!(parse(b" 42 ", &all()).unwrap(), Node::Num("42"));
    assert_eq!(parse(b"null", &Filter::default()).unwrap(), Node::Null);
}

fn nested(n: usize) -> Vec<u8> {
    let mut v = vec![b'['; n];
    v.extend(std::iter::repeat_n(b']', n));
    v
}

#[test]
fn nesting_at_the_limit_parses() {
    assert!(parse(&nested(MAX_DEPTH), &all()).is_ok());
}

#[test]
fn nesting_past_the_limit_is_a_depth_error() {
    assert_eq!(
        parse(&nested(MAX_DEPTH + 1), &all()),
        Err(ParseError::Depth)
    );
    assert_eq!(parse(&nested(600), &all()), Err(ParseError::Depth));
    // Unclosed and absurdly deep: still a counter, never the stack.
    assert_eq!(
        parse(&vec![b'['; 1_000_000], &all()),
        Err(ParseError::Depth)
    );
    let mut objs = b"{\"a\":".repeat(600);
    objs.extend(b"1");
    objs.extend(b"}".repeat(600));
    let deep_keys = (0..600).fold(all(), |f, _| keys(vec![("a", f)]));
    assert_eq!(parse(&objs, &deep_keys), Err(ParseError::Depth));
}

#[test]
fn deep_nesting_inside_a_skipped_subtree_is_fine() {
    let mut doc = b"{\"keep\":1,\"noise\":".to_vec();
    doc.extend(nested(100_000));
    doc.push(b'}');
    let out = parse(&doc, &keys(vec![("keep", all())])).unwrap();
    assert_eq!(members(&out), &[(Cow::Borrowed("keep"), Node::Num("1"))]);
}

#[test]
fn filter_prunes_siblings_and_noise() {
    let mut doc = br#"{"a":{"b":"want","c":{"#.to_vec();
    let mut first = true;
    while doc.len() < 1 << 20 {
        if !first {
            doc.push(b',');
        }
        first = false;
        doc.extend(format!(r#""k{}":[1,"x\"y",{{"z":null}}]"#, doc.len()).bytes());
    }
    doc.extend(b"}}}");
    let f = keys(vec![("a", keys(vec![("b", all())]))]);
    let out = parse(&doc, &f).unwrap();
    let root = members(&out);
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].0, "a");
    assert_eq!(
        members(&root[0].1),
        &[(Cow::Borrowed("b"), Node::Str(Cow::Borrowed("want")))]
    );
}

#[test]
fn filter_with_no_entry_skips_containers_but_reads_scalars() {
    let f = keys(vec![("o", Filter::default()), ("s", Filter::default())]);
    let out = parse(br#"{"o":{"x":1},"s":"v"}"#, &f).unwrap();
    assert_eq!(
        members(&out),
        &[
            (Cow::Borrowed("o"), Node::Skipped),
            (Cow::Borrowed("s"), Node::Str(Cow::Borrowed("v")))
        ]
    );
    // A keys filter meeting an array, and an array filter meeting an object.
    assert_eq!(parse(b"[1]", &keys(vec![("a", all())])), Ok(Node::Skipped));
    let elems = Filter {
        elems: Some(Box::new(all())),
        ..Filter::default()
    };
    assert_eq!(parse(br#"{"a":1}"#, &elems), Ok(Node::Skipped));
}

#[test]
fn array_filters_keep_positions() {
    let doc = br#"[{"t":0},{"t":1},{"t":2}]"#;
    let idx = Filter {
        idxs: vec![(1, keys(vec![("t", all())]))],
        ..Filter::default()
    };
    let out = parse(doc, &idx).unwrap();
    let Node::Arr(items) = out else {
        panic!("expected Arr")
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], Node::Skipped);
    assert_eq!(members(&items[1])[0].1, Node::Num("1"));
    assert_eq!(items[2], Node::Skipped);

    let every = Filter {
        elems: Some(Box::new(keys(vec![("t", all())]))),
        ..Filter::default()
    };
    let Node::Arr(items) = parse(doc, &every).unwrap() else {
        panic!("expected Arr")
    };
    assert_eq!(members(&items[2])[0].1, Node::Num("2"));
}

#[test]
fn every_occurrence_of_a_wanted_key_is_built_and_a_path_read_takes_the_last() {
    use canonic_core::{Doc, Key};
    let f = keys(vec![("a", keys(vec![("b", all())]))]);
    let out = parse(br#"{"a":{"b":1},"a":[1],"a":{"b":2}}"#, &f).unwrap();
    let m = members(&out);
    assert_eq!(m.len(), 3);
    assert_eq!(m[1].1, Node::Skipped);
    let key = |s: &str, tag| Key { s: s.into(), tag };
    let b = (&out).get(&key("a", 0)).and_then(|a| a.get(&key("b", 1)));
    assert_eq!(b, Some(&Node::Num("2")));
}

#[test]
fn many_duplicate_keys_parse_in_linear_time() {
    let n = 160_000;
    let mut doc = b"{".to_vec();
    doc.extend(b"\"a\":[],".repeat(n));
    doc.extend(b"\"a\":[]}");
    let f = keys(vec![("a", keys(vec![("b", all())]))]);
    let started = std::time::Instant::now();
    let out = parse(&doc, &f).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(members(&out).len(), n + 1);
    assert!(elapsed.as_secs() < 2, "took {elapsed:?}");
}

#[test]
fn escaped_keys_match_the_filter() {
    let f = keys(vec![("q\"k", all())]);
    let out = parse(br#"{"q\u0022k":1}"#, &f).unwrap();
    assert_eq!(members(&out)[0].1, Node::Num("1"));
}

#[test]
fn trailing_content_is_ignored() {
    assert_eq!(parse(b"{} garbage", &all()), Ok(Node::Obj(vec![])));
}

#[test]
fn syntax_errors_carry_offsets() {
    assert_eq!(parse(b"", &all()), Err(ParseError::Syntax(0)));
    assert_eq!(parse(br#"{"a" 1}"#, &all()), Err(ParseError::Syntax(5)));
    assert_eq!(parse(br#"{"a":1,}"#, &all()), Err(ParseError::Syntax(7)));
    assert_eq!(parse(b"[1 2]", &all()), Err(ParseError::Syntax(3)));
    assert_eq!(parse(b"[tru]", &all()), Err(ParseError::Syntax(1)));
    assert_eq!(parse(b"01", &all()), Ok(Node::Num("0")));
    assert_eq!(parse(b"-", &all()), Err(ParseError::Syntax(1)));
    assert_eq!(parse(b"1.", &all()), Err(ParseError::Syntax(2)));
    assert_eq!(parse(b"1e+", &all()), Err(ParseError::Syntax(3)));
    assert_eq!(parse(b"\"abc", &all()), Err(ParseError::Syntax(4)));
    assert_eq!(parse(b"[1,", &all()), Err(ParseError::Syntax(3)));
    assert!(parse(br#"{"noise":[1,"#, &keys(vec![("a", all())])).is_err());
}

// Keys and strings from canonic's hostile.test.ts, written as JSON string bodies.
const NASTY: &[&str] = &[
    r#"\""#,
    "'",
    "`",
    "${",
    "${process.exit(1)}",
    r"\\",
    r#"\\\""#,
    r"\n",
    r"\r\n",
    " ",
    "\u{a0}",
    "*/",
    "//",
    "</script>",
    "__proto__",
    "constructor",
    "prototype",
    "toString",
    "hasOwnProperty",
    "}); throw 1; ({",
    r#"\"]); throw 1; //"#,
    r"\ud800",
    r"\udfff",
    r"\u0000",
    "",
];

#[test]
fn hostile_keys_and_values_round_trip() {
    for k1 in NASTY {
        for k2 in NASTY {
            let doc = format!(r#"{{"{k1}":{{"{k2}":"V"}},"{k2}":["{k1}"]}}"#);
            let want1 = unescape(k1.as_bytes()).into_owned();
            let want2 = unescape(k2.as_bytes()).into_owned();
            let f = keys(vec![(&want1, keys(vec![(&want2, all())]))]);
            let out = parse(doc.as_bytes(), &f).unwrap();
            let inner = &members(&out)[0];
            assert_eq!(inner.0, want1);
            assert_eq!(members(&inner.1)[0].1, Node::Str(Cow::Borrowed("V")));
            assert!(parse(doc.as_bytes(), &all()).is_ok());
            // Every truncation returns, never panics.
            for cut in 0..doc.len() {
                let _ = parse(&doc.as_bytes()[..cut], &f);
                let _ = parse(&doc.as_bytes()[..cut], &all());
            }
        }
    }
}

fn pruned() -> Filter {
    keys(vec![
        ("a", keys(vec![("b", all())])),
        (
            "l",
            Filter {
                idxs: vec![(0, all())],
                elems: Some(Box::new(keys(vec![("x", all())]))),
                ..Filter::default()
            },
        ),
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = parse(&bytes, &all());
        let _ = parse(&bytes, &pruned());
        let _ = unescape(&bytes);
    }

    #[test]
    fn json_alphabet_never_panics(
        bytes in proptest::collection::vec(
            prop::sample::select(b"{}[]\",:\\ u0123456789abdfeEnrtl.-+\xff\xEF\xBB".to_vec()),
            0..256,
        )
    ) {
        let _ = parse(&bytes, &all());
        let _ = parse(&bytes, &pruned());
        let _ = unescape(&bytes);
    }
}

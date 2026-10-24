//! Port of canonic/packages/canonic/test/transforms.test.ts plus the tables in docs/transforms.md.
//! JS arrays are `Value::Container` for single-value transforms. `join` over an array uses `apply_list`.

use canonic_core::{
    ArgError, Builtin, CastTo, NamePart, Semantic, Transform, Value, builtin,
    default_transform_names, validate,
};
use proptest::prelude::*;
use serde_json::json;
use std::borrow::Cow;

fn s(x: &str) -> Value<'static> {
    Value::Str(Cow::Owned(x.to_string()))
}

fn tx(name: &str, arg: Option<serde_json::Value>) -> Builtin {
    builtin(name, arg.as_ref()).unwrap_or_else(|e| panic!("{name} {arg:?}: {e:?}"))
}

fn run(name: &str, arg: Option<serde_json::Value>, v: Value<'static>) -> Value<'static> {
    tx(name, arg).apply(v, None)
}

fn arg(a: &str) -> Option<serde_json::Value> {
    Some(json!(a))
}

#[test]
fn cases() {
    use Value::{Bool, Container, Float, Int, Null};
    let cases: Vec<(
        &str,
        Option<serde_json::Value>,
        Value<'static>,
        Value<'static>,
    )> = vec![
        ("cast", arg("number"), s("42"), Int(42)),
        ("cast", arg("number"), s(" 4.5e3 "), Int(4500)),
        ("cast", arg("number"), Int(7), Int(7)),
        ("cast", arg("number"), s("1.5"), Float(1.5)),
        ("cast", arg("number"), s("1,234"), Null),
        ("cast", arg("number"), s("0x10"), Null),
        ("cast", arg("integer"), s("0b11"), Null),
        // TypeScript checks for the prefix before trimming, so its Number(" 0x10") is 16. A provider's
        // "0x10" is an id either way, and this port rejects it after the trim too.
        ("cast", arg("number"), s(" 0x10"), Null),
        ("cast", arg("number"), s("0o7"), Null),
        ("cast", arg("number"), s(""), Null),
        ("cast", arg("number"), s("  "), Null),
        ("cast", arg("number"), Bool(true), Null),
        ("cast", arg("number"), s("Infinity"), Null),
        ("cast", arg("number"), s("-Infinity"), Null),
        ("cast", arg("number"), s("1e400"), Null),
        ("cast", arg("number"), Float(f64::INFINITY), Null),
        ("cast", arg("number"), Float(f64::NAN), Null),
        (
            "cast",
            arg("number"),
            s("\u{a0} 12 \u{3000}\u{feff}"),
            Int(12),
        ),
        ("cast", arg("number"), s("\u{85}12"), Null),
        ("cast", arg("number"), s(".5"), Float(0.5)),
        ("cast", arg("number"), s("1."), Int(1)),
        ("cast", arg("number"), s("+.5e1"), Int(5)),
        ("cast", arg("number"), s("."), Null),
        ("cast", arg("number"), s("1e"), Null),
        ("cast", arg("number"), s("1_0"), Null),
        ("cast", arg("number"), s("nan"), Null),
        ("cast", arg("number"), s("inf"), Null),
        (
            "cast",
            arg("number"),
            Value::BigInt("123456789012345678901234".into()),
            Float(1.2345678901234568e23),
        ),
        ("cast", arg("integer"), s("12"), Int(12)),
        ("cast", arg("integer"), s("12.5"), Null),
        ("cast", arg("integer"), Float(12.0), Int(12)),
        ("cast", arg("integer"), Float(1e300), Float(1e300)),
        ("cast", arg("integer"), s("-0"), Int(0)),
        ("cast", arg("boolean"), s("Yes"), Bool(true)),
        ("cast", arg("boolean"), s("0"), Bool(false)),
        ("cast", arg("boolean"), s(" FALSE\t"), Bool(false)),
        ("cast", arg("boolean"), Int(1), Bool(true)),
        ("cast", arg("boolean"), Float(-0.0), Bool(false)),
        ("cast", arg("boolean"), Int(2), Null),
        ("cast", arg("boolean"), s("maybe"), Null),
        ("cast", arg("string"), Int(12), s("12")),
        ("cast", arg("string"), Float(1.0), s("1")),
        ("cast", arg("string"), Float(1e21), s("1e+21")),
        ("cast", arg("string"), Float(-0.0), s("0")),
        (
            "cast",
            arg("string"),
            Value::BigInt("123456789012345678901234".into()),
            s("123456789012345678901234"),
        ),
        ("cast", arg("string"), Bool(false), s("false")),
        ("cast", arg("string"), s("x"), s("x")),
        ("cast", arg("string"), Container, Null),
        ("cast", arg("string"), Float(f64::NAN), Null),
        ("lower", None, s("ÀDA"), s("àda")),
        ("lower", None, s("ADA"), s("ada")),
        ("lower", None, s("ada"), s("ada")),
        ("lower", None, Int(5), Null),
        ("trim", None, s("\t ada \n"), s("ada")),
        ("trim", None, s("\u{feff}\u{3000}ada\u{a0}"), s("ada")),
        ("trim", None, s("\u{85}ada\u{85}"), s("\u{85}ada\u{85}")),
        ("trim", None, Int(5), Null),
        (
            "split_name",
            arg("first"),
            s("  Ada King Lovelace "),
            s("Ada"),
        ),
        (
            "split_name",
            arg("last"),
            s("  Ada King Lovelace "),
            s("Lovelace"),
        ),
        ("split_name", arg("first"), s("Ada King Lovelace"), s("Ada")),
        (
            "split_name",
            arg("last"),
            s("Ada King Lovelace"),
            s("Lovelace"),
        ),
        ("split_name", arg("first"), s("Ada"), s("Ada")),
        ("split_name", arg("last"), s("Ada"), Null),
        ("split_name", arg("first"), s("   "), Null),
        ("split_name", arg("last"), s("   "), Null),
        ("split_name", arg("last"), s("Ada Lovelace Jr."), s("Jr.")),
        ("join", None, s("already joined"), s("already joined")),
        ("join", None, s(""), Null),
        ("join", None, Container, Null),
        ("join", None, Int(1), Null),
        ("parse_date", None, s("2021"), s("2021")),
        ("parse_date", None, s("2021-03"), s("2021-03")),
        ("parse_date", None, s("2021-03-01"), s("2021-03-01")),
        ("parse_date", None, s(" 2021-03-01 "), s("2021-03-01")),
        (
            "parse_date",
            None,
            s("2021-03-01T23:30:00-05:00"),
            s("2021-03-01"),
        ),
        ("parse_date", None, s("2021-03-01 23:30"), s("2021-03-01")),
        ("parse_date", None, s("2020-02-29"), s("2020-02-29")),
        ("parse_date", None, s("2000-02-29"), s("2000-02-29")),
        ("parse_date", None, s("1900-02-29"), Null),
        ("parse_date", None, s("2021-02-29"), Null),
        ("parse_date", None, s("2021-13"), Null),
        ("parse_date", None, s("2021-00"), Null),
        ("parse_date", None, s("2021-04-31"), Null),
        ("parse_date", None, s("03/2021"), Null),
        ("parse_date", None, s("2021-3-1"), Null),
        ("parse_date", None, s("June 2014"), Null),
        ("parse_date", None, s("2021-03-01X"), Null),
        ("parse_date", None, Int(1614556800), s("2021-03-01")),
        ("parse_date", None, Int(1614556800000), s("2021-03-01")),
        ("parse_date", None, Int(1727600000), s("2024-09-29")),
        ("parse_date", None, Float(1614556800.0), s("2021-03-01")),
        ("parse_date", None, Float(1.5), Null),
        ("parse_date", None, Int(1700000000000000), Null),
        ("parse_date", None, Int(253402300799999), s("9999-12-31")),
        ("parse_date", None, Int(253402300800000), Null),
        ("parse_date", None, Float(1e20), Null),
        ("parse_date", None, Int(0), Null),
        ("parse_date", None, Int(-5), Null),
        (
            "parse_date",
            None,
            Value::BigInt("100000000000000000000".into()),
            Null,
        ),
        ("normalize_phone", None, Int(-5551234567), Null),
        ("normalize_phone", None, Float(12345.67), Null),
        ("normalize_phone", None, Int(14155550100), s("14155550100")),
        (
            "normalize_phone",
            None,
            Float(4155550100.0),
            s("4155550100"),
        ),
        (
            "normalize_phone",
            None,
            s("+1 (415) 555-0100"),
            s("+14155550100"),
        ),
        ("normalize_phone", None, s("415.555.0100"), s("4155550100")),
        ("normalize_phone", None, Int(4155550100), s("4155550100")),
        ("normalize_phone", None, s("555-01"), Null),
        ("normalize_phone", None, s("+1 415 555 0100 ext 2"), Null),
        ("normalize_phone", None, s("1234567890123456"), Null),
        ("normalize_phone", None, s("+"), Null),
        (
            "normalize_linkedin",
            None,
            s("https://www.linkedin.com/in/ada-l/"),
            s("https://www.linkedin.com/in/ada-l"),
        ),
        (
            "normalize_linkedin",
            None,
            s("uk.linkedin.com/in/Ada?trk=x"),
            s("https://www.linkedin.com/in/ada"),
        ),
        (
            "normalize_linkedin",
            None,
            s("http://linkedin.com/company/acme#about"),
            s("https://www.linkedin.com/company/acme"),
        ),
        (
            "normalize_linkedin",
            None,
            s("linkedin.com/school/mit"),
            s("https://www.linkedin.com/school/mit"),
        ),
        (
            "normalize_linkedin",
            None,
            s("https://www.linkedin.com/jobs/view/4012"),
            Null,
        ),
        (
            "normalize_linkedin",
            None,
            s("https://evil.com/in/ada"),
            Null,
        ),
        (
            "normalize_linkedin",
            None,
            s("https://notlinkedin.com/in/ada"),
            Null,
        ),
        (
            "normalize_linkedin",
            None,
            s("https://www.linkedin.com/in/"),
            Null,
        ),
        (
            "to_domain",
            None,
            s("https://www.Acme.com/about?x=1"),
            s("acme.com"),
        ),
        ("to_domain", None, s("ada@acme.co.uk"), s("acme.co.uk")),
        (
            "to_domain",
            None,
            s("blog.acme.com:8080"),
            s("blog.acme.com"),
        ),
        ("to_domain", None, s("acme.com."), s("acme.com")),
        ("to_domain", None, s("localhost"), Null),
        ("to_domain", None, s("acme..com"), Null),
        ("to_domain", None, s("-acme.com"), Null),
        ("to_domain", None, s("ac me.com"), Null),
        ("to_domain", None, s("bücher.de"), Null),
    ];
    for (name, a, input, expected) in cases {
        let got = run(name, a.clone(), input.clone());
        assert_eq!(got, expected, "{name}({a:?}) on {input:?}");
    }
}

#[test]
fn integral_casts_keep_float_bits_when_not_i64() {
    // -0 stays a number. 2^63 does not fit i64 and stays a float.
    assert_eq!(
        run("cast", arg("number"), Value::Float(9.3e18)),
        Value::Float(9.3e18)
    );
    assert_eq!(
        run(
            "cast",
            arg("integer"),
            Value::BigInt("-99999999999999999999".into())
        ),
        Value::Float(-1e20)
    );
}

#[test]
fn int_casts_stay_exact() {
    for to in ["number", "integer"] {
        for i in [9007199254740993, i64::MAX, i64::MIN] {
            assert_eq!(
                run("cast", arg(to), Value::Int(i)),
                Value::Int(i),
                "{to} {i}"
            );
        }
        // A string goes through Number() and rounds, as in TypeScript.
        assert_eq!(
            run("cast", arg(to), s("9007199254740993")),
            Value::Int(9007199254740992)
        );
    }
}

#[test]
fn money_extremes_match_typescript() {
    // Number() of 401 digits is Infinity, and TypeScript's `Infinity > 0` passes it.
    let huge = format!("1{}", "0".repeat(400));
    assert_eq!(validate(Semantic::Money, s(&huge)), s(&huge));
    // JSON.parse reads a 401-digit literal as Infinity, which the number branch rejects.
    assert_eq!(
        validate(Semantic::Money, Value::BigInt(huge.into())),
        Value::Null
    );
}

#[test]
fn join_lists() {
    let j =
        |a: Option<serde_json::Value>, items: Vec<Value<'static>>| tx("join", a).apply_list(&items);
    let n = Value::Null;
    assert_eq!(
        j(None, vec![s("Ada"), n.clone(), s(""), s("Lovelace")]),
        s("Ada Lovelace")
    );
    assert_eq!(j(arg("-"), vec![s("a"), Value::Int(1), s("b")]), s("a-1-b"));
    assert_eq!(j(None, vec![s("a"), Value::Float(1.0)]), s("a 1"));
    assert_eq!(
        j(
            None,
            vec![
                Value::Float(1e21),
                Value::Float(f64::NAN),
                Value::Bool(true)
            ]
        ),
        s("1e+21")
    );
    assert_eq!(
        j(None, vec![Value::BigInt("123456789012345678901".into())]),
        s("123456789012345678901")
    );
    assert_eq!(j(None, vec![n.clone(), s("")]), n);
    assert_eq!(j(None, vec![Value::Container, Value::Container]), n);
    assert_eq!(j(None, vec![]), n);
    assert_eq!(j(arg(", "), vec![s("a"), s("b")]), s("a, b"));
    // Only join reads a list. Every other transform sees an array as a container.
    assert_eq!(tx("lower", None).apply_list(&[s("A")]), n);
}

#[test]
fn builtin_arguments() {
    let b = |name: &str, a: Option<serde_json::Value>| builtin(name, a.as_ref());
    assert_eq!(
        b("cast", arg("integer")),
        Ok(Builtin::Cast(CastTo::Integer))
    );
    assert_eq!(
        b("split_name", arg("last")),
        Ok(Builtin::SplitName(NamePart::Last))
    );
    assert_eq!(b("join", None), Ok(Builtin::Join(None)));
    assert_eq!(b("join", arg("-")), Ok(Builtin::Join(Some("-".into()))));
    assert_eq!(b("lower", None), Ok(Builtin::Lower));
    assert_eq!(b("to_domain", None), Ok(Builtin::ToDomain));
    for (name, a) in [
        ("cast", None),
        ("cast", arg("float")),
        ("cast", arg("constructor")),
        ("cast", Some(json!(1))),
        ("split_name", None),
        ("split_name", arg("middle")),
        ("join", Some(json!(null))),
        ("join", Some(json!(5))),
        ("lower", Some(json!(null))),
        ("trim", arg("x")),
    ] {
        assert_eq!(
            b(name, a.clone()),
            Err(ArgError::InvalidArgument),
            "{name} {a:?}"
        );
    }
    for name in ["fix_email", "constructor", "toString", "", "Lower"] {
        assert_eq!(b(name, None), Err(ArgError::UnknownTransform), "{name}");
    }
    // The nine of language 1. tests/transforms2.rs checks the language 2 ones.
    for name in &default_transform_names()[..9] {
        let a = match *name {
            "cast" => arg("string"),
            "split_name" => arg("first"),
            _ => None,
        };
        assert!(b(name, a).is_ok(), "{name}");
    }
    assert_eq!(default_transform_names().len(), 19);
}

#[test]
fn builtin_serde_round_trip() {
    for b in all_builtins() {
        let text = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Builtin>(&text).unwrap(), b);
    }
}

#[test]
fn validators() {
    use Semantic::*;
    let cases: Vec<(Semantic, Value<'static>, bool)> = vec![
        (Email, s("Ada@Acme.com"), true),
        (Email, s("N/A"), false),
        (Email, s("a@b@acme.com"), false),
        (Email, s("@acme.com"), false),
        (Email, s("ada @acme.com"), false),
        (Email, s("ada@localhost"), false),
        (Phone, s("+1 (415) 555-0100"), true),
        (Phone, Value::Int(4155550100), true),
        (Phone, s("n/a"), false),
        (Domain, s("acme.com"), true),
        (Domain, s("Acme.COM"), true),
        (Domain, s("https://acme.com"), false),
        (LinkedinUrl, s("https://www.linkedin.com/in/ada"), true),
        (LinkedinUrl, s("https://acme.com/in/ada"), false),
        (Url, s("https://acme.com/x"), true),
        (Url, s("HTTP://Acme.com"), true),
        (Url, s("acme.com"), false),
        (Url, s("ftp://acme.com"), false),
        (IsoDate, s("2014-06-01"), true),
        (IsoDate, s("2014-06"), true),
        (IsoDate, s("2024-02-30"), false),
        (IsoDate, s(" 2014-06-01"), false),
        (IsoDate, s("June 2014"), false),
        (Money, Value::Float(1200.5), true),
        (Money, Value::Int(3), true),
        (Money, Value::BigInt("100000000000000000000".into()), true),
        (Money, Value::BigInt("-100000000000000000000".into()), false),
        (Money, s("$1,200.50"), true),
        (Money, s("USD 3.2M"), true),
        (Money, s("1200 EUR"), true),
        (Money, s("€40k"), true),
        (Money, s("₹ 5"), false),
        (Money, s("$"), false),
        (Money, s("USD"), false),
        (Money, s("-5"), false),
        (Money, s("12 apples"), false),
        (Money, s("1..2"), false),
        (Money, s("0.00"), false),
        (Money, Value::Int(0), false),
        (Money, Value::Float(f64::INFINITY), false),
    ];
    for (sem, v, ok) in cases {
        let expected = if ok { v.clone() } else { Value::Null };
        assert_eq!(validate(sem, v.clone()), expected, "{sem:?} {v:?}");
    }
}

const SEMANTICS: [Semantic; 7] = [
    Semantic::Email,
    Semantic::Phone,
    Semantic::Domain,
    Semantic::LinkedinUrl,
    Semantic::Url,
    Semantic::IsoDate,
    Semantic::Money,
];

fn all_builtins() -> Vec<Builtin> {
    use Builtin::*;
    vec![
        Cast(CastTo::Number),
        Cast(CastTo::Integer),
        Cast(CastTo::Boolean),
        Cast(CastTo::String),
        Lower,
        Trim,
        SplitName(NamePart::First),
        SplitName(NamePart::Last),
        Join(None),
        Join(Some(", ".into())),
        ParseDate,
        NormalizePhone,
        NormalizeLinkedin,
        ToDomain,
    ]
}

fn junk() -> Vec<Value<'static>> {
    use Value::*;
    vec![
        Null,
        Bool(true),
        Bool(false),
        Int(0),
        Int(-1),
        Float(-0.0),
        Float(f64::NAN),
        Float(f64::INFINITY),
        Float(f64::NEG_INFINITY),
        s(""),
        s(" "),
        s("  "),
        s("N/A"),
        s("null"),
        s("NaN"),
        s("1e400"),
        // {}, [], [null], {a: 1} and any depth of nesting are all one leaf kind.
        Container,
        Str(String::from_utf8_lossy(b"ab\xff\xfecd").into_owned().into()),
    ]
}

/// Items in the junk list a transform legitimately converts, as in the TypeScript test.
fn accepts(b: &Builtin, j: &Value) -> bool {
    let finite = |j: &Value| match j {
        Value::Int(_) => true,
        Value::Float(f) => f.is_finite(),
        _ => false,
    };
    let is_str = matches!(j, Value::Str(_));
    match b {
        Builtin::Cast(CastTo::String) => is_str || matches!(j, Value::Bool(_)) || finite(j),
        Builtin::Cast(CastTo::Boolean) => {
            matches!(j, Value::Bool(_) | Value::Int(0)) || matches!(j, Value::Float(f) if *f == 0.0)
        }
        Builtin::Cast(_) => finite(j),
        Builtin::Lower | Builtin::Trim => is_str,
        Builtin::SplitName(NamePart::First) => matches!(j, Value::Str(x) if !x.trim().is_empty()),
        Builtin::Join(_) => matches!(j, Value::Str(x) if !x.is_empty()),
        _ => false,
    }
}

#[test]
fn transforms_return_null_on_junk() {
    for b in all_builtins() {
        for j in junk() {
            if accepts(&b, &j) {
                continue;
            }
            assert_eq!(b.apply(j.clone(), None), Value::Null, "{b:?} on {j:?}");
        }
        assert_eq!(b.apply_list(&[]), Value::Null, "{b:?} on []");
        assert_eq!(b.apply_list(&[Value::Null]), Value::Null, "{b:?} on [null]");
        if !matches!(b, Builtin::Join(_)) {
            assert_eq!(b.apply_list(&junk()), Value::Null, "{b:?} on a list");
        }
    }
}

#[test]
fn junk_that_converts() {
    let replaced = String::from_utf8_lossy(b"ab\xff\xfecd").into_owned();
    assert_eq!(run("trim", None, s(&replaced)), s(&replaced));
    assert_eq!(run("lower", None, s("AB\u{fffd}")), s("ab\u{fffd}"));
    assert_eq!(
        run("cast", arg("number"), Value::Float(-0.0)),
        Value::Int(0)
    );
    assert_eq!(run("cast", arg("string"), s("")), s(""));
}

#[test]
fn validators_return_null_on_junk() {
    for sem in SEMANTICS {
        for j in junk() {
            assert_eq!(validate(sem, j.clone()), Value::Null, "{sem:?} on {j:?}");
        }
    }
}

fn any_value() -> impl Strategy<Value = Value<'static>> {
    prop_oneof![
        Just(Value::Null),
        Just(Value::Container),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Int),
        "-?[1-9][0-9]{19,40}".prop_map(|d| Value::BigInt(d.into())),
        any::<f64>().prop_map(Value::Float),
        prop_oneof![Just(f64::NAN), Just(f64::INFINITY), Just(-0.0)].prop_map(Value::Float),
        any::<String>().prop_map(|x| Value::Str(x.into())),
        "[ \t0-9a-zA-Z@./:+()#?$€,_-]{0,40}".prop_map(|x| Value::Str(x.into())),
        "(https?://)?(www\\.)?[a-z]{1,8}(\\.[a-z]{2,3}){1,2}(/(in|company|school|x)(/[a-zA-Z]{0,6})?)?"
            .prop_map(|x| Value::Str(x.into())),
        "[0-9]{4}(-[0-9]{2}(-[0-9]{2}([T ].{0,6})?)?)?".prop_map(|x| Value::Str(x.into())),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn never_panics(v in any_value(), list in prop::collection::vec(any_value(), 0..4)) {
        for b in all_builtins() {
            let _ = b.apply(v.clone(), None);
            let _ = b.apply_list(&list);
        }
        for sem in SEMANTICS {
            let out = validate(sem, v.clone());
            prop_assert!(out.is_null() || out == v || matches!(v, Value::Float(f) if f.is_nan()));
        }
    }

    #[test]
    fn ascii_trim_and_lower_match_str(x in "[\\x00-\\x7f]{0,40}") {
        prop_assert_eq!(run("trim", None, s(&x)), s(x.trim()));
        prop_assert_eq!(run("lower", None, s(&x)), s(&x.to_lowercase()));
    }
}

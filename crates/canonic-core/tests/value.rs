use canonic_core::{Value, js_number_to_string};
use std::borrow::Cow;

#[test]
fn formats_like_javascript() {
    assert_eq!(js_number_to_string(1.0), "1");
    assert_eq!(js_number_to_string(1e21), "1e+21");
    assert_eq!(js_number_to_string(0.1 + 0.2), "0.30000000000000004");
    assert_eq!(js_number_to_string(4500.0), "4500");
    assert_eq!(js_number_to_string(-0.0), "0");
    assert_eq!(js_number_to_string(1.5e-7), "1.5e-7");
    assert_eq!(
        js_number_to_string(123456789012345680000.0),
        "123456789012345680000"
    );
}

#[test]
fn formats_edge_cases_like_javascript() {
    assert_eq!(js_number_to_string(f64::NAN), "NaN");
    assert_eq!(js_number_to_string(f64::INFINITY), "Infinity");
    assert_eq!(js_number_to_string(f64::NEG_INFINITY), "-Infinity");
    assert_eq!(js_number_to_string(0.000001), "0.000001");
    assert_eq!(js_number_to_string(-1.5), "-1.5");
    assert_eq!(js_number_to_string(123.456), "123.456");
    assert_eq!(js_number_to_string(1.2345e25), "1.2345e+25");
    assert_eq!(js_number_to_string(5e-324), "5e-324");
    assert_eq!(js_number_to_string(f64::MAX), "1.7976931348623157e+308");
    // Exact tie between two shortest candidates: JS picks the even digit.
    assert_eq!(
        js_number_to_string(1727756845721204.0 + 0.25),
        "1727756845721204.2"
    );
    assert_eq!(
        js_number_to_string(-(981446413237567.0 + 0.25)),
        "-981446413237567.2"
    );
}

#[test]
fn round_trips_finite_floats() {
    // Deterministic xorshift over raw bit patterns; covers every exponent range.
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..200_000 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let x = f64::from_bits(s);
        if x.is_finite() {
            assert_eq!(js_number_to_string(x).parse::<f64>().unwrap(), x, "{x:e}");
        }
    }
}

#[test]
fn into_owned_and_is_null() {
    let text = String::from("abc");
    let v = Value::Str(Cow::Borrowed(&text));
    let owned: Value<'static> = v.clone().into_owned();
    assert_eq!(owned, v);
    assert_eq!(
        Value::BigInt(Cow::Borrowed("123")).into_owned(),
        Value::BigInt(Cow::Owned("123".into()))
    );
    assert!(Value::Null.is_null());
    assert!(!Value::Int(0).is_null());
    assert!(!Value::Container.is_null());
}

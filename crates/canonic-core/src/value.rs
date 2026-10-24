use std::borrow::Cow;

/// A scalar leaf value. `Container` stands for an object or array leaf.
#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Null,
    Bool(bool),
    Int(i64),
    BigInt(Cow<'a, str>),
    Float(f64),
    Str(Cow<'a, str>),
    Container,
}

impl Value<'_> {
    pub fn into_owned(self) -> Value<'static> {
        match self {
            Value::Null => Value::Null,
            Value::Bool(b) => Value::Bool(b),
            Value::Int(i) => Value::Int(i),
            Value::BigInt(s) => Value::BigInt(Cow::Owned(s.into_owned())),
            Value::Float(f) => Value::Float(f),
            Value::Str(s) => Value::Str(Cow::Owned(s.into_owned())),
            Value::Container => Value::Container,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// Formats `f` exactly like JavaScript's `Number.prototype.toString()`
/// (ECMA-262 Number::toString, radix 10).
pub fn js_number_to_string(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f == 0.0 {
        return "0".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let sign = if f < 0.0 { "-" } else { "" };
    let abs = f.abs();
    // `{:e}` yields the shortest round-trip digits, for example "1.5e-7" or "1e21",
    // but rounds exact ties up. ECMA-262 wants the candidate closest to `f`,
    // ties to even, which is what exact-precision `{:.*e}` produces.
    let shortest = format!("{abs:e}");
    let (mantissa, _) = shortest
        .split_once('e')
        .expect("{:e} always has an exponent");
    let k = mantissa.replace('.', "").len();
    let nearest = format!("{abs:.*e}", k - 1);
    let sci = if nearest.parse::<f64>() == Ok(abs) {
        nearest
    } else {
        shortest
    };
    let (mantissa, exp) = sci.split_once('e').expect("{:e} always has an exponent");
    let digits = mantissa.replace('.', "");
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().expect("{:e} exponent is an integer") + 1;

    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(n as usize);
        format!("{int}.{frac}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e_sign = if n - 1 < 0 { '-' } else { '+' };
        let e = (n - 1).abs();
        match digits.split_at(1) {
            (d, "") => format!("{d}e{e_sign}{e}"),
            (d, rest) => format!("{d}.{rest}e{e_sign}{e}"),
        }
    };
    format!("{sign}{body}")
}

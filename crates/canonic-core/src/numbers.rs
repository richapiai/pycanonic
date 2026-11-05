//! Language 2 number and compare transforms (R51). The arithmetic is on f64, as JavaScript does it.
//! A result that is not finite, or a division by zero, gives `Null`.

use crate::transforms::{as_f64, js_number, number};
use crate::value::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumOp {
    Add,
    Sub,
    Mul,
    Div,
    CeilDiv,
    FloorDiv,
    Min,
    Max,
    Eq,
    Lt,
    Lte,
    Gt,
    Gte,
}

impl NumOp {
    pub const ALL: [(&'static str, NumOp); 13] = [
        ("add", NumOp::Add),
        ("sub", NumOp::Sub),
        ("mul", NumOp::Mul),
        ("div", NumOp::Div),
        ("ceil_div", NumOp::CeilDiv),
        ("floor_div", NumOp::FloorDiv),
        ("min", NumOp::Min),
        ("max", NumOp::Max),
        ("eq", NumOp::Eq),
        ("lt", NumOp::Lt),
        ("lte", NumOp::Lte),
        ("gt", NumOp::Gt),
        ("gte", NumOp::Gte),
    ];

    pub fn from_name(name: &str) -> Option<NumOp> {
        Self::ALL.iter().find(|(n, _)| *n == name).map(|(_, o)| *o)
    }

    /// `eq`, `lt`, `lte`, `gt` and `gte`, which give a boolean and cannot fold.
    pub fn is_compare(self) -> bool {
        matches!(self, Self::Eq | Self::Lt | Self::Lte | Self::Gt | Self::Gte)
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, o)| *o == self)
            .map_or("", |(n, _)| n)
    }
}

/// The number a value holds: a number, or a string that holds one. Anything else is `None`.
fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Str(s) => js_number(s),
        v => as_f64(v),
    }
    .filter(|f| f.is_finite())
}

fn step(op: NumOp, a: f64, b: f64) -> Value<'static> {
    let r = match op {
        NumOp::Add => a + b,
        NumOp::Sub => a - b,
        NumOp::Mul => a * b,
        NumOp::Min => a.min(b),
        NumOp::Max => a.max(b),
        NumOp::Div | NumOp::CeilDiv | NumOp::FloorDiv if b == 0.0 => return Value::Null,
        NumOp::Div => a / b,
        NumOp::CeilDiv => (a / b).ceil(),
        NumOp::FloorDiv => (a / b).floor(),
        NumOp::Eq => return Value::Bool(a == b),
        NumOp::Lt => return Value::Bool(a < b),
        NumOp::Lte => return Value::Bool(a <= b),
        NumOp::Gt => return Value::Bool(a > b),
        NumOp::Gte => return Value::Bool(a >= b),
    };
    if r.is_finite() {
        number(r)
    } else {
        Value::Null
    }
}

/// One source and a constant: `v op k`.
pub(crate) fn with_constant(op: NumOp, k: f64, v: &Value) -> Value<'static> {
    num(v).map_or(Value::Null, |a| step(op, a, k))
}

/// Folds the sources of a `from` list left to right. Fewer than two give `Null`.
pub(crate) fn fold(op: NumOp, xs: &[Value]) -> Value<'static> {
    let [first, rest @ ..] = xs else {
        return Value::Null;
    };
    if rest.is_empty() {
        return Value::Null;
    }
    let mut acc = num(first).map_or(Value::Null, number);
    for x in rest {
        acc = match (num(&acc), num(x)) {
            (Some(a), Some(b)) => step(op, a, b),
            _ => Value::Null,
        };
    }
    acc
}

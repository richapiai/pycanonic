//! Language 2: grouping and aggregates over collected element documents. Group keys compare
//! by their key text (`cond::key_text`), the equality `eq` and `unique` use. `min` and `max` use
//! the shared value order (`cond::Rank`).

use crate::cond::{Rank, key_text, number};
use crate::doc::{Doc, KeyPath, read};
use crate::transforms::Builtin;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AggOp {
    First,
    Last,
    Min,
    Max,
    Count,
    Sum,
    Any,
    All,
    Join,
}

/// Each op by its spec name, in the order the spec error lists them.
pub const AGG_OPS: [(&str, AggOp); 9] = [
    ("first", AggOp::First),
    ("last", AggOp::Last),
    ("min", AggOp::Min),
    ("max", AggOp::Max),
    ("count", AggOp::Count),
    ("sum", AggOp::Sum),
    ("any", AggOp::Any),
    ("all", AggOp::All),
    ("join", AggOp::Join),
];

impl AggOp {
    pub fn name(self) -> &'static str {
        AGG_OPS.iter().find(|(_, o)| *o == self).unwrap().0
    }
}

/// `elems` in groups by the value at `key`. Consecutive groups start whenever the key changes.
/// Otherwise a group holds every element with its key, in order of first appearance. An element
/// with a missing or null key is a group of its own.
pub fn groups<D: Doc>(elems: Vec<D>, key: &KeyPath, consecutive: bool) -> Vec<Vec<D>> {
    let mut out: Vec<Vec<D>> = Vec::new();
    // Consecutive: the key of the last element. Otherwise: each key's group.
    let mut last: Option<String> = None;
    let mut at: HashMap<String, usize> = HashMap::new();
    for e in elems {
        let k = read(Some(&e), key).and_then(|v| key_text(&v));
        let i = match &k {
            None => None,
            Some(k) if consecutive => last.as_ref().filter(|l| *l == k).map(|_| out.len() - 1),
            Some(k) => at.get(k).copied(),
        };
        match i {
            Some(i) => out[i].push(e),
            None => {
                if let (false, Some(k)) = (consecutive, &k) {
                    at.insert(k.clone(), out.len());
                }
                out.push(vec![e]);
            }
        }
        if consecutive {
            last = k;
        }
    }
    out
}

/// What an aggregate gives: a value it computes, or one of its inputs as it is (`first`, `last`,
/// `min`, `max`), which can be a container.
#[derive(Debug, Clone, PartialEq)]
pub enum Agg<D> {
    Value(Value<'static>),
    Doc(D),
}

/// `op` over `values` (`None`: missing or null). Nulls are left out, or with `nulls_propagate` any
/// null makes the result null. An empty input gives 0 for `count` and `sum`, `false` for `any`,
/// `true` for `all`, null otherwise. `None` is an error: `sum` of a value that is not a number,
/// `any` or `all` of one that is not a boolean, `min` or `max` of values that are not all numbers
/// or all strings.
pub fn aggregate<D: Doc>(
    op: AggOp,
    values: Vec<Option<D>>,
    nulls_propagate: bool,
    sep: &str,
) -> Option<Agg<D>> {
    if nulls_propagate && values.iter().any(Option::is_none) {
        return Some(Agg::Value(Value::Null));
    }
    let xs: Vec<D> = values.into_iter().flatten().collect();
    let doc = |d: Option<D>| Some(d.map_or(Agg::Value(Value::Null), Agg::Doc));
    match op {
        AggOp::First => doc(xs.into_iter().next()),
        AggOp::Last => doc(xs.into_iter().last()),
        AggOp::Count => Some(Agg::Value(Value::Int(xs.len() as i64))),
        AggOp::Sum => sum(&xs).map(Agg::Value),
        AggOp::Min | AggOp::Max => {
            let ranked: Vec<(Rank, D)> =
                (xs.into_iter()).map(|d| (Rank::of(Some(&d)), d)).collect();
            let comparable = |r: &Rank| {
                matches!(
                    (r, ranked.first().map(|x| &x.0)),
                    (Rank::Num(_), Some(Rank::Num(_))) | (Rank::Str(_), Some(Rank::Str(_)))
                )
            };
            if !ranked.iter().all(|(r, _)| comparable(r)) {
                return None;
            }
            // The first of equal values wins.
            let want = match op {
                AggOp::Min => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Greater,
            };
            let best = (ranked.into_iter()).reduce(|a, b| match b.0.cmp(&a.0, false) == want {
                true => b,
                false => a,
            });
            doc(best.map(|(_, d)| d))
        }
        AggOp::Any | AggOp::All => {
            let bs = (xs.iter())
                .map(|d| match d.leaf() {
                    Value::Bool(b) => Some(b),
                    _ => None,
                })
                .collect::<Option<Vec<bool>>>()?;
            let v = match op {
                AggOp::Any => bs.iter().any(|b| *b),
                _ => bs.iter().all(|b| *b),
            };
            Some(Agg::Value(Value::Bool(v)))
        }
        // As the `join` transform: strings and numbers, empty strings left out, null for none.
        AggOp::Join => {
            let leaves: Vec<Value> = xs.iter().map(|d| d.leaf()).collect();
            let v = Builtin::Join(Some(sep.into())).apply_list(&leaves);
            Some(Agg::Value(v.into_owned()))
        }
    }
}

/// An integer while every value is one and the total fits, a float otherwise.
fn sum<D: Doc>(xs: &[D]) -> Option<Value<'static>> {
    let (mut int, mut float) = (Some(0i64), 0.0);
    for d in xs {
        match d.leaf() {
            Value::Int(i) => {
                int = int.and_then(|s| s.checked_add(i));
                float += i as f64;
            }
            v @ (Value::Float(_) | Value::BigInt(_)) => {
                int = None;
                float += number(&v)?;
            }
            _ => return None,
        }
    }
    Some(int.map_or(Value::Float(float), Value::Int))
}

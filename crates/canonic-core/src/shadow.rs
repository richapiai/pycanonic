//! Shadow comparison of a candidate spec against the promoted one, and the auto-merge gate, as
//! shadow.ts. Both map the same responses; every canonical field is classified on every response.

use crate::doc::Doc;
use crate::drift::{AlertKind, FieldAlert};
use crate::exec::MAX_NODES;
use crate::json::MAX_DEPTH;
use crate::sink::{Out, RustSink, Sink};
use crate::value::js_number_to_string;
use crate::write::write_out;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldDiff {
    /// Responses where at least one side had a value. Two nulls say nothing and aren't counted.
    pub compared: u64,
    pub same: u64,
    /// Promoted null, candidate non-null, or a list whose elements only gained values. What a fixed rename looks like.
    pub improvements: u64,
    /// Promoted non-null, candidate null, or a list whose elements only lost values.
    pub regressions: u64,
    /// Both non-null and different in some value, or lists of different length.
    pub conflicts: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadowReport {
    pub responses: u64,
    pub routing: Vec<String>,
    /// In the order a JavaScript object lists them: array-index paths ("0", "12") first in
    /// numeric order, then the rest as first seen.
    pub fields: Vec<(String, FieldDiff)>,
}

/// A document as an `Out` with keys by name (`Out::Map`), or `None` past `depth` levels or
/// `budget` values.
fn tree<D: Doc>(d: &D, depth: usize, budget: &mut usize) -> Option<Out> {
    if depth == 0 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Some(n) = d.len() {
        let items = (0..n).map(|i| {
            d.item(i)
                .map_or(Some(Out::Null), |x| tree(&x, depth - 1, budget))
        });
        return items.collect::<Option<_>>().map(Out::List);
    }
    if let Some(m) = d.members() {
        let members = m
            .into_iter()
            .map(|(k, v)| Some((k.into_owned(), tree(&v, depth - 1, budget)?)));
        return members.collect::<Option<_>>().map(Out::Map);
    }
    Some(RustSink.scalar(d.leaf()))
}

/// A canonical value as `ShadowDiff` and `merge_gate` read it, or `None` for a host object deeper
/// than 512 levels or with more than 1,000,000 values (one that contains itself or shares its children).
pub fn to_out_bounded<D: Doc>(d: &D) -> Option<Out> {
    let mut budget = MAX_NODES;
    tree(d, MAX_DEPTH, &mut budget)
}

/// The members of an object. Canonical values have keys by name: an `Out::Obj` needs its plan's names.
fn obj(v: &Out) -> Option<&[(String, Out)]> {
    match v {
        Out::Map(m) => Some(m),
        Out::Obj(_) => panic!("canonical values need keys by name (Out::Map), not Out::Obj"),
        _ => None,
    }
}

/// Canonical leaves by dotted path, in first-seen order. An array is one leaf. A repeated path
/// (a key "a.b" next to "a": {"b": ..}) keeps its first place and its last value, as a JavaScript Map.
struct Leaves<'a> {
    list: Vec<(String, &'a Out)>,
    at: HashMap<String, usize>,
}

impl<'a> Leaves<'a> {
    fn of(o: &'a Out) -> Self {
        let mut l = Leaves {
            list: Vec::new(),
            at: HashMap::new(),
        };
        l.collect(o, "");
        l
    }

    fn collect(&mut self, o: &'a Out, prefix: &str) {
        for (k, v) in obj(o).unwrap_or_default() {
            let path = format!("{prefix}{k}");
            if obj(v).is_some() {
                self.collect(v, &format!("{path}."));
                continue;
            }
            match self.at.entry(path) {
                Entry::Occupied(e) => self.list[*e.get()].1 = v,
                Entry::Vacant(e) => {
                    self.list.push((e.key().clone(), v));
                    e.insert(self.list.len() - 1);
                }
            }
        }
    }

    fn get(&self, path: &str) -> Option<&'a Out> {
        self.at.get(path).map(|&i| self.list[i].1)
    }
}

/// Null, a missing key, and an object with nothing but those: all "no value", as they are outside arrays.
fn blank(v: &Out) -> bool {
    matches!(v, Out::Null) || obj(v).is_some_and(|m| m.iter().all(|(_, x)| blank(x)))
}

fn num(v: &Out) -> Option<f64> {
    match v {
        Out::Int(i) => Some(*i as f64),
        Out::Float(f) => Some(*f),
        Out::BigInt(s) => s.parse().ok(),
        _ => None,
    }
}

/// `a === b` for scalars: an integer equals the same float, NaN equals nothing.
fn scalar_eq(a: &Out, b: &Out) -> bool {
    match (a, b) {
        (Out::Str(x), Out::Str(y)) => x == y,
        (Out::Bool(x), Out::Bool(y)) => x == y,
        (Out::Int(x), Out::Int(y)) => x == y,
        _ => matches!((num(a), num(b)), (Some(x), Some(y)) if x == y),
    }
}

/// True when b has every value a has, at the same place: b may fill what a left blank, never change it.
/// Key order doesn't count and a null key equals a missing one, so either side can be output that
/// wasn't built by map(), such as the code a spec replaces. Arrays must have the same length.
fn within(a: &Out, b: &Out) -> bool {
    if blank(a) || scalar_eq(a, b) {
        return true;
    }
    if let Out::List(xs) = a {
        return matches!(b, Out::List(ys) if xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| within(x, y)));
    }
    match (obj(a), obj(b)) {
        (Some(m), Some(n)) => m.iter().all(|(k, x)| {
            let y = n
                .iter()
                .find(|(j, _)| j == k)
                .map_or(&Out::Null, |(_, y)| y);
            within(x, y)
        }),
        _ => false,
    }
}

fn equal(a: &Out, b: &Out) -> bool {
    within(a, b) && within(b, a)
}

/// How the candidate's value relates to the promoted one. For a list, filling fields the promoted spec
/// left null in its elements is an improvement and the reverse a regression, the same as for a scalar;
/// a changed value or a different length is a conflict.
fn classify(d: &mut FieldDiff, promoted: &Out, candidate: &Out) {
    let fills = within(promoted, candidate);
    let loses = within(candidate, promoted);
    *match (fills, loses) {
        (true, true) => &mut d.same,
        (true, false) => &mut d.improvements,
        (false, true) => &mut d.regressions,
        (false, false) => &mut d.conflicts,
    } += 1;
}

pub struct ShadowDiff {
    report: ShadowReport,
    at: HashMap<String, usize>,
}

impl ShadowDiff {
    /// routing: the promoted artifact's routing fields (`plan.routing` names).
    pub fn new(routing: Vec<String>) -> Self {
        ShadowDiff {
            report: ShadowReport {
                responses: 0,
                routing,
                fields: Vec::new(),
            },
            at: HashMap::new(),
        }
    }

    /// Records one response: the promoted and candidate map() output for the same body, as finite
    /// documents (a parsed `Node`); see `add_bounded` for host objects.
    pub fn add<D: Doc>(&mut self, promoted: &D, candidate: &D) {
        let full = |d: &D| tree(d, usize::MAX, &mut { usize::MAX }).expect("unbounded");
        self.add_out(&full(promoted), &full(candidate));
    }

    /// `add`, unless either side is past `to_out_bounded`'s limits: then nothing is recorded and
    /// the result is false.
    pub fn add_bounded<D: Doc>(&mut self, promoted: &D, candidate: &D) -> bool {
        match (to_out_bounded(promoted), to_out_bounded(candidate)) {
            (Some(p), Some(c)) => {
                self.add_out(&p, &c);
                true
            }
            _ => false,
        }
    }

    fn add_out(&mut self, promoted: &Out, candidate: &Out) {
        self.report.responses += 1;
        let p = Leaves::of(promoted);
        let c = Leaves::of(candidate);
        let only_c = c.list.iter().filter(|(k, _)| p.get(k).is_none());
        for (path, _) in p.list.iter().chain(only_c) {
            let fields = &mut self.report.fields;
            let i = *self.at.entry(path.clone()).or_insert_with(|| {
                fields.push((path.clone(), FieldDiff::default()));
                fields.len() - 1
            });
            let a = p.get(path).unwrap_or(&Out::Null);
            let b = c.get(path).unwrap_or(&Out::Null);
            if matches!((a, b), (Out::Null, Out::Null)) {
                continue;
            }
            let d = &mut fields[i].1;
            d.compared += 1;
            classify(d, a, b);
        }
    }

    pub fn snapshot(&self) -> ShadowReport {
        let mut r = self.report.clone();
        r.fields
            .sort_by_key(|(k, _)| index(k).map_or((1, 0), |n| (0, n)));
        r
    }
}

/// A JavaScript array-index key: canonical decimal below 2^32 - 1.
fn index(k: &str) -> Option<u32> {
    let canonical =
        !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) && (k == "0" || !k.starts_with('0'));
    k.parse().ok().filter(|&n| canonical && n != u32::MAX)
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateOptions {
    /// Share of compared responses a non-routing field may regress or conflict on.
    pub max_change: f64,
    /// `(candidate, expected)`: the candidate's routing values for entities whose true values are
    /// known, next to those values, keys by name. Routing improvements only merge when every known
    /// entity matches: a validator accepts a personal email in place of a work email, a known entity doesn't.
    pub known: Vec<(Out, Out)>,
    /// Alerts a FillMonitor raised on the candidate's stats during the soak. Any of them blocks the merge.
    pub fill_alerts: Vec<FieldAlert>,
}

impl Default for GateOptions {
    fn default() -> Self {
        GateOptions {
            max_change: 0.01,
            known: Vec::new(),
            fill_alerts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    pub merge: bool,
    pub reasons: Vec<String>,
}

/// `JSON.stringify(v)`.
fn stringify(v: &Out) -> String {
    let mut buf = Vec::new();
    write_out(&[], v, &mut buf);
    String::from_utf8(buf).expect("JSON text is UTF-8")
}

/// `x.toFixed(3)`: an exact tie rounds away from zero, where `{:.3}` rounds it to even.
fn fixed3(x: f64) -> String {
    if x.is_nan() || x.abs() >= 1e21 {
        return js_number_to_string(x);
    }
    let k = x * 1000.0;
    // A tie at three decimals is a multiple of 1/16, so `k` is exact.
    if (x * 16.0).fract() == 0.0 && k.fract().abs() == 0.5 {
        return format!("{:.3}", (k + 0.5 * k.signum()) / 1000.0);
    }
    format!("{:.3}", x + 0.0)
}

/// The auto-merge rule. Routing fields: no regressions, no conflicts, improvements confirmed by known
/// entities. Every other field: regressions plus conflicts at most max_change of compared. No fill
/// alert on the candidate. Anything else waits for a person, and reasons says why.
pub fn merge_gate(report: &ShadowReport, opts: &GateOptions) -> GateResult {
    let known: Vec<(Leaves<'_>, Leaves<'_>)> = opts
        .known
        .iter()
        .map(|(c, e)| (Leaves::of(c), Leaves::of(e)))
        .collect();
    // An empty report proves nothing about the candidate.
    let mut reasons: Vec<String> = Vec::new();
    if report.responses == 0 {
        reasons.push("no responses compared".into());
    }
    for (path, d) in &report.fields {
        if report.routing.contains(path) {
            if d.regressions > 0 {
                reasons.push(format!("{path}: {} routing regressions", d.regressions));
            }
            if d.conflicts > 0 {
                reasons.push(format!("{path}: {} routing conflicts", d.conflicts));
            }
            // An improvement is only trusted when some known entity states this field's true value.
            if d.improvements > 0 && !known.iter().any(|(_, e)| e.get(path).is_some()) {
                reasons.push(format!(
                    "{path}: {} routing improvements with no known-entity check",
                    d.improvements
                ));
            }
        } else if (d.regressions + d.conflicts) as f64 > opts.max_change * d.compared as f64 {
            reasons.push(format!(
                "{path}: {} regressions and {} conflicts in {} compared",
                d.regressions, d.conflicts, d.compared
            ));
        }
    }
    for (i, (c, e)) in known.iter().enumerate() {
        for path in &report.routing {
            let Some(want) = e.get(path) else { continue };
            let got = c.get(path).unwrap_or(&Out::Null);
            if !equal(got, want) {
                reasons.push(format!(
                    "known entity {i}: {path} is {}, expected {}",
                    stringify(got),
                    stringify(want)
                ));
            }
        }
    }
    for a in &opts.fill_alerts {
        let kind = match a.kind {
            AlertKind::Fill => "fill",
            AlertKind::Errors => "errors",
        };
        reasons.push(format!(
            "{}: {kind} alert on the candidate ({} vs {})",
            a.field,
            fixed3(a.rate),
            fixed3(a.baseline)
        ));
    }
    GateResult {
        merge: reasons.is_empty(),
        reasons,
    }
}

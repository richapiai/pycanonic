//! Drift signals for a provider, as drift.ts: the shape of its responses, and fill and error rates
//! per field. A renamed key maps to null without any error, so these two are how a rename gets noticed.

use crate::doc::Doc;
use crate::stats::{FieldStats, StatsSnapshot};
use crate::write::write_str;
use std::borrow::Cow;
use std::collections::HashMap;

/// `JSON.stringify(k)`.
fn quoted(k: &str) -> String {
    let mut buf = Vec::with_capacity(k.len() + 2);
    write_str(k, &mut buf);
    String::from_utf8(buf).expect("escaping keeps UTF-8")
}

fn walk<D: Doc>(members: Vec<(Cow<'_, str>, D)>, prefix: &str, out: &mut Vec<String>) {
    for (k, x) in members {
        if x.is_null() {
            continue;
        }
        let path = format!("{prefix}{}", quoted(&k));
        if let Some(n) = x.len() {
            elements(&x, n, &format!("{path}[]."), out);
            out.push(path + "[]");
        } else {
            if let Some(m) = x.members() {
                walk(m, &format!("{path}."), out);
            }
            out.push(path);
        }
    }
}

fn elements<D: Doc>(arr: &D, n: usize, prefix: &str, out: &mut Vec<String>) {
    for e in (0..n).filter_map(|i| arr.item(i)) {
        if let Some(m) = e.members() {
            walk(m, prefix, out);
        }
    }
}

/// Sorted key paths of a parsed response. A null value counts as absent, and an array is one path
/// ending in `[]` whatever its length, with the keys of its object elements merged under it. Keys
/// are written as `JSON.stringify` writes them, and sorted by UTF-16 code units as JavaScript sorts.
pub fn shape<D: Doc>(doc: &D) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(m) = doc.members() {
        walk(m, "", &mut out);
    } else if let Some(n) = doc.len() {
        out.push("[]".to_string());
        elements(doc, n, "[].", &mut out);
    }
    out.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    out.dedup();
    out
}

/// 32-bit FNV-1a over the UTF-16 code units of each `shape` path, each followed by `\n`, as 8 hex
/// digits. Equal for responses with the same shape, and equal to drift.ts's for the same document.
pub fn fingerprint<D: Doc>(doc: &D) -> String {
    let mut h: u32 = 0x811c9dc5;
    for path in shape(doc) {
        for c in path.encode_utf16().chain([10]) {
            h = (h ^ u32::from(c)).wrapping_mul(0x01000193);
        }
    }
    format!("{h:08x}")
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FillMonitorOptions {
    /// Responses a window needs before it's judged; smaller windows are merged with the next.
    pub min_responses: u64,
    /// Judged windows the baseline averages over; 0 keeps them all (JavaScript's `slice(-0)`).
    pub history: usize,
    /// How far past the baseline a rate must move, in standard errors.
    pub sigmas: f64,
    /// Fill drops smaller than this fraction of the baseline are ignored.
    pub min_drop: f64,
    /// Error-rate rises smaller than this (absolute) are ignored.
    pub min_rise: f64,
}

impl Default for FillMonitorOptions {
    fn default() -> Self {
        FillMonitorOptions {
            min_responses: 200,
            history: 24,
            sigmas: 4.0,
            min_drop: 0.25,
            min_rise: 0.01,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Fill,
    Errors,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldAlert {
    pub field: String,
    pub kind: AlertKind,
    pub rate: f64,
    pub baseline: f64,
    /// What the rate is out of: responses, or elements for a field inside an item ("jobs[].title").
    pub responses: u64,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    responses: f64,
    fill: f64,
    errors: f64,
}

/// Compares each window of artifact stats (snapshot then reset once per period) with the trailing
/// windows before it. A field that alerts keeps its old baseline, so the alert repeats every window
/// until the spec is fixed instead of fading as the bad rate becomes the norm.
pub struct FillMonitor {
    opts: FillMonitorOptions,
    pending: Option<StatsSnapshot>,
    past: HashMap<String, Vec<Window>>,
}

impl FillMonitor {
    pub fn new(opts: FillMonitorOptions) -> Self {
        FillMonitor {
            opts,
            pending: None,
            past: HashMap::new(),
        }
    }

    /// Feeds one window. Returns the fields whose fill fell or whose errors rose, once the window is big enough.
    pub fn check(&mut self, window: &StatsSnapshot) -> Vec<FieldAlert> {
        let w = match self.pending.take() {
            Some(p) => merge(&p, window),
            None => window.clone(),
        };
        if w.responses < self.opts.min_responses {
            self.pending = Some(w);
            return Vec::new();
        }
        let mut alerts = Vec::new();
        for (field, s) in &w.fields {
            // A field inside an item is a rate over elements, and needs as many of those as a window needs responses.
            // a window short on elements is skipped, not carried over; merge per field if sparse arrays need judging
            if s.total.is_some_and(|t| t < self.opts.min_responses) {
                continue;
            }
            let now = Window {
                responses: s.total.unwrap_or(w.responses) as f64,
                fill: s.fill as f64,
                errors: s.errors as f64,
            };
            let past = self.past.entry(field.clone()).or_default();
            let found = if past.is_empty() {
                Vec::new()
            } else {
                judge(&self.opts, field, now, past)
            };
            if found.is_empty() {
                past.push(now);
                let h = self.opts.history;
                if h > 0 && past.len() > h {
                    past.drain(..past.len() - h);
                }
            }
            alerts.extend(found);
        }
        alerts
    }
}

fn judge(o: &FillMonitorOptions, field: &str, w: Window, past: &[Window]) -> Vec<FieldAlert> {
    let n = w.responses;
    let total: f64 = past.iter().map(|p| p.responses).sum();
    let mut out = Vec::new();
    let alert = |kind, rate, baseline| FieldAlert {
        field: field.to_string(),
        kind,
        rate,
        baseline,
        responses: n as u64,
    };
    let fill_base = past.iter().map(|p| p.fill).sum::<f64>() / total;
    let fill_rate = w.fill / n;
    if fill_base - fill_rate
        > f64::max(
            o.sigmas * std_err(past, |p| p.fill, n),
            o.min_drop * fill_base,
        )
    {
        out.push(alert(AlertKind::Fill, fill_rate, fill_base));
    }
    let err_base = past.iter().map(|p| p.errors).sum::<f64>() / total;
    let err_rate = w.errors / n;
    if err_rate - err_base > f64::max(o.sigmas * std_err(past, |p| p.errors, n), o.min_rise) {
        out.push(alert(AlertKind::Errors, err_rate, err_base));
    }
    out
}

/// Standard error of a rate over n responses, from the baseline with add-one smoothing so 0% and
/// 100% still vary. A plucked array counts an error per element, so its rate can pass 1; it is capped there.
fn std_err(past: &[Window], key: fn(&Window) -> f64, n: f64) -> f64 {
    let total: f64 = past.iter().map(|p| p.responses).sum();
    let p = f64::min(
        1.0,
        (past.iter().map(key).sum::<f64>() + 1.0) / (total + 2.0),
    );
    (p * (1.0 - p) / n).sqrt()
}

/// `b`'s fields with `a`'s counts added; a field only in `a` is dropped, as drift.ts merges.
fn merge(a: &StatsSnapshot, b: &StatsSnapshot) -> StatsSnapshot {
    let old: HashMap<&str, &FieldStats> = a.fields.iter().map(|(f, s)| (f.as_str(), s)).collect();
    let fields = b
        .fields
        .iter()
        .map(|(f, s)| {
            let o = old.get(f.as_str());
            let total = s.total.map(|t| o.and_then(|o| o.total).unwrap_or(0) + t);
            let stats = FieldStats {
                fill: o.map_or(0, |o| o.fill) + s.fill,
                errors: o.map_or(0, |o| o.errors) + s.errors,
                total,
            };
            (f.clone(), stats)
        })
        .collect();
    StatsSnapshot {
        responses: a.responses + b.responses,
        fields,
    }
}

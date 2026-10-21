// Runs every case of the vendored TypeScript corpus (conformance/cases, see conformance/README.md)
// through map_bytes + write_json, map_json, extract and the stats snapshot, and fails listing every
// case that diverges. The comparison rule lives in `compare`.

use canonic_core::{Artifact, RustSink, Sink, compile, write_json};
use serde_json::{Map, Value as J, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

mod compare {
    use serde_json::Value as J;

    /// The first difference between `got` and `want` as `path: got != want`, or `None` when equal.
    pub fn diff(path: &str, got: &J, want: &J) -> Option<String> {
        let differ = || Some(format!("{path}: got {got}, want {want}"));
        match (got, want) {
            (J::Number(a), J::Number(b)) => (!num_eq(a, b)).then(differ).flatten(),
            (J::Object(a), J::Object(b)) => {
                let (ka, kb): (Vec<_>, Vec<_>) = (a.keys().collect(), b.keys().collect());
                if ka != kb {
                    return Some(format!("{path}: keys {ka:?}, want {kb:?}"));
                }
                a.iter()
                    .zip(b.values())
                    .find_map(|((k, x), y)| diff(&format!("{path}.{k}"), x, y))
            }
            (J::Array(a), J::Array(b)) if a.len() == b.len() => a
                .iter()
                .zip(b)
                .enumerate()
                .find_map(|(i, (x, y))| diff(&format!("{path}[{i}]"), x, y)),
            _ => (got != want).then(differ).flatten(),
        }
    }

    /// By value; beyond 2^53 within one double step of `a`.
    fn num_eq(a: &serde_json::Number, b: &serde_json::Number) -> bool {
        if let (Some(x), Some(y)) = (a.as_i128(), b.as_i128()) {
            return x == y;
        }
        let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
        x == y || (x.abs() >= 9007199254740992.0 && (x - y).abs() <= x.abs().next_up() - x.abs())
    }

    /// Case text with every lone surrogate escape (`\ud800`) rewritten to the U+FFFD escape, which is what
    /// both runners compare against and what serde_json can hold. Surrogate pairs stay.
    pub fn fffd(text: &str) -> String {
        let b = text.as_bytes();
        let hex = |i: usize| {
            b.get(i..i + 6)
                .filter(|s| s[0] == b'\\' && s[1] == b'u')
                .and_then(|s| u16::from_str_radix(std::str::from_utf8(&s[2..]).ok()?, 16).ok())
        };
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'\\' {
                let n = text[i..].chars().next().unwrap().len_utf8();
                out.push_str(&text[i..i + n]);
                i += n;
                continue;
            }
            match hex(i) {
                Some(0xD800..=0xDBFF) if matches!(hex(i + 6), Some(0xDC00..=0xDFFF)) => {
                    out.push_str(&text[i..i + 12]);
                    i += 12;
                }
                Some(0xD800..=0xDFFF) => {
                    out.push_str("\\ufffd");
                    i += 6;
                }
                _ => {
                    out.push_str(&text[i..i + 2]);
                    i += 2;
                }
            }
        }
        out
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn cases(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            cases(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

/// The case names listed in conformance/EXCLUDED.md, one `- `name`: reason` line each.
fn excluded() -> Vec<String> {
    let text = fs::read_to_string(root().join("EXCLUDED.md")).unwrap();
    text.lines()
        .filter_map(|l| l.strip_prefix("- `")?.split('`').next().map(String::from))
        .collect()
}

/// What the TypeScript artifact maps a body it never parsed to: canonical objects of nulls.
fn blank(v: &J) -> J {
    match v {
        J::Object(m) => J::Object(m.iter().map(|(k, x)| (k.clone(), blank(x))).collect()),
        _ => J::Null,
    }
}

fn written(a: &Artifact, out: &canonic_core::Out) -> J {
    let mut buf = Vec::new();
    write_json(&a.plan, out, &mut buf);
    serde_json::from_slice(&buf).unwrap()
}

fn stats(a: &Artifact) -> J {
    let s = a.stats.snapshot();
    let fields: Map<String, J> = s
        .fields
        .into_iter()
        .map(|(name, f)| {
            let mut o = json!({ "fill": f.fill, "errors": f.errors });
            if let Some(t) = f.total {
                o["total"] = json!(t);
            }
            (name, o)
        })
        .collect();
    json!({ "responses": s.responses, "fields": fields })
}

/// Every divergence of one case, empty when it conforms.
fn run(case: &J) -> Vec<String> {
    let compiled = compile(&case["spec"], &case["canonical"], &[]);
    if let Some(want) = case["error"].as_str() {
        return match compiled {
            Err(e) if e.message == want => vec![],
            Err(e) => vec![format!("error {:?}, want {want:?}", e.message)],
            Ok(_) => vec![format!("compiled, want error {want:?}")],
        };
    }
    let a = match compiled.map(|p| Artifact::new(p, HashMap::new())) {
        Ok(Ok(a)) => a,
        Ok(Err(e)) | Err(e) => return vec![format!("compile: {}", e.message)],
    };
    let ex = &case["expected"];
    let maps = ex["map"].as_array().unwrap();
    let shape = blank(maps.iter().find(|m| !m.is_null()).unwrap_or(&J::Null));
    let mut fails = vec![];
    let mut unparsed = vec![];
    let mut texts = vec![];
    for (i, input) in case["inputs"].as_array().unwrap().iter().enumerate() {
        let body = input.as_str().unwrap().as_bytes();
        if maps[i].is_null() {
            unparsed.push((i, body));
        } else {
            let mut text = Vec::new();
            write_json(&a.plan, &a.map_bytes(body), &mut text);
            let got: J = serde_json::from_slice(&text).unwrap();
            fails.extend(compare::diff(&format!("map[{i}]"), &got, &maps[i]));
            texts.push((i, body, text));
        }
        if case["duplicate_keys"] != json!(true) {
            let got: Map<String, J> = a
                .plan
                .routing
                .iter()
                .zip(a.extract(body))
                .map(|(r, v)| {
                    (
                        r.name.clone(),
                        v.map_or(J::Null, |v| written(&a, &RustSink.scalar(v))),
                    )
                })
                .collect();
            fails.extend(compare::diff(
                &format!("extract[{i}]"),
                &J::Object(got),
                &ex["extract"][i],
            ));
        }
    }
    // TypeScript maps each body once and never maps one JSON.parse rejects, so the second pass
    // through map_json and the unparsed bodies go after the stats snapshot.
    fails.extend(compare::diff("stats", &stats(&a), &ex["stats"]));
    for (i, body, text) in texts {
        if a.map_json(body) != text {
            fails.push(format!("map_json[{i}]: differs from write_json(map_bytes)"));
        }
    }
    for (i, body) in unparsed {
        fails.extend(compare::diff(
            &format!("map[{i}] (unparsed)"),
            &written(&a, &a.map_bytes(body)),
            &shape,
        ));
    }
    fails
}

#[test]
fn every_corpus_case_conforms() {
    let mut paths = vec![];
    cases(&root().join("cases"), &mut paths);
    paths.sort();
    assert!(!paths.is_empty(), "no corpus: run conformance/sync.sh");
    let skip = excluded();
    let mut failed = vec![];
    for p in &paths {
        let text = compare::fffd(&fs::read_to_string(p).unwrap());
        let case: J = serde_json::from_str(&text).unwrap();
        let name = case["name"].as_str().unwrap();
        if skip.iter().any(|s| s == name) {
            continue;
        }
        let fails = run(&case);
        if !fails.is_empty() {
            failed.push(format!("{name}:\n  {}", fails.join("\n  ")));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {} cases diverge:\n{}",
        failed.len(),
        paths.len(),
        failed.join("\n")
    );
}

#[test]
fn lone_surrogate_escapes_become_fffd_and_pairs_stay() {
    let (lone, pair) = (
        ["\\", "ud800x\\", "udfff"].concat(),
        ["\\", "ud83d\\", "ude00"].concat(),
    );
    assert_eq!(compare::fffd(&lone), ["\\", "ufffdx\\", "ufffd"].concat());
    assert_eq!(compare::fffd(&pair), pair);
    let escaped_backslash = ["\\\\", "ud800"].concat();
    assert_eq!(compare::fffd(&escaped_backslash), escaped_backslash);
    assert!(compare::diff("", &json!(9007199254740993u64), &json!(9007199254740992.0)).is_none());
    assert!(compare::diff("", &json!(1), &json!(1.0)).is_none());
    assert!(compare::diff("", &json!({"a":1,"b":2}), &json!({"b":2,"a":1})).is_some());
}

// Runs every case of the vendored TypeScript corpus (conformance/cases, see conformance/README.md)
// through map_bytes + write_json, map_json, extract and the stats snapshot, and fails listing every
// case that diverges. The comparison rule lives in `tests/common/mod.rs`.

use canonic_core::{Artifact, RustSink, Sink, compile, write_json};
use serde_json::{Map, Value as J, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

mod common;
use common as compare;

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

/// What the TypeScript artifact maps a body it never parsed to: the canonical schema's objects,
/// every other field null.
fn blank(schema: &J) -> J {
    match schema["properties"].as_object() {
        Some(ps) if schema["type"] == "object" => {
            J::Object(ps.iter().map(|(k, x)| (k.clone(), blank(x))).collect())
        }
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

/// Every divergence of one case, empty when it conforms. `Err` for a case that is malformed.
fn run(case: &J) -> Result<Vec<String>, String> {
    let compiled = compile(&case["spec"], &case["canonical"], &[]);
    if let Some(want) = case["error"].as_str() {
        return Ok(match compiled {
            Err(e) if e.message == want => vec![],
            Err(e) => vec![format!("error {:?}, want {want:?}", e.message)],
            Ok(_) => vec![format!("compiled, want error {want:?}")],
        });
    }
    let a = match compiled.map(|p| Artifact::new(p, HashMap::new())) {
        Ok(Ok(a)) => a,
        Ok(Err(e)) | Err(e) => return Ok(vec![format!("compile: {}", e.message)]),
    };
    let ex = &case["expected"];
    let maps = ex["map"].as_array().ok_or("expected.map is not an array")?;
    let inputs = case["inputs"].as_array().ok_or("inputs is not an array")?;
    if inputs.len() != maps.len() {
        return Err(format!("{} inputs, {} maps", inputs.len(), maps.len()));
    }
    let shape = blank(&case["canonical"]);
    let mut fails = vec![];
    let mut unparsed = vec![];
    let mut texts = vec![];
    for (i, input) in inputs.iter().enumerate() {
        let body = input
            .as_str()
            .ok_or(format!("inputs[{i}] is not a string"))?
            .as_bytes();
        if maps[i].is_null() {
            unparsed.push((i, body));
        } else {
            let mut text = Vec::new();
            write_json(&a.plan, &a.map_bytes(body, None).unwrap(), &mut text);
            let got: J = serde_json::from_slice(&text)
                .map_err(|e| format!("map[{i}]: write_json wrote invalid JSON: {e}"))?;
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
        if a.map_json(body, None).unwrap() != text {
            fails.push(format!("map_json[{i}]: differs from write_json(map_bytes)"));
        }
    }
    for (i, body) in unparsed {
        fails.extend(compare::diff(
            &format!("map[{i}] (unparsed)"),
            &written(&a, &a.map_bytes(body, None).unwrap()),
            &shape,
        ));
    }
    Ok(fails)
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
        let case: Result<J, String> = fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&compare::fffd(&t)).map_err(|e| e.to_string()));
        let name = match &case {
            Ok(c) => c["name"].as_str().unwrap_or("(no name)").to_string(),
            Err(_) => p.display().to_string(),
        };
        if skip.contains(&name) {
            continue;
        }
        let fails = case
            .and_then(|c| run(&c))
            .unwrap_or_else(|e| vec![format!("malformed: {e}")]);
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
    let (exact, rounded) = (json!(9007199254740993u64), json!(9007199254740992u64));
    assert!(compare::diff("", &exact, &rounded).is_none());
    assert!(compare::diff("", &json!(9007199254740996u64), &rounded).is_some());
    assert!(compare::diff("", &json!(1), &json!(1.0)).is_none());
    assert!(compare::diff("", &json!({"a":1,"b":2}), &json!({"b":2,"a":1})).is_some());
}

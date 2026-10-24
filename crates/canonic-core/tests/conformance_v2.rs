// Runs every case in conformance/cases-v2 (format in conformance/cases-v2/README.md) through
// map_bytes + write_json and map_json, and fails listing every case that diverges. The comparison
// rule lives in `tests/common/mod.rs`.

use canonic_core::{Artifact, compile, write_json};
use serde_json::Value as J;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

mod common;

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

/// Every divergence of one case, empty when it conforms. `Err` for a case that is malformed.
fn run(case: &J) -> Result<Vec<String>, String> {
    let compiled = compile(&case["spec"], &case["schema"], &[]);
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
    let inputs = case["inputs"].as_array().ok_or("inputs is not an array")?;
    let mut fails = vec![];
    for (i, input) in inputs.iter().enumerate() {
        let body = match (input.get("body"), input.get("raw")) {
            (Some(b), None) => serde_json::to_vec(b).unwrap(),
            (None, Some(J::String(r))) => r.clone().into_bytes(),
            _ => {
                return Err(format!(
                    "inputs[{i}] needs exactly one of body, raw (a string)"
                ));
            }
        };
        let want = &input["expected"];
        let ctx = Some(&input["context"]).filter(|c| !c.is_null());
        if let Some(reason) = want.get("$rejected") {
            let got = [
                ("map_bytes", a.map_bytes(&body, ctx).err()),
                ("map_json", a.map_json(&body, ctx).err()),
            ];
            for (way, r) in got {
                if r.as_ref().map(|r| r.reason.as_str()) != reason.as_str() {
                    fails.push(format!("{way}[{i}]: got {r:?}, want Rejected({reason})"));
                }
            }
            continue;
        }
        let mut text = Vec::new();
        write_json(&a.plan, &a.map_bytes(&body, ctx).unwrap(), &mut text);
        for (way, out) in [
            ("map_bytes", text),
            ("map_json", a.map_json(&body, ctx).unwrap()),
        ] {
            match serde_json::from_slice::<J>(&out) {
                Ok(got) => fails.extend(common::diff(&format!("{way}[{i}]"), &got, want)),
                Err(e) => fails.push(format!("{way}[{i}]: invalid JSON: {e}")),
            }
        }
    }
    Ok(fails)
}

#[test]
fn every_v2_case_conforms() {
    let mut paths = vec![];
    cases(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases-v2"),
        &mut paths,
    );
    paths.sort();
    assert!(!paths.is_empty(), "no cases in conformance/cases-v2");
    let mut failed = vec![];
    for p in &paths {
        let fails = fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&common::fffd(&t)).map_err(|e| e.to_string()))
            .and_then(|c| run(&c))
            .unwrap_or_else(|e| vec![format!("malformed: {e}")]);
        if !fails.is_empty() {
            failed.push(format!("{}:\n  {}", p.display(), fails.join("\n  ")));
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

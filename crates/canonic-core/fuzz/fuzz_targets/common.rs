// The bench spec over its canonical schema, plain and anchored (canonic/bench/person.js), and a
// language 2 spec that uses one field per language 2 rule.

use canonic_core::{Artifact, compile};
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn artifacts() -> [Artifact; 2] {
    let spec: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/specs/bench.json")).unwrap();
    let schema: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/specs/canonical/bench.json"
    ))
    .unwrap();
    let mut anchored = spec.clone();
    anchored["anchors"] = json!({ "email": "\"workEmail\":", "phone": "\"mobile\":", "company.domain": "\"website\":" });
    anchored["fields"]["email"] =
        json!({ "from": "contact.workEmail", "transform": ["trim", "lower"] });
    [spec, anchored]
        .map(|s| Artifact::new(compile(&s, &schema, &[]).unwrap(), HashMap::new()).unwrap())
}

// Only the map_bytes target uses it.
#[allow(dead_code)]
pub fn lang2() -> Artifact {
    let f: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/specs/lang2-all.json")).unwrap();
    Artifact::new(
        compile(&f["spec"], &f["schema"], &[]).unwrap(),
        HashMap::new(),
    )
    .unwrap()
}

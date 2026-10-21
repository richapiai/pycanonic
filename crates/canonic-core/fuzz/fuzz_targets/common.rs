// The bench spec over its canonical schema, plain and anchored (canonic/bench/person.js).

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

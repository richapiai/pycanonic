#![no_main]

mod common;

use canonic_core::{Artifact, write_json};
use libfuzzer_sys::fuzz_target;
use serde_json::json;
use std::sync::LazyLock;

static ARTIFACTS: LazyLock<[Artifact; 2]> = LazyLock::new(common::artifacts);
static LANG2: LazyLock<Artifact> = LazyLock::new(common::lang2);

// The first byte picks the request context of the language 2 spec, the rest is the body.
fuzz_target!(|data: &[u8]| {
    let a = &ARTIFACTS[0];
    a.map_bytes(data, None).unwrap();
    a.map_json(data, None).unwrap();
    let Some((&seed, body)) = data.split_first() else {
        return;
    };
    let ctx = match seed % 4 {
        0 => None,
        1 => Some(json!({})),
        2 => Some(json!({ "page": seed / 4 })),
        _ => Some(json!({ "page": "x" })),
    };
    let a = &*LANG2;
    // A reject must be the same on both ways in, and the text must match.
    match (
        a.map_bytes(body, ctx.as_ref()),
        a.map_json(body, ctx.as_ref()),
    ) {
        (Ok(out), Ok(text)) => {
            let mut buf = Vec::new();
            write_json(&a.plan, &out, &mut buf);
            assert_eq!(buf, text);
        }
        (Err(x), Err(y)) => assert_eq!(x.reason, y.reason),
        (x, y) => panic!(
            "map_bytes and map_json disagree: {:?} {:?}",
            x.is_ok(),
            y.is_ok()
        ),
    }
});

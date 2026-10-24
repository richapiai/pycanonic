// The Rust half of scripts/bench_hot.py: each op cycles through all 64 bench bodies, as the Python
// table does, so the gap between the two is the binding. `cargo bench -p canonic-core --bench hot`.

use canonic_core::{Artifact, compile, filter_for, parse};
use divan::{Bencher, black_box};
use serde_json::Value as J;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

fn fixture(name: &str) -> J {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

static BODIES: LazyLock<Vec<Vec<u8>>> = LazyLock::new(|| {
    fixture("bench_bodies.json")["raws"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().as_bytes().to_vec())
        .collect()
});

fn artifact(spec: J) -> Artifact {
    let plan = compile(&spec, &fixture("specs/canonical/bench.json"), &[]).unwrap();
    Artifact::new(plan, HashMap::new()).unwrap()
}

static ARTIFACT: LazyLock<Artifact> = LazyLock::new(|| artifact(fixture("specs/bench.json")));

static ANCHORED: LazyLock<Artifact> = LazyLock::new(|| {
    let mut spec = fixture("specs/bench.json");
    spec["anchors"] = serde_json::json!({ "email": "\"workEmail\":", "phone": "\"mobile\":", "company.domain": "\"website\":" });
    spec["fields"]["email"] =
        serde_json::json!({ "from": "contact.workEmail", "transform": ["trim", "lower"] });
    artifact(spec)
});

fn main() {
    divan::main();
}

/// `f` over every body in turn. Divan reports the time per 64, read it as per body / 64.
fn cycle<T>(b: Bencher, f: impl Fn(&[u8]) -> T + Sync) {
    b.bench(|| {
        for body in BODIES.iter() {
            black_box(f(black_box(body)));
        }
    });
}

#[divan::bench]
fn structurals_x64(b: Bencher) {
    cycle(b, canonic_core::__bench::structurals);
}

#[divan::bench]
fn parse_x64(b: Bencher) {
    let filter = filter_for(&ARTIFACT.plan);
    cycle(b, |x| drop(black_box(parse(x, &filter).unwrap())));
}

#[divan::bench]
fn exec_x64(b: Bencher) {
    let filter = filter_for(&ARTIFACT.plan);
    let nodes: Vec<_> = BODIES.iter().map(|x| parse(x, &filter).unwrap()).collect();
    b.bench(|| {
        for n in &nodes {
            black_box(ARTIFACT.map_doc(&black_box(n)));
        }
    });
}

#[divan::bench]
fn map_bytes_x64(b: Bencher) {
    cycle(b, |x| ARTIFACT.map_bytes(x));
}

#[divan::bench]
fn map_json_x64(b: Bencher) {
    cycle(b, |x| ARTIFACT.map_json(x));
}

#[divan::bench]
fn extract_scanner_x64(b: Bencher) {
    cycle(b, |x| ARTIFACT.extract(x));
}

#[divan::bench]
fn extract_anchored_x64(b: Bencher) {
    cycle(b, |x| ANCHORED.extract(x));
}

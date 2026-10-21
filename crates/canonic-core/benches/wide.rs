// The 501-field payload of bench/interpreters.py (tests/fixtures/wide_bodies.json, written by
// scripts/export_wide.py), per stage. Each op cycles through all 64 bodies; read times per 64.
// `cargo bench -p canonic-core --bench wide`.

use canonic_core::{Artifact, compile, filter_for, parse};
use divan::{Bencher, black_box};
use serde_json::Value as J;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

static WIDE: LazyLock<(Artifact, Vec<Vec<u8>>)> = LazyLock::new(|| {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wide_bodies.json");
    let j: J = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let plan = compile(&j["spec"], &j["schema"], &[]).unwrap();
    let raws = (j["raws"].as_array().unwrap().iter())
        .map(|r| r.as_str().unwrap().as_bytes().to_vec())
        .collect();
    (Artifact::new(plan, HashMap::new()).unwrap(), raws)
});

fn main() {
    divan::main();
}

#[divan::bench]
fn parse_x64(b: Bencher) {
    let filter = filter_for(&WIDE.0.plan);
    b.bench(|| {
        for r in &WIDE.1 {
            drop(black_box(parse(black_box(r), &filter).unwrap()));
        }
    });
}

#[divan::bench]
fn exec_x64(b: Bencher) {
    let filter = filter_for(&WIDE.0.plan);
    let nodes: Vec<_> = WIDE.1.iter().map(|r| parse(r, &filter).unwrap()).collect();
    b.bench(|| {
        for n in &nodes {
            black_box(WIDE.0.map_doc(&black_box(n)));
        }
    });
}

#[divan::bench]
fn map_bytes_x64(b: Bencher) {
    b.bench(|| {
        for r in &WIDE.1 {
            black_box(WIDE.0.map_bytes(black_box(r)));
        }
    });
}

#[divan::bench]
fn map_json_x64(b: Bencher) {
    b.bench(|| {
        for r in &WIDE.1 {
            black_box(WIDE.0.map_json(black_box(r)));
        }
    });
}

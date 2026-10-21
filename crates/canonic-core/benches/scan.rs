// Stage 1 scan, pruned parse and map over bench body 0 (6,189 bytes). `cargo bench -p canonic-core`.

use canonic_core::{Artifact, Filter, compile, filter_for, parse};
use divan::{Bencher, black_box, counter::BytesCount};
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

static BODY: LazyLock<Vec<u8>> = LazyLock::new(|| {
    fixture("bench_bodies.json")["raws"][0]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec()
});

static ARTIFACT: LazyLock<Artifact> = LazyLock::new(|| {
    let plan = compile(
        &fixture("specs/bench.json"),
        &fixture("specs/canonical/bench.json"),
        &[],
    )
    .unwrap();
    Artifact::new(plan, HashMap::new()).unwrap()
});

static FILTER: LazyLock<Filter> = LazyLock::new(|| filter_for(&ARTIFACT.plan));

fn main() {
    divan::main();
}

#[divan::bench]
fn structurals(b: Bencher) {
    b.counter(BytesCount::of_slice(&*BODY))
        .bench(|| canonic_core::json::simd::structurals(black_box(&BODY)));
}

#[divan::bench]
fn parse_bench_filter(b: Bencher) {
    b.counter(BytesCount::of_slice(&*BODY))
        .bench(|| parse(black_box(&BODY), &FILTER).unwrap());
}

#[divan::bench]
fn map_bytes(b: Bencher) {
    b.counter(BytesCount::of_slice(&*BODY))
        .bench(|| ARTIFACT.map_bytes(black_box(&BODY)));
}

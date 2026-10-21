#![no_main]

mod common;

use canonic_core::Artifact;
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

static ARTIFACTS: LazyLock<[Artifact; 2]> = LazyLock::new(common::artifacts);

fuzz_target!(|data: &[u8]| {
    let a = &ARTIFACTS[0];
    a.map_bytes(data);
    a.map_json(data);
});

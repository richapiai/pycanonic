#![no_main]

mod common;

use canonic_core::Artifact;
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

static ARTIFACTS: LazyLock<[Artifact; 2]> = LazyLock::new(common::artifacts);

fuzz_target!(|data: &[u8]| {
    for a in &*ARTIFACTS {
        let r = a.extract(data);
        assert_eq!(r.len(), a.plan.routing.len());
    }
});

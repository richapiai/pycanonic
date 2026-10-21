#![no_main]

mod common;

use canonic_core::Artifact;
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

static ARTIFACTS: LazyLock<[Artifact; 2]> = LazyLock::new(common::artifacts);

// The first byte picks the chunk sizes, the rest is the body.
fuzz_target!(|data: &[u8]| {
    let Some((&seed, body)) = data.split_first() else {
        return;
    };
    for a in &*ARTIFACTS {
        let mut s = a.stream();
        let mut i = 0;
        let mut n = seed as usize;
        while i < body.len() {
            n = n.wrapping_mul(31).wrapping_add(7);
            let end = (i + 1 + n % 64).min(body.len());
            s.push(&body[i..end]);
            i = end;
        }
        assert_eq!(s.result().len(), a.plan.routing.len());
    }
});

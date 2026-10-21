#![deny(unconditional_recursion)]

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

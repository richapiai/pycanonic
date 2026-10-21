#![deny(unconditional_recursion)]

pub mod value;

pub use value::{Value, js_number_to_string};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

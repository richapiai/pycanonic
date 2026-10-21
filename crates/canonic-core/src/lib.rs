#![deny(unconditional_recursion)]

pub mod spec;
pub mod value;

pub use spec::{Issue, Spec, Status, parse_spec, validate_spec};
pub use value::{Value, js_number_to_string};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

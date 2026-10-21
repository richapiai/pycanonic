#![deny(unconditional_recursion)]

pub mod canonical;
pub mod spec;
pub mod transforms;
pub mod value;

pub use canonical::{Kind, Node, Schema, Semantic, load_schema, validate_canonical};
pub use spec::{Issue, Spec, Status, parse_spec, validate_spec};
pub use transforms::{
    ArgError, Builtin, CastTo, NamePart, Transform, builtin, default_transform_names, validate,
};
pub use value::{Value, js_number_to_string};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#![deny(unconditional_recursion)]

pub mod artifact;
pub mod canonical;
pub mod doc;
pub mod drift;
pub mod exec;
mod extract;
pub mod json;
pub mod plan;
pub mod resolve;
pub mod shadow;
pub mod sink;
pub mod spec;
pub mod stats;
pub mod transforms;
pub mod value;
pub mod write;

pub use artifact::{Artifact, filter_for};
pub use canonical::{Kind, Node, Schema, Semantic, load_schema, validate_canonical};
pub use doc::{Doc, Key};
pub use drift::{
    AlertKind, FieldAlert, FillMonitor, FillMonitorOptions, fingerprint, hash_shape, shape,
    shape_bounded,
};
pub use exec::{Program, execute, execute_field};
pub use extract::ExtractStream;
pub use json::{Filter, ParseError, parse, unescape};
pub use plan::{CompileError, Field, ListKind, Meta, Named, Plan, RoutingField, Scalar, TxRef};
pub use resolve::compile;
pub use shadow::{
    FieldDiff, GateOptions, GateResult, ShadowDiff, ShadowReport, merge_gate, to_out_bounded,
};
pub use sink::{Out, RustSink, Sink};
pub use spec::{Issue, Path, Source, Spec, Status, Step, parse_spec, validate_spec};
pub use stats::{FieldStats, Stats, StatsSnapshot};
pub use transforms::{
    ArgError, Builtin, CastTo, NamePart, Transform, builtin, default_transform_names, validate,
};
pub use value::{Value, js_number_to_string};
pub use write::write_json;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

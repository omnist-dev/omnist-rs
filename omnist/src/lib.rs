pub mod document;
pub mod error;
pub mod formats;
pub mod infer;
pub mod materialize;
pub mod oml;
pub mod ops;
pub mod osd;
pub mod report;
pub mod schema;

pub use error::{
    DocumentError, MaterializeError, OmnistError, ParseError, SchemaError, WriteError,
};
pub use infer::{AnyFallback, infer, infer_with_report};
pub use materialize::materialize;
pub use report::{Adjustment, Severity, WriteReport};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

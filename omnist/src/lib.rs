pub mod document;
pub mod error;
pub mod infer;
pub mod materialize;
pub mod oml;
pub mod ops;
pub mod osd;
pub mod schema;

pub use error::{
    DocumentError, MaterializeError, OmnistError, ParseError, SchemaError, WriteError,
};
pub use infer::infer;
pub use materialize::materialize;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

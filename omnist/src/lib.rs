//! # omnist
//!
//! Rust implementation of the Omnist data-interchange specification.
//!
//! Omnist provides a single canonical, lossless data model ([`document::Doc`])
//! across five supported formats (JSON, YAML, TOML, XML, and OML), paired with an
//! algebraic schema language (OSD, [`schema::Schema`]) for schema validation,
//! type inference, normalization, subschema extraction, and compatibility checking.
//!
//! Treat the vendored `omnist-spec` specification as the normative source of
//! truth for all data model rules, format mappings, and schema algebra.

#![deny(missing_docs)]
#![warn(rustdoc::all)]

pub mod document;
pub mod error;
pub mod formats;
pub mod infer;
pub mod materialize;
pub mod oml;
pub mod ops;
pub mod osd;
pub mod registry;
pub mod report;
pub mod schema;

pub use error::{
    DocumentError, FormatError, MaterializeError, OmnistError, ParseError, SchemaError, WriteError,
};
pub use infer::{AnyFallback, infer, infer_with_report};
pub use materialize::materialize;
pub use registry::{Format, formats, get_format, register_format};
pub use report::{Adjustment, Severity, WriteReport};

/// The version of the `omnist` crate, matching Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

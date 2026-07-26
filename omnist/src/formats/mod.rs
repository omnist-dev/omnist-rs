//! Codecs over the canonical Document model. Ported from
//! `~/dev/omnist/omnist/formats.py`; each format's reader parses text into
//! [`crate::document::Doc`] and its writer projects a `Doc` back to text.
//!
//! Unlike [`crate::oml`] (omnist's own format, always lossless), JSON/YAML/
//! TOML/XML can each fail to hold some value losslessly. Writing is
//! **lenient by default**: the writer adjusts the value and records the
//! change in a [`crate::report::WriteReport`]; `strict = true` raises
//! [`crate::error::WriteError`] (carrying the report) instead. See
//! [`crate::report`].
//!
//! This issue (#16) added the first of the four: [`json`]. Issue #18 added
//! [`yaml`]; issue #20 added [`toml`]; issue #22 adds [`xml`], the last and
//! structurally different one -- see `xml.rs`'s own doc comment.

pub mod json;
pub mod toml;
pub mod xml;
pub mod yaml;

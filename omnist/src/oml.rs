//! OML (Omnist Markup Language) -- the native codec for the Document model.
//!
//! Ported from `~/dev/omnist/omnist/oml.py` (issue #10). OML is omnist's own
//! serialization format: every Document -- every ordered, possibly-repeated,
//! possibly-interleaved edge list, and all seven scalar kinds (`string`,
//! `integer`, `number`, `boolean`, `date`, `time`, `datetime`) plus `null` --
//! round-trips through OML exactly, with no adjustment ever needed (unlike
//! JSON/YAML/TOML/XML).
//!
//! This module implements the **OML-Core** grammar in full for both
//! [`read_oml`] and [`write_oml`], plus the **OML-Extended** raw-string
//! (`'...'`, E2) and triple-quoted multiline-string (`"""..."""`, E3)
//! spellings on read only -- [`write_oml`] only ever emits OML-Core
//! double-quoted strings, matching the Python reference.
//!
//! ## Layout (issue #53)
//!
//! [`scanner`] tokenizes source text, [`parser`] consumes those tokens into
//! a [`RawNode`], and [`writer`] renders a `RawNode` back to OML-Core
//! source. This top-level module keeps the module doc overview, the four
//! `pub fn`s ([`read_oml`], [`write_oml`], [`write_oml_compact`],
//! [`check_oml`]), and the [`Codec`](crate::formats::Codec) adapter --
//! nothing about `crate::oml::*` paths changed by the split.
//!
//! ## Architecture (per issue #1/#10, "architecture freedom")
//!
//! Python's reader is a single-pass scanner built around one compiled
//! "master" regex, deferring line/col computation and scalar-value
//! construction until actually needed -- a Python-performance-specific
//! design (see the module's PR #168 for the profile that motivated it), not
//! a behavioral requirement. This port uses a straightforward hand-written
//! recursive-descent scanner/parser over `Vec<char>` instead: idiomatic
//! Rust, and there's no equivalent hot-path reason to defer decoding here.
//! Observable behavior (parse results, round-trips, error content) matches
//! the Python reference; exact error wording does not need to.
//!
//! ## Node representation
//!
//! [`crate::document::RawNode`] -- not [`crate::document::Value`] -- is the
//! type this codec reads into and writes from. `Value::Object`'s `IndexMap`
//! can't hold a repeated key, so it only represents "repeated label" as a
//! *contiguous* run (an array value under one key); OML must round-trip
//! arbitrary **interleaving** of repeated labels losslessly (its whole
//! reason for existing -- "no adjustment ever needed"), which only
//! `RawNode`'s literal edge list can hold exactly.
//!
//! ## Depth guard (omnist-ts#37 / omnist-ts#70)
//!
//! [`write_oml`] takes a plain, unchecked [`crate::document::RawNode`] --
//! exactly like Python's `write_oml(node)`, which accepts any hand-built
//! canonical node, not necessarily one that passed through a depth-checked
//! builder. So the writer calls the shared
//! [`crate::document::check_write_depth`] guard itself, at every nesting
//! level, rather than assuming its input already got checked somewhere
//! upstream -- the exact bug class omnist-ts#37/#70 were: a writer (or a
//! second writer) that skipped this because *some* builder happened to
//! guard depth already.

#[cfg(test)]
use crate::document::Scalar;
use crate::document::{self, RawNode};
use crate::error::{ParseError, WriteError};
#[cfg(test)]
use crate::formats::int_cap::MAX_INT_DIGITS;

mod parser;
mod scanner;
mod writer;

use parser::Parser;
use scanner::Scanner;
use writer::{write_edges, write_edges_compact, write_scalar, write_temporal_leaf};

/// Parse OML source into a canonical [`RawNode`] (edge-list or leaf).
///
/// Supports the full OML-Core grammar, plus OML-Extended raw-string (`'...'`)
/// and triple-quoted multiline-string (`"""..."""`) spellings -- see the
/// module doc comment.
pub fn read_oml(text: &str) -> Result<RawNode, ParseError> {
    let sc = Scanner::new(text);
    let mut parser = Parser::new(sc)?;
    parser.parse_document()
}

/// Render a canonical [`RawNode`] as OML-Core source, pretty-printed with
/// `indent` spaces per nesting level.
///
/// OML is lossless for every Document: there's never an adjustment to
/// report, so there's no `strict=`/report machinery -- writing always
/// succeeds, unless the input itself nests deeper than
/// [`crate::document::MAX_DEPTH`] (see the module doc comment on the depth
/// guard).
pub fn write_oml(node: &RawNode, indent: usize) -> Result<String, WriteError> {
    match node {
        RawNode::Leaf(s) => Ok(write_scalar(s)),
        RawNode::TemporalLeaf(s) => Ok(write_temporal_leaf(s)),
        RawNode::Edges(edges) => write_edges(edges, 0, indent, 0),
    }
}

/// Single-line ("compact") rendering: edges joined by `"; "`, no
/// newlines/padding. Mirrors Python's `write_oml(..., indent=None)`. Both
/// forms round-trip through [`read_oml`].
pub fn write_oml_compact(node: &RawNode) -> Result<String, WriteError> {
    match node {
        RawNode::Leaf(s) => Ok(write_scalar(s)),
        RawNode::TemporalLeaf(s) => Ok(write_temporal_leaf(s)),
        RawNode::Edges(edges) => write_edges_compact(edges, 0),
    }
}

/// Report what writing OML would adjust, without producing output. Added
/// for issue #31 (the format registry): OML is lossless for every
/// `Document` (see this module's doc comment), so there is never anything
/// to report -- mirrors Python's `check_oml`, which is exactly `return
/// WriteReport()`. Every other builtin format has a `check_*` function
/// already; this is the OML counterpart, needed so the `"oml"` registry
/// entry has a `check` callable like the other four.
pub fn check_oml(_doc: &crate::document::Doc) -> crate::report::WriteReport {
    crate::report::WriteReport::new()
}

/// Marker type implementing [`crate::formats::Codec`] for OML -- adapts
/// [`read_oml`]/[`write_oml`]/[`check_oml`] to the registry's uniform
/// `Doc`-in/`Doc`-out shape, exactly as `registry::builtins` did by hand
/// before this issue: `read` bridges `read_oml`'s [`RawNode`] result
/// through [`crate::document::Doc::from_raw`], and `write` bridges the
/// other way through `Doc::to_raw` before calling `write_oml` with its
/// documented default indent (2).
pub(crate) struct Oml;

impl crate::formats::Codec for Oml {
    const NAME: &'static str = "oml";

    fn read(text: &str) -> Result<document::Doc, crate::error::OmnistError> {
        let raw: RawNode = read_oml(text)?;
        document::Doc::from_raw(raw).map_err(Into::into)
    }

    fn write(doc: &document::Doc) -> Result<String, crate::error::OmnistError> {
        write_oml(&doc.to_raw(), 2).map_err(Into::into)
    }

    fn check(doc: &document::Doc) -> crate::report::WriteReport {
        check_oml(doc)
    }
}

#[cfg(test)]
mod tests;

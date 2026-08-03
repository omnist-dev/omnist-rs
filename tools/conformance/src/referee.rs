//! The comparison referee -- omnist-spec's `docs/conformance-harness.md`
//! §4/§6.2.
//!
//! Uses `omnist`'s own library (direct calls, never a CLI subprocess --
//! `omnist` is a real library crate first, per issue #82's locked-in
//! calling-convention decision) to parse OML/OSD text and judge structural
//! equality. Deliberately small: no fixture-format parsing, no
//! per-operation dispatch -- that's the (later-step) fixture and vector
//! runners' job. Ported in spirit from Python's `omnist`'s
//! `tools/conformance/referee.py` and omnist-ts's
//! `tools/conformance/referee.ts` (same architecture); this file follows
//! Rust idiom, not either one's syntax (workflow-playbook.md's
//! "architecture freedom").
//!
//! ## Comparison strategy (locked in by issue #82, documented here)
//!
//! Document comparison goes through [`omnist::document::Doc::to_raw`] to a
//! [`omnist::document::RawNode`], which already derives `PartialEq`, rather
//! than adding `PartialEq` to `Doc` itself: `Doc`'s internal arena +
//! `NodeId` representation is not guaranteed canonical across
//! structurally-equal documents (e.g. two `Doc`s built by different code
//! paths could use different arena layouts for the same tree), so deriving
//! equality directly on `Doc` would be unsound. `RawNode`'s
//! edges-as-`Vec<(String, RawNode)>` representation *is* the canonical,
//! order-preserving form, so its derived `PartialEq` is the right one to
//! use.
//!
//! Schema comparison has two legitimate modes, chosen per operation, never
//! guessed:
//! - `"exact"`: via `Schema`'s own derived `PartialEq` (every record name
//!   and every field's label/type/cardinality must match -- used for
//!   `normalize`/`prune`/`extract`, whose output naming is spec-determined).
//! - `"isomorphic"`: via `omnist::ops::is_isomorphic`, which is public but
//!   deliberately outside the crate's committed public-API surface per its
//!   own doc comment -- fine to use from a conformance harness, which is
//!   exactly the kind of internal-but-trusted caller that carve-out is for
//!   (used for `infer`, whose generated record names are
//!   implementation-derived, never canonical).

use omnist::document::{Doc, RawNode};
use omnist::oml::read_oml;
use omnist::ops::is_isomorphic;
use omnist::osd::parse_schema;

/// Structural, order-sensitive equality of two OML texts, via
/// `Doc::to_raw()` -> `RawNode` (see module docs for why not `Doc` itself
/// directly). `read_oml` already returns a `RawNode`, so the round trip
/// through `Doc::from_raw`/`Doc::to_raw` here is deliberate, not
/// incidental: it proves the comparison strategy holds even when an actual
/// value under test arrives as a `Doc` (e.g. a future op wired in Track 1
/// that hands back a `Doc` rather than a `RawNode` directly), not only in
/// the specific case where `read_oml`'s output already happens to be
/// raw-comparable.
///
/// Returns an error string (rather than `omnist`'s own error types) if
/// either side fails to parse or construct -- a harness-level concern
/// distinct from the errors under test.
pub fn compare_document(actual_oml_text: &str, expected_oml_text: &str) -> Result<bool, String> {
    let actual_raw = read_oml(actual_oml_text).map_err(|e| format!("actual: {e}"))?;
    let expected_raw = read_oml(expected_oml_text).map_err(|e| format!("expected: {e}"))?;
    let actual = doc_to_raw_roundtrip(actual_raw)?;
    let expected = doc_to_raw_roundtrip(expected_raw)?;
    Ok(actual == expected)
}

/// Round-trips a `RawNode` through `Doc::from_raw`/`Doc::to_raw` -- the
/// exact path a `Doc`-typed value under test would take before comparison.
fn doc_to_raw_roundtrip(raw: RawNode) -> Result<RawNode, String> {
    let doc: Doc = Doc::from_raw(raw).map_err(|e| format!("{e}"))?;
    Ok(doc.to_raw())
}

/// The two legitimate schema-comparison modes (§4/§6.2) -- chosen per
/// operation, never guessed.
pub fn compare_schema(
    actual_osd_text: &str,
    expected_osd_text: &str,
    mode: &str,
) -> Result<bool, String> {
    let actual = parse_schema(actual_osd_text).map_err(|e| format!("actual: {e}"))?;
    let expected = parse_schema(expected_osd_text).map_err(|e| format!("expected: {e}"))?;
    match mode {
        "exact" => Ok(actual == expected),
        "isomorphic" => Ok(is_isomorphic(&actual, &expected)),
        other => panic!("unknown comparison mode {other:?}; expected 'exact' or 'isomorphic'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_document_equal_texts_are_equal() {
        assert_eq!(compare_document("a: 1\n", "a: 1\n"), Ok(true));
    }

    #[test]
    fn compare_document_reordered_edges_are_not_equal() {
        assert_eq!(compare_document("a: 1\nb: 2\n", "b: 2\na: 1\n"), Ok(false));
    }

    #[test]
    fn compare_document_bad_actual_oml_is_an_error() {
        let result = compare_document("[[[", "a: 1\n");
        assert!(result.is_err());
    }

    #[test]
    fn compare_document_bad_expected_oml_is_an_error() {
        let result = compare_document("a: 1\n", "[[[");
        assert!(result.is_err());
    }

    #[test]
    fn compare_schema_exact_mode_matches_schema_partial_eq() {
        let a = "record R {\n    \"x\": string,\n}\nroot R\n";
        let b = "record R {\n    \"x\": string,\n}\nroot R\n";
        assert_eq!(compare_schema(a, b, "exact"), Ok(true));
    }

    #[test]
    fn compare_schema_bad_expected_osd_is_an_error() {
        let a = "record R {\n    \"x\": string,\n}\nroot R\n";
        let result = compare_schema(a, "not valid osd", "exact");
        assert!(result.is_err());
    }

    #[test]
    #[should_panic(expected = "unknown comparison mode")]
    fn compare_schema_unknown_mode_panics() {
        let a = "record R {\n    \"x\": string,\n}\nroot R\n";
        let _ = compare_schema(a, a, "bogus");
    }

    /// `doc_to_raw_roundtrip`'s `Doc::from_raw` error path is unreachable
    /// via `read_oml` (the OML parser enforces the same `MAX_DEPTH` guard
    /// while scanning, per `oml.rs`'s own "Depth guard" module docs, so a
    /// `RawNode` it hands back can never be too deep for `Doc::from_raw`
    /// to reject). Exercise it directly with a hand-built, over-deep
    /// `RawNode` that bypasses the parser entirely -- the one way a real
    /// (if unlikely) caller could still hit this path, e.g. a future
    /// operation that hands the referee a `RawNode` assembled some other
    /// way. Proves the branch is real defensive code, not dead code.
    #[test]
    fn doc_to_raw_roundtrip_rejects_a_hand_built_over_deep_raw_node() {
        let mut node = RawNode::Leaf(omnist::document::Scalar::Null);
        for _ in 0..(omnist::document::MAX_DEPTH + 10) {
            node = RawNode::Edges(vec![("x".to_string(), node)]);
        }
        assert!(doc_to_raw_roundtrip(node).is_err());
    }
}

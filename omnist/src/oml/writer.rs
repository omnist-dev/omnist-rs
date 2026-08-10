//! OML writer (OML-Core only): renders a [`RawNode`] as OML source text.
//!
//! See the parent module's doc comment for the overall architecture
//! rationale (issue #10).
//!
//! ## Temporal literals
//!
//! A leaf writes bare (no quotes) only when it's a
//! [`RawNode::TemporalLeaf`], never by guessing from a `Scalar::Str`'s
//! shape -- issue #99. Shape-guessing (any date/time/datetime-*shaped*
//! string writes bare, regardless of provenance) was the pre-#99
//! behavior, matching the omnist-ts#52 fix's original intent
//! (`read_oml("a: 12:00")` producing `Scalar::Str("12:00")` must write
//! back bare, not `"12:00"`) -- but it silently over-applied: an ordinary
//! JSON/YAML string that merely *looks* like a date got promoted to a
//! genuine temporal literal on write, corrupting it on the next read
//! (confirmed live, filed as #99). `RawNode::TemporalLeaf` (`document.rs`)
//! now carries that provenance explicitly instead: `crate::oml`'s own
//! parser tags a genuinely-read bare literal, and
//! `crate::materialize`'s Date/Time/Datetime schema upgrades tag their
//! result the same way. Every other `Scalar::Str`, however shaped,
//! always quotes.
//!
//! `.0` on integer-valued floats: [`write_float`] renders so the value
//! always re-tokenizes as `NUMDEC`/`NEGINF`/`NANLIT`/`POSINF` on read,
//! never `INTEGER` -- an integer-valued float (`1.0`) must keep a decimal
//! point on write (Rust's `Display` for `f64` omits the trailing `.0`,
//! unlike Python's `repr()`), or the OML round-trip would silently
//! reclassify it as `Scalar::Int` on read-back.

use crate::document::{RawNode, Scalar, check_write_depth};
use crate::error::WriteError;
use crate::formats::string_escape::{OML_ESCAPES, write_quoted};

use super::parser::RESERVED;

/// `node_depth` is *this edges list's own* depth, matching
/// `document.rs`'s `push_raw`/`build_node` convention exactly: the guard is
/// checked for every node (container *and* leaf) at its own depth, with a
/// child one level deeper than its parent container -- not just at
/// container boundaries. This is what makes the boundary case line up with
/// `document.rs`'s own tests (a leaf at exactly `MAX_DEPTH` is accepted;
/// one past it is rejected), and is the literal fix for the omnist-ts#37/
/// #70 bug class: every depth-costing step is guarded, not only "one
/// nesting level" as a proxy for it.
pub(super) fn write_edges(
    edges: &[(String, RawNode)],
    depth: usize,
    indent: usize,
    node_depth: usize,
) -> Result<String, WriteError> {
    check_write_depth(node_depth, "$")?;
    let pad = " ".repeat(indent * depth);
    let mut lines = Vec::with_capacity(edges.len());
    for (label, child) in edges {
        let lab = write_label(label);
        match child {
            RawNode::Edges(inner) if inner.is_empty() => {
                check_write_depth(node_depth + 1, "$")?;
                lines.push(format!("{pad}{lab}: {{}}"));
            }
            RawNode::Edges(inner) => {
                let body = write_edges(inner, depth + 1, indent, node_depth + 1)?;
                lines.push(format!("{pad}{lab}: {{\n{body}\n{pad}}}"));
            }
            RawNode::Leaf(s) => {
                check_write_depth(node_depth + 1, "$")?;
                lines.push(format!("{pad}{lab}: {}", write_scalar(s)));
            }
        }
    }
    Ok(lines.join("\n"))
}

pub(super) fn write_edges_compact(
    edges: &[(String, RawNode)],
    node_depth: usize,
) -> Result<String, WriteError> {
    check_write_depth(node_depth, "$")?;
    let mut parts = Vec::with_capacity(edges.len());
    for (label, child) in edges {
        let lab = write_label(label);
        match child {
            RawNode::Edges(inner) if inner.is_empty() => {
                check_write_depth(node_depth + 1, "$")?;
                parts.push(format!("{lab}: {{}}"));
            }
            RawNode::Edges(inner) => {
                let body = write_edges_compact(inner, node_depth + 1)?;
                parts.push(format!("{lab}: {{ {body} }}"));
            }
            RawNode::Leaf(s) => {
                check_write_depth(node_depth + 1, "$")?;
                parts.push(format!("{lab}: {}", write_scalar(s)));
            }
        }
    }
    Ok(parts.join("; "))
}

fn is_bare_label(label: &str) -> bool {
    let mut chars = label.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    // \Z (Rust: full match, no trailing-newline-before-end laxity) --
    // matches every remaining char strictly, unlike a `$`-style anchor that
    // would also accept a trailing "\n" (that subtlety is why the Python
    // reference uses `\Z`, not `$`, for this pattern -- see oml.py).
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && !RESERVED.contains(&label)
        && label != "nan"
        && label != "inf"
}

fn write_label(label: &str) -> String {
    if is_bare_label(label) {
        label.to_string()
    } else {
        write_string(label)
    }
}

pub(super) fn write_scalar(v: &Scalar) -> String {
    match v {
        Scalar::Null => "null".to_string(),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Int(i) => i.to_string(),
        Scalar::Float(f) => write_float(*f),
        // Always quoted -- no shape-guessing (issue #99). A genuinely
        // temporal-kinded value is a `Date`/`Time`/`Datetime` variant
        // (issue #105), not a shape-matched `Str` -- see the arm below.
        Scalar::Str(s) => write_string(s),
        // Bare, no quotes -- by construction (see `Scalar::Date`'s own
        // doc comment) always already a validated, canonical spelling.
        Scalar::Date(s) | Scalar::Time(s) | Scalar::Datetime(s) => s.clone(),
    }
}

fn write_float(v: f64) -> String {
    crate::formats::float_fmt::float_to_string(v, "nan", "inf", "-inf")
}

/// Escapes every occurrence of a special character -- a per-char loop, not
/// a find/replace pass, so this can never under-sanitize by only touching
/// the *first* match (the general "regex-in-a-writer" risk flagged by
/// issue #10's omnist-ts#36-equivalent note).
fn write_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    write_quoted(s, &OML_ESCAPES, &mut out);
    out
}

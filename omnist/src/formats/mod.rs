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

pub(crate) mod float_fmt;
pub(crate) mod int_cap;
pub mod json;
pub(crate) mod textpos;
pub mod toml;
pub mod xml;
pub mod yaml;

use crate::document::Value;

/// One position `visit_grouped` reaches, passed to its callback alongside
/// the current path. Kept as a single enum (rather than two separate
/// closures) so callers only need one `&mut` capture of their accumulator
/// (e.g. a `WriteReport`) -- two closures both borrowing the same
/// `&mut WriteReport` for the whole walk don't borrow-check, since both
/// would be alive simultaneously across the recursion.
pub(crate) enum Visited<'a> {
    /// A `(label, child)` map entry, fired once per label regardless of
    /// whether `label`'s value is a same-label array -- e.g. for
    /// `yaml.rs`'s NEL-in-label scan, which must not fire once per array
    /// entry.
    Edge { label: &'a str },
    /// A value actually reached by the traversal (a leaf, or a
    /// non-`Object` node on the way down) -- e.g. `json.rs`'s
    /// NaN/Infinity check or `yaml.rs`'s NEL-in-value check, both of
    /// which only care about a subset of node kinds and filter for it
    /// themselves.
    Node { value: &'a Value },
}

/// Shared traversal for the two codec scanners built directly over a grouped
/// `Value` tree (`json::collect_leaves`/`check_json`, `yaml::scan_nel`) --
/// see issue #51. Both re-implemented the same recursion and the same
/// same-label-array path-numbering rule; this walker does it once.
///
/// `path` is a single reused buffer: every recursive step pushes its segment
/// (via [`crate::report::push_child_path`]), recurses, then truncates back --
/// so a full walk of an all-valid document allocates a path `String` only
/// when `f` itself decides to keep one (e.g. to store it in a
/// `WriteReport`), never once per edge just to *have* a path available
/// (issue #44).
///
/// `toml.rs::strip_nulls` (which transforms the tree, not merely visits it)
/// and `xml.rs::scan_xml_into` (a different tree type, `RawNode`, not
/// `Value`) don't fit this shape and keep their own recursion -- they still
/// use [`crate::report::child_path`] for the path-numbering rule itself.
pub(crate) fn visit_grouped(
    node: &Value,
    path: &mut String,
    f: &mut impl FnMut(Visited<'_>, &str),
) {
    match node {
        Value::Object(map) => {
            for (label, child) in map {
                let base = path.len();
                path.push('.');
                path.push_str(label);
                f(Visited::Edge { label }, path.as_str());

                match child {
                    Value::Array(items) => {
                        path.truncate(base);
                        for (i, item) in items.iter().enumerate() {
                            let ibase = path.len();
                            crate::report::push_child_path(path, label, i);
                            visit_grouped(item, path, f);
                            path.truncate(ibase);
                        }
                    }
                    other => {
                        visit_grouped(other, path, f);
                        path.truncate(base);
                    }
                }
            }
        }
        other => f(Visited::Node { value: other }, path.as_str()),
    }
}

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
pub(crate) mod string_escape;
pub(crate) mod textpos;
pub mod toml;
pub mod xml;
pub mod yaml;

use std::borrow::Cow;

use crate::document::{Doc, Value};
use crate::error::OmnistError;
use crate::report::WriteReport;

/// A temporal leaf's text as every writer spells it (spec E-37): the seconds
/// field is always written (`12:00` -> `12:00:00`; an omitted field is `00`)
/// and a datetime's date/time separator is `T`. Nothing else changes: the
/// offset, every fractional digit and the clock reading are kept. `datetime`
/// selects the date-time shape; a `date` is passed through [`Cow::Borrowed`]
/// by the callers and never reaches this function.
pub(crate) fn temporal_text(s: &str, datetime: bool) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    let start = if datetime { 11 } else { 0 };
    let clock_ok = bytes.len() >= start + 5
        && bytes[start + 2] == b':'
        && [0, 1, 3, 4]
            .iter()
            .all(|&i| bytes[start + i].is_ascii_digit());
    let missing_seconds = clock_ok && bytes.get(start + 5) != Some(&b':');
    let spaced = datetime && bytes.len() > 10 && bytes[10] == b' ';
    if !missing_seconds && !spaced {
        return Cow::Borrowed(s);
    }
    let mut out = s.to_string();
    if missing_seconds {
        out.insert_str(start + 5, ":00");
    }
    if spaced {
        out.replace_range(10..11, "T");
    }
    Cow::Owned(out)
}

/// The (read, write, check) contract every builtin format codec already
/// implements as a naming convention -- expressed once (issue #52).
///
/// `read`/`write`/`check` intentionally match the registry's
/// [`crate::registry::ReadFn`]/[`crate::registry::WriteFn`]/
/// [`crate::registry::CheckFn`] signatures exactly: no `strict`, no
/// `report`, no format-specific options (`write_json`'s `indent`,
/// `write_oml`'s `RawNode`/`indent` shape). Each format's richer public
/// `read_X`/`write_X(doc, ..., strict, report)`/`check_X` functions are
/// unchanged and remain the actual public API -- a `Codec` impl is thin
/// `pub(crate)` plumbing that calls them with the registry's documented
/// defaults (`indent: None`, `strict: false`, no report requested; OML's
/// `Doc::from_raw`/`to_raw` bridging), exactly what the hand-written
/// wrapper closures in `registry::builtins` did before this issue -- see
/// `registry.rs`'s module doc for why those defaults are the right ones
/// and why the registry's signatures are simpler than the public
/// functions'.
///
/// A richer trait sketch (scan/emit split, `strict`/`report`-aware
/// `write`) was considered and rejected: `write_json` needs `strict` to
/// pick lenient vs. strict *content* (NaN/Infinity substitution), not just
/// whether `finish_write` raises, so a `strict`-unaware `emit` provided
/// method would be wrong for JSON specifically and each impl would have to
/// override `write` anyway -- collapsing the supposed savings. This
/// leaner shape captures the actual duplication (the registry's adapter
/// closures) without forcing an artificial decomposition that fights each
/// format's real differences.
pub(crate) trait Codec: 'static {
    /// The name this codec is registered under (`"json"`, `"yaml"`, ...).
    const NAME: &'static str;

    fn read(text: &str) -> Result<Doc, OmnistError>;
    fn write(doc: &Doc) -> Result<String, OmnistError>;
    fn check(doc: &Doc) -> WriteReport;

    /// Build this codec's [`crate::registry::Format`] entry -- the
    /// wrapper-closure boilerplate `registry::builtins` used to hand-write
    /// once per format, now written once here instead.
    fn format() -> crate::registry::Format {
        crate::registry::Format::new(Self::NAME, Self::read, Self::write).with_check(Self::check)
    }
}

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
                // E-36: the edge's own path quotes a label that needs it.
                crate::report::push_child_path(path, label, None);
                f(Visited::Edge { label }, path.as_str());

                match child {
                    Value::Array(items) => {
                        path.truncate(base);
                        for (i, item) in items.iter().enumerate() {
                            let ibase = path.len();
                            crate::report::push_child_path(
                                path,
                                label,
                                crate::report::occurrence_index(i, items.len()),
                            );
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

#[cfg(test)]
mod temporal_text_tests {
    use super::temporal_text;
    use std::borrow::Cow;

    #[test]
    fn an_omitted_seconds_field_is_written_as_00() {
        assert_eq!(temporal_text("12:00", false), "12:00:00");
        assert_eq!(temporal_text("12:00+01:00", false), "12:00:00+01:00");
        assert_eq!(temporal_text("12:00Z", false), "12:00:00Z");
        assert_eq!(
            temporal_text("2024-01-15T12:00", true),
            "2024-01-15T12:00:00"
        );
        assert_eq!(
            temporal_text("2024-01-15T12:00-05:00", true),
            "2024-01-15T12:00:00-05:00"
        );
    }

    #[test]
    fn a_datetime_separator_is_t() {
        assert_eq!(
            temporal_text("2024-01-15 12:00:00", true),
            "2024-01-15T12:00:00"
        );
        assert_eq!(
            temporal_text("2024-01-15 12:00", true),
            "2024-01-15T12:00:00"
        );
    }

    #[test]
    fn nothing_else_changes() {
        for (s, datetime) in [
            ("12:00:00", false),
            ("12:00:00.5", false),
            ("12:00:00.123456+01:00", false),
            ("2024-01-15T12:00:00.5Z", true),
            ("2024-01-15", true),
            ("garbage", false),
            ("12", false),
            ("ab:cd", false),
            ("", true),
        ] {
            assert!(
                matches!(temporal_text(s, datetime), Cow::Borrowed(t) if t == s),
                "{s:?}"
            );
        }
    }
}

#[cfg(test)]
mod path_quoting_tests {
    use crate::document::{Doc, Value};
    use crate::formats::{json, toml, yaml};
    use indexmap::IndexMap;

    fn one(label: &str, v: Value) -> Doc {
        let mut m = IndexMap::new();
        m.insert(label.to_string(), v);
        Doc::of(&Value::Object(m)).unwrap()
    }

    const CASES: [(&str, &str); 3] = [("a.b", r#"$["a.b"]"#), ("", r#"$[""]"#), ("a b", "$.a b")];

    #[test]
    fn json_check_and_write_adjustments_quote_the_label() {
        for (label, want) in CASES {
            let nan = one(label, Value::Float(f64::NAN));
            assert_eq!(json::check_json(&nan).adjustments()[0].path, want);
            let t = one(label, Value::Time("12:00:00".to_string()));
            assert_eq!(json::check_json(&t).adjustments()[0].path, want);
            let mut rep = crate::report::WriteReport::new();
            json::write_json(&t, None, false, Some(&mut rep)).unwrap();
            assert_eq!(rep.adjustments()[0].path, want);
        }
    }

    #[test]
    fn yaml_adjustments_quote_the_label() {
        for (label, want) in CASES {
            let t = one(label, Value::Time("12:00:00".to_string()));
            assert_eq!(yaml::check_yaml(&t).adjustments()[0].path, want);
            let nel = one(label, Value::Str("a\u{85}b".to_string()));
            assert_eq!(yaml::check_yaml(&nel).adjustments()[0].path, want);
        }
        let key = one("a.b\u{85}", Value::Int(1.into()));
        assert_eq!(
            yaml::check_yaml(&key).adjustments()[0].path,
            "$[\"a.b\\u0085\"]"
        );
    }

    #[test]
    fn toml_time_with_an_offset_or_z_is_quoted_and_reported_at_a_quoted_path() {
        for (label, want) in CASES {
            for t in ["12:00:00+01:00", "12:00Z", "12:00:00z", "12:00:00-05:00"] {
                let doc = one(label, Value::Time(t.to_string()));
                let mut rep = crate::report::WriteReport::new();
                let text = toml::write_toml(&doc, false, Some(&mut rep)).unwrap();
                assert!(text.contains('"'), "{t}: {text}");
                assert_eq!(rep.adjustments()[0].path, want);
                assert_eq!(rep.adjustments()[0].code, "format.temporal-stringified");
                assert!(toml::read_toml(&text).is_ok(), "{text}");
            }
        }
    }
}

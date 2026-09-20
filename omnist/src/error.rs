//! Error hierarchy, `thiserror`-based (per issue #1 §5).
//!
//! `OmnistError` is the crate-wide top-level error; each module contributes
//! its own leaf error type as a variant (mirroring Python's
//! `OmnistError`/`SchemaError`/`ParseError`/`WriteError`/`DocumentError`
//! hierarchy in `~/dev/omnist/omnist/errors.py`). This issue (#4) adds only
//! `DocumentError`; the other leaf types land with their own modules.

use thiserror::Error;

/// A Document operation is invalid, or a plain value is not a legal Document.
///
/// Raised by [`crate::document`] when a construction or mutation would
/// produce something outside the Document model (a bare top-level array, an
/// array of arrays, nesting past the max depth) or when an operation doesn't
/// fit the node it's called on (e.g. reading `.value()` on an internal
/// node). The message carries the offending path, matching the Python
/// reference's `DocumentError` convention of embedding `path` in the text.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{path}: {message}")]
pub struct DocumentError {
    /// The path inside the document where the error occurred.
    pub path: String,
    /// The spec's stable machine-readable code (omnist-spec §8.3.2 and
    /// §8.3.8, e.g. `document.unlabeled-element`,
    /// `format.dtd-forbidden`), when the failure is one the taxonomy names.
    /// `None` for API-misuse errors (reading `.value()` on an internal
    /// node, `get_one` on a repeated label, ...) that no conformance
    /// diagnostic describes.
    pub code: Option<String>,
    /// Human-readable error description.
    pub message: String,
}

impl DocumentError {
    /// Construct a new `DocumentError` at the given path, with no
    /// taxonomy code (an API-misuse error).
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            code: None,
            message: message.into(),
        }
    }

    /// Construct a `DocumentError` carrying a spec taxonomy `code`.
    pub fn with_code(
        path: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            code: Some(code.into()),
            message: message.into(),
        }
    }
}

/// A Schema definition is invalid (bad cardinality, duplicate field label,
/// unknown scalar/ref name) -- raised by [`crate::schema`], [`crate::osd`],
/// [`crate::infer`], and [`crate::ops::extract`].
///
/// Breaking change in issue #122: `SchemaError` now carries machine-readable
/// `path` and `code` fields alongside human-readable `message`, matching the
/// spec's schema well-formedness and algebra error taxonomy (spec §8.3.3 & §8.3.6).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct SchemaError {
    /// The path (record/field context or "$") where the schema error occurred.
    pub path: String,
    /// Stable machine-readable error code (e.g. "schema.unknown-type", spec §8.3.3).
    pub code: String,
    /// Human-readable error description.
    pub message: String,
}

impl SchemaError {
    /// Construct a structured `SchemaError` with path, code, and message.
    pub fn new(
        path: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            code: code.into(),
            message: message.into(),
        }
    }
}

/// An OML source string could not be parsed -- raised by
/// [`crate::oml::read_oml`], mirroring Python's `ParseError` in
/// `~/dev/omnist/omnist/errors.py`. Carries the same "line N, col N: msg"
/// convention the Python reference's scanner/parser produce.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("line {line}, col {col}: {message}")]
pub struct ParseError {
    /// Line number where parsing failed (1-indexed).
    pub line: usize,
    /// Column number where parsing failed (1-indexed).
    pub col: usize,
    /// The spec's stable machine-readable code (omnist-spec §8.3.1, e.g.
    /// `parse.unexpected-token`, `parse.codec-syntax`).
    pub code: String,
    /// Human-readable parse failure description.
    pub message: String,
}

impl ParseError {
    /// Construct a new `ParseError` with position coordinates and the
    /// spec's `parse.*` code.
    pub fn new(
        line: usize,
        col: usize,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            line,
            col,
            code: code.into(),
            message: message.into(),
        }
    }

    /// Construct a `parse.codec-syntax` error (omnist-spec §8.3.1): input a
    /// JSON/YAML/TOML/XML codec could not accept, whether malformed in its
    /// own format or refused by a byte-level precondition the spec imposes
    /// ahead of the codec (E-24).
    pub fn codec_syntax(line: usize, col: usize, message: impl Into<String>) -> Self {
        Self::new(line, col, "parse.codec-syntax", message)
    }

    /// The `text position` path (`line:col`, omnist-spec §8.4) of this
    /// error.
    pub fn position(&self) -> String {
        format!("{}:{}", self.line, self.col)
    }
}

/// An unknown format name was looked up in the format registry -- raised by
/// [`crate::registry::get_format`] (and therefore
/// [`crate::document::Doc::from_format`]/`to_format`/`check_format`),
/// mirroring Python's `OmnistError(f"unknown format {name!r}; registered:
/// ...")` raised directly (not as a distinct exception subclass) in
/// `~/dev/omnist/omnist/registry.py::get_format`. Given its own leaf type
/// here (rather than reusing `DocumentError`/`SchemaError`) because "no such
/// registered format" isn't a Document-shape or Schema-definition problem --
/// it's specifically a registry lookup miss, issue #31.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct FormatError(pub String);

impl FormatError {
    /// Construct a new `FormatError` for an unknown format name.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// An in-memory Document could not be written -- raised by
/// [`crate::oml::write_oml`] (depth guard only; OML is otherwise lossless
/// for every Document -- see that module's doc comment) and, from issue
/// #16 onward, by `strict=true` format writers via
/// [`crate::report::finish_write`], mirroring Python's
/// `WriteError(str(rep), report=rep)`. The optional [`crate::report::WriteReport`]
/// carries the adjustments that triggered a strict-mode raise; `None` for
/// every other `WriteError` site (e.g. the depth guard, which has no
/// report to attach).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct WriteError {
    /// Human-readable write failure description.
    pub message: String,
    /// Optional accumulated `WriteReport` when written in strict mode.
    pub report: Option<crate::report::WriteReport>,
    /// The Document path of the value that could not be written, when the
    /// failure is one the spec's taxonomy names (omnist-spec §8.3.8 and
    /// §8.3.9); `None` otherwise.
    pub path: Option<String>,
    /// The spec's stable code for the failure (`write.unsupported-value`,
    /// `format.multiple-roots`, ...); `None` when the taxonomy has no code
    /// for it.
    pub code: Option<String>,
}

impl WriteError {
    /// Construct a new `WriteError` with no report.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            report: None,
            path: None,
            code: None,
        }
    }

    /// Construct a `WriteError` that carries the structured `(path, code)`
    /// diagnostic the spec's taxonomy assigns to it.
    pub fn with_diagnostic(
        path: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            message: message.into(),
            report: None,
            path: Some(path.into()),
            code: Some(code.into()),
        }
    }

    /// Construct a `WriteError` carrying the [`crate::report::WriteReport`]
    /// that caused a strict-mode write to raise.
    pub fn with_report(message: impl Into<String>, report: crate::report::WriteReport) -> Self {
        Self {
            message: message.into(),
            report: Some(report),
            path: None,
            code: None,
        }
    }

    /// The report attached to this error, if any (only strict-mode format
    /// writers attach one).
    pub fn report(&self) -> Option<&crate::report::WriteReport> {
        self.report.as_ref()
    }
}

impl From<DocumentError> for WriteError {
    fn from(e: DocumentError) -> Self {
        WriteError::new(e.message)
    }
}

/// A freshly-read node could not be made to conform to a `Schema` --
/// raised by [`crate::materialize::materialize`] (issue #14), mirroring
/// Python's `ParseError(str(res), errors=res.errors)` raised by
/// `~/dev/omnist/omnist/deserialize.py`. Wraps a
/// [`crate::schema::ValidationResult`] directly rather than duplicating its
/// `(path, message, code)` collection machinery -- `materialize` already
/// walks the tree using the exact same shape-check rules `Schema::validate`
/// does, so its error report reuses the same collector type.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct MaterializeError(pub crate::schema::ValidationResult);

impl MaterializeError {
    /// Construct a new `MaterializeError` wrapping a `ValidationResult`.
    pub fn new(result: crate::schema::ValidationResult) -> Self {
        Self(result)
    }

    /// Access the inner `ValidationResult`.
    pub fn result(&self) -> &crate::schema::ValidationResult {
        &self.0
    }

    /// Slice of all validation errors that caused materialization to fail.
    pub fn errors(&self) -> &[crate::schema::ValidationError] {
        self.0.errors()
    }
}

/// Crate-wide top-level error, mirroring Python's `OmnistError` base class.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OmnistError {
    /// An invalid Document operation or structure.
    #[error(transparent)]
    Document(#[from] DocumentError),
    /// An invalid Schema definition.
    #[error(transparent)]
    Schema(#[from] SchemaError),
    /// Document failed to conform to target Schema during materialization.
    #[error(transparent)]
    Materialize(#[from] MaterializeError),
    /// Source text syntax error with line/column coordinates.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// Document could not be written to destination format.
    #[error(transparent)]
    Write(#[from] WriteError),
    /// Format name not recognized in registry.
    #[error(transparent)]
    Format(#[from] FormatError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_error_display_includes_path_and_message() {
        let e = DocumentError::new("$.foo", "not a Document value");
        assert_eq!(e.to_string(), "$.foo: not a Document value");
    }

    #[test]
    fn omnist_error_wraps_document_error_transparently() {
        let doc_err = DocumentError::new("$.foo", "boom");
        let wrapped: OmnistError = doc_err.clone().into();
        assert_eq!(wrapped.to_string(), doc_err.to_string());
        assert!(matches!(wrapped, OmnistError::Document(ref inner) if *inner == doc_err));
    }

    #[test]
    fn document_error_clone_and_eq() {
        let a = DocumentError::new("$", "x");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn schema_error_display_and_eq() {
        let e = SchemaError::new("R.a", "schema.unknown-type", "unknown type 'Missing'");
        assert_eq!(e.path, "R.a");
        assert_eq!(e.code, "schema.unknown-type");
        assert_eq!(e.message, "unknown type 'Missing'");
        assert_eq!(e.to_string(), "unknown type 'Missing'");
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn omnist_error_wraps_schema_error_transparently() {
        let schema_err = SchemaError::new("$", "schema.syntax", "boom");
        let wrapped: OmnistError = schema_err.clone().into();
        assert_eq!(wrapped.to_string(), schema_err.to_string());
        assert!(matches!(wrapped, OmnistError::Schema(ref inner) if *inner == schema_err));
    }

    #[test]
    fn parse_error_display_includes_line_col_and_message() {
        let e = ParseError::new(3, 7, "parse.unexpected-token", "stray character '@'");
        assert_eq!(e.to_string(), "line 3, col 7: stray character '@'");
    }

    #[test]
    fn parse_error_carries_a_code_and_a_text_position_path() {
        let e = ParseError::new(3, 7, "parse.trailing-content", "x");
        assert_eq!(e.code, "parse.trailing-content");
        assert_eq!(e.position(), "3:7");
        let c = ParseError::codec_syntax(1, 1, "bad");
        assert_eq!(c.code, "parse.codec-syntax");
        assert_eq!(c.position(), "1:1");
    }

    #[test]
    fn document_error_code_is_none_unless_the_taxonomy_names_it() {
        assert_eq!(DocumentError::new("$", "misuse").code, None);
        let e = DocumentError::with_code("$.a", "document.unlabeled-element", "x");
        assert_eq!(e.code.as_deref(), Some("document.unlabeled-element"));
        assert_eq!(e.path, "$.a");
        assert_eq!(e.to_string(), "$.a: x");
    }

    #[test]
    fn write_error_diagnostic_fields_are_set_only_by_with_diagnostic() {
        let plain = WriteError::new("boom");
        assert_eq!((plain.path, plain.code), (None, None));
        let d = WriteError::with_diagnostic("$.n", "write.unsupported-value", "nope");
        assert_eq!(d.path.as_deref(), Some("$.n"));
        assert_eq!(d.code.as_deref(), Some("write.unsupported-value"));
        let r = WriteError::with_report("strict", crate::report::WriteReport::new());
        assert_eq!((r.path, r.code), (None, None));
    }

    #[test]
    fn omnist_error_wraps_parse_error_transparently() {
        let e = ParseError::new(1, 1, "parse.unexpected-token", "boom");
        let wrapped: OmnistError = e.clone().into();
        assert_eq!(wrapped.to_string(), e.to_string());
        assert!(matches!(wrapped, OmnistError::Parse(ref inner) if *inner == e));
    }

    #[test]
    fn write_error_display_and_from_document_error() {
        let e = WriteError::new("nesting exceeds the maximum depth (200)");
        assert_eq!(e.to_string(), "nesting exceeds the maximum depth (200)");
        let doc_err = DocumentError::new("$", "nesting exceeds the maximum depth (200)");
        let from_doc: WriteError = doc_err.into();
        assert_eq!(from_doc, e);
    }

    #[test]
    fn omnist_error_wraps_write_error_transparently() {
        let e = WriteError::new("boom");
        let wrapped: OmnistError = e.clone().into();
        assert_eq!(wrapped.to_string(), e.to_string());
        assert!(matches!(wrapped, OmnistError::Write(ref inner) if *inner == e));
    }

    #[test]
    fn materialize_error_new_result_and_errors_accessors() {
        let fields = vec![crate::schema::Field::required("x", crate::schema::STRING).unwrap()];
        let rec = crate::schema::Record::new(fields).unwrap();
        let mut env: indexmap::IndexMap<String, crate::schema::Record> = indexmap::IndexMap::new();
        env.insert("Root".to_string(), rec);
        let schema = crate::schema::Schema::new(crate::schema::Ref::new("Root"), env).unwrap();
        // An empty node under a schema requiring field "x" -- one
        // cardinality error, giving a non-empty `ValidationResult` to test
        // the accessors against.
        let node = crate::document::RawNode::Edges(vec![]);
        let res = crate::materialize::materialize(&node, Some(&schema))
            .unwrap_err()
            .0;
        assert!(!res.ok());

        let e = MaterializeError::new(res.clone());
        assert_eq!(e.result(), &res);
        assert_eq!(e.errors(), res.errors());
        assert_eq!(e.to_string(), res.to_string());
    }

    #[test]
    fn omnist_error_wraps_materialize_error_transparently() {
        let res = crate::schema::ValidationResult::new();
        let e = MaterializeError::new(res);
        let wrapped: OmnistError = e.clone().into();
        assert_eq!(wrapped.to_string(), e.to_string());
        assert!(matches!(wrapped, OmnistError::Materialize(ref inner) if *inner == e));
    }
}

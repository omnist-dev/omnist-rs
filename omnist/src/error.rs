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
    pub path: String,
    pub message: String,
}

impl DocumentError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// A Schema definition is invalid (bad cardinality, duplicate field label,
/// unknown scalar/ref name) -- raised by [`crate::schema`] at construction
/// time, mirroring Python's `SchemaError` in `~/dev/omnist/omnist/errors.py`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct SchemaError(pub String);

impl SchemaError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// An OML source string could not be parsed -- raised by
/// [`crate::oml::read_oml`], mirroring Python's `ParseError` in
/// `~/dev/omnist/omnist/errors.py`. Carries the same "line N, col N: msg"
/// convention the Python reference's scanner/parser produce.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("line {line}, col {col}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl ParseError {
    pub fn new(line: usize, col: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            col,
            message: message.into(),
        }
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
    pub message: String,
    pub report: Option<crate::report::WriteReport>,
}

impl WriteError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            report: None,
        }
    }

    /// Construct a `WriteError` carrying the [`crate::report::WriteReport`]
    /// that caused a strict-mode write to raise.
    pub fn with_report(message: impl Into<String>, report: crate::report::WriteReport) -> Self {
        Self {
            message: message.into(),
            report: Some(report),
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
    pub fn new(result: crate::schema::ValidationResult) -> Self {
        Self(result)
    }

    pub fn result(&self) -> &crate::schema::ValidationResult {
        &self.0
    }

    pub fn errors(&self) -> &[crate::schema::ValidationError] {
        self.0.errors()
    }
}

/// Crate-wide top-level error, mirroring Python's `OmnistError` base class.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OmnistError {
    #[error(transparent)]
    Document(#[from] DocumentError),
    #[error(transparent)]
    Schema(#[from] SchemaError),
    #[error(transparent)]
    Materialize(#[from] MaterializeError),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Write(#[from] WriteError),
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
        let e = SchemaError::new("unknown type 'Missing'");
        assert_eq!(e.to_string(), "unknown type 'Missing'");
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn omnist_error_wraps_schema_error_transparently() {
        let schema_err = SchemaError::new("boom");
        let wrapped: OmnistError = schema_err.clone().into();
        assert_eq!(wrapped.to_string(), schema_err.to_string());
        assert!(matches!(wrapped, OmnistError::Schema(ref inner) if *inner == schema_err));
    }

    #[test]
    fn parse_error_display_includes_line_col_and_message() {
        let e = ParseError::new(3, 7, "stray character '@'");
        assert_eq!(e.to_string(), "line 3, col 7: stray character '@'");
    }

    #[test]
    fn omnist_error_wraps_parse_error_transparently() {
        let e = ParseError::new(1, 1, "boom");
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

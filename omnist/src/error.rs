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

/// An in-memory Document could not be written as OML -- raised by
/// [`crate::oml::write_oml`], mirroring Python's `WriteError`. In practice
/// the only way this fires is the shared depth guard (OML is otherwise
/// lossless for every Document -- see the module doc comment on
/// `crate::oml`).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct WriteError(pub String);

impl WriteError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<DocumentError> for WriteError {
    fn from(e: DocumentError) -> Self {
        WriteError(e.message)
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
}

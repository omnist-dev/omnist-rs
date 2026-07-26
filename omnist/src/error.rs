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

/// Crate-wide top-level error, mirroring Python's `OmnistError` base class.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OmnistError {
    #[error(transparent)]
    Document(#[from] DocumentError),
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
        match wrapped {
            OmnistError::Document(inner) => assert_eq!(inner, doc_err),
        }
    }

    #[test]
    fn document_error_clone_and_eq() {
        let a = DocumentError::new("$", "x");
        let b = a.clone();
        assert_eq!(a, b);
    }
}

//! Adjustment reports for lossy writes.
//!
//! Ported from `~/dev/omnist/omnist/report.py`. Writing a [`crate::document::Doc`]
//! to a format that can't hold every value losslessly (JSON has no native
//! date/time type and no `NaN`/`Infinity`; TOML has no `null`) means the
//! writer has to *adjust* the data. Each adjustment is recorded as an
//! [`Adjustment`] in a [`WriteReport`] rather than lost silently. The same
//! report drives three behaviours, matching the Python reference exactly:
//!
//! * **lenient** (default) -- adjust and move on; the caller may ignore the
//!   report.
//! * **inspect** -- pass a `report: Option<&mut WriteReport>` to a writer (or
//!   call a format's `check_*`) to see what changed without stopping.
//! * **strict** (`strict: true`) -- [`finish_write`] returns
//!   [`crate::error::WriteError`] (carrying the report) if anything had to
//!   be adjusted.
//!
//! Each adjustment has a [`Severity`]: `Warning` (conventional/recoverable --
//! a date written as a string) or `Error` (likely to surprise or corrupt --
//! `NaN` written as JSON `null`). `strict` ignores severity and raises on
//! anything, matching Python's `finish_write`.

use crate::error::WriteError;

/// The path-numbering rule every codec scanner applies to a same-label
/// array's entries: the first occurrence gets the bare path
/// (`"{path}.{label}"`), later ones are indexed (`"{path}.{label}[{i}]"`).
/// Lives next to [`Adjustment`], whose `path` field this format feeds.
///
/// Used by scanners that build the full path eagerly (`toml.rs::strip_nulls`,
/// `xml.rs::scan_xml_into`, whose own recursion doesn't fit the shared
/// `formats::visit_grouped` walker -- see that function's doc comment). The
/// grouped-`Value` walkers (`json`/`yaml`) instead reuse a single path buffer
/// via [`push_child_path`], never allocating a `String` per edge.
pub(crate) fn child_path(path: &str, label: &str, index: usize) -> String {
    let mut s = String::with_capacity(path.len() + label.len() + 8);
    s.push_str(path);
    push_child_path(&mut s, label, index);
    s
}

/// Same rule as [`child_path`], but writes into a caller-owned buffer instead
/// of allocating a new `String`. Callers that walk a whole tree can push a
/// segment, recurse, then `buf.truncate` back -- one buffer, reused for
/// every edge, instead of one allocation per edge.
pub(crate) fn push_child_path(buf: &mut String, label: &str, index: usize) {
    use std::fmt::Write;
    buf.push('.');
    buf.push_str(label);
    if index != 0 {
        write!(buf, "[{index}]").expect("writing to a String never fails");
    }
}

/// How surprising/lossy a single [`Adjustment`] is. `strict` mode raises on
/// either severity; only [`WriteReport::is_ok`] (Python's `__bool__`)
/// distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Recoverable adjustment (e.g. date written as string).
    Warning,
    /// Lossy or corrupting adjustment (e.g. NaN written as null).
    Error,
}

/// One thing a writer changed to make the data fit the target format.
/// Mirrors Python's `Adjustment` `NamedTuple` field-for-field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adjustment {
    /// Same path style as validation, e.g. `"$.order.total"`.
    pub path: String,
    /// Stable, machine-checkable code, e.g. `"null.omitted"`.
    pub code: String,
    /// Human-readable sentence.
    pub message: String,
    /// The severity level of this adjustment.
    pub severity: Severity,
}

/// Everything a writer adjusted. Mirrors Python's `WriteReport`: truthy
/// (see [`WriteReport::is_ok`]) when there are no error-severity entries --
/// warnings alone are fine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteReport {
    adjustments: Vec<Adjustment>,
}

impl WriteReport {
    /// Create an empty `WriteReport`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one adjustment.
    pub fn add(
        &mut self,
        path: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        severity: Severity,
    ) {
        self.adjustments.push(Adjustment {
            path: path.into(),
            code: code.into(),
            message: message.into(),
            severity,
        });
    }

    /// All recorded adjustments, in the order they were added.
    pub fn adjustments(&self) -> &[Adjustment] {
        &self.adjustments
    }

    /// Only the `Warning`-severity adjustments.
    pub fn warnings(&self) -> Vec<&Adjustment> {
        self.adjustments
            .iter()
            .filter(|a| a.severity == Severity::Warning)
            .collect()
    }

    /// Only the `Error`-severity adjustments.
    pub fn errors(&self) -> Vec<&Adjustment> {
        self.adjustments
            .iter()
            .filter(|a| a.severity == Severity::Error)
            .collect()
    }

    /// Python's `__bool__`: `true` (safe) iff there are no error-severity
    /// entries -- warnings alone don't flip this.
    pub fn is_ok(&self) -> bool {
        self.errors().is_empty()
    }

    /// Returns `true` iff no adjustments have been recorded.
    pub fn is_empty(&self) -> bool {
        self.adjustments.is_empty()
    }

    /// Total number of recorded adjustments.
    pub fn len(&self) -> usize {
        self.adjustments.len()
    }

    /// Iterator over all recorded adjustments.
    pub fn iter(&self) -> std::slice::Iter<'_, Adjustment> {
        self.adjustments.iter()
    }
}

impl<'a> IntoIterator for &'a WriteReport {
    type Item = &'a Adjustment;
    type IntoIter = std::slice::Iter<'a, Adjustment>;

    fn into_iter(self) -> Self::IntoIter {
        self.adjustments.iter()
    }
}

impl std::fmt::Display for WriteReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.adjustments.is_empty() {
            return write!(f, "no adjustments");
        }
        let mut first = true;
        for a in &self.adjustments {
            if !first {
                writeln!(f)?;
            }
            first = false;
            let sev = match a.severity {
                Severity::Warning => "warning",
                Severity::Error => "error",
            };
            write!(f, "{sev}: {}: {}", a.path, a.message)?;
        }
        Ok(())
    }
}

/// The standard `strict`/`report` handling every format writer applies to
/// its own accumulated [`WriteReport`], mirroring Python's `finish_write`.
///
/// If `report` is given, `rep`'s adjustments are copied into it. If `strict`
/// and `rep` has any adjustments, returns [`WriteError`] carrying `rep`.
/// Otherwise returns `text`.
pub fn finish_write(
    text: String,
    rep: WriteReport,
    strict: bool,
    report: Option<&mut WriteReport>,
) -> Result<String, WriteError> {
    if let Some(out) = report {
        out.adjustments.extend(rep.adjustments.iter().cloned());
    }
    if strict && !rep.is_empty() {
        return Err(WriteError::with_report(rep.to_string(), rep));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_report_is_empty_and_ok() {
        let rep = WriteReport::new();
        assert!(rep.is_empty());
        assert_eq!(rep.len(), 0);
        assert!(rep.is_ok());
        assert_eq!(rep.to_string(), "no adjustments");
    }

    #[test]
    fn add_records_warning_and_error_separately() {
        let mut rep = WriteReport::new();
        rep.add(
            "$.a",
            "temporal.stringified",
            "written as a string",
            Severity::Warning,
        );
        rep.add(
            "$.b",
            "float.special",
            "NaN is not valid JSON",
            Severity::Error,
        );
        assert_eq!(rep.len(), 2);
        assert!(!rep.is_empty());
        assert_eq!(rep.warnings().len(), 1);
        assert_eq!(rep.errors().len(), 1);
        assert!(
            !rep.is_ok(),
            "an error-severity entry makes the report falsy"
        );
    }

    #[test]
    fn warnings_only_report_is_still_ok() {
        let mut rep = WriteReport::new();
        rep.add(
            "$.a",
            "temporal.stringified",
            "written as a string",
            Severity::Warning,
        );
        assert!(rep.is_ok());
    }

    #[test]
    fn display_lists_each_adjustment() {
        let mut rep = WriteReport::new();
        rep.add("$.a", "code.a", "message a", Severity::Warning);
        rep.add("$.b", "code.b", "message b", Severity::Error);
        assert_eq!(
            rep.to_string(),
            "warning: $.a: message a\nerror: $.b: message b"
        );
    }

    #[test]
    fn iter_and_into_iter_yield_adjustments_in_order() {
        let mut rep = WriteReport::new();
        rep.add("$.a", "code.a", "m", Severity::Warning);
        rep.add("$.b", "code.b", "m", Severity::Warning);
        let paths: Vec<&str> = rep.iter().map(|a| a.path.as_str()).collect();
        assert_eq!(paths, vec!["$.a", "$.b"]);
        let paths2: Vec<&str> = (&rep).into_iter().map(|a| a.path.as_str()).collect();
        assert_eq!(paths2, vec!["$.a", "$.b"]);
    }

    #[test]
    fn adjustments_accessor_matches_add_order() {
        let mut rep = WriteReport::new();
        rep.add("$.a", "code.a", "m", Severity::Warning);
        assert_eq!(rep.adjustments().len(), 1);
        assert_eq!(rep.adjustments()[0].code, "code.a");
    }

    #[test]
    fn finish_write_lenient_returns_text_regardless_of_adjustments() {
        let mut rep = WriteReport::new();
        rep.add("$.a", "code.a", "m", Severity::Error);
        let out = finish_write("text".to_string(), rep, false, None).unwrap();
        assert_eq!(out, "text");
    }

    #[test]
    fn finish_write_strict_raises_on_any_adjustment() {
        let mut rep = WriteReport::new();
        rep.add(
            "$.a",
            "float.special",
            "NaN is not valid JSON",
            Severity::Warning,
        );
        let err = finish_write("text".to_string(), rep, true, None).unwrap_err();
        // message uses Display (path + human text), not the machine `code`.
        assert!(!err.to_string().contains("float.special"));
        assert!(err.to_string().contains("$.a"));
        assert_eq!(err.report().unwrap().len(), 1);
    }

    #[test]
    fn finish_write_strict_with_no_adjustments_still_returns_text() {
        let rep = WriteReport::new();
        let out = finish_write("text".to_string(), rep, true, None).unwrap();
        assert_eq!(out, "text");
    }

    #[test]
    fn finish_write_copies_into_caller_supplied_report() {
        let mut rep = WriteReport::new();
        rep.add("$.a", "code.a", "m", Severity::Warning);
        let mut out_report = WriteReport::new();
        let out = finish_write("text".to_string(), rep, false, Some(&mut out_report)).unwrap();
        assert_eq!(out, "text");
        assert_eq!(out_report.len(), 1);
        assert_eq!(out_report.adjustments()[0].path, "$.a");
    }

    #[test]
    fn finish_write_strict_still_populates_caller_report_before_erroring() {
        let mut rep = WriteReport::new();
        rep.add("$.a", "code.a", "m", Severity::Error);
        let mut out_report = WriteReport::new();
        let err = finish_write("text".to_string(), rep, true, Some(&mut out_report)).unwrap_err();
        assert_eq!(out_report.len(), 1);
        assert_eq!(err.report().unwrap().len(), 1);
    }
}

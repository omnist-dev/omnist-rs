//! Leading byte-order-mark handling for every read surface (omnist-spec
//! D-15 and D-21, `docs/02-document-model.md` section 2.5).
//!
//! This is the ONE place a leading `U+FEFF` is stripped or rejected. Every
//! reader (OML, OSD, JSON, YAML, TOML, XML) calls [`strip_leading_bom`] on
//! its input before doing anything else, and no reader or library may strip
//! the mark a second time:
//!
//! - **D-15**: exactly one `U+FEFF` at offset zero is consumed and
//!   contributes nothing to the Document.
//! - **D-21**: a second `U+FEFF` still standing at offset zero of the text
//!   that remains is a rejection, reported at text position `1:1` (computed
//!   on the remaining text, not the original input), never silently
//!   consumed -- whatever the parsing library behind a codec would do with
//!   it. The rejection's code is `parse.unexpected-token` on OML and OSD and
//!   `parse.codec-syntax` on JSON, YAML, TOML and XML (spec section 8.3.1,
//!   E-24); each reader maps [`DoubledBom`] onto its own error type.
//! - A `U+FEFF` anywhere other than offset zero is ordinary content and is
//!   preserved byte for byte (D-15, D-21).
//!
//! Writers never emit a BOM (D-15); that is pinned by the
//! `writers_never_emit_a_bom` test over every writer.
//!
//! The mark is always spelled as the escape `'\u{FEFF}'` in source, never as
//! a raw (invisible) character; `no_tracked_source_file_contains_a_raw_bom`
//! enforces it.

/// The byte-order mark, `U+FEFF`.
pub(crate) const BOM: char = '\u{FEFF}';

/// A second leading `U+FEFF` was found after the first was stripped (D-21).
///
/// The rejection is always at text position `1:1`, so the type carries
/// nothing; each reader builds its own error from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DoubledBom;

/// The human-readable message every reader attaches to the D-21 rejection.
pub(crate) const DOUBLED_BOM_MESSAGE: &str = "a second leading U+FEFF byte-order mark; exactly one is consumed and a further one is rejected (D-21)";

/// Strip at most one leading `U+FEFF` (D-15) and reject a second (D-21).
///
/// Returns the text the reader should scan. Only offset zero is examined: a
/// `U+FEFF` anywhere else is left alone.
pub(crate) fn strip_leading_bom(text: &str) -> Result<&str, DoubledBom> {
    match text.strip_prefix(BOM) {
        None => Ok(text),
        Some(rest) if rest.starts_with(BOM) => Err(DoubledBom),
        Some(rest) => Ok(rest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_without_a_mark_is_returned_unchanged() {
        assert_eq!(strip_leading_bom("a: 1"), Ok("a: 1"));
        assert_eq!(strip_leading_bom(""), Ok(""));
    }

    #[test]
    fn exactly_one_leading_mark_is_stripped() {
        assert_eq!(strip_leading_bom("\u{FEFF}a: 1"), Ok("a: 1"));
        assert_eq!(strip_leading_bom("\u{FEFF}"), Ok(""));
    }

    #[test]
    fn a_second_leading_mark_is_rejected() {
        assert_eq!(strip_leading_bom("\u{FEFF}\u{FEFF}a: 1"), Err(DoubledBom));
        assert_eq!(
            strip_leading_bom("\u{FEFF}\u{FEFF}\u{FEFF}"),
            Err(DoubledBom)
        );
        assert_eq!(strip_leading_bom("\u{FEFF}\u{FEFF}"), Err(DoubledBom));
    }

    #[test]
    fn a_mark_that_is_not_at_offset_zero_is_ordinary_content() {
        assert_eq!(strip_leading_bom("a\u{FEFF}b"), Ok("a\u{FEFF}b"));
        assert_eq!(strip_leading_bom(" \u{FEFF}"), Ok(" \u{FEFF}"));
        // One mark stripped; the interior one survives.
        assert_eq!(strip_leading_bom("\u{FEFF}a\u{FEFF}"), Ok("a\u{FEFF}"));
    }

    #[test]
    fn the_mark_is_not_whitespace_to_any_predicate_the_readers_use() {
        // A reader that skips whitespace with one of these would swallow the
        // mark silently (found in omnist-ts's OSD scanner).
        assert!(!BOM.is_whitespace());
        assert!(!BOM.is_ascii_whitespace());
        assert!(!regex::Regex::new(r"\s").unwrap().is_match("\u{FEFF}"));
        assert!(!BOM.is_alphabetic());
        assert!(!BOM.is_alphanumeric());
    }

    /// No tracked source file may contain a raw (invisible) U+FEFF -- the
    /// mark is always written as an escape. Scans this file too, so the
    /// check cannot miss itself.
    #[test]
    fn no_tracked_source_file_contains_a_raw_bom() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut offenders = Vec::new();
        scan(&root, &mut offenders);
        assert!(
            offenders.is_empty(),
            "raw U+FEFF (bytes EF BB BF) found; write it as the escape \\u{{FEFF}}: {offenders:?}"
        );
    }

    /// The scan itself must be able to fail: a temp tree holding a file with
    /// the three BOM bytes (built from bytes, never a raw literal) is reported,
    /// and a missing directory is ignored.
    #[test]
    fn the_scan_reports_a_file_containing_the_bom_bytes() {
        let dir = std::env::temp_dir().join("omnist-bom-scan-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("bad.rs"), [b'/', 0xEF, 0xBB, 0xBF]).unwrap();
        std::fs::write(dir.join("clean.rs"), b"fn main() {}").unwrap();
        std::fs::write(dir.join("image.png"), [0xEF, 0xBB, 0xBF]).unwrap();
        let mut offenders = Vec::new();
        scan(&dir, &mut offenders);
        assert_eq!(offenders.len(), 1, "{offenders:?}");
        assert!(offenders[0].ends_with("bad.rs"));
        let mut none = Vec::new();
        scan(&dir.join("does-not-exist"), &mut none);
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn is_text_source(path: &std::path::Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some(
                "rs" | "toml"
                    | "md"
                    | "yml"
                    | "yaml"
                    | "json"
                    | "html"
                    | "css"
                    | "js"
                    | "txt"
                    | "lock"
            )
        )
    }

    fn scan(dir: &std::path::Path, offenders: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.filter_map(Result::ok) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Build output, VCS metadata, the vendored spec (its vectors
            // legitimately carry the mark, escaped or not) and mdBook
            // output are not this crate's source.
            if matches!(name.as_str(), "target" | ".git" | "vendor" | "node_modules") {
                continue;
            }
            if path.is_dir() {
                scan(&path, offenders);
            } else if is_text_source(&path)
                && let Ok(bytes) = std::fs::read(&path)
                && bytes.windows(3).any(|w| w == [0xEF, 0xBB, 0xBF])
            {
                offenders.push(path.display().to_string());
            }
        }
    }
}

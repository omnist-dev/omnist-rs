//! Shared "1-based (line, column) of an offset" helper, factored out of
//! `toml.rs`, `xml.rs`, and `json.rs` (issue #48) -- all three had declared
//! byte-for-byte identical byte-offset versions of the same three-line
//! loop (`xml.rs`'s own doc comment already said as much). `json.rs`'s
//! scanner was char-vec-based at the time issue #48 was scoped, but issue
//! #43 rewrote it to scan by byte offset directly (see
//! `json.rs::Parser::new`'s doc comment) before #48 landed, so `json.rs`
//! calls this same byte-offset helper rather than needing a char-index
//! variant.
//!
//! This is a pure refactor: no observable behavior change. Every existing
//! `ParseError` line/column value is reproduced exactly by this function.

/// 1-based (line, column) of byte offset `pos` in `text`.
///
/// Previously duplicated verbatim as `toml.rs::line_col` and
/// `xml.rs::line_col` -- see issue #48.
pub(crate) fn line_col_bytes(text: &str, pos: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut last_nl: Option<usize> = None;
    for (i, b) in text.as_bytes()[..pos.min(text.len())].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            last_nl = Some(i);
        }
    }
    let col = match last_nl {
        Some(i) => pos - i,
        None => pos + 1,
    };
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_bytes_first_line_first_column() {
        assert_eq!(line_col_bytes("abc", 0), (1, 1));
    }

    #[test]
    fn line_col_bytes_reports_line_two_after_a_newline() {
        // Forces the newline-counting branch and its `Some(i)` column-
        // offset arm, neither reachable from any single-line position.
        assert_eq!(line_col_bytes("a\nbc", 3), (2, 2));
    }
}

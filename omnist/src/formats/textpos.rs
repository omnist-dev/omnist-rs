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

/// 1-based (line, column) of byte offset `pos` in `text`, for OML and OSD
/// text positions (omnist-spec E-28, E-29): the column counts Unicode code
/// points from the start of the line (an astral character is one), and a
/// line ends at LF only (a CRLF is one break, a lone CR is not one). The
/// byte offset still locates the failure; the column is derived from it.
/// One pass over `text[..pos]`, so linear in the offset.
pub(crate) fn line_col_chars(text: &str, pos: usize) -> (usize, usize) {
    let head = &text.as_bytes()[..pos.min(text.len())];
    let line = 1 + head.iter().filter(|b| **b == b'\n').count();
    let line_start = head.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    // Count every byte that is not a UTF-8 continuation byte: one per code
    // point, and no panic if `pos` falls inside a character.
    let col = 1 + head[line_start..]
        .iter()
        .filter(|b| (**b & 0xC0) != 0x80)
        .count();
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

    fn ch(cp: u32) -> String {
        char::from_u32(cp).unwrap().to_string()
    }

    #[test]
    fn line_col_chars_counts_code_points_not_bytes() {
        // BMP non-ASCII (2 bytes), astral (4 bytes), combining mark (its own
        // code point), tab (one column).
        assert_eq!(line_col_chars(&format!("{}x", ch(0xE9)), 2), (1, 2));
        assert_eq!(line_col_chars(&format!("{}x", ch(0x1F600)), 4), (1, 2));
        assert_eq!(line_col_chars(&format!("e{}x", ch(0x301)), 3), (1, 3));
        assert_eq!(line_col_chars("\tx", 1), (1, 2));
    }

    #[test]
    fn line_col_chars_lines_advance_at_lf_only() {
        assert_eq!(line_col_chars("a\r\nb", 3), (2, 1));
        assert_eq!(line_col_chars("a\rb", 2), (1, 3));
        let t = format!("{0}\n{0}x", ch(0x1F600));
        assert_eq!(line_col_chars(&t, 9), (2, 2));
    }

    #[test]
    fn line_col_chars_clamps_and_tolerates_mid_character_offsets() {
        assert_eq!(line_col_chars("ab", 99), (1, 3));
        assert_eq!(line_col_chars(&ch(0x1F600), 2), (1, 2));
    }
}

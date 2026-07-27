//! Shared table-driven quoted-string escaper, factored out of
//! `oml.rs`, `formats/json.rs`, `formats/toml.rs`, and `formats/yaml.rs`
//! (issue #50) -- those four modules each hand-rolled the same
//! `for c in s.chars() { match c { ... } }` escape loop, differing only in
//! which characters get named escapes, what a bare control character's
//! escape looks like (`\u{04x}` vs `\x{02x}`), and (TOML only) one extra
//! non-C0 character that also needs escaping.
//!
//! This is a pure refactor: no observable behavior change. Each format's
//! `EscapeSpec` below reproduces that format's previous arm-by-arm behavior
//! exactly -- see the equivalence tests in this module and in each format's
//! own test module.
//!
//! ## All-occurrences invariant (omnist-ts#36-class guard)
//!
//! [`write_quoted`] is a single `for c in s.chars()` loop that inspects and
//! emits every character in turn -- it can never under-sanitize by only
//! touching the first match of an illegal character, the same property each
//! of the four original per-format loops had individually (and the same
//! property `formats/xml.rs`'s separate, structurally different
//! entity-escaper -- out of scope for this refactor -- maintains via its own
//! `chars().map()`). This module's tests assert that multiple, including
//! adjacent, occurrences of the same escapable character are *all* escaped,
//! for every format's spec.

/// How a bare control character (one with no named escape) is rendered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ControlStyle {
    /// `\u{04x}` -- JSON, TOML, OML.
    HexU,
    /// `\x{02x}` -- YAML.
    HexX,
}

/// A format's escaping rules for [`write_quoted`].
pub(crate) struct EscapeSpec {
    /// Named escapes, checked in order before falling back to `control`.
    /// Every spec includes `"` and `\`; formats differ in which of
    /// `\n`/`\r`/`\t`/`\b`/`\f` (and, for YAML, `U+0085` -> `\N`) they add.
    pub named: &'static [(char, &'static str)],
    /// Rendering for any other C0 control character (or `also_escape`
    /// member) that isn't covered by `named`.
    pub control: ControlStyle,
    /// Characters that must be escaped via `control` even though they
    /// aren't C0 (`< 0x20`). Only TOML uses this, for `U+007F`.
    pub also_escape: &'static [char],
}

/// `formats/json.rs::write_json_string`'s previous escape table.
pub(crate) const JSON_ESCAPES: EscapeSpec = EscapeSpec {
    named: &[
        ('"', "\\\""),
        ('\\', "\\\\"),
        ('\n', "\\n"),
        ('\r', "\\r"),
        ('\t', "\\t"),
        ('\u{08}', "\\b"),
        ('\u{0c}', "\\f"),
    ],
    control: ControlStyle::HexU,
    also_escape: &[],
};

/// `formats/toml.rs::write_toml_string`'s previous escape table -- identical
/// to JSON's except `U+007F` is also escaped.
pub(crate) const TOML_ESCAPES: EscapeSpec = EscapeSpec {
    named: JSON_ESCAPES.named,
    control: ControlStyle::HexU,
    also_escape: &['\u{7f}'],
};

/// `oml.rs::write_string`'s previous escape table -- like JSON's but without
/// the `\b`/`\f` named escapes (those C0 chars fall through to `control`).
pub(crate) const OML_ESCAPES: EscapeSpec = EscapeSpec {
    named: &[
        ('"', "\\\""),
        ('\\', "\\\\"),
        ('\n', "\\n"),
        ('\r', "\\r"),
        ('\t', "\\t"),
    ],
    control: ControlStyle::HexU,
    also_escape: &[],
};

/// `formats/yaml.rs::write_yaml_string`'s previous escape table (the
/// quoted-branch loop only -- `needs_quoting`'s plain-vs-quoted decision is
/// untouched). No named `\r` (a literal `\r` falls through to `control` and
/// renders as `\x0d`, matching the original arm-less behavior), and
/// `U+0085` gets the YAML-specific `\N` escape.
pub(crate) const YAML_ESCAPES: EscapeSpec = EscapeSpec {
    named: &[
        ('"', "\\\""),
        ('\\', "\\\\"),
        ('\n', "\\n"),
        ('\t', "\\t"),
        ('\u{0085}', "\\N"),
    ],
    control: ControlStyle::HexX,
    also_escape: &[],
};

/// Writes `s` as a double-quoted, escaped string into `out`, per `spec`.
///
/// A single per-char loop: every character is inspected and emitted exactly
/// once, so an illegal/escapable character can never be under-sanitized by
/// only having its first occurrence touched (the omnist-ts#36 class of bug
/// -- see this module's doc comment).
pub(crate) fn write_quoted(s: &str, spec: &EscapeSpec, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        if let Some((_, escaped)) = spec.named.iter().find(|(named_c, _)| *named_c == c) {
            out.push_str(escaped);
        } else if (c as u32) < 0x20 || spec.also_escape.contains(&c) {
            match spec.control {
                ControlStyle::HexU => out.push_str(&format!("\\u{:04x}", c as u32)),
                ControlStyle::HexX => out.push_str(&format!("\\x{:02x}", c as u32)),
            }
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------- old implementations
    //
    // Kept here only long enough to run the exhaustive equivalence checks
    // below against the new table-driven `write_quoted`, then deleted --
    // per issue #50's requirement to check before removing any original.

    fn old_json(s: &str) -> String {
        let mut out = String::new();
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn old_toml(s: &str) -> String {
        let mut out = String::new();
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 || c == '\u{7f}' => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn old_oml(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for ch in s.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn old_yaml_quoted_branch(s: &str) -> String {
        let mut out = String::new();
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\u{0085}' => out.push_str("\\N"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn new_out(s: &str, spec: &EscapeSpec) -> String {
        let mut out = String::new();
        write_quoted(s, spec, &mut out);
        out
    }

    /// Every `char` in `0..=0x10FFFF` (skipping the surrogate range, which
    /// isn't a valid Rust `char`), as a single-character string, plus a
    /// handful of multi-char / multi-byte-Unicode / repeated-illegal-char
    /// strings -- the exhaustive corpus issue #50 asks for.
    fn exhaustive_single_char_corpus() -> Vec<String> {
        let mut v = Vec::new();
        for cp in 0u32..=0x10FFFF {
            if let Some(c) = char::from_u32(cp) {
                v.push(c.to_string());
            }
        }
        v
    }

    fn extra_corpus() -> Vec<String> {
        vec![
            String::new(),
            "plain ascii, nothing to escape".to_string(),
            "quote\"quote\"quote\"".to_string(),
            "back\\slash\\back\\slash".to_string(),
            "\r\r\r\r".to_string(),
            "\n\n\n\n".to_string(),
            "\t\t\t\t".to_string(),
            "\u{08}\u{08}\u{0c}\u{0c}".to_string(),
            "\u{01}\u{01}\u{01}".to_string(),
            "\u{7f}\u{7f}\u{7f}".to_string(),
            "\u{0085}\u{0085}".to_string(),
            "mixed \" \\ \n \r \t \u{08} \u{0c} \u{01} \u{7f} \u{0085} end".to_string(),
            "unicode: héllo wörld 日本語 🎉🎉🎉".to_string(),
            "\u{1}\"\\\n\r\t\u{1}\"\\\n\r\t".to_string(),
        ]
    }

    #[test]
    fn json_equivalence_exhaustive_and_extra_corpus() {
        let mut checked = 0usize;
        for s in exhaustive_single_char_corpus()
            .into_iter()
            .chain(extra_corpus())
        {
            assert_eq!(
                old_json(&s),
                new_out(&s, &JSON_ESCAPES),
                "mismatch for {s:?}"
            );
            checked += 1;
        }
        assert!(checked > 1_100_000, "expected >1.1M cases, got {checked}");
    }

    #[test]
    fn toml_equivalence_exhaustive_and_extra_corpus() {
        let mut checked = 0usize;
        for s in exhaustive_single_char_corpus()
            .into_iter()
            .chain(extra_corpus())
        {
            assert_eq!(
                old_toml(&s),
                new_out(&s, &TOML_ESCAPES),
                "mismatch for {s:?}"
            );
            checked += 1;
        }
        assert!(checked > 1_100_000, "expected >1.1M cases, got {checked}");
    }

    #[test]
    fn oml_equivalence_exhaustive_and_extra_corpus() {
        let mut checked = 0usize;
        for s in exhaustive_single_char_corpus()
            .into_iter()
            .chain(extra_corpus())
        {
            assert_eq!(old_oml(&s), new_out(&s, &OML_ESCAPES), "mismatch for {s:?}");
            checked += 1;
        }
        assert!(checked > 1_100_000, "expected >1.1M cases, got {checked}");
    }

    #[test]
    fn yaml_equivalence_exhaustive_and_extra_corpus() {
        let mut checked = 0usize;
        for s in exhaustive_single_char_corpus()
            .into_iter()
            .chain(extra_corpus())
        {
            assert_eq!(
                old_yaml_quoted_branch(&s),
                new_out(&s, &YAML_ESCAPES),
                "mismatch for {s:?}"
            );
            checked += 1;
        }
        assert!(checked > 1_100_000, "expected >1.1M cases, got {checked}");
    }

    // ------------------------------------------------ all-occurrences guard
    //
    // Explicit, per-format check that *every* occurrence of an escapable
    // character is escaped, not just the first -- the omnist-ts#36 class of
    // bug this refactor must not reintroduce in any of the four formats.

    #[test]
    fn json_escapes_every_occurrence_not_just_the_first() {
        let out = new_out("a\"b\"c\\d\\e\u{01}f\u{01}", &JSON_ESCAPES);
        assert_eq!(out, "\"a\\\"b\\\"c\\\\d\\\\e\\u0001f\\u0001\"");
    }

    #[test]
    fn toml_escapes_every_occurrence_not_just_the_first() {
        let out = new_out("a\u{7f}b\u{7f}c\"d\"e", &TOML_ESCAPES);
        assert_eq!(out, "\"a\\u007fb\\u007fc\\\"d\\\"e\"");
    }

    #[test]
    fn oml_escapes_every_occurrence_not_just_the_first() {
        let out = new_out("a\rb\rc\nd\ne", &OML_ESCAPES);
        assert_eq!(out, "\"a\\rb\\rc\\nd\\ne\"");
    }

    #[test]
    fn yaml_escapes_every_occurrence_not_just_the_first() {
        let out = new_out("a\u{0085}b\u{0085}c\"d\"e", &YAML_ESCAPES);
        assert_eq!(out, "\"a\\Nb\\Nc\\\"d\\\"e\"");
    }

    #[test]
    fn adjacent_repeats_of_the_same_illegal_char_are_all_escaped() {
        // Adjacent (not just non-adjacent) repeats -- the shape a
        // first-match-only regression would be most likely to miss.
        assert_eq!(
            new_out("\u{01}\u{01}\u{01}", &JSON_ESCAPES),
            "\"\\u0001\\u0001\\u0001\""
        );
        assert_eq!(
            new_out("\u{7f}\u{7f}\u{7f}", &TOML_ESCAPES),
            "\"\\u007f\\u007f\\u007f\""
        );
        assert_eq!(new_out("\r\r\r", &OML_ESCAPES), "\"\\r\\r\\r\"");
        assert_eq!(
            new_out("\u{0085}\u{0085}\u{0085}", &YAML_ESCAPES),
            "\"\\N\\N\\N\""
        );
    }
}

//! Shared digit-cap guard for integer literals, factored out of
//! `oml.rs`, `formats/json.rs`, `formats/toml.rs`, and `formats/yaml.rs`
//! (issue #49) -- those four modules each declared their own copy of
//! `MAX_INT_DIGITS` and hand-spelled the same two-tier error-message pair,
//! with each file's comment explicitly noting it was a copy of the others.
//!
//! This is a pure refactor: no observable behavior change. The per-format
//! *findings* documented in each module's own doc comment (provenance,
//! divergences from Python, etc.) stay where they are -- only the constant
//! and the two message constructors move here.
//!
//! The one wrinkle: `json.rs` (via `oml.rs`-style `error_at`) does not
//! embed a format-name prefix in its message text, while `toml.rs` and
//! `yaml.rs` embed `"invalid TOML: "`/`"invalid YAML: "` inline. Both
//! constructors below take that prefix as a parameter -- pass `""` to
//! reproduce the unprefixed form -- so every call site's produced string
//! is byte-identical to what it emitted before this refactor.

/// Same guard, same constant, previously copied into `oml.rs`,
/// `formats/json.rs`, `formats/toml.rs`, and `formats/yaml.rs` -- see
/// issue #49. Provenance: omnist-ts#54, CPython's
/// `sys.set_int_max_str_digits`.
pub(crate) const MAX_INT_DIGITS: usize = 4300;

/// The digit-cap rejection message, spelled identically for every format
/// modulo each format's own error-prefix convention.
///
/// `prefix` is prepended verbatim -- pass `""` for formats (OML, JSON) that
/// don't embed a format-name prefix in this message, or `"invalid TOML: "`/
/// `"invalid YAML: "` for the formats that do.
pub(crate) fn over_cap_message(prefix: &str, digit_count: usize) -> String {
    format!(
        "{prefix}integer literal has {digit_count} digits, exceeding the {MAX_INT_DIGITS}-digit \
         limit (security: unbounded-digit int-to-str conversion is superlinear)"
    )
}

/// The "fits the cap but not an `i64`" message, spelled identically for
/// every format modulo each format's own error-prefix convention.
///
/// `prefix` is prepended verbatim, same convention as [`over_cap_message`].
pub(crate) fn out_of_range_message(prefix: &str, literal: &str) -> String {
    format!("{prefix}integer literal {literal:?} is out of range for a 64-bit integer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_cap_message_no_prefix() {
        assert_eq!(
            over_cap_message("", 4301),
            "integer literal has 4301 digits, exceeding the 4300-digit limit (security: \
             unbounded-digit int-to-str conversion is superlinear)"
        );
    }

    #[test]
    fn over_cap_message_with_prefix() {
        assert_eq!(
            over_cap_message("invalid TOML: ", 4301),
            "invalid TOML: integer literal has 4301 digits, exceeding the 4300-digit limit \
             (security: unbounded-digit int-to-str conversion is superlinear)"
        );
    }

    #[test]
    fn out_of_range_message_no_prefix() {
        assert_eq!(
            out_of_range_message("", "99999999999999999999"),
            "integer literal \"99999999999999999999\" is out of range for a 64-bit integer"
        );
    }

    #[test]
    fn out_of_range_message_with_prefix() {
        assert_eq!(
            out_of_range_message("invalid YAML: ", "99999999999999999999"),
            "invalid YAML: integer literal \"99999999999999999999\" is out of range for a \
             64-bit integer"
        );
    }
}

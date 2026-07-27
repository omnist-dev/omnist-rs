//! Shared float-writing logic, factored out of `oml.rs`, `formats/json.rs`,
//! `formats/toml.rs`, `formats/xml.rs`, and `formats/yaml.rs` (issue #47) --
//! those five modules each declared their own copy of the same
//! render-then-inspect float writer, differing only in how each format
//! spells NaN/+Infinity/-Infinity.
//!
//! This is a pure refactor: no observable behavior change. Each format's
//! non-special-value rendering was already identical -- `x.to_string()`,
//! with a trailing `.0` appended only if the rendered string doesn't
//! already contain `.`/`e`/`E`. That check (not a fixed magnitude cutoff)
//! is issue #46's fix: Rust's `f64::to_string()` drops the decimal point
//! for integral values >= 1e17, e.g. `1e17.to_string() ==
//! "100000000000000000"`, which a magnitude-cutoff check would miss.
//!
//! The one wrinkle: the five call sites split into two shapes -- `json.rs`,
//! `toml.rs`, and `yaml.rs` append into a caller-owned `&mut String` buffer,
//! while `xml.rs` and `oml.rs` return an owned `String`. [`write_float`]
//! takes the `&mut String` shape (the majority); [`float_to_string`] is a
//! thin wrapper for the two return-a-`String` call sites.

/// Appends `x`'s float rendering to `out`, using `nan`/`inf`/`neg_inf` as
/// the literal spellings for the three non-finite cases -- each format
/// spells these differently (e.g. JSON's `NaN`/`Infinity`/`-Infinity` vs.
/// YAML's `.nan`/`.inf`/`-.inf` vs. TOML/XML/OML's `nan`/`inf`/`-inf`), so
/// callers pass their own spelling table rather than this module
/// homogenizing it.
pub(crate) fn write_float(x: f64, nan: &str, inf: &str, neg_inf: &str, out: &mut String) {
    if x.is_nan() {
        out.push_str(nan);
    } else if x.is_infinite() {
        out.push_str(if x > 0.0 { inf } else { neg_inf });
    } else {
        // Match `json.dumps`'s (and every other format's) float rendering
        // of an integral value, e.g. `1.0` (not `1`) -- `repr(1.0) ==
        // "1.0"` in Python. Rust's `f64` `Display` never adds a decimal
        // point on its own -- and for large enough integral magnitudes
        // (>= 1e17) it renders a bare digit run with no `.`/`e`/`E` at all
        // -- so the correct, magnitude-independent test is whether the
        // *rendered string* already contains one of those markers, not
        // whether `x` is below some fixed cutoff (issue #46).
        let s = x.to_string();
        if s.contains('.') || s.contains('e') || s.contains('E') {
            out.push_str(&s);
        } else {
            out.push_str(&s);
            out.push_str(".0");
        }
    }
}

/// Same rendering as [`write_float`], returned as an owned `String` -- for
/// the two call sites (`xml.rs`, `oml.rs`) that build the whole value as a
/// `String` rather than appending into a shared buffer.
pub(crate) fn float_to_string(x: f64, nan: &str, inf: &str, neg_inf: &str) -> String {
    let mut out = String::new();
    write_float(x, nan, inf, neg_inf, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_float_covers_every_branch() {
        let mut out = String::new();
        write_float(f64::NAN, "NaN", "Infinity", "-Infinity", &mut out);
        assert_eq!(out, "NaN");
        out.clear();
        write_float(f64::INFINITY, "NaN", "Infinity", "-Infinity", &mut out);
        assert_eq!(out, "Infinity");
        out.clear();
        write_float(f64::NEG_INFINITY, "NaN", "Infinity", "-Infinity", &mut out);
        assert_eq!(out, "-Infinity");
        out.clear();
        write_float(1.5, "NaN", "Infinity", "-Infinity", &mut out);
        assert_eq!(out, "1.5");
        out.clear();
        write_float(2.0, "NaN", "Infinity", "-Infinity", &mut out);
        assert_eq!(out, "2.0");
    }

    #[test]
    fn write_float_uses_the_caller_supplied_spelling_table() {
        let mut out = String::new();
        write_float(f64::NAN, ".nan", ".inf", "-.inf", &mut out);
        assert_eq!(out, ".nan");
        out.clear();
        write_float(f64::NEG_INFINITY, ".nan", ".inf", "-.inf", &mut out);
        assert_eq!(out, "-.inf");
    }

    #[test]
    fn round_trips_integral_float_at_and_above_1e17_boundary_issue_46() {
        for x in [1.0e17, 1.0e18, -1.23e17, 9.9e16_f64] {
            let s = float_to_string(x, "nan", "inf", "-inf");
            assert!(s.contains('.'), "x={x} s={s}");
            let back: f64 = s.parse().unwrap();
            assert_eq!(back, x, "x={x} s={s}");
        }
    }

    #[test]
    fn float_to_string_matches_write_float() {
        assert_eq!(float_to_string(1.5, "nan", "inf", "-inf"), "1.5");
        assert_eq!(float_to_string(f64::NAN, "nan", "inf", "-inf"), "nan");
        assert_eq!(float_to_string(f64::INFINITY, "nan", "inf", "-inf"), "inf");
        assert_eq!(
            float_to_string(f64::NEG_INFINITY, "nan", "inf", "-inf"),
            "-inf"
        );
    }
}

//! Runtime-configurable safety limits (omnist-spec section 2.4, D-9 to D-13).
//!
//! The spec requires every implementation to enforce a *finite* limit on
//! nesting depth, node count and integer digits, lets it pick the numbers
//! (D-10), and requires it to document them (D-11). This module is where
//! this port documents them and where a caller changes them.
//!
//! | Limit | Default | Ceiling | Field |
//! |---|---|---|---|
//! | Nesting depth | [`DEFAULT_MAX_DEPTH`] (200) | [`MAX_DEPTH_CEILING`] (1 000) | [`Limits::max_depth`] |
//! | Node count (containers) | [`DEFAULT_MAX_NODES`] (1 000 000) | [`MAX_NODES_CEILING`] (10 000 000) | [`Limits::max_nodes`] |
//! | Integer digits | [`DEFAULT_MAX_INT_DIGITS`] (4 300) | [`MAX_INT_DIGITS_CEILING`] (43 000) | [`Limits::max_int_digits`] |
//!
//! The default of each is the spec's reference default; the ceilings are
//! this port's own (the spec recommends none for these three): a limit is a
//! denial-of-service bound, not a tuning knob without an upper end, and the
//! depth ceiling in particular keeps the recursive readers and writers well
//! inside a thread's stack.
//!
//! As for [`crate::formats::yaml::YamlReadOptions`], `0` selects the default
//! (a zero or unset value never widens a limit) and a value above its
//! ceiling is *refused* by [`Limits::validate`], never clamped.
//!
//! A node is a container (an edge list), as D-9 counts them: keys and scalar
//! values are not nodes, so a flat mapping of any number of scalar entries is
//! one node. A document whose deepest leaf sits at depth `max_depth` is
//! accepted and one level deeper is refused; likewise for the other two
//! (a count or digit length equal to the limit is accepted).
//!
//! Exceeding a limit raises `document.limit.depth`, `document.limit.nodes`
//! or `document.limit.int-digits` as a
//! [`crate::error::DocumentError`] (E-11): depth and node count are
//! properties of the document as a whole and carry path `$`, an over-long
//! integer carries the Document path of that integer.
//!
//! The limits apply to every route into the model: the `*_with` readers of
//! each format ([`crate::oml::read_oml_with`],
//! [`crate::formats::json::read_json_with`],
//! [`crate::formats::yaml::YamlReadOptions::with_limits`],
//! [`crate::formats::toml::read_toml_with`],
//! [`crate::formats::xml::read_xml_with`]) and the Document constructors
//! [`crate::document::Doc::of_with`] and [`crate::document::Doc::from_raw_with`].
//! A [`crate::document::Doc`] remembers the limits it was built under, so a
//! later `add`/`set` on it enforces the same ones.

use crate::error::{DocumentError, OmnistError, ParseError};
use num_bigint::BigInt;

/// The reference default for the maximum nesting depth (spec section 2.4).
pub const DEFAULT_MAX_DEPTH: u32 = 200;
/// The largest maximum nesting depth [`Limits::validate`] accepts.
pub const MAX_DEPTH_CEILING: u32 = 1_000;
/// The reference default for the maximum node count (spec section 2.4).
pub const DEFAULT_MAX_NODES: u32 = 1_000_000;
/// The largest maximum node count [`Limits::validate`] accepts.
pub const MAX_NODES_CEILING: u32 = 10_000_000;
/// The reference default for the maximum integer digits (spec section 2.4).
pub const DEFAULT_MAX_INT_DIGITS: u32 = 4_300;
/// The largest maximum integer digit count [`Limits::validate`] accepts.
pub const MAX_INT_DIGITS_CEILING: u32 = 43_000;

/// The three universal safety limits of spec section 2.4, taken by the
/// `*_with` readers and by [`crate::document::Doc::from_raw_with`].
///
/// ```
/// use omnist::limits::Limits;
/// use omnist::formats::json::read_json_with;
///
/// let limits = Limits::default().with_max_depth(3);
/// assert!(read_json_with(r#"{"a": {"b": {"c": 1}}}"#, &limits).is_ok());
/// let err = read_json_with(r#"{"a": {"b": {"c": {"d": 1}}}}"#, &limits).unwrap_err();
/// assert!(err.to_string().contains("maximum depth (3)"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The maximum nesting depth, counted from the Document root (the root
    /// is depth 0). A leaf at exactly this depth is accepted, one level
    /// deeper is refused with `document.limit.depth`.
    ///
    /// `0` selects [`DEFAULT_MAX_DEPTH`] (200); values above
    /// [`MAX_DEPTH_CEILING`] (1 000) are rejected by [`Limits::validate`].
    pub max_depth: u32,
    /// The maximum number of nodes (containers) one Document may hold. A
    /// document with exactly this many is accepted, one more is refused with
    /// `document.limit.nodes`.
    ///
    /// `0` selects [`DEFAULT_MAX_NODES`] (1 000 000); values above
    /// [`MAX_NODES_CEILING`] (10 000 000) are rejected by [`Limits::validate`].
    pub max_nodes: u32,
    /// The maximum number of decimal digits in an `integer` literal, sign
    /// excluded. A literal of exactly this many digits is accepted, one more
    /// is refused with `document.limit.int-digits`.
    ///
    /// `0` selects [`DEFAULT_MAX_INT_DIGITS`] (4 300); values above
    /// [`MAX_INT_DIGITS_CEILING`] (43 000) are rejected by
    /// [`Limits::validate`].
    pub max_int_digits: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: DEFAULT_MAX_DEPTH,
            max_nodes: DEFAULT_MAX_NODES,
            max_int_digits: DEFAULT_MAX_INT_DIGITS,
        }
    }
}

impl Limits {
    /// These limits with [`Limits::max_depth`] set to `n` (the struct is
    /// `#[non_exhaustive]`, so a downstream crate builds it from
    /// [`Default`] with these).
    #[must_use]
    pub fn with_max_depth(mut self, n: u32) -> Self {
        self.max_depth = n;
        self
    }

    /// These limits with [`Limits::max_nodes`] set to `n`.
    #[must_use]
    pub fn with_max_nodes(mut self, n: u32) -> Self {
        self.max_nodes = n;
        self
    }

    /// These limits with [`Limits::max_int_digits`] set to `n`.
    #[must_use]
    pub fn with_max_int_digits(mut self, n: u32) -> Self {
        self.max_int_digits = n;
        self
    }

    /// The maximum depth these limits select: the configured value, or
    /// [`DEFAULT_MAX_DEPTH`] when it is `0`.
    pub fn effective_max_depth(&self) -> u32 {
        if self.max_depth == 0 {
            DEFAULT_MAX_DEPTH
        } else {
            self.max_depth
        }
    }

    /// The maximum node count these limits select: the configured value, or
    /// [`DEFAULT_MAX_NODES`] when it is `0`.
    pub fn effective_max_nodes(&self) -> u32 {
        if self.max_nodes == 0 {
            DEFAULT_MAX_NODES
        } else {
            self.max_nodes
        }
    }

    /// The maximum integer digit count these limits select: the configured
    /// value, or [`DEFAULT_MAX_INT_DIGITS`] when it is `0`.
    pub fn effective_max_int_digits(&self) -> u32 {
        if self.max_int_digits == 0 {
            DEFAULT_MAX_INT_DIGITS
        } else {
            self.max_int_digits
        }
    }

    /// Checks the limits: each must not exceed its ceiling
    /// ([`MAX_DEPTH_CEILING`], [`MAX_NODES_CEILING`],
    /// [`MAX_INT_DIGITS_CEILING`]). Every `*_with` entry point calls this
    /// first and returns its error unchanged (a [`DocumentError`] with path
    /// `$` and no taxonomy code: it is API misuse, not a document defect).
    pub fn validate(&self) -> Result<(), DocumentError> {
        for (name, value, ceiling, default) in [
            (
                "max_depth",
                self.max_depth,
                MAX_DEPTH_CEILING,
                DEFAULT_MAX_DEPTH,
            ),
            (
                "max_nodes",
                self.max_nodes,
                MAX_NODES_CEILING,
                DEFAULT_MAX_NODES,
            ),
            (
                "max_int_digits",
                self.max_int_digits,
                MAX_INT_DIGITS_CEILING,
                DEFAULT_MAX_INT_DIGITS,
            ),
        ] {
            if value > ceiling {
                return Err(DocumentError::new(
                    "$",
                    format!(
                        "{name} {value} exceeds the ceiling {ceiling} (0 selects the default \
                         {default})"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Validated, widened form the readers carry around.
    pub(crate) fn resolve(&self) -> Result<Resolved, DocumentError> {
        self.validate()?;
        Ok(Resolved::from_limits(self))
    }
}

/// [`Limits`] after validation and default substitution, in the `usize`
/// the readers compare against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) max_depth: usize,
    pub(crate) max_nodes: usize,
    pub(crate) max_int_digits: usize,
}

impl Resolved {
    /// The defaults, as a `const` so constructors need no `Result`.
    pub(crate) const DEFAULT: Resolved = Resolved {
        max_depth: DEFAULT_MAX_DEPTH as usize,
        max_nodes: DEFAULT_MAX_NODES as usize,
        max_int_digits: DEFAULT_MAX_INT_DIGITS as usize,
    };

    fn from_limits(l: &Limits) -> Resolved {
        Resolved {
            max_depth: l.effective_max_depth() as usize,
            max_nodes: l.effective_max_nodes() as usize,
            max_int_digits: l.effective_max_int_digits() as usize,
        }
    }

    /// The `document.limit.depth` error. Depth is a property of the whole
    /// document, so its path is `$` (E-11, and the `document-model/limits`
    /// vectors).
    pub(crate) fn depth_error(&self) -> DocumentError {
        DocumentError::with_code(
            "$",
            "document.limit.depth",
            format!("nesting exceeds the maximum depth ({})", self.max_depth),
        )
    }

    /// The `document.limit.nodes` error, at `$` for the same reason.
    pub(crate) fn nodes_error(&self) -> DocumentError {
        DocumentError::with_code(
            "$",
            "document.limit.nodes",
            format!(
                "document exceeds the maximum node count ({})",
                self.max_nodes
            ),
        )
    }

    /// Lifts a reader's own limit error, raised with a text position because
    /// the scanner has no Document path (`document.limit.depth` /
    /// `document.limit.nodes`), to the `$` Document-path form E-11 requires;
    /// any other parse error is returned as it is.
    pub(crate) fn lift_parse_error(&self, e: ParseError) -> OmnistError {
        match e.code.as_str() {
            "document.limit.depth" => self.depth_error().into(),
            "document.limit.nodes" => self.nodes_error().into(),
            _ => e.into(),
        }
    }

    /// A Document constructor reports depth and node-count violations at the
    /// path where it noticed them; a reader reports them at `$` (they are
    /// properties of the whole input, E-11).
    pub(crate) fn whole_document(&self, mut e: DocumentError) -> DocumentError {
        if matches!(
            e.code.as_deref(),
            Some("document.limit.depth" | "document.limit.nodes")
        ) {
            e.path = "$".to_string();
        }
        e
    }

    /// The `document.limit.int-digits` error for the integer at `path`.
    pub(crate) fn int_digits_error(&self, path: &str) -> DocumentError {
        DocumentError::with_code(
            path,
            "document.limit.int-digits",
            crate::formats::int_cap::over_cap_message("", self.max_int_digits),
        )
    }
}

/// Whether `i` has more than `max` decimal digits (sign excluded). Cheap for
/// every ordinary value: the bit length bounds the digit count from above,
/// and only an integer that could exceed `max` is rendered.
pub(crate) fn int_exceeds(i: &BigInt, max: usize) -> bool {
    // digits(n) <= floor(bits * log10(2)) + 1, and 30103/100000 > log10(2).
    let upper = usize::try_from(i.bits().saturating_mul(30_103) / 100_000).unwrap_or(usize::MAX);
    if upper.saturating_add(1) <= max {
        return false;
    }
    i.magnitude().to_str_radix(10).len() > max
}

/// Stands in, inside a reader's intermediate tree, for an integer literal
/// whose digit run is over the cap, so the reader never pays the
/// superlinear digit-string-to-integer conversion for it.
///
/// The placeholder is `10^max`, which has `max + 1` digits, so it is itself
/// over the cap: the Document constructor that finally receives the tree
/// ([`crate::document::Doc::of_with`] / [`crate::document::Doc::from_raw_with`])
/// refuses it with `document.limit.int-digits` *at the right Document path*,
/// which a reader still scanning text cannot compute (the path depends on
/// how many times a label repeats in its node, known only once the node is
/// complete). A genuine literal can never equal it: it would have had to
/// pass the cap, and `10^max` does not.
pub(crate) struct IntGuard {
    max: usize,
    placeholder: Option<BigInt>,
}

impl IntGuard {
    pub(crate) fn new(max: usize) -> IntGuard {
        IntGuard {
            max,
            placeholder: None,
        }
    }

    /// The maximum digit count this guard enforces.
    pub(crate) fn max(&self) -> usize {
        self.max
    }

    /// The stand-in for an over-cap literal; remembers that one was seen.
    pub(crate) fn placeholder(&mut self) -> BigInt {
        let max = self.max;
        self.placeholder
            .get_or_insert_with(|| {
                BigInt::from(10u8).pow(u32::try_from(max).expect("max is a validated u32"))
            })
            .clone()
    }

    /// Whether any over-cap literal was replaced by a placeholder.
    pub(crate) fn seen(&self) -> bool {
        self.placeholder.is_some()
    }

    /// For a reader whose intermediate tree can silently drop a value (a
    /// duplicate key keeping its last value) *after* an over-cap literal was
    /// replaced: if the finished Document built without complaint but a
    /// placeholder was seen, the literal was dropped from the tree, and the
    /// limit still applies to the input (D-13, MUST NOT silently accept). The
    /// path of a dropped value is no longer known, so it is `$`.
    pub(crate) fn finish(&self, resolved: &Resolved) -> Result<(), DocumentError> {
        if self.seen() {
            return Err(resolved.int_digits_error("$"));
        }
        Ok(())
    }
}

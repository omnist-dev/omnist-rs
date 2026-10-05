//! Runtime-configurable safety limits (omnist-spec section 2.4, D-9 to D-13).
//!
//! The spec requires every implementation to enforce a *finite* limit on
//! nesting depth, node count and integer digits, lets it pick the numbers
//! (D-10), and requires it to document them (D-11). It also says an
//! implementation SHOULD enforce a finite maximum input size in bytes (D-23),
//! which this port does. This module is where this port documents them and
//! where a caller changes them.
//!
//! | Limit | Default | Ceiling | Field |
//! |---|---|---|---|
//! | Nesting depth | [`DEFAULT_MAX_DEPTH`] (200) | [`MAX_DEPTH_CEILING`] (250) | [`Limits::max_depth`] |
//! | Node count (containers) | [`DEFAULT_MAX_NODES`] (1 000 000) | [`MAX_NODES_CEILING`] (10 000 000) | [`Limits::max_nodes`] |
//! | Integer digits | [`DEFAULT_MAX_INT_DIGITS`] (4 300) | [`MAX_INT_DIGITS_CEILING`] (43 000) | [`Limits::max_int_digits`] |
//! | Input size, bytes | [`DEFAULT_MAX_INPUT_BYTES`] (64 MiB) | [`MAX_INPUT_BYTES_CEILING`] (1 GiB) | [`Limits::max_input_bytes`] |
//!
//! The default of each of the first three is the spec's reference default;
//! the spec names no default for the input size (D-24), so 64 MiB is this
//! port's own choice: it is the cap that bounds parse cost, and the section
//! on the input size below says how it was chosen. The ceilings are
//! this port's own (the spec recommends none for these three): a limit is a
//! denial-of-service bound, not a tuning knob without an upper end, and the
//! depth ceiling in particular keeps the recursive readers inside a thread's
//! stack: measured with a 2 MB thread stack, the JSON, OML, YAML and XML
//! readers and `Doc` handle 300 levels in a debug build and 1 000 in a
//! release build, but overflow at 400 in debug, so 250 is safe in both
//! profiles. A caller on a smaller stack than 2 MB must lower the depth.
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
//! Exceeding a limit raises `document.limit.depth`, `document.limit.nodes`,
//! `document.limit.int-digits` or `document.limit.input-size` as a
//! [`crate::error::DocumentError`] (E-11): depth and node count are
//! properties of the document as a whole and carry path `$`, an over-long
//! integer carries the Document path of that integer, and an input over the
//! size limit carries `$` too (it is a property of the input as a whole).
//!
//! ## The input size (D-23, D-24)
//!
//! The size is **bytes of the input as received, not characters**: `\u{e9}`
//! counts as two, and a leading byte-order mark counts as three, because the
//! length is taken before the mark is stripped (D-15) and before any
//! decoding or parsing. The check is the first thing every reader does, so
//! `document.limit.input-size` is reported ahead of `parse.invalid-encoding`,
//! of a doubled-BOM error and of every other diagnostic. An input of exactly
//! the maximum is accepted. It applies to all five formats, to a YAML input
//! with no alias included, and to [`crate::document::Doc::from_format`] for
//! any registered format; the CLI also stops reading a file or stdin at the
//! maximum plus one byte instead of buffering the rest (`--max-input-bytes`).
//!
//! The default, 64 MiB, is chosen to bound the cost of the slowest codec
//! library, not to promise that an input that large parses quickly: the
//! spec says a cap bounds parse cost without making any parse fast, and
//! asks an implementation to measure its slowest codec at the size it
//! chooses (D-24). `docs/limitations.md` records the measurement and the
//! behaviour change: before 0.9.0-alpha a reader took any size.
//! Only Documents read from text or bytes are bounded: an OSD schema text is
//! not (D-25 asks for a schema-size bound but gives it no code), and a
//! Document built from native values has no input bytes.
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

/// The default maximum input size in bytes (spec D-23, D-24): 64 MiB. The
/// spec names no default; this port's is chosen in the module documentation.
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
/// The largest maximum input size [`Limits::validate`] accepts: 1 GiB.
pub const MAX_INPUT_BYTES_CEILING: u64 = 1 << 30;

/// The reference default for the maximum nesting depth (spec section 2.4).
pub const DEFAULT_MAX_DEPTH: u32 = 200;
/// The largest maximum nesting depth [`Limits::validate`] accepts.
pub const MAX_DEPTH_CEILING: u32 = 250;
/// The reference default for the maximum node count (spec section 2.4).
pub const DEFAULT_MAX_NODES: u32 = 1_000_000;
/// The largest maximum node count [`Limits::validate`] accepts.
pub const MAX_NODES_CEILING: u32 = 10_000_000;
/// The reference default for the maximum integer digits (spec section 2.4).
pub const DEFAULT_MAX_INT_DIGITS: u32 = 4_300;
/// The largest maximum integer digit count [`Limits::validate`] accepts.
pub const MAX_INT_DIGITS_CEILING: u32 = 43_000;

/// The safety limits of spec section 2.4 (nesting depth, node count, integer
/// digits) and the maximum input size of D-23, taken by the `*_with` readers
/// and, for the first three, by [`crate::document::Doc::from_raw_with`].
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
    /// [`MAX_DEPTH_CEILING`] (250) are rejected by [`Limits::validate`].
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
    /// The maximum input size in bytes (D-23), counted on the input as
    /// received: before a leading BOM is stripped and before decoding. An
    /// input of exactly this many bytes is accepted, one more is refused
    /// with `document.limit.input-size` at `$`, ahead of every other
    /// diagnostic. Only the readers use it; a Document built from native
    /// values or a [`crate::document::RawNode`] has no input bytes.
    ///
    /// `0` selects [`DEFAULT_MAX_INPUT_BYTES`] (64 MiB); values above
    /// [`MAX_INPUT_BYTES_CEILING`] (1 GiB) are rejected by
    /// [`Limits::validate`].
    pub max_input_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: DEFAULT_MAX_DEPTH,
            max_nodes: DEFAULT_MAX_NODES,
            max_int_digits: DEFAULT_MAX_INT_DIGITS,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
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

    /// These limits with [`Limits::max_input_bytes`] set to `n`.
    #[must_use]
    pub fn with_max_input_bytes(mut self, n: u64) -> Self {
        self.max_input_bytes = n;
        self
    }

    /// The maximum input size these limits select: the configured value, or
    /// [`DEFAULT_MAX_INPUT_BYTES`] when it is `0`.
    pub fn effective_max_input_bytes(&self) -> u64 {
        if self.max_input_bytes == 0 {
            DEFAULT_MAX_INPUT_BYTES
        } else {
            self.max_input_bytes
        }
    }

    /// D-23 for a caller that holds the input as bytes (the CLI): validates
    /// the limits, then refuses an input of `len` bytes that is over the
    /// maximum with `document.limit.input-size` at `$`, and accepts one of
    /// exactly the maximum. `hint` says how to raise it, in the caller's own
    /// terms. Call it before decoding the bytes.
    pub fn check_input_len(&self, len: u64, hint: &str) -> Result<(), DocumentError> {
        self.validate()?;
        let max = self.effective_max_input_bytes();
        if len > max {
            return Err(input_size_error(max, hint));
        }
        Ok(())
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
    /// [`MAX_INT_DIGITS_CEILING`], [`MAX_INPUT_BYTES_CEILING`]). Every
    /// `*_with` entry point calls this
    /// first and returns its error unchanged (a [`DocumentError`] with path
    /// `$` and no taxonomy code: it is API misuse, not a document defect).
    pub fn validate(&self) -> Result<(), DocumentError> {
        for (name, value, ceiling, default) in [
            (
                "max_depth",
                u64::from(self.max_depth),
                u64::from(MAX_DEPTH_CEILING),
                u64::from(DEFAULT_MAX_DEPTH),
            ),
            (
                "max_nodes",
                u64::from(self.max_nodes),
                u64::from(MAX_NODES_CEILING),
                u64::from(DEFAULT_MAX_NODES),
            ),
            (
                "max_int_digits",
                u64::from(self.max_int_digits),
                u64::from(MAX_INT_DIGITS_CEILING),
                u64::from(DEFAULT_MAX_INT_DIGITS),
            ),
            (
                "max_input_bytes",
                self.max_input_bytes,
                MAX_INPUT_BYTES_CEILING,
                DEFAULT_MAX_INPUT_BYTES,
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
        Ok(self.resolve_validated())
    }

    /// [`Limits::resolve`] for limits already checked by [`Limits::validate`]
    /// (the YAML options validate everything up front).
    pub(crate) fn resolve_validated(&self) -> Resolved {
        Resolved::from_limits(self)
    }
}

/// [`Limits`] after validation and default substitution, in the `usize`
/// the readers compare against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) max_depth: usize,
    pub(crate) max_nodes: usize,
    pub(crate) max_int_digits: usize,
    pub(crate) max_input_bytes: u64,
}

/// The D-23 refusal: `document.limit.input-size` at `$` (the size is a
/// property of the input as a whole, E-4a). `hint` says how to raise the
/// maximum, in the caller's own terms (a library option, a CLI flag).
pub fn input_size_error(max_input_bytes: u64, hint: &str) -> DocumentError {
    DocumentError::with_code(
        "$",
        "document.limit.input-size",
        format!("input exceeds the maximum input size ({max_input_bytes} bytes); {hint}"),
    )
}

/// How to raise the maximum from the library, for the refusal's message.
const RAISE_HINT: &str = "raise Limits::max_input_bytes to read it";

impl Resolved {
    /// The defaults, as a `const` so constructors need no `Result`.
    pub(crate) const DEFAULT: Resolved = Resolved {
        max_depth: DEFAULT_MAX_DEPTH as usize,
        max_nodes: DEFAULT_MAX_NODES as usize,
        max_int_digits: DEFAULT_MAX_INT_DIGITS as usize,
        max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
    };

    fn from_limits(l: &Limits) -> Resolved {
        Resolved {
            max_depth: l.effective_max_depth() as usize,
            max_nodes: l.effective_max_nodes() as usize,
            max_int_digits: l.effective_max_int_digits() as usize,
            max_input_bytes: l.effective_max_input_bytes(),
        }
    }

    /// D-23: refuses an input of more than the maximum number of bytes and
    /// accepts one of exactly that many. `len` is the length of the input as
    /// received, taken before any BOM strip, decoding or parsing; every
    /// reader calls this first.
    pub(crate) fn check_input_size(&self, len: usize) -> Result<(), DocumentError> {
        if len as u64 > self.max_input_bytes {
            return Err(input_size_error(self.max_input_bytes, RAISE_HINT));
        }
        Ok(())
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

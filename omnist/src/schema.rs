//! The Schema model -- Record/Scalar/Ref, per `docs/design/model.md` (issue
//! #6). Ported from `~/dev/omnist/omnist/schema.py`.
//!
//! * **Record** -- a closed set of fields, each `(label, type, cardinality)`.
//!   Cardinality is the *unordered* number of times a label may appear.
//! * **Scalar** -- exactly one of seven predefined value types (`string`,
//!   `integer`, `number`, `boolean`, `date`, `time`, `datetime`), optionally
//!   nullable. There is no user-declared value-domain composition (no
//!   union/enum/literal) -- see `docs/design/model.md` §2/§5 for why: a
//!   composable value-domain would make schema-directed deserialization
//!   ambiguous (a value could satisfy more than one candidate with no
//!   principled way to choose).
//! * **Ref** -- a pointer into the schema's named environment (records
//!   only); enables reuse and recursion.
//! * **Any** (`FieldType::Any`) -- accepts every legal document value
//!   unchecked. Ported from Python's `AnyType`/`ANY` singleton, which has
//!   been fully implemented and shipped there since v0.5.0 -- not a
//!   speculative or deferred feature (the *separate*, still-unresolved
//!   question is whether `any` should be a *permanent* part of the spec
//!   long-term; that governance question is untouched by this port simply
//!   catching up to Python's existing behavior, see omnist-rs issue #29).
//!
//! A field's type is a `Ref`, a `Scalar`, or `Any`. There are no inline
//! records and no separate array type -- "array" is a field with
//! cardinality `max > 1`.
//! Validation ignores order (per `docs/design/model.md` §7).
//!
//! ## Temporal shape-check
//!
//! `is_iso_date`, `is_iso_time`, and `is_iso_datetime` are the single
//! source of truth for "is this string shaped like (and a semantically
//! valid) date/time/datetime," `pub(crate)` so a future `materialize`/
//! `infer` module can reuse the exact same check instead of writing a
//! second, independently-maintained copy (per the porting playbook's
//! pitfall list). Each check is stricter than a bare shape regex: the regex
//! only rules out the wrong *spelling* (Python's `datetime.fromisoformat`
//! is deliberately wider -- it also accepts ISO-8601 basic format
//! (`20240101`), week dates, and other spellings this crate's docs never
//! promise) -- the calendar/clock fields are additionally range-checked
//! (e.g. a syntactically-shaped `"2024-02-30"` is still rejected).

use indexmap::IndexMap;
use regex::Regex;
use std::sync::LazyLock;

use crate::document::{self, Scalar as DocScalar};
use crate::error::SchemaError;

// ---------------------------------------------------------------------------
// Temporal shape-check (shared by `validate` today; reusable by a future
// materialize/infer module without duplication).
// ---------------------------------------------------------------------------

/// Hyphenated ISO date shape: `YYYY-MM-DD`.
pub(crate) static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<y>\d{4})-(?P<mo>\d{2})-(?P<da>\d{2})$").unwrap());

/// Colon-separated ISO time shape: `HH:MM[:SS[.ffffff]][+-HH:MM]`.
pub(crate) static TIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<h>\d{2}):(?P<m>\d{2})(:(?P<s>\d{2})(\.(?P<f>\d{1,6}))?)?(?P<off>[+\-]\d{2}:\d{2})?$",
    )
    .unwrap()
});

/// `T`-joined ISO datetime shape: date, literal `T`, then the time shape.
pub(crate) static DATETIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<y>\d{4})-(?P<mo>\d{2})-(?P<da>\d{2})T(?P<h>\d{2}):(?P<m>\d{2})(:(?P<s>\d{2})(\.(?P<f>\d{1,6}))?)?(?P<off>[+\-]\d{2}:\d{2})?$",
    )
    .unwrap()
});

pub(crate) fn is_leap_year(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

pub(crate) fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Whether `(y, m, d)` is a real calendar date (`datetime.date`'s domain:
/// year 1..=9999, per Python's `MINYEAR`/`MAXYEAR`).
pub(crate) fn valid_ymd(y: u32, m: u32, d: u32) -> bool {
    (1..=9999).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)
}

/// Whether `(h, m, s)` is a real clock time (`00:00:00..=23:59:59`).
pub(crate) fn valid_hms(h: u32, m: u32, s: u32) -> bool {
    h <= 23 && m <= 59 && s <= 59
}

/// Whether an optional `[+-]HH:MM` offset capture is absent, or present and
/// in range (`00:00..=23:59` on both sides, mirroring a plain time value).
fn valid_offset(off: Option<regex::Match<'_>>) -> bool {
    match off {
        None => true,
        Some(m) => {
            let text = m.as_str();
            // text is "[+-]HH:MM" per TIME_RE/DATETIME_RE's own `off` group.
            let oh: u32 = text[1..3].parse().unwrap_or(u32::MAX);
            let om: u32 = text[4..6].parse().unwrap_or(u32::MAX);
            oh <= 23 && om <= 59
        }
    }
}

/// Pulls a *mandatory* named group's digits out as a `u32`. Only used for
/// groups that `DATE_RE`/`TIME_RE`/`DATETIME_RE` require unconditionally
/// (`y`/`mo`/`da`/`h`/`m` are never inside a `(...)?` group) -- so once
/// `.captures()` has matched at all, these are guaranteed present and
/// numeric (`\d{2}`/`\d{4}` can't fail to parse as `u32`), and there is no
/// reachable failure branch to test here (see the module's coverage note
/// in the PR description for how this was confirmed empirically rather
/// than assumed).
fn mandatory_u32(caps: &regex::Captures<'_>, name: &str) -> u32 {
    caps.name(name)
        .expect("group is mandatory in the pattern")
        .as_str()
        .parse()
        .expect("group is all-digits per the pattern")
}

/// Is `s` shaped like, and a semantically valid, hyphenated ISO date
/// (`YYYY-MM-DD`)? Narrower than `datetime.fromisoformat` by design -- see
/// the module doc comment.
pub(crate) fn is_iso_date(s: &str) -> bool {
    let Some(caps) = DATE_RE.captures(s) else {
        return false;
    };
    valid_ymd(
        mandatory_u32(&caps, "y"),
        mandatory_u32(&caps, "mo"),
        mandatory_u32(&caps, "da"),
    )
}

/// Is `s` shaped like, and a semantically valid, ISO time
/// (`HH:MM[:SS[.ffffff]][+-HH:MM]`)?
pub(crate) fn is_iso_time(s: &str) -> bool {
    let Some(caps) = TIME_RE.captures(s) else {
        return false;
    };
    is_valid_time_captures(&caps)
}

fn is_valid_time_captures(caps: &regex::Captures<'_>) -> bool {
    let h = mandatory_u32(caps, "h");
    let m = mandatory_u32(caps, "m");
    // Seconds default to 0 when the `:SS` group is absent (shape allows
    // `HH:MM` alone); when present it's mandatory digits, same guarantee
    // as `mandatory_u32`'s other callers.
    let s = caps.name("s").map_or(0, |c| c.as_str().parse().unwrap());
    valid_hms(h, m, s) && valid_offset(caps.name("off"))
}

/// Is `s` shaped like, and a semantically valid, `T`-joined ISO datetime
/// (`YYYY-MM-DDTHH:MM[:SS[.ffffff]][+-HH:MM]`)? Deliberately **excludes** a
/// bare date string -- `datetime.fromisoformat` is lenient there (a
/// date-only string parses fine, defaulting the missing time to midnight),
/// which would silently treat "no time given" as "the time is exactly
/// midnight," not the same value. Callers that need "datetime, and not
/// also a bare date" should additionally check `!is_iso_date(s)`, mirroring
/// the Python reference's `matches_kind("datetime", …)`.
pub(crate) fn is_iso_datetime(s: &str) -> bool {
    let Some(caps) = DATETIME_RE.captures(s) else {
        return false;
    };
    let ok_date = valid_ymd(
        mandatory_u32(&caps, "y"),
        mandatory_u32(&caps, "mo"),
        mandatory_u32(&caps, "da"),
    );
    ok_date && is_valid_time_captures(&caps)
}

/// Canonicalizes an already-`is_iso_time`-validated time string to its
/// normalized form: a missing `:SS` defaults to `:00`, and a present
/// fractional-seconds part is zero-padded to 6 digits (matching Python's
/// real `datetime.time`/`datetime.datetime` `.isoformat()` output, which is
/// what the OML temporal grammar's canonical form is defined against). Any
/// UTC offset is carried through unchanged -- it's already in the
/// grammar's canonical `+HH:MM`/`-HH:MM` shape.
///
/// # Panics
/// Panics if `s` doesn't match `TIME_RE` -- callers must validate with
/// `is_iso_time` first.
pub(crate) fn canonicalize_iso_time(s: &str) -> String {
    let caps = TIME_RE
        .captures(s)
        .expect("caller must validate with is_iso_time first");
    canonicalize_time_captures(&caps)
}

/// Canonicalizes an already-`is_iso_datetime`-validated datetime string the
/// same way as [`canonicalize_iso_time`], with the date portion (already
/// fully-specified by the grammar -- `YYYY-MM-DD` has no optional parts)
/// carried through unchanged.
///
/// # Panics
/// Panics if `s` doesn't match `DATETIME_RE` -- callers must validate with
/// `is_iso_datetime` first.
pub(crate) fn canonicalize_iso_datetime(s: &str) -> String {
    let caps = DATETIME_RE
        .captures(s)
        .expect("caller must validate with is_iso_datetime first");
    let y = caps.name("y").expect("mandatory group").as_str();
    let mo = caps.name("mo").expect("mandatory group").as_str();
    let da = caps.name("da").expect("mandatory group").as_str();
    format!("{y}-{mo}-{da}T{}", canonicalize_time_captures(&caps))
}

/// Shared canonicalization of the `h`/`m`/`s`/`f`/`off` capture groups that
/// `TIME_RE` and `DATETIME_RE` both define identically for the time
/// portion.
fn canonicalize_time_captures(caps: &regex::Captures<'_>) -> String {
    let h = caps.name("h").expect("mandatory group").as_str();
    let m = caps.name("m").expect("mandatory group").as_str();
    let s = caps.name("s").map_or("00", |c| c.as_str());
    let mut out = format!("{h}:{m}:{s}");
    if let Some(f) = caps.name("f") {
        out.push('.');
        let digits = f.as_str();
        out.push_str(digits);
        for _ in digits.len()..6 {
            out.push('0');
        }
    }
    if let Some(off) = caps.name("off") {
        out.push_str(off.as_str());
    }
    out
}

// ---------------------------------------------------------------------------
// Scalar
// ---------------------------------------------------------------------------

/// One of the seven predefined value kinds a [`Scalar`] can hold (spec §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarKind {
    /// The `string` scalar kind (UTF-8 character string, spec §2.2).
    String,
    /// The `integer` scalar kind (arbitrary-precision integer, spec §2.2).
    Integer,
    /// The `number` scalar kind (IEEE 754 double precision float or integer, spec §2.2).
    Number,
    /// The `boolean` scalar kind (`true` or `false`, spec §2.2).
    Boolean,
    /// The `date` scalar kind (ISO 8601 calendar date `YYYY-MM-DD`, spec §2.2).
    Date,
    /// The `time` scalar kind (ISO 8601 clock time `hh:mm:ss`, spec §2.2).
    Time,
    /// The `datetime` scalar kind (ISO 8601 combined date-time, spec §2.2).
    Datetime,
}

impl ScalarKind {
    /// All seven kinds, in the order the Python reference declares them.
    pub const ALL: [ScalarKind; 7] = [
        ScalarKind::String,
        ScalarKind::Integer,
        ScalarKind::Number,
        ScalarKind::Boolean,
        ScalarKind::Date,
        ScalarKind::Time,
        ScalarKind::Datetime,
    ];

    /// Return the canonical OSD name of this scalar kind (`"string"`, `"integer"`, etc., spec §2.2).
    pub fn as_str(&self) -> &'static str {
        match self {
            ScalarKind::String => "string",
            ScalarKind::Integer => "integer",
            ScalarKind::Number => "number",
            ScalarKind::Boolean => "boolean",
            ScalarKind::Date => "date",
            ScalarKind::Time => "time",
            ScalarKind::Datetime => "datetime",
        }
    }

    /// Parse a scalar kind name, as it would appear in schema text (`"date"`
    /// etc). Mirrors `Scalar.__init__`'s `SCALAR_NAMES` check.
    pub fn parse(name: &str) -> Result<ScalarKind, SchemaError> {
        ScalarKind::ALL
            .into_iter()
            .find(|k| k.as_str() == name)
            .ok_or_else(|| {
                let names: Vec<&str> = ScalarKind::ALL.iter().map(|k| k.as_str()).collect();
                SchemaError::new(
                    name,
                    "schema.unknown-type",
                    format!("unknown scalar {name:?}; expected one of {names:?}"),
                )
            })
    }
}

/// One of the seven predefined value types, optionally nullable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scalar {
    kind: ScalarKind,
    nullable: bool,
}

impl Scalar {
    /// Construct a new `Scalar` with the given [`ScalarKind`] and nullability flag (spec §2.2, §3).
    pub const fn new(kind: ScalarKind, nullable: bool) -> Self {
        Scalar { kind, nullable }
    }

    /// Construct from a scalar kind's name (as it appears in schema text),
    /// mirroring the Python constructor's runtime name check.
    pub fn named(name: &str, nullable: bool) -> Result<Self, SchemaError> {
        Ok(Scalar::new(ScalarKind::parse(name)?, nullable))
    }

    /// The value kind of this scalar.
    pub fn kind(&self) -> ScalarKind {
        self.kind
    }

    /// Whether this scalar accepts `null` values (the `?` suffix in OSD syntax).
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }
}

impl std::fmt::Display for Scalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{}",
            self.kind.as_str(),
            if self.nullable { "?" } else { "" }
        )
    }
}

/// Non-nullable `string` scalar constant (spec §2.2, §3).
pub const STRING: Scalar = Scalar::new(ScalarKind::String, false);
/// Non-nullable `integer` scalar constant (spec §2.2, §3).
pub const INTEGER: Scalar = Scalar::new(ScalarKind::Integer, false);
/// Non-nullable `number` scalar constant (spec §2.2, §3).
pub const NUMBER: Scalar = Scalar::new(ScalarKind::Number, false);
/// Non-nullable `boolean` scalar constant (spec §2.2, §3).
pub const BOOLEAN: Scalar = Scalar::new(ScalarKind::Boolean, false);
/// Non-nullable `date` scalar constant (spec §2.2, §3).
pub const DATE: Scalar = Scalar::new(ScalarKind::Date, false);
/// Non-nullable `time` scalar constant (spec §2.2, §3).
pub const TIME: Scalar = Scalar::new(ScalarKind::Time, false);
/// Non-nullable `datetime` scalar constant (spec §2.2, §3).
pub const DATETIME: Scalar = Scalar::new(ScalarKind::Datetime, false);

/// A copy of `scalar` that also accepts `null` (the `?` form).
pub fn nullable(scalar: Scalar) -> Scalar {
    Scalar::new(scalar.kind, true)
}

// ---------------------------------------------------------------------------
// Ref
// ---------------------------------------------------------------------------

/// A reference to a named record in a [`Schema`]'s environment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ref {
    /// The name of the target record in the schema environment.
    pub name: String,
}

impl Ref {
    /// Construct a new `Ref` pointing to a named record.
    pub fn new(name: impl Into<String>) -> Self {
        Ref { name: name.into() }
    }
}

impl std::fmt::Display for Ref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ref({})", self.name)
    }
}

/// A field's type: a `Ref` to a named record, a `Scalar`, or `Any` (accepts
/// every legal document value -- ported from Python's `AnyType`/`ANY`
/// singleton, shipped there since v0.5.0). `Any` is not a `Scalar` (it has
/// no kind and no nullable flag -- null is already included) and not a
/// `Ref` (it names nothing), so it gets its own unit variant rather than
/// being folded into either.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldType {
    /// A scalar type slot.
    Scalar(Scalar),
    /// A reference to a named record in the schema environment.
    Ref(Ref),
    /// An `any` type slot accepting any legal document value without validation.
    Any,
}

impl From<Scalar> for FieldType {
    fn from(s: Scalar) -> Self {
        FieldType::Scalar(s)
    }
}

impl From<Ref> for FieldType {
    fn from(r: Ref) -> Self {
        FieldType::Ref(r)
    }
}

// ---------------------------------------------------------------------------
// Field / Record
// ---------------------------------------------------------------------------

/// One named, cardinality-bound field slot of a record: `label` of `type`,
/// occurring `[min, max]` times (`max = None` is unbounded).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Field {
    /// The label of the field.
    pub label: String,
    /// The field's type.
    pub ty: FieldType,
    /// Minimum occurrence count required.
    pub min: usize,
    /// Maximum occurrence count allowed (`None` indicates unbounded cardinality).
    pub max: Option<usize>,
}

impl Field {
    /// Construct a new `Field` with label, type, and cardinality bounds `[min, max]` (spec §3, §3.3).
    pub fn new(
        label: impl Into<String>,
        ty: impl Into<FieldType>,
        min: usize,
        max: Option<usize>,
    ) -> Result<Self, SchemaError> {
        let label = label.into();
        if let Some(max) = max
            && max < min
        {
            return Err(SchemaError::new(
                label.clone(),
                "schema.invalid-cardinality",
                format!("field {label:?} has an invalid cardinality [{min},{max}]"),
            ));
        }
        Ok(Field {
            label,
            ty: ty.into(),
            min,
            max,
        })
    }

    /// A required, exactly-once field (`[1,1]`) -- the common case.
    pub fn required(
        label: impl Into<String>,
        ty: impl Into<FieldType>,
    ) -> Result<Self, SchemaError> {
        Field::new(label, ty, 1, Some(1))
    }

    /// Human-readable description of this field's cardinality bounds.
    pub fn cardinality_str(&self) -> String {
        match (self.min, self.max) {
            (1, Some(1)) => "exactly 1".to_string(),
            (0, Some(1)) => "0 or 1".to_string(),
            (min, None) => format!("at least {min}"),
            (min, Some(max)) => format!("between {min} and {max}"),
        }
    }
}

/// A closed set of named fields (constrained by its child labels).
///
/// `fields` preserves declaration order (needed for canonical OSD
/// rendering and `prune`'s declaration-order environment reconstruction --
/// see `ops/mod.rs`'s module docs), but equality below deliberately treats
/// it as an unordered set: omnist-spec's `docs/03-schema-model.md` Sec3.1
/// states fields are an unordered set at the model layer, so two records
/// declaring the same fields in a different order must compare equal.
/// Confirmed empirically by this crate's own conformance self-test
/// (`tools/conformance`, fixture
/// `_referee-self-test/01-schema-exact-equal-different-field-order`) before
/// this manual `PartialEq` replaced the field-order-sensitive derive.
#[derive(Debug, Clone)]
pub struct Record {
    fields: Vec<Field>,
    by_label: IndexMap<String, usize>,
}

impl PartialEq for Record {
    fn eq(&self, other: &Self) -> bool {
        if self.fields.len() != other.fields.len() {
            return false;
        }
        // No duplicate labels within a record (enforced by `Record::new`),
        // so sorting by label gives each side a canonical, comparable
        // order regardless of declaration order.
        let mut a: Vec<&Field> = self.fields.iter().collect();
        let mut b: Vec<&Field> = other.fields.iter().collect();
        a.sort_by(|x, y| x.label.cmp(&y.label));
        b.sort_by(|x, y| x.label.cmp(&y.label));
        a == b
    }
}

impl Eq for Record {}

impl Record {
    /// Rejects a duplicate field label, matching Python's `Record.__init__`.
    pub fn new(fields: Vec<Field>) -> Result<Self, SchemaError> {
        let mut by_label = IndexMap::with_capacity(fields.len());
        for (i, f) in fields.iter().enumerate() {
            if by_label.insert(f.label.clone(), i).is_some() {
                return Err(SchemaError::new(
                    &f.label,
                    "schema.duplicate-field",
                    format!("duplicate field label {:?} in a record", f.label),
                ));
            }
        }
        Ok(Record { fields, by_label })
    }

    /// The fields of this record in declaration order.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Look up a field by its label.
    pub fn field(&self, label: &str) -> Option<&Field> {
        self.by_label.get(label).map(|&i| &self.fields[i])
    }
}

// ---------------------------------------------------------------------------
// Validation result
// ---------------------------------------------------------------------------

/// A stable machine-readable validation failure code, mirroring the Python
/// reference's `Error.code` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// A field was found in the document that is not defined on the record.
    UnexpectedField,
    /// The number of field occurrences fell outside `[min, max]`.
    Cardinality,
    /// A value did not match the expected type.
    TypeMismatch,
    /// A `null` value was encountered for a non-nullable field.
    NullNotAllowed,
    /// A record was encountered where a scalar was expected, or vice-versa.
    ShapeMismatch,
}

impl ErrorCode {
    /// Stable error code string matching `omnist-spec` §5.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::UnexpectedField => "unexpected-field",
            ErrorCode::Cardinality => "cardinality",
            ErrorCode::TypeMismatch => "type-mismatch",
            ErrorCode::NullNotAllowed => "null-not-allowed",
            ErrorCode::ShapeMismatch => "shape-mismatch",
        }
    }
}

/// One validation failure: where, what, and a stable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The path where the validation error occurred.
    pub path: String,
    /// Human-readable error description.
    pub message: String,
    /// Stable machine-readable error code.
    pub code: ErrorCode,
}

/// The outcome of [`Schema::validate`]: empty on success, one entry per
/// problem found (validation collects every error, not just the first).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationResult {
    errors: Vec<ValidationError>,
}

impl ValidationResult {
    /// Create an empty validation result.
    pub fn new() -> Self {
        ValidationResult::default()
    }

    /// Returns `true` if there are no validation errors.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Slice of all validation errors collected during validation.
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// `pub(crate)` (rather than private) so `materialize` can share this
    /// exact multi-error-collection mechanism instead of duplicating it --
    /// per issue #14, materialize's shape-check pass reuses the same
    /// `ValidationResult`/`ValidationError`/`ErrorCode` types `validate`
    /// already built.
    pub(crate) fn add(
        &mut self,
        path: impl Into<String>,
        message: impl Into<String>,
        code: ErrorCode,
    ) {
        self.errors.push(ValidationError {
            path: path.into(),
            message: message.into(),
            code,
        });
    }
}

impl std::fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ok() {
            return write!(f, "valid");
        }
        writeln!(f, "invalid:")?;
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "  at {}: {}", e.path, e.message)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Value matching
// ---------------------------------------------------------------------------

/// Does `value` match scalar kind `kind`? Mirrors Python's `matches_kind` --
/// validation only *checks*, it never converts (see `docs/design/model.md`
/// §10).
///
/// **Verified against Python's real `schema.py::matches_kind` before
/// implementing** (issue #105's first attempt got this wrong by reasoning
/// from first principles instead of checking -- caught by the parity
/// corpus, `omnist/tests/parity.rs`, before merge): `Date`/`Time`/
/// `Datetime` match **either** the real `document::Scalar` variant **or**
/// a plain `Str` whose text independently shape-validates
/// (`is_iso_date`/`is_iso_time`/`is_iso_datetime`) -- Python's own
/// `matches_kind` does exactly this hybrid check (a real `datetime.date`
/// object, or a string `_is_iso`-shaped for one), so a JSON- or
/// XML-sourced document with a date-shaped string field genuinely
/// satisfies `kind: date` at `validate` time in Python, with no
/// `materialize` upgrade required first. This is unlike `Integer`/
/// `Number`, which stay strict (Python's own `matches_kind` has no
/// string-shape fallback for those).
pub fn matches_kind(value: &DocScalar, kind: ScalarKind) -> bool {
    match kind {
        ScalarKind::String => matches!(value, DocScalar::Str(_)),
        ScalarKind::Boolean => matches!(value, DocScalar::Bool(_)),
        ScalarKind::Integer => matches!(value, DocScalar::Int(_)),
        ScalarKind::Number => matches!(value, DocScalar::Int(_) | DocScalar::Float(_)),
        ScalarKind::Date => {
            matches!(value, DocScalar::Date(_))
                || matches!(value, DocScalar::Str(s) if is_iso_date(s))
        }
        ScalarKind::Time => {
            matches!(value, DocScalar::Time(_))
                || matches!(value, DocScalar::Str(s) if is_iso_time(s))
        }
        // Deliberately excludes a bare date string -- see is_iso_datetime's
        // doc comment for why "datetime" and "date" must stay disjoint,
        // mirroring Python's `_is_iso(value, _dt.datetime) and not
        // _is_iso(value, _dt.date)`.
        ScalarKind::Datetime => {
            matches!(value, DocScalar::Datetime(_))
                || matches!(value, DocScalar::Str(s) if is_iso_datetime(s) && !is_iso_date(s))
        }
    }
}

/// The most specific scalar kind name a [`document::Scalar`] value matches,
/// for error messages (`integer` is reported even though it also matches
/// `number`).
pub(crate) fn value_kind_name(v: &DocScalar) -> &'static str {
    match v {
        DocScalar::Null => "null",
        DocScalar::Bool(_) => "boolean",
        DocScalar::Int(_) => "integer",
        DocScalar::Float(_) => "number",
        DocScalar::Str(_) => "string",
        DocScalar::Date(_) => "date",
        DocScalar::Time(_) => "time",
        DocScalar::Datetime(_) => "datetime",
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// A resolved field type: a record (via a `Ref`), a bare `Scalar`, or `Any`.
/// A resolved field type: a record (via a `Ref`), a bare `Scalar`, or `Any`.
pub enum Resolved<'a> {
    /// A resolved record in the schema environment.
    Record(&'a Record),
    /// A resolved scalar type.
    Scalar(Scalar),
    /// A resolved `any` type.
    Any,
}

/// A schema: a root reference plus an environment of named records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    root: Ref,
    env: IndexMap<String, Record>,
}

impl Schema {
    /// Builds a schema and immediately checks every `Ref` (the root's, and
    /// every field's) resolves within `env` -- mirrors Python's
    /// `Schema.__init__` calling `check_refs()` unconditionally. Also
    /// enforces S-3 (omnist-spec docs/03-schema-model.md): no record may be
    /// named after a scalar keyword or `any`, since a bare name in type
    /// position always resolves to the builtin first and such a record could
    /// never be referenced. `osd.rs`'s parser already enforces this; this is
    /// the same check at the builder-API surface (omnist-rs#76).
    pub fn new(root: Ref, env: IndexMap<String, Record>) -> Result<Self, SchemaError> {
        Self::check_reserved_names(&env)?;
        let schema = Schema { root, env };
        schema.check_refs()?;
        Ok(schema)
    }

    fn check_reserved_names(env: &IndexMap<String, Record>) -> Result<(), SchemaError> {
        for name in env.keys() {
            if name == "any" {
                return Err(SchemaError::new(
                    "any",
                    "schema.reserved-name",
                    format!(
                        "'any' is a reserved type name and cannot be used as a record name                          (record {name:?})"
                    ),
                ));
            }
            if ScalarKind::ALL.iter().any(|k| k.as_str() == name) {
                return Err(SchemaError::new(
                    name,
                    "schema.reserved-name",
                    format!(
                        "{name:?} is a reserved scalar name; a record cannot be defined with                          this name, or it could never be referenced (a bare name in a type                          position always means the builtin scalar)"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Reference to the root record of the schema.
    pub fn root(&self) -> &Ref {
        &self.root
    }

    /// Map of named records comprising the schema environment.
    pub fn env(&self) -> &IndexMap<String, Record> {
        &self.env
    }

    fn check_refs(&self) -> Result<(), SchemaError> {
        if !self.env.contains_key(&self.root.name) {
            return Err(SchemaError::new(
                "$",
                "schema.unknown-type",
                format!("unknown type {:?}", self.root.name),
            ));
        }
        for (rec_name, rec) in &self.env {
            for f in rec.fields() {
                if let FieldType::Ref(r) = &f.ty {
                    if !self.env.contains_key(&r.name) {
                        return Err(SchemaError::new(
                            format!("{rec_name}.{}", f.label),
                            "schema.unknown-type",
                            format!("unknown type {:?}", r.name),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// A bare `Scalar` resolves to itself; a `Ref` is a single environment
    /// lookup -- `check_refs` already guarantees every `Ref` resolves, so
    /// this never errors once a `Schema` exists.
    pub fn resolve(&self, ty: &FieldType) -> Resolved<'_> {
        match ty {
            FieldType::Scalar(s) => Resolved::Scalar(*s),
            FieldType::Any => Resolved::Any,
            FieldType::Ref(r) => Resolved::Record(
                self.env
                    .get(&r.name)
                    .expect("check_refs guarantees every Ref resolves"),
            ),
        }
    }

    /// Validates `cursor` (and everything beneath it) against this schema's
    /// root type, collecting every problem found rather than stopping at
    /// the first.
    pub fn validate(&self, cursor: &document::Cursor<'_>) -> ValidationResult {
        let mut res = ValidationResult::new();
        // `path` is the one path `String` this walk allocates up front (the
        // root); every deeper edge reuses this same buffer (push a segment,
        // recurse, truncate back) rather than allocating its own -- see
        // issue #44. `res.add` still copies out a path when an error is
        // actually recorded (`impl Into<String>` on a `&str` allocates), but
        // that only happens for edges that actually produce a problem.
        let mut path = cursor.path.clone();
        self.conform(
            cursor,
            &FieldType::Ref(self.root.clone()),
            &mut res,
            &mut path,
        );
        res
    }

    /// Returns `true` iff `cursor` conforms to this schema with 0 errors (spec §5).
    pub fn accepts(&self, cursor: &document::Cursor<'_>) -> bool {
        self.validate(cursor).ok()
    }

    fn conform(
        &self,
        cursor: &document::Cursor<'_>,
        ty: &FieldType,
        res: &mut ValidationResult,
        path: &mut String,
    ) {
        match self.resolve(ty) {
            // `any` accepts every legal Document value unchecked -- there is
            // nothing to conform against, mirroring Python's
            // `_conform`: `if isinstance(d, AnyType): return`.
            Resolved::Any => {}
            Resolved::Scalar(s) => self.conform_scalar(cursor, s, res, path),
            Resolved::Record(r) => self.conform_record(cursor, r, res, path),
        }
    }

    fn conform_scalar(
        &self,
        cursor: &document::Cursor<'_>,
        s: Scalar,
        res: &mut ValidationResult,
        path: &str,
    ) {
        if !cursor.is_leaf() {
            res.add(
                path,
                format!("expected a {} value, got an object", s.kind().as_str()),
                ErrorCode::ShapeMismatch,
            );
            return;
        }
        let v = cursor
            .value()
            .expect("is_leaf() true implies value() succeeds");
        if matches!(v, DocScalar::Null) {
            if !s.is_nullable() {
                res.add(path, "null not allowed here", ErrorCode::NullNotAllowed);
            }
            return;
        }
        if !matches_kind(v, s.kind()) {
            res.add(
                path,
                format!(
                    "expected {}, got {} ({})",
                    s.kind().as_str(),
                    value_kind_name(v),
                    v
                ),
                ErrorCode::TypeMismatch,
            );
        }
    }

    /// Walks every edge without building a path `String` for it up front
    /// (issue #44): `path` is a single buffer shared across the whole
    /// `validate` walk. Each edge pushes its own segment (`.label` or
    /// `.label[i]`, via [`crate::report::push_child_path`]) onto `path`,
    /// recurses/reports using that borrowed `&str`, then truncates `path`
    /// back before moving to the next edge -- so a document with no
    /// unexpected fields, cardinality problems, or type mismatches never
    /// allocates a path `String` per edge, only the one `res.add` actually
    /// needs to keep (via `impl Into<String>`) when a problem is found.
    fn conform_record(
        &self,
        cursor: &document::Cursor<'_>,
        rec: &Record,
        res: &mut ValidationResult,
        path: &mut String,
    ) {
        if cursor.is_leaf() {
            res.add(
                path.as_str(),
                "expected an object, got a value",
                ErrorCode::ShapeMismatch,
            );
            return;
        }
        let edges = cursor
            .raw_edges()
            .expect("is_leaf() false implies raw_edges() succeeds");
        let mut counts: IndexMap<&str, usize> = IndexMap::new();
        for (label, i, child_id) in &edges {
            *counts.entry(*label).or_insert(0) += 1;
            let base = path.len();
            crate::report::push_child_path(path, label, *i);
            match rec.field(label) {
                None => res.add(
                    path.as_str(),
                    "unexpected field",
                    ErrorCode::UnexpectedField,
                ),
                Some(f) => {
                    let child = cursor.seek(*child_id);
                    self.conform(&child, &f.ty, res, path);
                }
            }
            path.truncate(base);
        }
        for f in rec.fields() {
            let c = counts.get(f.label.as_str()).copied().unwrap_or(0);
            if c < f.min || f.max.is_some_and(|max| c > max) {
                res.add(
                    path.as_str(),
                    format!(
                        "field {:?} occurs {} time(s), expected {}",
                        f.label,
                        c,
                        f.cardinality_str()
                    ),
                    ErrorCode::Cardinality,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Doc, Value};
    use indexmap::IndexMap as Map;

    #[test]
    fn record_partial_eq_treats_field_order_as_insignificant_but_field_count_as_significant() {
        let r1 = Record::new(vec![
            Field::required("x", STRING).unwrap(),
            Field::required("y", STRING).unwrap(),
        ])
        .unwrap();
        let r2 = Record::new(vec![
            Field::required("y", STRING).unwrap(),
            Field::required("x", STRING).unwrap(),
        ])
        .unwrap();
        assert_eq!(r1, r2, "declaration order must not affect Record equality");

        let r3 = Record::new(vec![Field::required("x", STRING).unwrap()]).unwrap();
        assert_ne!(
            r1, r3,
            "a different field count must never compare equal, regardless of order"
        );
    }

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = Map::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    // -- Scalar construction ---------------------------------------------

    #[test]
    fn scalar_construction_and_equality() {
        let a = Scalar::new(ScalarKind::String, false);
        let b = Scalar::new(ScalarKind::String, false);
        assert_eq!(a, b);
        assert_ne!(a, Scalar::new(ScalarKind::String, true));
        assert_eq!(a, STRING);
    }

    #[test]
    fn scalar_nullable_flag() {
        assert!(!STRING.is_nullable());
        let n = nullable(STRING);
        assert!(n.is_nullable());
        assert_eq!(n.kind(), ScalarKind::String);
    }

    #[test]
    fn scalar_named_accepts_every_known_name() {
        for k in ScalarKind::ALL {
            assert_eq!(Scalar::named(k.as_str(), false).unwrap().kind(), k);
        }
    }

    #[test]
    fn scalar_named_rejects_unknown_name() {
        let err = Scalar::named("bogus", false).unwrap_err();
        assert!(err.to_string().contains("unknown scalar"));
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn scalar_display_shows_nullable_suffix() {
        assert_eq!(STRING.to_string(), "string");
        assert_eq!(nullable(STRING).to_string(), "string?");
    }

    // -- Field cardinality --------------------------------------------------

    #[test]
    fn field_rejects_max_less_than_min() {
        let err = Field::new("a", STRING, 2, Some(1)).unwrap_err();
        assert!(err.to_string().contains("invalid cardinality"));
        assert!(err.to_string().contains("[2,1]"));
    }

    #[test]
    fn field_accepts_max_equal_to_min_and_unbounded_max() {
        assert!(Field::new("a", STRING, 1, Some(1)).is_ok());
        assert!(Field::new("a", STRING, 0, None).is_ok());
    }

    #[test]
    fn field_cardinality_str_matches_python_phrasing() {
        assert_eq!(
            Field::new("a", STRING, 1, Some(1))
                .unwrap()
                .cardinality_str(),
            "exactly 1"
        );
        assert_eq!(
            Field::new("a", STRING, 0, Some(1))
                .unwrap()
                .cardinality_str(),
            "0 or 1"
        );
        assert_eq!(
            Field::new("a", STRING, 1, None).unwrap().cardinality_str(),
            "at least 1"
        );
        assert_eq!(
            Field::new("a", STRING, 2, Some(5))
                .unwrap()
                .cardinality_str(),
            "between 2 and 5"
        );
    }

    // -- Record: duplicate field labels -------------------------------------

    #[test]
    fn record_rejects_duplicate_field_label() {
        let a1 = Field::required("a", STRING).unwrap();
        let a2 = Field::required("a", INTEGER).unwrap();
        let err = Record::new(vec![a1, a2]).unwrap_err();
        assert!(err.to_string().contains("duplicate field label"));
        assert!(err.to_string().contains("\"a\""));
    }

    #[test]
    fn record_field_lookup_and_ordering() {
        let f_b = Field::required("b", STRING).unwrap();
        let f_a = Field::required("a", INTEGER).unwrap();
        let rec = Record::new(vec![f_b.clone(), f_a.clone()]).unwrap();
        assert_eq!(rec.fields(), &[f_b, f_a]);
        assert_eq!(rec.field("a").unwrap().label, "a");
        assert!(rec.field("missing").is_none());
    }

    // -- Schema: unknown Ref target -----------------------------------------

    #[test]
    fn schema_rejects_unknown_root_ref() {
        let err = Schema::new(Ref::new("Missing"), Map::new()).unwrap_err();
        assert!(err.to_string().contains("unknown type"));
        assert!(err.to_string().contains("Missing"));
    }

    #[test]
    fn schema_rejects_unknown_field_ref() {
        let mut env = Map::new();
        env.insert(
            "Root".to_string(),
            Record::new(vec![Field::required("x", Ref::new("Missing")).unwrap()]).unwrap(),
        );
        let err = Schema::new(Ref::new("Root"), env).unwrap_err();
        assert!(err.to_string().contains("unknown type"));
        assert!(err.to_string().contains("Missing"));
    }

    #[test]
    fn schema_rejects_a_record_named_after_a_scalar_keyword() {
        // S-3 (omnist-spec docs/03-schema-model.md): a record named "string"
        // could never be referenced, since a bare name in type position
        // always resolves to the builtin scalar first. omnist-rs#76.
        let mut env = Map::new();
        env.insert(
            "Root".to_string(),
            Record::new(vec![Field::required("x", STRING).unwrap()]).unwrap(),
        );
        env.insert("string".to_string(), Record::new(vec![]).unwrap());
        let err = Schema::new(Ref::new("Root"), env).unwrap_err();
        assert!(err.to_string().contains("reserved scalar name"));
        assert!(err.to_string().contains("\"string\""));
    }

    #[test]
    fn schema_rejects_a_record_named_any() {
        // S-3, the `any` half of the same constraint. omnist-rs#76.
        let mut env = Map::new();
        env.insert(
            "Root".to_string(),
            Record::new(vec![Field::required("x", STRING).unwrap()]).unwrap(),
        );
        env.insert("any".to_string(), Record::new(vec![]).unwrap());
        let err = Schema::new(Ref::new("Root"), env).unwrap_err();
        assert!(err.to_string().contains("reserved type name"));
    }

    #[test]
    fn schema_accepts_a_valid_self_referential_environment() {
        let mut env = Map::new();
        env.insert(
            "Node".to_string(),
            Record::new(vec![
                Field::required("value", STRING).unwrap(),
                Field::new("child", Ref::new("Node"), 0, Some(1)).unwrap(),
            ])
            .unwrap(),
        );
        assert!(Schema::new(Ref::new("Node"), env).is_ok());
    }

    fn service_schema() -> Schema {
        let mut env = Map::new();
        env.insert(
            "Database".to_string(),
            Record::new(vec![
                Field::required("type", STRING).unwrap(),
                Field::required("server", STRING).unwrap(),
                Field::required("port", INTEGER).unwrap(),
            ])
            .unwrap(),
        );
        env.insert(
            "Service".to_string(),
            Record::new(vec![
                Field::required("host", STRING).unwrap(),
                Field::required("port", INTEGER).unwrap(),
                Field::new("databases", Ref::new("Database"), 1, None).unwrap(),
                Field::new("tags", STRING, 0, None).unwrap(),
            ])
            .unwrap(),
        );
        Schema::new(Ref::new("Service"), env).unwrap()
    }

    fn valid_service_doc() -> Value {
        obj(&[
            ("host", Value::Str("api.internal".into())),
            ("port", Value::Int((8443).into())),
            (
                "databases",
                Value::Array(vec![obj(&[
                    ("type", Value::Str("prod".into())),
                    ("server", Value::Str("db1".into())),
                    ("port", Value::Int((5432).into())),
                ])]),
            ),
            (
                "tags",
                Value::Array(vec![
                    Value::Str("prod".into()),
                    Value::Str("us-east".into()),
                ]),
            ),
        ])
    }

    // -- cardinality semantics (worked example from model.md's appendix) ---

    #[test]
    fn cardinality_semantics_worked_example() {
        let schema = service_schema();
        let doc = Doc::of(&valid_service_doc()).unwrap();
        let res = schema.validate(&doc.root());
        assert!(res.ok(), "{res}");
    }

    #[test]
    fn cardinality_rejects_too_few_databases() {
        let schema = service_schema();
        let v = obj(&[
            ("host", Value::Str("h".into())),
            ("port", Value::Int((1).into())),
        ]);
        let doc = Doc::of(&v).unwrap();
        let res = schema.validate(&doc.root());
        assert!(!res.ok());
        assert!(
            res.errors()
                .iter()
                .any(|e| e.code == ErrorCode::Cardinality && e.message.contains("\"databases\""))
        );
    }

    #[test]
    fn cardinality_ignores_order() {
        // Same fields, different declaration/edge order -- must still
        // validate (per model.md §7: "order ignored").
        let schema = service_schema();
        let v = obj(&[
            (
                "databases",
                Value::Array(vec![obj(&[
                    ("port", Value::Int((1).into())),
                    ("type", Value::Str("t".into())),
                    ("server", Value::Str("s".into())),
                ])]),
            ),
            ("port", Value::Int((8443).into())),
            ("host", Value::Str("h".into())),
        ]);
        let doc = Doc::of(&v).unwrap();
        assert!(schema.accepts(&doc.root()));
    }

    // -- unexpected field / closedness --------------------------------------

    #[test]
    fn unexpected_field_is_rejected() {
        let schema = service_schema();
        let mut v = valid_service_doc();
        if let Value::Object(m) = &mut v {
            m.insert("extra".to_string(), Value::Int((1).into()));
        }
        let doc = Doc::of(&v).unwrap();
        let res = schema.validate(&doc.root());
        assert!(!res.ok());
        assert!(
            res.errors()
                .iter()
                .any(|e| e.code == ErrorCode::UnexpectedField && e.path.contains("extra"))
        );
    }

    // -- shape mismatch / type mismatch / null handling ---------------------

    #[test]
    fn scalar_expected_but_object_found_is_shape_mismatch() {
        let schema = service_schema();
        let v = obj(&[
            ("host", obj(&[])),
            ("port", Value::Int((1).into())),
            (
                "databases",
                Value::Array(vec![obj(&[
                    ("type", Value::Str("t".into())),
                    ("server", Value::Str("s".into())),
                    ("port", Value::Int((1).into())),
                ])]),
            ),
        ]);
        let doc = Doc::of(&v).unwrap();
        let res = schema.validate(&doc.root());
        assert!(
            res.errors()
                .iter()
                .any(|e| e.code == ErrorCode::ShapeMismatch)
        );
    }

    #[test]
    fn record_expected_but_scalar_found_is_shape_mismatch() {
        let mut env = Map::new();
        env.insert("Root".to_string(), Record::new(vec![]).unwrap());
        let schema = Schema::new(Ref::new("Root"), env).unwrap();
        let doc = Doc::of(&Value::Int((1).into())).unwrap();
        let res = schema.validate(&doc.root());
        assert!(!res.ok());
        assert_eq!(res.errors()[0].code, ErrorCode::ShapeMismatch);
    }

    #[test]
    fn type_mismatch_reports_expected_and_actual() {
        let schema = service_schema();
        let mut v = valid_service_doc();
        if let Value::Object(m) = &mut v {
            m.insert("port".to_string(), Value::Str("not a number".into()));
        }
        let doc = Doc::of(&v).unwrap();
        let res = schema.validate(&doc.root());
        let e = res
            .errors()
            .iter()
            .find(|e| e.code == ErrorCode::TypeMismatch)
            .unwrap();
        assert!(e.message.contains("expected integer"));
        assert!(e.message.contains("got string"));
    }

    #[test]
    fn null_rejected_for_non_nullable_scalar_but_accepted_when_nullable() {
        let mut env = Map::new();
        env.insert(
            "Root".to_string(),
            Record::new(vec![Field::required("v", STRING).unwrap()]).unwrap(),
        );
        let schema = Schema::new(Ref::new("Root"), env).unwrap();
        let doc = Doc::of(&obj(&[("v", Value::Null)])).unwrap();
        let res = schema.validate(&doc.root());
        assert!(!res.ok());
        assert_eq!(res.errors()[0].code, ErrorCode::NullNotAllowed);

        let mut env2 = Map::new();
        env2.insert(
            "Root".to_string(),
            Record::new(vec![Field::required("v", nullable(STRING)).unwrap()]).unwrap(),
        );
        let schema2 = Schema::new(Ref::new("Root"), env2).unwrap();
        let doc2 = Doc::of(&obj(&[("v", Value::Null)])).unwrap();
        assert!(schema2.accepts(&doc2.root()));
    }

    #[test]
    fn accepts_and_validation_result_display() {
        let schema = service_schema();
        let doc = Doc::of(&valid_service_doc()).unwrap();
        assert!(schema.accepts(&doc.root()));
        assert_eq!(schema.validate(&doc.root()).to_string(), "valid");

        let bad = Doc::of(&obj(&[])).unwrap();
        let res = schema.validate(&bad.root());
        assert!(!res.ok());
        let s = res.to_string();
        assert!(s.starts_with("invalid:\n  at "));
    }

    // -- matches_kind: bool must not satisfy integer/number -----------------

    #[test]
    fn bool_never_satisfies_integer_or_number() {
        assert!(!matches_kind(&DocScalar::Bool(true), ScalarKind::Integer));
        assert!(!matches_kind(&DocScalar::Bool(true), ScalarKind::Number));
        assert!(matches_kind(&DocScalar::Bool(true), ScalarKind::Boolean));
    }

    #[test]
    fn integer_satisfies_number_but_not_vice_versa() {
        assert!(matches_kind(
            &DocScalar::Int((3).into()),
            ScalarKind::Number
        ));
        assert!(!matches_kind(&DocScalar::Float(3.0), ScalarKind::Integer));
    }

    // -- temporal shape-check: date -----------------------------------------

    #[test]
    fn is_iso_date_accepts_valid_dates() {
        assert!(is_iso_date("2024-01-01"));
        assert!(is_iso_date("9999-12-31"));
    }

    #[test]
    fn is_iso_date_rejects_wrong_shape_and_invalid_calendar_dates() {
        // Wrong shape: fromisoformat-is-wider cases this crate deliberately
        // does NOT accept (basic format, single-digit month/day).
        assert!(!is_iso_date("20240101"));
        assert!(!is_iso_date("2024-1-1"));
        assert!(!is_iso_date("2024-W01-1"));
        assert!(!is_iso_date("not-a-date"));
        // Right shape, invalid calendar date.
        assert!(!is_iso_date("2024-13-01"));
        assert!(!is_iso_date("2024-02-30"));
        assert!(!is_iso_date("2024-00-01"));
        assert!(!is_iso_date("2024-01-00"));
        assert!(!is_iso_date("0000-01-01"));
    }

    #[test]
    fn is_iso_date_thirty_day_months() {
        assert!(is_iso_date("2024-04-30"));
        assert!(!is_iso_date("2024-04-31"));
        assert!(is_iso_date("2024-06-30"));
        assert!(is_iso_date("2024-09-30"));
        assert!(is_iso_date("2024-11-30"));
    }

    #[test]
    fn days_in_month_rejects_an_out_of_range_month_directly() {
        // White-box: `days_in_month`'s `_ => 0` arm is unreachable through
        // `valid_ymd` (which only calls it after `(1..=12).contains(&m)`
        // already passed) -- call the private helper directly to prove the
        // arm itself is correct, rather than leaving it untested.
        assert_eq!(days_in_month(2024, 13), 0);
        assert_eq!(days_in_month(2024, 0), 0);
    }

    #[test]
    fn is_iso_date_leap_year_boundary() {
        assert!(is_iso_date("2024-02-29")); // 2024 is a leap year
        assert!(!is_iso_date("2023-02-29")); // 2023 is not
        assert!(is_iso_date("2000-02-29")); // divisible by 400
        assert!(!is_iso_date("1900-02-29")); // divisible by 100, not 400
    }

    // -- temporal shape-check: time ------------------------------------------

    #[test]
    fn is_iso_time_accepts_valid_times() {
        assert!(is_iso_time("12:00:00"));
        assert!(is_iso_time("12:00"));
        assert!(is_iso_time("12:00:00.5"));
        assert!(is_iso_time("12:00:00.123456"));
        assert!(is_iso_time("12:00:00+02:00"));
        assert!(is_iso_time("23:59:59"));
    }

    #[test]
    fn is_iso_time_rejects_out_of_range_and_malformed() {
        assert!(!is_iso_time("25:00:00"));
        assert!(!is_iso_time("12:60:00"));
        assert!(!is_iso_time("24:00:00"));
        assert!(!is_iso_time("12:00:00+24:00"));
        assert!(!is_iso_time("12:00:00+99:99"));
        assert!(!is_iso_time("12:00:00.1234567")); // 7 fractional digits
        assert!(!is_iso_time("1:00:00")); // not zero-padded
        assert!(is_iso_time("12:00:00+23:59"));
    }

    // -- temporal shape-check: datetime, and the `_is_iso` vs
    //    `fromisoformat`-is-wider / date-vs-datetime exclusivity ------------

    #[test]
    fn is_iso_datetime_accepts_valid_timestamps() {
        assert!(is_iso_datetime("2024-01-01T12:00:00"));
        assert!(is_iso_datetime("2024-01-01T12:00"));
        assert!(is_iso_datetime("2024-01-01T12:00:00+02:00"));
        assert!(is_iso_datetime("2024-01-01T12:00:00.123456"));
    }

    #[test]
    fn is_iso_datetime_rejects_bare_date_and_invalid_components() {
        assert!(!is_iso_datetime("2024-01-01"));
        assert!(!is_iso_datetime("2024-01-01T25:00:00"));
        assert!(!is_iso_datetime("2024-13-01T12:00:00"));
    }

    #[test]
    fn matches_kind_datetime_excludes_bare_date_string_and_a_real_date() {
        // The exact _is_iso-vs-fromisoformat subtlety, verified against
        // Python's real `matches_kind` (issue #105): a bare date-only
        // string satisfies `Date` but not `Datetime` (`fromisoformat` on
        // `datetime` would succeed, defaulting the missing time to
        // midnight -- not the same value as "no time given"). The same
        // disjointness holds for the real, non-string variants: a genuine
        // `Scalar::Date` never matches `Datetime` and vice versa.
        let v = DocScalar::Str("2024-01-01".to_string());
        assert!(matches_kind(&v, ScalarKind::Date));
        assert!(!matches_kind(&v, ScalarKind::Datetime));

        let dt = DocScalar::Str("2024-01-01T00:00:00".to_string());
        assert!(matches_kind(&dt, ScalarKind::Datetime));
        assert!(!matches_kind(&dt, ScalarKind::Date));

        let d_real = DocScalar::Date("2024-01-01".to_string());
        assert!(matches_kind(&d_real, ScalarKind::Date));
        assert!(!matches_kind(&d_real, ScalarKind::Datetime));

        let dt_real = DocScalar::Datetime("2024-01-01T00:00:00".to_string());
        assert!(matches_kind(&dt_real, ScalarKind::Datetime));
        assert!(!matches_kind(&dt_real, ScalarKind::Date));
    }

    #[test]
    fn matches_kind_date_and_time_directly() {
        // Both the real variant and a shape-matching plain string satisfy
        // Date/Time (verified against Python's real `matches_kind`, issue
        // #105 -- Python's own hybrid check, not just this port's).
        assert!(matches_kind(
            &DocScalar::Date("2024-01-01".into()),
            ScalarKind::Date
        ));
        assert!(matches_kind(
            &DocScalar::Str("2024-01-01".into()),
            ScalarKind::Date
        ));
        assert!(!matches_kind(
            &DocScalar::Str("not-a-date".into()),
            ScalarKind::Date
        ));
        assert!(matches_kind(
            &DocScalar::Time("12:00:00".into()),
            ScalarKind::Time
        ));
        assert!(matches_kind(
            &DocScalar::Str("12:00:00".into()),
            ScalarKind::Time
        ));
        assert!(!matches_kind(
            &DocScalar::Str("25:00:00".into()),
            ScalarKind::Time
        ));
        assert!(!matches_kind(&DocScalar::Int((1).into()), ScalarKind::Date));
        assert!(!matches_kind(&DocScalar::Int((1).into()), ScalarKind::Time));
    }

    #[test]
    fn schema_root_and_env_accessors() {
        let schema = service_schema();
        assert_eq!(schema.root().name, "Service");
        assert!(schema.env().contains_key("Database"));
        assert!(schema.env().contains_key("Service"));
    }

    // -- FieldType From conversions / Ref, Display --------------------------

    // -- `any` field type: accepts every legal value unchecked ------------

    #[test]
    fn any_field_accepts_scalars_and_objects_unchecked() {
        let mut env = Map::new();
        env.insert(
            "Root".to_string(),
            Record::new(vec![Field::required("x", FieldType::Any).unwrap()]).unwrap(),
        );
        let schema = Schema::new(Ref::new("Root"), env).unwrap();

        // Not an array here: cardinality (how many times a label occurs) is
        // checked independently of the field's type, `any` included -- an
        // array under a `[1,1]` field is still a cardinality violation, not
        // an `any`-typed acceptance question. See the array/cardinality
        // case in a separate `[0,]`-cardinality field below.
        for v in [
            Value::Str("hi".into()),
            Value::Int((1).into()),
            Value::Float(1.5),
            Value::Bool(true),
            Value::Null,
            obj(&[("nested", Value::Int((1).into()))]),
        ] {
            let doc = Doc::of(&obj(&[("x", v.clone())])).unwrap();
            assert!(schema.accepts(&doc.root()), "any field rejected {v:?}");
        }
    }

    #[test]
    fn any_field_with_array_cardinality_accepts_repeated_values_unchecked() {
        let mut env = Map::new();
        env.insert(
            "Root".to_string(),
            Record::new(vec![Field::new("x", FieldType::Any, 0, None).unwrap()]).unwrap(),
        );
        let schema = Schema::new(Ref::new("Root"), env).unwrap();
        let doc = Doc::of(&obj(&[(
            "x",
            Value::Array(vec![Value::Int((1).into()), Value::Str("mixed".into())]),
        )]))
        .unwrap();
        assert!(schema.accepts(&doc.root()));
    }

    #[test]
    fn any_resolves_without_touching_env() {
        let env: Map<String, Record> = Map::new();
        let schema = Schema::new(Ref::new("Root"), {
            let mut e = env;
            e.insert("Root".to_string(), Record::new(vec![]).unwrap());
            e
        })
        .unwrap();
        assert!(matches!(schema.resolve(&FieldType::Any), Resolved::Any));
    }

    #[test]
    fn field_type_from_conversions() {
        let ft: FieldType = STRING.into();
        assert_eq!(ft, FieldType::Scalar(STRING));
        let ft2: FieldType = Ref::new("X").into();
        assert_eq!(ft2, FieldType::Ref(Ref::new("X")));
    }

    #[test]
    fn ref_display() {
        assert_eq!(Ref::new("Foo").to_string(), "ref(Foo)");
    }

    #[test]
    fn error_code_as_str_covers_every_variant() {
        assert_eq!(ErrorCode::UnexpectedField.as_str(), "unexpected-field");
        assert_eq!(ErrorCode::Cardinality.as_str(), "cardinality");
        assert_eq!(ErrorCode::TypeMismatch.as_str(), "type-mismatch");
        assert_eq!(ErrorCode::NullNotAllowed.as_str(), "null-not-allowed");
        assert_eq!(ErrorCode::ShapeMismatch.as_str(), "shape-mismatch");
    }

    #[test]
    fn value_kind_name_covers_every_variant() {
        assert_eq!(value_kind_name(&DocScalar::Null), "null");
        assert_eq!(value_kind_name(&DocScalar::Bool(true)), "boolean");
        assert_eq!(value_kind_name(&DocScalar::Int((1).into())), "integer");
        assert_eq!(value_kind_name(&DocScalar::Float(1.0)), "number");
        assert_eq!(value_kind_name(&DocScalar::Str("x".into())), "string");
        assert_eq!(
            value_kind_name(&DocScalar::Date("2024-01-01".into())),
            "date"
        );
        assert_eq!(value_kind_name(&DocScalar::Time("12:00:00".into())), "time");
        assert_eq!(
            value_kind_name(&DocScalar::Datetime("2024-01-01T12:00:00".into())),
            "datetime"
        );
    }
}

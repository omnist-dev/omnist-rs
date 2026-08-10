//! Schema-directed deserialization: make a freshly-read [`RawNode`] conform
//! to a [`Schema`], or report every reason it doesn't.
//!
//! Ported from `~/dev/omnist/omnist/deserialize.py` (issue #14). Readers
//! hand back text-shaped values: JSON/YAML/TOML have no `date`/`time`
//! type, so a temporal field arrives as an ISO-8601 string; a whole-number
//! float may need to become an int (or vice versa) to match what the
//! schema declares. [`materialize`] walks the node together with the
//! schema, upgrading each leaf **only when the conversion is value-exact**
//! (`1.0 -> 1` for an `integer` field, `1 -> 1.0` for a `number` field --
//! see the module's scalar-upgrade table below) and checking every
//! record's shape (closed fields, cardinality) in the same pass -- not a
//! second top-down walk delegating to [`crate::schema::Schema::validate`]
//! afterward. That would mean re-walking the same tree twice with
//! different traversal code for no reason: `materialize` already knows, at
//! every node, exactly which field/type the schema expects there, so
//! upgrading and shape-checking happen together in one pass, matching the
//! Python reference's stated rationale.
//!
//! ## No native temporal `Scalar` variant
//!
//! [`crate::document::Scalar`] has no `date`/`time`/`datetime` variant (see
//! `document.rs`'s module doc) -- every value this port ever materializes
//! is `Null`/`Bool`/`Int`/`Float`/`Str`. So unlike the Python reference
//! (which actually constructs a `datetime.date`/`time`/`datetime` object),
//! "upgrading" a temporal field here can only ever mean "is this string
//! shaped like, and a semantically valid, ISO date/time/datetime" -- it
//! stays a `Str` either way. That check is exactly
//! [`crate::schema::is_iso_date`]/[`is_iso_time`][crate::schema::is_iso_time]/
//! [`is_iso_datetime`][crate::schema::is_iso_datetime], reused here rather
//! than reimplemented -- the exact "validate and materialize must share
//! one strict parser" pitfall the porting playbook calls out, and the same
//! functions [`crate::schema::matches_kind`] already uses.
//!
//! ## Scalar upgrade table (value-exact only)
//!
//! | field kind | accepts as-is           | upgrades                         |
//! |---|---|---|
//! | `string`   | `Str`                   | (none)                           |
//! | `boolean`  | `Bool`                  | (none)                           |
//! | `integer`  | `Int`                   | `Float` with a zero fractional part |
//! | `number`   | `Float`                 | `Int` (always promoted to `Float`, matching the Python reference's `float(value)`) |
//! | `date`/`time`/`datetime` | `Str` shaped per the shared temporal check | (none -- see above) |
//!
//! `null` is accepted only when the field's `Scalar` is nullable,
//! regardless of kind.
//!
//! ## No `strict=` switch
//!
//! There's no separate strict/non-strict mode: [`materialize`] takes
//! `schema: Option<&Schema>` -- `None` is the well-defined "opt out of
//! validation entirely" case (the node is returned exactly as read,
//! untouched), matching the Python reference's `schema=None` convention at
//! the reader call sites.
//!
//! ## The `any` type
//!
//! An `Any`-typed field passes its node through completely untouched --
//! no shape check, no scalar upgrade -- mirroring Python's
//! `_materialize_type`: `if isinstance(d, AnyType): return node`.

use crate::document::{RawNode, Scalar as DocScalar};
use crate::error::MaterializeError;
use crate::schema::{ErrorCode, FieldType, Resolved, ScalarKind, Schema, ValidationResult};
use num_traits::{FromPrimitive, ToPrimitive};

/// A copy of `node` with leaf values upgraded to match `schema`, guaranteed
/// to conform to it -- or every reason it can't, collected into one
/// [`MaterializeError`] (never just the first problem found).
///
/// `schema = None` is a no-op passthrough: `node` is cloned back unchanged,
/// with no validation performed at all.
pub fn materialize(node: &RawNode, schema: Option<&Schema>) -> Result<RawNode, MaterializeError> {
    let Some(schema) = schema else {
        return Ok(node.clone());
    };
    let mut res = ValidationResult::new();
    let root_ty = FieldType::Ref(schema.root().clone());
    // One path `String`, allocated once and reused for the whole walk (push
    // a segment per edge, recurse, truncate back) rather than one `format!`
    // per edge regardless of whether that edge ever reports anything --
    // see issue #44.
    let mut path = String::from("$");
    let out = materialize_type(node, schema, &root_ty, &mut path, &mut res);
    if !res.ok() {
        return Err(MaterializeError(res));
    }
    Ok(out)
}

fn materialize_type(
    node: &RawNode,
    schema: &Schema,
    ty: &FieldType,
    path: &mut String,
    res: &mut ValidationResult,
) -> RawNode {
    match schema.resolve(ty) {
        // `any` accepts every legal value unchecked -- pass the node through
        // exactly as read, no shape check or scalar upgrade.
        Resolved::Any => node.clone(),
        Resolved::Scalar(s) => materialize_scalar(node, s.kind(), s.is_nullable(), path, res),
        Resolved::Record(rec) => materialize_record(node, schema, rec, path, res),
    }
}

fn materialize_record(
    node: &RawNode,
    schema: &Schema,
    rec: &crate::schema::Record,
    path: &mut String,
    res: &mut ValidationResult,
) -> RawNode {
    let RawNode::Edges(edges) = node else {
        res.add(
            path.as_str(),
            "expected an object, got a value",
            ErrorCode::ShapeMismatch,
        );
        return node.clone();
    };
    let mut out: Vec<(String, RawNode)> = Vec::with_capacity(edges.len());
    let mut counts: indexmap::IndexMap<&str, usize> = indexmap::IndexMap::new();
    for (label, child) in edges {
        let i = *counts.entry(label.as_str()).or_insert(0);
        counts.insert(label.as_str(), i + 1);
        let base = path.len();
        crate::report::push_child_path(path, label, i);
        match rec.field(label) {
            None => {
                res.add(
                    path.as_str(),
                    "unexpected field",
                    ErrorCode::UnexpectedField,
                );
                out.push((label.clone(), child.clone()));
            }
            Some(f) => {
                let m = materialize_type(child, schema, &f.ty, path, res);
                out.push((label.clone(), m));
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
    RawNode::Edges(out)
}

fn materialize_scalar(
    node: &RawNode,
    kind: ScalarKind,
    nullable: bool,
    path: &str,
    res: &mut ValidationResult,
) -> RawNode {
    let value = match node {
        RawNode::Leaf(v) => v,
        RawNode::Edges(_) => {
            res.add(
                path,
                format!("expected a {} value, got an object", kind.as_str()),
                ErrorCode::ShapeMismatch,
            );
            return node.clone();
        }
    };
    if matches!(value, DocScalar::Null) {
        if !nullable {
            res.add(path, "null not allowed here", ErrorCode::NullNotAllowed);
        }
        return RawNode::Leaf(DocScalar::Null);
    }
    if let Some(upgraded) = try_upgrade(value, kind) {
        return RawNode::Leaf(upgraded);
    }
    res.add(
        path,
        format!(
            "{value} cannot be read as {} (not a value-exact conversion)",
            kind.as_str()
        ),
        ErrorCode::TypeMismatch,
    );
    node.clone()
}

/// Value-exact upgrade table -- `None` means the value cannot become
/// `kind` without loss or ambiguity (a `type-mismatch` at the call site).
fn try_upgrade(value: &DocScalar, kind: ScalarKind) -> Option<DocScalar> {
    match (kind, value) {
        (ScalarKind::String, DocScalar::Str(_)) => Some(value.clone()),
        (ScalarKind::Boolean, DocScalar::Bool(_)) => Some(value.clone()),
        (ScalarKind::Integer, DocScalar::Int(_)) => Some(value.clone()),
        (ScalarKind::Integer, DocScalar::Float(f)) => {
            // Arbitrary-precision (issue #104): no upper/lower bound to
            // check anymore -- `BigInt` has no range limit, so any finite
            // whole-number float upgrades. `BigInt::from_f64` decomposes
            // the float's exact mantissa*2^exponent value (well-defined
            // for any finite f64, not an approximation), consistent with
            // `f.fract() == 0.0` already having confirmed there's no
            // fractional part to lose.
            if f.is_finite() && f.fract() == 0.0 {
                num_bigint::BigInt::from_f64(*f).map(DocScalar::Int)
            } else {
                None
            }
        }
        // "number" always ends up a Float, even if it arrived as an Int --
        // matches the Python reference's unconditional `float(value)`.
        // `BigInt::to_f64` saturates to +/-infinity for a magnitude
        // beyond f64's finite range rather than failing -- matches this
        // codebase's existing Float model, which already renders
        // `inf`/`-inf` as first-class values (see `formats::float_fmt`).
        (ScalarKind::Number, DocScalar::Int(i)) => {
            Some(DocScalar::Float(i.to_f64().unwrap_or(f64::INFINITY)))
        }
        (ScalarKind::Number, DocScalar::Float(_)) => Some(value.clone()),
        // Issue #105: upgrading a plain string to Date/Time/Datetime now
        // constructs the real `Scalar` variant (previously stayed `Str`,
        // tagged only via the now-removed `RawNode::TemporalLeaf`
        // write-hint). `Time`/`Datetime` canonicalize (fills a missing
        // `:SS`, zero-pads a short fraction) the same way OML's own
        // bare-literal grammar already does -- `is_iso_time`/
        // `is_iso_datetime` accept non-canonical spellings that this
        // variant's own invariant (always canonical) requires
        // normalizing first. `Date` has no optional grammar components,
        // so its source spelling is already canonical.
        (ScalarKind::Date, DocScalar::Str(s)) if crate::schema::is_iso_date(s) => {
            Some(DocScalar::Date(s.clone()))
        }
        (ScalarKind::Time, DocScalar::Str(s)) if crate::schema::is_iso_time(s) => {
            Some(DocScalar::Time(crate::schema::canonicalize_iso_time(s)))
        }
        (ScalarKind::Datetime, DocScalar::Str(s))
            if crate::schema::is_iso_datetime(s) && !crate::schema::is_iso_date(s) =>
        {
            Some(DocScalar::Datetime(
                crate::schema::canonicalize_iso_datetime(s),
            ))
        }
        // Identity: a value already correctly typed (e.g. re-materializing
        // an already-materialized document, or one read directly from
        // OML's/TOML's own native temporal grammar) stays as-is -- mirrors
        // the `Integer`/`Int` and `Number`/`Float` identity arms above.
        (ScalarKind::Date, DocScalar::Date(_))
        | (ScalarKind::Time, DocScalar::Time(_))
        | (ScalarKind::Datetime, DocScalar::Datetime(_)) => Some(value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

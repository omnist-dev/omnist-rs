//! Field-signature helpers for schema minimization (and isomorphism).
//! Ported from `~/dev/omnist/omnist/ops/signature.py`.
//!
//! [`local_signature`] is the target-blind structural key used as the
//! *initial* partition for `minimize`'s partition refinement: a key
//! including ref target names would be too strong a starting point --
//! records that turn out to be equivalent because their ref targets are
//! themselves equivalent-but-differently-named would never even land in
//! the same starting block. It captures a field's label, cardinality, and
//! scalar-or-ref *shape*, but excludes ref target names (those are compared
//! by evolving block id during `minimize`'s refinement instead).

use crate::schema::{FieldType, Record, ScalarKind};

/// A field's target-blind shape: `Scalar(kind, nullable)`, `Ref` (the
/// target record's name is deliberately excluded -- see the module doc
/// comment), or `Any`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShapeKey {
    Scalar(ScalarKind, bool),
    Ref,
    Any,
}

/// One field's signature entry: `(label, min, max, shape)`.
pub type FieldKey = (String, usize, Option<usize>, ShapeKey);

/// A record's target-blind structural key: every field's [`FieldKey`],
/// sorted by label.
pub type LocalSignature = Vec<FieldKey>;

/// Target-blind structural key for a record: fields sorted by label, each
/// keyed by `(label, min, max, shape)`.
///
/// Fields are sorted by label rather than kept in declaration order:
/// validation ignores field order (a `Record` is a *set* of labeled fields),
/// so two records that declare the same fields in a different order accept
/// exactly the same documents and MUST land in the same initial partition
/// block -- keying by declaration order would incorrectly split them.
pub fn local_signature(rec: &Record) -> LocalSignature {
    let mut fields: LocalSignature = rec
        .fields()
        .iter()
        .map(|f| {
            let shape = match &f.ty {
                FieldType::Scalar(s) => ShapeKey::Scalar(s.kind(), s.is_nullable()),
                FieldType::Ref(_) => ShapeKey::Ref,
                FieldType::Any => ShapeKey::Any,
            };
            (f.label.clone(), f.min, f.max, shape)
        })
        .collect();
    // `Record::new` already rejects duplicate labels, so this sort can never
    // need a tie-breaker -- codepoint order via `String`'s default `Ord`
    // (byte-wise on valid UTF-8, which coincides with codepoint order),
    // never a locale-aware comparison (see the omnist-ts#56 regression test
    // in `tests.rs`).
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    fields
}

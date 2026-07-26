//! Subschema compatibility and equivalence. Ported from
//! `~/dev/omnist/omnist/ops/subschema.py`.
//!
//! Implements the paper's Algorithm 4 (SubschemaSA) restricted to omnist's
//! counting cardinality languages; [`equivalent`] is bidirectional
//! inclusion.
//!
//! Algorithm 4 assumes its precondition MakeUsefulSA (useless-state removal,
//! `super::prune`) has already run: the coinductive cycle rule below only
//! coincides with true (finite-document) language inclusion once every
//! A-side record is known satisfiable. Rather than requiring callers to
//! pre-prune, [`compatible_with`] computes `a`'s satisfiable set once up
//! front and consults it directly -- an unsatisfiable A-side record is
//! vacuously a subschema of anything (it emits no documents at all), and an
//! optional A-field whose type is unsatisfiable is skipped (it can never
//! actually be emitted, so it imposes no obligation on B).

use indexmap::{IndexMap, IndexSet};

use crate::schema::{FieldType, Record, Scalar, ScalarKind, Schema};

use super::prune::satisfiable_set;

/// True if every document `a` accepts is also accepted by `b` (`a` is a
/// subschema / `b` is backward-compatible).
pub fn compatible_with(a: &Schema, b: &Schema) -> bool {
    let sat_a = satisfiable_set(a);
    let mut memo: IndexMap<(String, String), bool> = IndexMap::new();
    sub(
        a,
        &FieldType::Ref(a.root().clone()),
        b,
        &FieldType::Ref(b.root().clone()),
        &sat_a,
        &mut memo,
    )
}

/// True if both schemas accept exactly the same documents.
pub fn equivalent(a: &Schema, b: &Schema) -> bool {
    compatible_with(a, b) && compatible_with(b, a)
}

/// The memo key is `(a-ref-name, b-ref-name)`, guarding only the ref/ref
/// case -- the only one that can cycle. Scalar comparisons never recurse,
/// so they need no memoization to terminate.
fn sub(
    sa: &Schema,
    ta: &FieldType,
    sb: &Schema,
    tb: &FieldType,
    sat_a: &IndexSet<String>,
    memo: &mut IndexMap<(String, String), bool>,
) -> bool {
    match (ta, tb) {
        (FieldType::Ref(ra), _) if !sat_a.contains(&ra.name) => true,
        (FieldType::Scalar(a), FieldType::Scalar(b)) => scalar_sub(*a, *b),
        (FieldType::Ref(ra), FieldType::Ref(rb)) => {
            let key = (ra.name.clone(), rb.name.clone());
            if let Some(&v) = memo.get(&key) {
                return v;
            }
            // Coinductive assumption while descending, mirroring the Python
            // reference: a cycle that never disagrees is compatible.
            memo.insert(key.clone(), true);
            let reca = sa
                .env()
                .get(&ra.name)
                .expect("Schema's own invariant: every Ref resolves within its env");
            let recb = sb
                .env()
                .get(&rb.name)
                .expect("Schema's own invariant: every Ref resolves within its env");
            let result = record_sub(sa, reca, sb, recb, sat_a, memo);
            memo.insert(key, result);
            result
        }
        // A value type vs. an object type (or vice versa) is never
        // compatible.
        _ => false,
    }
}

fn record_sub(
    sa: &Schema,
    a: &Record,
    sb: &Schema,
    b: &Record,
    sat_a: &IndexSet<String>,
    memo: &mut IndexMap<(String, String), bool>,
) -> bool {
    // Every label A may emit must be allowed by B, with a cardinality range
    // B's covers and a type B accepts.
    for fa in a.fields() {
        if fa.max == Some(0) {
            continue; // A never emits this label
        }
        if fa.min == 0
            && let FieldType::Ref(r) = &fa.ty
            && !sat_a.contains(&r.name)
        {
            continue; // A never actually emits this label either
        }
        let Some(fb) = b.field(&fa.label) else {
            return false; // B is closed and has no such field
        };
        if !(fb.min <= fa.min && le(fa.max, fb.max)) {
            return false; // [fa.min,fa.max] not a subset of B's range
        }
        if !sub(sa, &fa.ty, sb, &fb.ty, sat_a, memo) {
            return false;
        }
    }
    // Every label B *requires* must be guaranteed by A.
    for fb in b.fields() {
        if fb.min >= 1 {
            match a.field(&fb.label) {
                None => return false,
                Some(fa) if fa.min < fb.min => return false,
                _ => {}
            }
        }
    }
    true
}

/// `x <= y`, treating `None` as `+infinity` (unbounded max).
fn le(x: Option<usize>, y: Option<usize>) -> bool {
    match y {
        None => true,
        Some(y) => match x {
            None => false,
            Some(x) => x <= y,
        },
    }
}

fn scalar_sub(a: Scalar, b: Scalar) -> bool {
    if a.is_nullable() && !b.is_nullable() {
        return false;
    }
    if a.kind() == b.kind() {
        return true;
    }
    a.kind() == ScalarKind::Integer && b.kind() == ScalarKind::Number
}

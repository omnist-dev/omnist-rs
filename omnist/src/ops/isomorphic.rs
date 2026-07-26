//! Schema isomorphism -- ported from `~/dev/omnist/omnist/ops/isomorphic.py`.
//!
//! Two schemas are equivalent iff their minimized (normalized) forms are
//! isomorphic. That gives a second, algorithm-independent decision procedure
//! for `equivalent`, structurally unrelated to bidirectional
//! `subschema::compatible_with`, so the two can be cross-checked against
//! each other in tests (the "dual-algorithm oracle" -- see `tests.rs`, the
//! `minimize`/`isomorphic` triple-check strategy from the issue).
//!
//! [`is_isomorphic`] is deliberately not part of the crate's public surface
//! commitment the way `subschema::equivalent` is -- it exists purely as an
//! independent oracle for tests, matching the Python reference's choice to
//! keep `_isomorphic` private.
//!
//! Algorithm: parallel traversal from both roots, building a bijection
//! `name_a -> name_b` (and its inverse) between env record names as the
//! traversal discovers pairs. At each visited record pair, `local_signature`
//! must match; since it sorts fields by label and ref/scalar shape is part
//! of the key, fields on the two sides line up one-to-one by label once the
//! signatures agree. For each ref-typed field, the two targets are
//! recursively required to be isomorphic, with the bijection enforced
//! consistently in both directions.
//!
//! Both inputs are assumed already normalized (pruned + minimized) by the
//! caller -- this module does not call `normalize` itself.

use indexmap::IndexMap;

use crate::schema::{FieldType, Schema};

use super::prune::is_empty;
use super::signature::local_signature;

/// True iff normalized schemas `a` and `b` are isomorphic: there is a
/// bijection between their env record names under which the two root
/// records (and everything reachable from them) match exactly.
///
/// **Empty-schema convention.** If both `a` and `b` are unsatisfiable, they
/// are treated as isomorphic (both accept the empty language). If exactly
/// one is empty, they are *not* isomorphic.
pub fn is_isomorphic(a: &Schema, b: &Schema) -> bool {
    let (empty_a, empty_b) = (is_empty(a), is_empty(b));
    if empty_a || empty_b {
        return empty_a && empty_b;
    }

    let mut map_ab: IndexMap<String, String> = IndexMap::new();
    let mut map_ba: IndexMap<String, String> = IndexMap::new();
    walk(
        a,
        a.root().name.clone(),
        b,
        b.root().name.clone(),
        &mut map_ab,
        &mut map_ba,
    )
}

fn walk(
    a: &Schema,
    na: String,
    b: &Schema,
    nb: String,
    map_ab: &mut IndexMap<String, String>,
    map_ba: &mut IndexMap<String, String>,
) -> bool {
    if map_ab.contains_key(&na) || map_ba.contains_key(&nb) {
        // Already visited on at least one side: the bijection must agree
        // both ways, or the schemas aren't isomorphic.
        return map_ab.get(&na) == Some(&nb) && map_ba.get(&nb) == Some(&na);
    }

    map_ab.insert(na.clone(), nb.clone());
    map_ba.insert(nb.clone(), na.clone());

    let ra = a
        .env()
        .get(&na)
        .expect("caller only walks names taken from a Schema's own root/Ref graph");
    let rb = b
        .env()
        .get(&nb)
        .expect("caller only walks names taken from a Schema's own root/Ref graph");
    if local_signature(ra) != local_signature(rb) {
        return false;
    }

    // local_signature sorts fields by label and includes the label in its
    // key, so two records with equal signatures declare exactly the same
    // set of labels -- fields on the two sides line up one-to-one by label.
    for fa in ra.fields() {
        let fb = rb
            .field(&fa.label)
            .expect("equal local_signature guarantees the same label set on both sides");
        if let (FieldType::Ref(ra_ref), FieldType::Ref(rb_ref)) = (&fa.ty, &fb.ty)
            && !walk(
                a,
                ra_ref.name.clone(),
                b,
                rb_ref.name.clone(),
                map_ab,
                map_ba,
            )
        {
            return false;
        }
    }
    true
}

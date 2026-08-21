//! Subschema extraction (paper Algorithm 5, ExtractSubschema). Ported from
//! `~/dev/omnist/omnist/ops/extract.py`.
//!
//! Given a schema and a set of *permissible labels* `keep` (the paper's
//! `X'`), produces the minimal subschema that recognizes only documents
//! built from those labels.
//!
//! Algorithm:
//!
//! 1. For every record in the env, delete any field whose label is not in
//!    `keep`.
//! 2. If a deleted field had `min >= 1` (mandatory), that record is
//!    *invalidated* -- the paper's "state removed": there is no way to
//!    build a document at that record's shape without a label that's no
//!    longer available.
//! 3. **Propagate.** A record with a *mandatory* field whose type is an
//!    invalidated record is itself invalidated, and so on transitively -- a
//!    least-fixpoint closure, same shape as `super::prune`'s satisfiability
//!    fixpoint.
//! 4. If the root ends up invalidated, there is no valid subschema for this
//!    `keep` set at all: [`extract`] returns a [`SchemaError`] naming the
//!    first offending label and record.
//! 5. Otherwise, invalidated records (and fields typed to them, along with
//!    any fields already dropped in step 1) are gone; the result is run
//!    through [`super::prune::prune`] and [`super::minimize::normalize`]
//!    (Algorithm 5's own final MakeUseful + Minimize step).
//!
//! **Design decision: mandatory deletion is an error, not silently-optional**
//! -- matching the Python reference. Silently loosening a deleted mandatory
//! field to optional would mean the result no longer reflects Algorithm 5's
//! semantics, and would more often hide a mistake in the caller's `keep` set
//! than express an intentional relaxation.

use indexmap::{IndexMap, IndexSet};

use crate::error::SchemaError;
use crate::schema::{FieldType, Record, Ref, Schema};

use super::minimize::normalize;
use super::prune::prune;

/// The minimal subschema of `s` that only recognizes documents built from
/// labels in `keep`. Returns a [`SchemaError`] if deleting the other labels
/// would invalidate the root record (see the module doc comment).
///
/// Takes a concrete `&[&str]` rather than a generic `IntoIterator` on
/// purpose: a generic parameter here would monomorphize a separate copy of
/// `extract` per distinct caller argument type (`Vec<String>`, an array
/// literal, an empty slice, ...), and `cargo llvm-cov` counts per-
/// instantiation coverage separately -- so a fully-tested generic version
/// could still report less than 100% simply because not every
/// instantiation was independently exercised, without any real gap in
/// behavior coverage. A single concrete signature sidesteps that entirely.
pub fn extract(s: &Schema, keep: &[&str]) -> Result<Schema, SchemaError> {
    let keep_set: IndexSet<String> = keep.iter().map(|s| (*s).to_string()).collect();

    // Step 1+2: per-record field deletion, tracking which records are
    // directly invalidated by the loss of a mandatory field, and the first
    // offending (label, record) pair for the error message.
    let mut trimmed: IndexMap<String, Record> = IndexMap::new();
    let mut invalidated: IndexSet<String> = IndexSet::new();
    let mut first_offender: Option<(String, String)> = None;

    for (name, rec) in s.env() {
        let mut kept_fields = Vec::new();
        for f in rec.fields() {
            if keep_set.contains(&f.label) {
                kept_fields.push(f.clone());
            } else if f.min >= 1 {
                if first_offender.is_none() {
                    first_offender = Some((f.label.clone(), name.clone()));
                }
                invalidated.insert(name.clone());
            }
        }
        trimmed.insert(
            name.clone(),
            Record::new(kept_fields).expect(
                "dropping fields from an already-valid Record cannot introduce a duplicate label",
            ),
        );
    }

    // Step 3: propagate invalidation -- a record with a mandatory field
    // typed to an invalidated record is itself invalidated. Least fixpoint,
    // same shape as prune's satisfiable_set.
    let mut changed = true;
    while changed {
        changed = false;
        for (name, rec) in &trimmed {
            if invalidated.contains(name) {
                continue;
            }
            for f in rec.fields() {
                if f.min >= 1
                    && let FieldType::Ref(r) = &f.ty
                    && invalidated.contains(&r.name)
                {
                    invalidated.insert(name.clone());
                    changed = true;
                    break;
                }
            }
        }
    }

    // Step 4: root invalidated -> no valid subschema.
    if invalidated.contains(&s.root().name) {
        let (label, record_name) = first_offender.expect(
            "root invalidated implies step 1 recorded an offender before propagation began",
        );
        return Err(SchemaError::new(
            format!("{record_name}.{label}"),
            "algebra.extract-invalidates-root",
            format!(
                "no valid subschema: removing label {label:?} deletes a mandatory field of record {record_name:?}"
            ),
        ));
    }

    // Step 5: drop invalidated records and any fields (mandatory or not)
    // that still point at one.
    let mut new_env: IndexMap<String, Record> = IndexMap::new();
    for (name, rec) in &trimmed {
        if invalidated.contains(name) {
            continue;
        }
        let fields: Vec<_> = rec
            .fields()
            .iter()
            .filter(|f| !matches!(&f.ty, FieldType::Ref(r) if invalidated.contains(&r.name)))
            .cloned()
            .collect();
        new_env.insert(
            name.clone(),
            Record::new(fields).expect(
                "dropping fields typed to an already-invalidated record cannot introduce a duplicate label",
            ),
        );
    }

    let result = Schema::new(Ref::new(s.root().name.clone()), new_env).expect(
        "dropping only invalidated records/fields leaves every surviving Ref resolvable within \
         the surviving env",
    );
    Ok(normalize(&prune(&result)))
}

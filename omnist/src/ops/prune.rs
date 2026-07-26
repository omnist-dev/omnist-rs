//! Satisfiability analysis and schema pruning. Ported from
//! `~/dev/omnist/omnist/ops/prune.py`.
//!
//! A record is *satisfiable* iff it admits at least one finite document, and
//! [`prune`] returns an equivalent schema with everything that can never
//! match removed. Satisfiability is a least fixpoint over the env's records:
//! a record is satisfiable iff every field with `min >= 1` is either a
//! `Scalar` or a `Ref` to a satisfiable record (fields with `min == 0` never
//! block satisfiability -- they simply need not be emitted).

use indexmap::{IndexMap, IndexSet};

use crate::schema::{Field, FieldType, Record, Ref, Schema};

/// The set of env record names that admit at least one finite document.
///
/// Least fixpoint: start with nothing known-satisfiable and repeatedly add
/// any record all of whose mandatory (`min >= 1`) fields are already
/// satisfiable. Monotonic on a finite env, so this always terminates. Only
/// ever queried by membership (never iterated for its own order), so an
/// `IndexSet` is used purely for the "no `HashMap`/`HashSet`" house style,
/// not because iteration order matters here.
pub fn satisfiable_set(s: &Schema) -> IndexSet<String> {
    let mut sat: IndexSet<String> = IndexSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for (name, rec) in s.env() {
            if sat.contains(name) {
                continue;
            }
            if record_satisfiable(rec, &sat) {
                sat.insert(name.clone());
                changed = true;
            }
        }
    }
    sat
}

fn record_satisfiable(rec: &Record, sat: &IndexSet<String>) -> bool {
    for f in rec.fields() {
        if f.min < 1 {
            continue;
        }
        if let FieldType::Ref(r) = &f.ty
            && !sat.contains(&r.name)
        {
            return false;
        }
    }
    true
}

/// True iff `s`'s root record is unsatisfiable -- the schema's language (the
/// set of documents it accepts) is empty.
pub fn is_empty(s: &Schema) -> bool {
    !satisfiable_set(s).contains(&s.root().name)
}

/// An equivalent schema with everything that can never match removed:
/// records unreachable from root are dropped; fields with `max == 0` are
/// dropped; optional (`min == 0`) fields whose type is an unsatisfiable
/// record are dropped; records left unreachable/unsatisfiable after the
/// above are dropped from the environment too.
///
/// **Root-unsatisfiable case.** If the root record itself is unsatisfiable
/// (`is_empty` is true), field pruning is *not* applied to the root: its
/// mandatory fields are exactly what make it unsatisfiable, and stripping
/// them would silently produce a *different*, satisfiable schema. Instead
/// the root record is kept as-is and only the rest of the environment is
/// reduced to what's reachable from it.
///
/// **Environment order (omnist-ts#56).** The returned environment iterates
/// `s.env()` in its own declaration order, filtered to `reachable` -- not
/// the other way round. `IndexSet::contains` is a membership check only;
/// iterating the *set* itself instead of the schema's own `IndexMap` is
/// exactly the bug TS's port had (traversal order leaking into the output
/// instead of preserving the input's authored order).
pub fn prune(s: &Schema) -> Schema {
    let sat = satisfiable_set(s);
    let root_ok = sat.contains(&s.root().name);
    let reachable = reachable_from_root(s, &sat, root_ok);

    let mut new_env: IndexMap<String, Record> = IndexMap::new();
    for (name, rec) in s.env() {
        if !reachable.contains(name) {
            continue;
        }
        if !root_ok && *name == s.root().name {
            new_env.insert(name.clone(), rec.clone());
        } else {
            new_env.insert(name.clone(), prune_record(rec, &sat));
        }
    }
    Schema::new(Ref::new(s.root().name.clone()), new_env).expect(
        "prune only drops unreachable records and never-emittable/unsatisfiable-optional \
         fields; every surviving Ref still resolves within the surviving env",
    )
}

fn reachable_from_root(s: &Schema, sat: &IndexSet<String>, root_ok: bool) -> IndexSet<String> {
    let mut seen: IndexSet<String> = IndexSet::new();
    let mut stack = vec![s.root().name.clone()];
    while let Some(name) = stack.pop() {
        if seen.contains(&name) {
            continue;
        }
        // Every name here is either the root or a Ref target found on an
        // already-visited record -- `Schema::new`'s `check_refs` guarantees
        // both always resolve, so a fallible lookup is dead code (see
        // `lint::reachable`'s identical note).
        let rec = s
            .env()
            .get(&name)
            .expect("Schema's own invariant: every Ref target resolves within its env");
        seen.insert(name.clone());
        let is_unpruned_root = name == s.root().name && !root_ok;
        for f in rec.fields() {
            if !is_unpruned_root {
                if f.max == Some(0) {
                    continue;
                }
                if f.min == 0
                    && let FieldType::Ref(r) = &f.ty
                    && !sat.contains(&r.name)
                {
                    continue;
                }
            }
            if let FieldType::Ref(r) = &f.ty {
                stack.push(r.name.clone());
            }
        }
    }
    seen
}

fn prune_record(rec: &Record, sat: &IndexSet<String>) -> Record {
    let kept: Vec<Field> = rec
        .fields()
        .iter()
        .filter(|f| {
            if f.max == Some(0) {
                return false;
            }
            if f.min == 0
                && let FieldType::Ref(r) = &f.ty
                && !sat.contains(&r.name)
            {
                return false;
            }
            true
        })
        .cloned()
        .collect();
    Record::new(kept).expect(
        "filtering fields out of an already-valid Record cannot introduce a duplicate label",
    )
}

//! Schema minimization: partition-refinement to the canonical minimal form.
//! Ported from `~/dev/omnist/omnist/ops/minimize.py`.
//!
//! `normalize(s)` returns an equivalent schema with the *fewest possible*
//! env records, unique up to record naming (paper Theorems 3-4).
//!
//! Algorithm:
//!
//! 1. `s = prune(s)` -- mandatory first step. Two semantically-equal records
//!    must not be kept apart by never-emittable fields or unreachable
//!    records; pruning first is what makes the partition canonical.
//! 2. **Initial partition**: env records grouped by `local_signature` -- a
//!    target-blind structural key, so records that might turn out
//!    equivalent via differently-named ref targets still start in the same
//!    block.
//! 3. **Refine**: split any block whose members disagree, for some label,
//!    on which *block* their same-labeled ref-typed field points to. Repeat
//!    until no block splits (a fixpoint -- always reached on a finite env).
//! 4. **Merge**: collapse each stable block to a single representative --
//!    its lexicographically smallest member name (deterministic) -- and
//!    remap every ref and the root to representatives.
//!
//! Special case: an unsatisfiable (empty-language) root. `prune` deliberately
//! leaves such a root's fields untouched (see its own doc comment), so
//! partition refinement over the unsatisfiable core isn't meaningful --
//! `normalize` just returns the pruned schema unchanged in that case.

use std::hash::Hash;

use indexmap::IndexMap;

use crate::schema::{Field, FieldType, Record, Ref, Schema};

use super::prune::{is_empty, prune};
use super::signature::{LocalSignature, local_signature};

/// Partitions `s.env`'s record names into structural-equivalence classes via
/// MinimizeSA-style partition refinement (module doc comment, steps 2-3):
/// an initial `local_signature` grouping refined to a fixpoint by which
/// *block* each same-labeled ref field points to.
///
/// Operates on `s.env` exactly as given -- it does **not** prune first, so
/// unreachable or unsatisfiable records are still classified. [`normalize`]
/// calls this after its own prune/is_empty steps; `lint` calls it on the raw
/// schema so duplicates are reported as authored. Each returned block is a
/// list of names; a block of length > 1 is a set of records with identical
/// structure.
pub fn equivalence_classes(s: &Schema) -> Vec<Vec<String>> {
    let mut names: Vec<String> = s.env().keys().cloned().collect();
    names.sort();

    let mut blocks: Vec<Vec<String>> = group_by(&names, |n| {
        local_signature(s.env().get(n).expect("n comes from s.env's own keys"))
    });
    let mut block_of: IndexMap<String, usize> = IndexMap::new();
    for (i, block) in blocks.iter().enumerate() {
        for n in block {
            block_of.insert(n.clone(), i);
        }
    }

    loop {
        let mut new_blocks: Vec<Vec<String>> = Vec::new();
        let mut new_block_of: IndexMap<String, usize> = IndexMap::new();
        for block in &blocks {
            let subs = group_by(block, |n| {
                refine_key(
                    s.env().get(n).expect("n comes from s.env's own keys"),
                    &block_of,
                )
            });
            for sub in subs {
                let idx = new_blocks.len();
                for n in &sub {
                    new_block_of.insert(n.clone(), idx);
                }
                new_blocks.push(sub);
            }
        }
        let changed = new_blocks.len() != blocks.len();
        blocks = new_blocks;
        block_of = new_block_of;
        if !changed {
            return blocks;
        }
    }
}

/// The canonical minimal schema equivalent to `s`: fewest env records,
/// unique up to record naming. See the module doc comment for the algorithm
/// (paper's Algorithm 2, MinimizeSA).
pub fn normalize(s: &Schema) -> Schema {
    let pruned = prune(s);
    if is_empty(&pruned) {
        return pruned;
    }

    let mut names: Vec<String> = pruned.env().keys().cloned().collect();
    names.sort();
    let blocks = equivalence_classes(&pruned);

    let mut rep: IndexMap<String, String> = IndexMap::new();
    for block in &blocks {
        let keep = block
            .iter()
            .min()
            .expect("equivalence_classes never returns an empty block")
            .clone();
        for n in block {
            rep.insert(n.clone(), keep.clone());
        }
    }

    let mut new_env: IndexMap<String, Record> = IndexMap::new();
    for name in &names {
        if rep.get(name) == Some(name) {
            new_env.insert(
                name.clone(),
                remap(
                    pruned
                        .env()
                        .get(name)
                        .expect("name comes from pruned.env's own keys"),
                    &rep,
                ),
            );
        }
    }
    // `rep` is built from `equivalence_classes(&pruned)`, which partitions
    // *every* name in `pruned.env()` into exactly one block -- and
    // `pruned.root().name` is always a key of `pruned.env()` (Schema's own
    // invariant: the root always resolves). So `rep` always has an entry
    // for it; a fallback here would be dead code (see `lint::reachable`'s
    // identical note on this pattern).
    let new_root_name = rep
        .get(&pruned.root().name)
        .cloned()
        .expect("every env record name, including the root's, is classified into rep");
    Schema::new(Ref::new(new_root_name), new_env)
        .expect("normalize only remaps refs to representative names that stay present in new_env")
}

fn group_by<K, F>(names: &[String], key_fn: F) -> Vec<Vec<String>>
where
    K: Eq + Hash,
    F: Fn(&String) -> K,
{
    let mut groups: IndexMap<K, Vec<String>> = IndexMap::new();
    for n in names {
        groups.entry(key_fn(n)).or_default().push(n.clone());
    }
    groups.into_values().collect()
}

/// A record's refinement key: its target-blind local signature, plus -- for
/// each field in label order -- the current block id of its ref target (or
/// `None` for a scalar field). Two records land in the same refined block
/// only if they agree on both.
type RefineKey = (
    LocalSignature,
    Vec<(String, usize, Option<usize>, Option<usize>)>,
);

fn refine_key(rec: &Record, block_of: &IndexMap<String, usize>) -> RefineKey {
    let mut fields: Vec<(String, usize, Option<usize>, Option<usize>)> = rec
        .fields()
        .iter()
        .map(|f| {
            let blk = match &f.ty {
                FieldType::Ref(r) => Some(
                    *block_of
                        .get(&r.name)
                        .expect("every ref target is classified before refine_key runs on it"),
                ),
                FieldType::Scalar(_) => None,
            };
            (f.label.clone(), f.min, f.max, blk)
        })
        .collect();
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    (local_signature(rec), fields)
}

fn remap(rec: &Record, rep: &IndexMap<String, String>) -> Record {
    let fields: Vec<Field> = rec
        .fields()
        .iter()
        .map(|f| {
            let ty = match &f.ty {
                // `remap` is only ever called on a record still present in
                // `pruned.env()`, and every Ref-typed field on it targets
                // another name in that same env (Schema's own invariant) --
                // which `rep` classifies for every name. A fallback here
                // would be dead code, same reasoning as `new_root_name`
                // above.
                FieldType::Ref(r) => FieldType::Ref(Ref::new(
                    rep.get(&r.name)
                        .cloned()
                        .expect("every ref target is classified into rep"),
                )),
                FieldType::Scalar(s) => FieldType::Scalar(*s),
            };
            Field::new(f.label.clone(), ty, f.min, f.max)
                .expect("remapping a ref target name changes neither label nor cardinality")
        })
        .collect();
    Record::new(fields)
        .expect("remap doesn't add/remove/rename fields, so it cannot introduce a duplicate label")
}

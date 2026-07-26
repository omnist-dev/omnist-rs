//! Non-destructive structural diagnostics for a schema. Ported from
//! `~/dev/omnist/omnist/ops/lint.py`.
//!
//! `Schema::validate` checks a *document* against a schema; `lint` checks
//! the *schema itself* for structural problems that parse fine but mean
//! parts of the schema can never do anything. It **reports, never
//! mutates** -- `prune`/`normalize` are the transforms that fix these
//! issues; `lint` only diagnoses them.
//!
//! Three checks (the Python reference's fourth, `any-field`, has no
//! counterpart here -- see `super`'s module doc comment on why `AnyType`
//! is out of scope for this port):
//!
//! * `unsatisfiable-record` (`warning`) -- a reachable record no finite
//!   document can match (e.g. a mandatory ref cycle). Reuses
//!   [`super::prune::satisfiable_set`] (its complement), intersected with
//!   reachable.
//! * `unreachable-record` (`warning`) -- a record defined in the env but not
//!   reachable from root by following any ref. A plain reachability walk
//!   (no pruning): every `Ref`-typed field is followed regardless of
//!   cardinality.
//! * `duplicate-record` (`warning`) -- two or more structurally identical
//!   records under different names. Reuses
//!   [`super::minimize::equivalence_classes`] on the *raw* schema, so
//!   duplicates are reported as authored.

use indexmap::IndexSet;

use crate::schema::{FieldType, Schema};

use super::minimize::equivalence_classes;
use super::prune::satisfiable_set;

/// One structural diagnostic. `code` is a stable machine-readable
/// identifier (`unsatisfiable-record`, `unreachable-record`,
/// `duplicate-record`); `severity` is `warning` or `info`; `location` is a
/// record name; `message` is a human-readable, actionable description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintFinding {
    pub code: &'static str,
    pub severity: &'static str,
    pub location: String,
    pub message: String,
}

/// Record names reachable from `s`'s root by a plain walk following every
/// `Ref`-typed field -- no pruning, cardinality ignored. A record reachable
/// only via an optional or unsatisfiable field still counts as referenced.
fn reachable(s: &Schema) -> IndexSet<String> {
    let mut seen: IndexSet<String> = IndexSet::new();
    let mut stack = vec![s.root().name.clone()];
    while let Some(name) = stack.pop() {
        if seen.contains(&name) {
            continue;
        }
        // Every name pushed onto `stack` is either `s.root().name` or a
        // `Ref` target found on an already-visited record's fields --
        // `Schema::new`'s `check_refs` guarantees both always resolve
        // within `s.env()`, so `.get` here can never miss. A fallible
        // `if let ... else { continue }` here would be dead code `cargo
        // llvm-cov` correctly flags as unreachable (see oml.rs's
        // `scan_number` for the same pattern), so it's replaced with an
        // `expect` documenting the invariant instead.
        let rec = s
            .env()
            .get(&name)
            .expect("Schema's own invariant: every Ref target resolves within its env");
        seen.insert(name.clone());
        for f in rec.fields() {
            if let FieldType::Ref(r) = &f.ty {
                stack.push(r.name.clone());
            }
        }
    }
    seen
}

/// Structural diagnostics for `s` -- see the module doc comment for the
/// checks. Returns findings sorted deterministically by `(code, location)`.
/// Never mutates `s`.
pub fn lint(s: &Schema) -> Vec<LintFinding> {
    let mut findings: Vec<LintFinding> = Vec::new();

    let reach = reachable(s);
    let sat = satisfiable_set(s);

    // unsatisfiable-record: reachable but not satisfiable. Iteration order
    // here doesn't matter for determinism -- the final `.sort_by` below is
    // what makes the output canonical, matching the Python reference's own
    // set-difference-then-sort shape.
    for name in &reach {
        if !sat.contains(name) {
            findings.push(LintFinding {
                code: "unsatisfiable-record",
                severity: "warning",
                location: name.clone(),
                message: format!(
                    "record {name:?} is reachable but unsatisfiable -- no finite document \
                     can match it (e.g. a mandatory ref cycle)"
                ),
            });
        }
    }

    // unreachable-record: defined in env but not reachable from root.
    for name in s.env().keys() {
        if !reach.contains(name) {
            findings.push(LintFinding {
                code: "unreachable-record",
                severity: "warning",
                location: name.clone(),
                message: format!(
                    "record {name:?} is defined but never reachable from the root; drop it \
                     with `schema prune`"
                ),
            });
        }
    }

    // duplicate-record: structurally identical records under different
    // names.
    for block in equivalence_classes(s) {
        if block.len() > 1 {
            let mut group = block.clone();
            group.sort();
            let location = group.join(", ");
            let keep = group[0].clone();
            let others: Vec<String> = group[1..].iter().map(|n| format!("{n:?}")).collect();
            findings.push(LintFinding {
                code: "duplicate-record",
                severity: "warning",
                location,
                message: format!(
                    "records {} are structurally identical to {keep:?}; merge them with \
                     `schema normalize`",
                    others.join(", ")
                ),
            });
        }
    }

    // Canonical ordering -- codepoint (byte-wise) comparison via `str`'s
    // default `Ord`, never locale-aware. See `tests.rs`'s omnist-ts#56
    // regression test.
    findings.sort_by(|a, b| (a.code, &a.location).cmp(&(b.code, &b.location)));
    findings
}

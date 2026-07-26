//! Schema algebra: `prune`, `minimize`/`normalize`, `subschema`, `extract`,
//! `lint`, `isomorphic`, `signature` -- ported from `~/dev/omnist/omnist/ops/`
//! (issue #12), one module per op (matching the Python layout, which is
//! already a reasonable structure per "architecture freedom").
//!
//! ## The `any` type is out of scope here
//!
//! The Python reference's `schema.py` has a fourth type kind, `AnyType`
//! (accepts any value unchecked), threaded through every op in this family.
//! `omnist-rs`'s [`crate::schema`] (issue #6) does not have an `AnyType`
//! equivalent -- whether to add one is an explicitly deferred open design
//! question upstream (not yet decided for this port), so every op below is
//! written against the two-kind [`crate::schema::FieldType`] (`Scalar` /
//! `Ref`) that issue #6 actually shipped. `lint`'s Python `any-field` check
//! has no Rust counterpart for the same reason -- there is nothing to
//! inventory yet.
//!
//! ## Determinism
//!
//! Every op here produces a "canonical form" (a pruned/minimized schema, a
//! sorted lint report, a deterministic equivalence-class partition). None of
//! this module's code uses `std::collections::HashMap`/`HashSet` --
//! `indexmap`'s `IndexMap`/`IndexSet` everywhere an ordered structure is
//! needed, and an explicit `.sort()` everywhere the Python reference itself
//! sorts for canonical output (see `signature::local_signature`, `lint::lint`,
//! `minimize::normalize`). See `tests.rs` for the repeated-run determinism
//! proof and the `omnist-ts#56` ordering-regression tests (codepoint, not
//! locale, order; `prune`'s declaration-order environment reconstruction).

pub mod extract;
pub mod isomorphic;
pub mod lint;
pub mod minimize;
pub mod prune;
pub mod signature;
pub mod subschema;

pub use extract::extract;
pub use isomorphic::is_isomorphic;
pub use lint::{LintFinding, lint};
pub use minimize::{equivalence_classes, normalize};
pub use prune::{is_empty, prune, satisfiable_set};
pub use signature::local_signature;
pub use subschema::{compatible_with, equivalent};

#[cfg(test)]
mod tests;

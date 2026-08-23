//! Schema algebra: `prune`, `minimize`/`normalize`, `subschema`, `extract`,
//! `lint`, `isomorphic`, `signature` -- ported from `~/dev/omnist/omnist/ops/`
//! (issue #12), one module per op (matching the Python layout, which is
//! already a reasonable structure per "architecture freedom").
//!
//! ## The `any` type
//!
//! [`crate::schema::FieldType`] has three kinds: `Scalar`, `Ref`, and `Any`
//! (issue #29). Every op in this family treats `Any` the same way the
//! Python reference's `AnyType` is treated in the corresponding op:
//! satisfiability treats it like a `Scalar` (always satisfiable, never
//! blocks a mandatory field); `local_signature`/minimize give it its own
//! target-blind shape key (`("any",)`); `subschema` treats `any` on the
//! superschema side as absorbing everything, and `any` only on the
//! subschema side as never compatible with a non-`any` target; `lint`'s
//! `lint.any-field` check inventories every `Any`-typed field in the schema.
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

//! Tests for the schema-algebra ops (issue #12).

use indexmap::IndexMap;

use crate::schema::{
    BOOLEAN, Field, FieldType, INTEGER, NUMBER, Record, Ref, STRING, Schema, nullable,
};

use super::*;

// ---------------------------------------------------------------------------
// Schema-building helpers
// ---------------------------------------------------------------------------

fn env(pairs: Vec<(&str, Record)>) -> IndexMap<String, Record> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn rec(fields: Vec<Field>) -> Record {
    Record::new(fields).unwrap()
}

fn f(label: &str, ty: impl Into<FieldType>, min: usize, max: Option<usize>) -> Field {
    Field::new(label, ty, min, max).unwrap()
}

fn req(label: &str, ty: impl Into<FieldType>) -> Field {
    Field::required(label, ty).unwrap()
}

fn opt(label: &str, ty: impl Into<FieldType>) -> Field {
    Field::new(label, ty, 0, Some(1)).unwrap()
}

// ---------------------------------------------------------------------------
// signature
// ---------------------------------------------------------------------------

#[test]
fn local_signature_sorts_by_label_and_excludes_ref_target_names() {
    let a = rec(vec![req("z", Ref::new("Other")), req("a", STRING)]);
    let b = rec(vec![req("z", Ref::new("Different")), req("a", STRING)]);
    assert_eq!(
        signature::local_signature(&a),
        signature::local_signature(&b)
    );
}

#[test]
fn local_signature_distinguishes_scalar_kind_and_nullability() {
    let a = rec(vec![req("x", STRING)]);
    let b = rec(vec![req("x", INTEGER)]);
    let c = rec(vec![req("x", nullable(STRING))]);
    assert_ne!(
        signature::local_signature(&a),
        signature::local_signature(&b)
    );
    assert_ne!(
        signature::local_signature(&a),
        signature::local_signature(&c)
    );
}

#[test]
fn local_signature_gives_any_its_own_shape_key_distinct_from_scalar_and_ref() {
    let any_rec = rec(vec![req("x", FieldType::Any)]);
    let scalar_rec = rec(vec![req("x", STRING)]);
    let ref_rec = rec(vec![req("x", Ref::new("Other"))]);
    assert_ne!(
        signature::local_signature(&any_rec),
        signature::local_signature(&scalar_rec)
    );
    assert_ne!(
        signature::local_signature(&any_rec),
        signature::local_signature(&ref_rec)
    );
}

// ---------------------------------------------------------------------------
// prune
// ---------------------------------------------------------------------------

#[test]
fn prune_drops_unreachable_records() {
    let e = env(vec![
        ("Root", rec(vec![req("x", STRING)])),
        ("Unreachable", rec(vec![req("y", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let pruned = prune::prune(&s);
    assert!(pruned.env().contains_key("Root"));
    assert!(!pruned.env().contains_key("Unreachable"));
}

#[test]
fn prune_drops_never_emittable_fields() {
    let e = env(vec![(
        "Root",
        rec(vec![f("dead", STRING, 0, Some(0)), req("x", STRING)]),
    )]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let pruned = prune::prune(&s);
    assert!(pruned.env().get("Root").unwrap().field("dead").is_none());
    assert!(pruned.env().get("Root").unwrap().field("x").is_some());
}

#[test]
fn prune_drops_optional_field_to_unsatisfiable_record() {
    // Cyclic, all-mandatory record: never satisfiable.
    let e = env(vec![
        (
            "Root",
            rec(vec![opt("child", Ref::new("Bad")), req("x", STRING)]),
        ),
        ("Bad", rec(vec![req("self", Ref::new("Bad"))])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let pruned = prune::prune(&s);
    assert!(pruned.env().get("Root").unwrap().field("child").is_none());
    assert!(!pruned.env().contains_key("Bad"));
}

#[test]
fn prune_keeps_unsatisfiable_root_fields_intact() {
    let e = env(vec![("Root", rec(vec![req("self", Ref::new("Root"))]))]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    assert!(prune::is_empty(&s));
    let pruned = prune::prune(&s);
    assert!(prune::is_empty(&pruned));
    assert!(pruned.env().get("Root").unwrap().field("self").is_some());
}

/// omnist-ts#56 regression: prune's environment reconstruction must follow
/// the *schema's own* declaration order, not the satisfiable/reachable
/// set's iteration order.
#[test]
fn prune_environment_order_matches_declaration_order() {
    let e = env(vec![
        ("Zeta", rec(vec![req("x", STRING)])),
        ("Alpha", rec(vec![req("x", STRING)])),
        (
            "Root",
            rec(vec![
                req("z", Ref::new("Zeta")),
                req("a", Ref::new("Alpha")),
            ]),
        ),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let pruned = prune::prune(&s);
    let names: Vec<&String> = pruned.env().keys().collect();
    assert_eq!(names, vec!["Zeta", "Alpha", "Root"]);
}

// ---------------------------------------------------------------------------
// isomorphic
// ---------------------------------------------------------------------------

#[test]
fn isomorphic_schemas_with_renamed_records() {
    let e_a = env(vec![("A", rec(vec![req("x", STRING)]))]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(isomorphic::is_isomorphic(&a, &b));
}

#[test]
fn non_isomorphic_schemas_differ_in_field_shape() {
    let e_a = env(vec![("A", rec(vec![req("x", STRING)]))]);
    let e_b = env(vec![("B", rec(vec![req("x", INTEGER)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(!isomorphic::is_isomorphic(&a, &b));
}

#[test]
fn both_empty_schemas_are_isomorphic() {
    let e_a = env(vec![("A", rec(vec![req("self", Ref::new("A"))]))]);
    let e_b = env(vec![(
        "B",
        rec(vec![req("x", Ref::new("B")), req("self", Ref::new("B"))]),
    )]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(prune::is_empty(&a) && prune::is_empty(&b));
    assert!(isomorphic::is_isomorphic(&a, &b));
}

#[test]
fn only_one_empty_schema_is_not_isomorphic() {
    let e_a = env(vec![("A", rec(vec![req("self", Ref::new("A"))]))]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(!isomorphic::is_isomorphic(&a, &b));
}

#[test]
fn isomorphic_handles_ref_cycles_via_bijection() {
    let e_a = env(vec![(
        "A",
        rec(vec![opt("next", Ref::new("A")), req("v", STRING)]),
    )]);
    let e_b = env(vec![(
        "Node",
        rec(vec![opt("next", Ref::new("Node")), req("v", STRING)]),
    )]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("Node"), e_b).unwrap();
    assert!(isomorphic::is_isomorphic(&a, &b));
}

/// A mismatch that's only visible after recursing into a ref target -- the
/// two roots' own `local_signature`s match (same single ref-typed field),
/// so the walk must actually descend to `P1`/`P2` to find the disagreement.
#[test]
fn isomorphic_false_when_mismatch_found_only_by_recursing_into_a_ref() {
    let e_a = env(vec![
        ("RootA", rec(vec![req("p", Ref::new("P1"))])),
        ("P1", rec(vec![req("v", STRING)])),
    ]);
    let e_b = env(vec![
        ("RootB", rec(vec![req("p", Ref::new("P2"))])),
        ("P2", rec(vec![req("v", INTEGER)])),
    ]);
    let a = Schema::new(Ref::new("RootA"), e_a).unwrap();
    let b = Schema::new(Ref::new("RootB"), e_b).unwrap();
    assert!(!isomorphic::is_isomorphic(&a, &b));
}

// ---------------------------------------------------------------------------
// minimize / isomorphic joint (triple-check strategy)
// ---------------------------------------------------------------------------

/// The triple-check oracle: minimize's result must (a) accept exactly the
/// same documents as the input, verified via the *independent*
/// `subschema::equivalent` algorithm (bidirectional inclusion, unrelated to
/// partition refinement), and (b) already be a fixpoint of minimization --
/// `is_isomorphic` (also independent of both) must find it isomorphic to
/// re-minimizing it. A literal `is_isomorphic(s, minimize(s))` is *not* the
/// right check when `s` itself has redundant structurally-identical
/// records: minimize legitimately shrinks the record count then, so the two
/// env graphs are no longer size-equal (never isomorphic in the strict
/// name-bijection sense), even though they remain semantically equivalent
/// -- exactly DFA minimization vs. the original, unminimized DFA. This
/// mirrors the Python reference's own test suite, which always calls
/// `_isomorphic(s.normalize(), t.normalize())`, never on a raw schema.
fn assert_minimize_preserves_semantics_and_reaches_a_fixpoint(s: &Schema) {
    let m = minimize::normalize(s);
    assert!(
        subschema::equivalent(s, &m),
        "minimize(s) must accept exactly the same documents as s"
    );
    let m2 = minimize::normalize(&m);
    assert!(
        isomorphic::is_isomorphic(&m, &m2),
        "re-minimizing an already-minimal schema must reach the same (isomorphic) fixpoint"
    );
}

#[test]
fn minimize_merges_structurally_identical_records() {
    let e = env(vec![
        (
            "Root",
            rec(vec![req("a", Ref::new("A")), req("b", Ref::new("B"))]),
        ),
        ("A", rec(vec![req("x", STRING)])),
        ("B", rec(vec![req("x", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let m = minimize::normalize(&s);
    // A and B are structurally identical -> collapse to one record.
    assert_eq!(m.env().len(), 2);
    assert_minimize_preserves_semantics_and_reaches_a_fixpoint(&s);
}

#[test]
fn minimize_merges_records_that_only_differ_by_any_field_naming() {
    let e = env(vec![
        (
            "Root",
            rec(vec![req("a", Ref::new("A")), req("b", Ref::new("B"))]),
        ),
        ("A", rec(vec![req("x", FieldType::Any)])),
        ("B", rec(vec![req("x", FieldType::Any)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let m = minimize::normalize(&s);
    assert_eq!(m.env().len(), 2);
    // A and B collapse to one record; its field is still `any` (remap is a
    // passthrough for non-ref types).
    let root_rec = &m.env()[&m.root().name];
    let FieldType::Ref(target) = &root_rec.field("a").unwrap().ty else {
        panic!("expected a ref field");
    };
    let merged = &m.env()[&target.name];
    assert_eq!(merged.field("x").unwrap().ty, FieldType::Any);
    assert_minimize_preserves_semantics_and_reaches_a_fixpoint(&s);
}

#[test]
fn minimize_is_a_no_op_on_an_already_minimal_schema() {
    let e = env(vec![("Root", rec(vec![req("x", STRING)]))]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let m = minimize::normalize(&s);
    assert_eq!(m.env().len(), 1);
    assert_minimize_preserves_semantics_and_reaches_a_fixpoint(&s);
}

#[test]
fn minimize_on_unsatisfiable_root_returns_pruned_schema_unchanged() {
    let e = env(vec![("Root", rec(vec![req("self", Ref::new("Root"))]))]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let m = minimize::normalize(&s);
    assert!(prune::is_empty(&m));
    assert_minimize_preserves_semantics_and_reaches_a_fixpoint(&s);
}

#[test]
fn minimize_handles_ref_cycles() {
    // Two isomorphic-but-differently-named cyclic linked lists sharing the
    // same shape; each should minimize down to a single self-referential
    // record.
    let e_a = env(vec![
        (
            "Root",
            rec(vec![opt("next", Ref::new("Mid")), req("v", STRING)]),
        ),
        (
            "Mid",
            rec(vec![opt("next", Ref::new("Root")), req("v", STRING)]),
        ),
    ]);
    let a = Schema::new(Ref::new("Root"), e_a).unwrap();
    let m = minimize::normalize(&a);
    assert_eq!(m.env().len(), 1);
    assert_minimize_preserves_semantics_and_reaches_a_fixpoint(&a);
}

#[test]
fn minimize_equivalence_classes_groups_duplicates() {
    let e = env(vec![
        (
            "Root",
            rec(vec![
                req("a", Ref::new("A")),
                req("b", Ref::new("B")),
                req("c", Ref::new("C")),
            ]),
        ),
        ("A", rec(vec![req("x", STRING)])),
        ("B", rec(vec![req("x", STRING)])),
        ("C", rec(vec![req("x", INTEGER)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let blocks = minimize::equivalence_classes(&s);
    let dup_block: Vec<&Vec<String>> = blocks.iter().filter(|b| b.len() > 1).collect();
    assert_eq!(dup_block.len(), 1);
    let mut names = dup_block[0].clone();
    names.sort();
    assert_eq!(names, vec!["A".to_string(), "B".to_string()]);
}

/// Forces `equivalence_classes`'s refine loop through more than one
/// iteration: `X` and `Y` share a `local_signature` initially (both have a
/// single ref-typed field `p`), so they start in the same block, and are
/// only split once refinement notices their `p` targets (`P`/`Q`) land in
/// different blocks -- which itself only happened because `P`/`Q` already
/// differ by scalar kind at the *initial* partition. Without this test the
/// refine loop's "still changing, keep looping" branch was never exercised.
#[test]
fn minimize_equivalence_classes_needs_more_than_one_refine_pass() {
    let e = env(vec![
        (
            "Root",
            rec(vec![req("x", Ref::new("X")), req("y", Ref::new("Y"))]),
        ),
        ("X", rec(vec![req("p", Ref::new("P"))])),
        ("Y", rec(vec![req("p", Ref::new("Q"))])),
        ("P", rec(vec![req("v", STRING)])),
        ("Q", rec(vec![req("v", INTEGER)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let blocks = minimize::equivalence_classes(&s);
    // Every one of the 5 records is structurally distinct -- X and Y must
    // NOT collapse together despite sharing an initial local_signature.
    assert_eq!(blocks.len(), 5);
    let m = minimize::normalize(&s);
    assert_eq!(m.env().len(), 5);
    assert_minimize_preserves_semantics_and_reaches_a_fixpoint(&s);
}

// ---------------------------------------------------------------------------
// subschema
// ---------------------------------------------------------------------------

#[test]
fn compatible_with_widening_cardinality_and_integer_to_number() {
    let e_a = env(vec![("A", rec(vec![req("x", INTEGER)]))]);
    let e_b = env(vec![("B", rec(vec![f("x", NUMBER, 0, Some(3))]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(subschema::compatible_with(&a, &b));
    assert!(!subschema::compatible_with(&b, &a));
}

#[test]
fn compatible_with_any_on_the_b_side_absorbs_any_a_side_field() {
    // `any` on B always absorbs A's field, whatever A's field is (scalar,
    // ref, or any itself).
    let e_a = env(vec![
        ("A", rec(vec![req("x", Ref::new("X"))])),
        ("X", rec(vec![req("v", STRING)])),
    ]);
    let e_b = env(vec![("B", rec(vec![req("x", FieldType::Any)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_any_on_the_a_side_is_never_compatible_with_a_non_any_b() {
    let e_a = env(vec![("A", rec(vec![req("x", FieldType::Any)]))]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(!subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_any_on_both_sides_is_compatible() {
    let e_a = env(vec![("A", rec(vec![req("x", FieldType::Any)]))]);
    let e_b = env(vec![("B", rec(vec![req("x", FieldType::Any)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(subschema::compatible_with(&a, &b));
    assert!(subschema::equivalent(&a, &b));
}

#[test]
fn compatible_with_rejects_missing_required_field_in_b() {
    let e_a = env(vec![("A", rec(vec![req("x", STRING)]))]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING), req("y", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    // a has no "y" that b requires.
    assert!(!subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_rejects_narrower_nullability() {
    let e_a = env(vec![("A", rec(vec![req("x", nullable(STRING))]))]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(!subschema::compatible_with(&a, &b));
    assert!(subschema::compatible_with(&b, &a));
}

#[test]
fn compatible_with_vacuously_true_for_unsatisfiable_a() {
    let e_a = env(vec![("A", rec(vec![req("self", Ref::new("A"))]))]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_a_scalar_field_is_never_compatible_with_a_record_field() {
    let e_a = env(vec![("A", rec(vec![req("x", STRING)]))]);
    let e_b = env(vec![
        ("B", rec(vec![req("x", Ref::new("X"))])),
        ("X", rec(vec![req("v", STRING)])),
    ]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(!subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_skips_a_never_emitted_field_in_a() {
    // "z" has max == 0 in A -- A never emits it, so B needn't declare it at
    // all.
    let e_a = env(vec![(
        "A",
        rec(vec![req("x", STRING), f("z", STRING, 0, Some(0))]),
    )]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_skips_an_optional_field_typed_to_an_unsatisfiable_a_record() {
    // "bad" is optional in A and typed to an unsatisfiable (cyclic
    // mandatory) A-side record -- A can never actually emit it, so B
    // needn't declare it either.
    let e_a = env(vec![
        (
            "A",
            rec(vec![req("x", STRING), opt("bad", Ref::new("Cycle"))]),
        ),
        ("Cycle", rec(vec![req("self", Ref::new("Cycle"))])),
    ]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_rejects_a_field_b_does_not_declare_at_all() {
    let e_a = env(vec![(
        "A",
        rec(vec![req("x", STRING), req("extra", STRING)]),
    )]);
    let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    // B is closed and has no "extra" field.
    assert!(!subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_unbounded_b_max_accepts_a_bounded_a_max() {
    let e_a = env(vec![("A", rec(vec![f("x", STRING, 0, Some(2))]))]);
    let e_b = env(vec![("B", rec(vec![f("x", STRING, 0, None)]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(subschema::compatible_with(&a, &b));
}

#[test]
fn compatible_with_unbounded_a_max_is_incompatible_with_a_bounded_b_max() {
    let e_a = env(vec![("A", rec(vec![f("x", STRING, 0, None)]))]);
    let e_b = env(vec![("B", rec(vec![f("x", STRING, 0, Some(2))]))]);
    let a = Schema::new(Ref::new("A"), e_a).unwrap();
    let b = Schema::new(Ref::new("B"), e_b).unwrap();
    assert!(!subschema::compatible_with(&a, &b));
}

#[test]
fn equivalent_matches_isomorphic_oracle_across_random_pairs() {
    // Dual-algorithm oracle: equivalent(a, b) must agree with
    // is_isomorphic(normalize(a), normalize(b)) across a handful of
    // schema pairs, including a cyclic one.
    let cases: Vec<(Schema, Schema, bool)> = vec![
        {
            let e_a = env(vec![("A", rec(vec![req("x", STRING)]))]);
            let e_b = env(vec![("B", rec(vec![req("x", STRING)]))]);
            (
                Schema::new(Ref::new("A"), e_a).unwrap(),
                Schema::new(Ref::new("B"), e_b).unwrap(),
                true,
            )
        },
        {
            let e_a = env(vec![("A", rec(vec![req("x", STRING)]))]);
            let e_b = env(vec![("B", rec(vec![req("x", INTEGER)]))]);
            (
                Schema::new(Ref::new("A"), e_a).unwrap(),
                Schema::new(Ref::new("B"), e_b).unwrap(),
                false,
            )
        },
        {
            let e_a = env(vec![
                (
                    "Root",
                    rec(vec![opt("next", Ref::new("Mid")), req("v", STRING)]),
                ),
                (
                    "Mid",
                    rec(vec![opt("next", Ref::new("Root")), req("v", STRING)]),
                ),
            ]);
            let e_b = env(vec![(
                "N",
                rec(vec![opt("next", Ref::new("N")), req("v", STRING)]),
            )]);
            (
                Schema::new(Ref::new("Root"), e_a).unwrap(),
                Schema::new(Ref::new("N"), e_b).unwrap(),
                true,
            )
        },
    ];
    for (a, b, expected) in cases {
        let via_subschema = subschema::equivalent(&a, &b);
        let via_isomorphic =
            isomorphic::is_isomorphic(&minimize::normalize(&a), &minimize::normalize(&b));
        assert_eq!(via_subschema, expected);
        assert_eq!(via_isomorphic, expected);
        assert_eq!(via_subschema, via_isomorphic);
    }
}

// ---------------------------------------------------------------------------
// extract
// ---------------------------------------------------------------------------

#[test]
fn extract_keeps_only_requested_labels() {
    let e = env(vec![(
        "Root",
        rec(vec![req("keep", STRING), opt("drop", STRING)]),
    )]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let out = extract::extract(&s, &["keep"]).unwrap();
    let root = out.env().get(out.root().name.as_str()).unwrap();
    assert!(root.field("keep").is_some());
    assert!(root.field("drop").is_none());
}

#[test]
fn extract_errors_when_root_mandatory_field_is_dropped() {
    let e = env(vec![("Root", rec(vec![req("mandatory", STRING)]))]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let err = extract::extract(&s, &[]).unwrap_err();
    assert!(err.to_string().contains("no valid subschema"));
    assert!(err.to_string().contains("mandatory"));
}

#[test]
fn extract_propagates_invalidation_through_a_chain() {
    // "child" is kept (so Root's mandatory ref field to Child survives step
    // 1 untouched), but Child's own mandatory field "gone" is not kept --
    // Child is invalidated directly in step 1, then that invalidation must
    // *propagate* to Root in step 3 (Root's field to Child is itself
    // mandatory), not be caught by step 1 alone.
    let e = env(vec![
        ("Root", rec(vec![req("child", Ref::new("Child"))])),
        ("Child", rec(vec![req("gone", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let err = extract::extract(&s, &["child"]).unwrap_err();
    assert!(err.to_string().contains("no valid subschema"));
    assert!(err.to_string().contains("gone"));
}

/// A record invalidated only via a field that's *optional* in its parent:
/// the parent survives (it never actually requires that field), so
/// `extract` must succeed while still dropping the invalidated record and
/// the now-dangling optional ref field that pointed to it (step 5's
/// per-record skip for an already-invalidated name).
#[test]
fn extract_drops_an_invalidated_record_reached_only_optionally() {
    let e = env(vec![
        (
            "Root",
            rec(vec![req("keep", STRING), opt("child", Ref::new("Child"))]),
        ),
        ("Child", rec(vec![req("gone", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let out = extract::extract(&s, &["keep", "child"]).unwrap();
    assert!(!out.env().contains_key("Child"));
    let root = out.env().get(out.root().name.as_str()).unwrap();
    assert!(root.field("child").is_none());
}

/// Two independent step-1 drops (one per unreachable-but-still-in-env
/// record): `first_offender` must be recorded once, on the first one seen,
/// and left alone on the second -- the error message names the first
/// offender specifically, not whichever happened to be found last.
#[test]
fn extract_first_offender_is_recorded_only_once_across_multiple_invalidations() {
    let e = env(vec![
        ("Root", rec(vec![req("child", Ref::new("Child"))])),
        ("Child", rec(vec![req("gone", STRING)])),
        ("AlsoBad", rec(vec![req("also_gone", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let err = extract::extract(&s, &["child"]).unwrap_err();
    // "gone" (Child) is declared before "also_gone" (AlsoBad) in the env,
    // so it must be the one named, even though AlsoBad is also invalidated.
    assert!(err.to_string().contains("gone"));
    assert!(!err.to_string().contains("also_gone"));
}

#[test]
fn extract_result_is_pruned_and_normalized() {
    let e = env(vec![
        (
            "Root",
            rec(vec![
                req("keep", STRING),
                req("a", Ref::new("A")),
                req("b", Ref::new("B")),
            ]),
        ),
        ("A", rec(vec![req("x", STRING)])),
        ("B", rec(vec![req("x", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let out = extract::extract(&s, &["keep", "a", "b", "x"]).unwrap();
    // A and B are structurally identical -- normalize should have merged
    // them.
    assert_eq!(out.env().len(), 2);
}

// ---------------------------------------------------------------------------
// lint
// ---------------------------------------------------------------------------

#[test]
fn lint_flags_unreachable_and_unsatisfiable_and_duplicate_records() {
    let e = env(vec![
        (
            "Root",
            rec(vec![req("a", Ref::new("A")), req("bad", Ref::new("Bad"))]),
        ),
        ("A", rec(vec![req("x", STRING)])),
        ("Dup", rec(vec![req("x", STRING)])),
        ("Bad", rec(vec![req("self", Ref::new("Bad"))])),
        ("Orphan", rec(vec![req("x", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let findings = lint::lint(&s);
    let codes: Vec<&str> = findings.iter().map(|f| f.code).collect();
    assert!(codes.contains(&"lint.unsatisfiable-record"));
    assert!(codes.contains(&"lint.unreachable-record"));
    assert!(codes.contains(&"lint.duplicate-record"));
    assert!(findings.iter().any(|f| f.location == "Bad"));
    assert!(findings.iter().any(|f| f.location == "Orphan"));
}

#[test]
fn lint_inventories_any_typed_fields_as_info_findings() {
    let e = env(vec![("Root", rec(vec![req("x", FieldType::Any)]))]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let findings = lint::lint(&s);
    let f = findings
        .iter()
        .find(|f| f.code == "lint.any-field")
        .expect("expected an any-field finding");
    assert_eq!(f.severity, "info");
    assert_eq!(f.location, "Root.x");
    assert!(f.message.contains("typed `any`"));
}

#[test]
fn lint_reports_no_any_field_findings_when_none_exist() {
    let e = env(vec![("Root", rec(vec![req("x", STRING)]))]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let findings = lint::lint(&s);
    assert!(!findings.iter().any(|f| f.code == "lint.any-field"));
}

#[test]
fn lint_is_sorted_by_code_then_location() {
    let e = env(vec![
        ("Root", rec(vec![req("x", STRING)])),
        ("Zeta", rec(vec![req("x", STRING)])),
        ("Alpha", rec(vec![req("x", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let findings = lint::lint(&s);
    let mut sorted = findings.clone();
    sorted.sort_by(|a, b| (a.code, &a.location).cmp(&(b.code, &b.location)));
    assert_eq!(findings, sorted);
}

/// omnist-ts#56 regression: lint's ordering must be codepoint order (byte-
/// wise on UTF-8), not locale-aware (`localeCompare`-style) order, which
/// would sort e.g. accented/mixed-case labels differently. This mixes case
/// and a non-ASCII (accented) record name where codepoint and locale order
/// diverge for common locales.
#[test]
fn lint_ordering_is_codepoint_not_locale_non_ascii_mixed_case() {
    let e = env(vec![
        (
            "Root",
            rec(vec![
                req("a", Ref::new("aardvark")),
                req("b", Ref::new("Zebra")),
                req("c", Ref::new("\u{e9}clair")), // "éclair" -- unreachable/unsatisfiable-record location
            ]),
        ),
        ("aardvark", rec(vec![req("self", Ref::new("aardvark"))])), // unsatisfiable
        ("Zebra", rec(vec![req("self", Ref::new("Zebra"))])),       // unsatisfiable
        (
            "\u{e9}clair",
            rec(vec![req("self", Ref::new("\u{e9}clair"))]),
        ), // unsatisfiable
        ("Orphan", rec(vec![req("x", STRING)])),
        ("aOrphan", rec(vec![req("x", STRING)])),
    ]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let findings = lint::lint(&s);
    let unsat_locations: Vec<&str> = findings
        .iter()
        .filter(|f| f.code == "lint.unsatisfiable-record")
        .map(|f| f.location.as_str())
        .collect();
    // Codepoint order: 'R' (0x52) < 'Z' (0x5A) < 'a' (0x61) < 'é' (0xE9) --
    // "Root" itself is unsatisfiable too (its mandatory fields all point at
    // unsatisfiable records) and sorts first; a locale-aware sort would
    // instead place "Zebra" after "aardvark" (case-insensitive) and
    // "éclair" among the "e"s.
    assert_eq!(
        unsat_locations,
        vec!["Root", "Zebra", "aardvark", "\u{e9}clair"]
    );

    let unreachable_locations: Vec<&str> = findings
        .iter()
        .filter(|f| f.code == "lint.unreachable-record")
        .map(|f| f.location.as_str())
        .collect();
    // "Orphan" (0x4F) sorts before "aOrphan" (0x61) in codepoint order.
    assert_eq!(unreachable_locations, vec!["Orphan", "aOrphan"]);
}

#[test]
fn lint_never_mutates_the_schema() {
    let e = env(vec![("Root", rec(vec![req("x", STRING)]))]);
    let s = Schema::new(Ref::new("Root"), e).unwrap();
    let before = s.clone();
    let _ = lint::lint(&s);
    assert_eq!(s, before);
}

// ---------------------------------------------------------------------------
// Determinism audit
// ---------------------------------------------------------------------------

/// A schema large enough that hash-based (HashMap/HashSet) iteration would
/// visibly reorder output across repeated runs, if any op leaked it.
fn large_schema() -> Schema {
    let mut e: IndexMap<String, Record> = IndexMap::new();
    // 40 leaf records with varying shapes, plus a root referencing all of
    // them, plus a chain of duplicate-shaped records to exercise minimize's
    // equivalence classes and lint's duplicate-record check at scale.
    let mut root_fields = Vec::new();
    for i in 0..40 {
        let name = format!("Leaf{i:02}");
        let ty = if i % 3 == 0 {
            INTEGER
        } else if i % 3 == 1 {
            STRING
        } else {
            BOOLEAN
        };
        e.insert(name.clone(), rec(vec![req("v", ty)]));
        root_fields.push(opt(&format!("f{i:02}"), Ref::new(name)));
    }
    e.insert("Root".to_string(), rec(root_fields));
    Schema::new(Ref::new("Root"), e).unwrap()
}

#[test]
fn determinism_repeated_runs_produce_identical_output_for_every_op() {
    let s = large_schema();

    let first_prune = prune::prune(&s);
    let first_normalize = minimize::normalize(&s);
    let first_lint = lint::lint(&s);
    let first_classes = minimize::equivalence_classes(&s);

    for _ in 0..25 {
        assert_eq!(prune::prune(&s), first_prune);
        assert_eq!(minimize::normalize(&s), first_normalize);
        assert_eq!(lint::lint(&s), first_lint);
        assert_eq!(minimize::equivalence_classes(&s), first_classes);
    }

    // Env key order specifically (not just set-equality) must be stable.
    let prune_order: Vec<String> = first_prune.env().keys().cloned().collect();
    for _ in 0..25 {
        let again_schema = prune::prune(&s);
        let again: Vec<String> = again_schema.env().keys().cloned().collect();
        assert_eq!(again, prune_order);
    }
}

#[test]
fn no_hashmap_or_hashset_in_ops_source() {
    // Static grep-style guard: the ops module must never reach for
    // std::collections::HashMap/HashSet (nondeterministic iteration order).
    // This is a coarse text check on the crate's own source, run as part of
    // the test suite so a future regression fails CI, not just an audit.
    let src_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/ops");
    // Only the non-test source modules -- this file itself legitimately
    // mentions "HashMap"/"HashSet" in prose (this very check, and doc
    // comments explaining what NOT to use).
    // mod.rs is excluded: it's pure doc-comments and re-exports (no logic
    // of its own), and its module doc comment explicitly *names*
    // `std::collections::HashMap` in prose to explain why the module
    // avoids it -- a legitimate mention, not a usage.
    let source_files = [
        "signature.rs",
        "isomorphic.rs",
        "prune.rs",
        "minimize.rs",
        "subschema.rs",
        "extract.rs",
        "lint.rs",
    ];
    for name in source_files {
        let path = std::path::Path::new(src_dir).join(name);
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {path:?}: {e}"));
        // Look for actual type usage (`HashMap<`/`HashSet<` or the
        // `std::collections::` path), not mere prose mentions in doc
        // comments explaining what NOT to use (e.g. this module's own
        // module-level doc comment).
        for needle in [
            "HashMap<",
            "HashSet<",
            "collections::HashMap",
            "collections::HashSet",
        ] {
            assert!(
                !contents.contains(needle),
                "{path:?} must not use HashMap/HashSet -- use IndexMap/IndexSet instead \
                 (found {needle:?})"
            );
        }
    }
}

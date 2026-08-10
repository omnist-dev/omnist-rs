use super::*;
use crate::document::Scalar as DocScalar;
use crate::schema::{
    BOOLEAN, DATE, DATETIME, Field, INTEGER, NUMBER, Record, Ref, STRING, Schema, TIME, nullable,
};
use indexmap::IndexMap;

fn leaf(v: DocScalar) -> RawNode {
    RawNode::Leaf(v)
}

fn edges(pairs: Vec<(&str, RawNode)>) -> RawNode {
    RawNode::Edges(pairs.into_iter().map(|(l, n)| (l.to_string(), n)).collect())
}

/// A `Root` record with one field per scalar kind under test, all
/// required, plus a nullable string field.
fn scalar_schema() -> Schema {
    let fields = vec![
        Field::required("s", STRING).unwrap(),
        Field::required("i", INTEGER).unwrap(),
        Field::required("n", NUMBER).unwrap(),
        Field::required("b", BOOLEAN).unwrap(),
        Field::required("d", DATE).unwrap(),
        Field::required("t", TIME).unwrap(),
        Field::required("dt", DATETIME).unwrap(),
        Field::new("ns", nullable(STRING), 0, Some(1)).unwrap(),
    ];
    let root = Record::new(fields).unwrap();
    let mut env = IndexMap::new();
    env.insert("Root".to_string(), root);
    Schema::new(Ref::new("Root"), env).unwrap()
}

// ---------------------------------------------------------------------------
// schema = None passthrough
// ---------------------------------------------------------------------------

#[test]
fn schema_none_is_a_no_op_passthrough() {
    let node = edges(vec![("anything", leaf(DocScalar::Str("x".into())))]);
    let out = materialize(&node, None).unwrap();
    assert_eq!(out, node);
}

// ---------------------------------------------------------------------------
// Per-scalar-kind exact-conversion upgrades
// ---------------------------------------------------------------------------

#[test]
fn string_field_accepts_string_as_is() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let out = materialize(&node, Some(&schema)).unwrap();
    let RawNode::Edges(out_edges) = out else {
        panic!("expected edges")
    };
    assert_eq!(
        out_edges[0],
        ("s".to_string(), leaf(DocScalar::Str("hi".into())))
    );
}

#[test]
fn integer_field_upgrades_whole_float() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Float(3.0))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let out = materialize(&node, Some(&schema)).unwrap();
    let RawNode::Edges(out_edges) = out else {
        panic!("expected edges")
    };
    assert_eq!(out_edges[1].1, leaf(DocScalar::Int((3).into())));
}

#[test]
fn already_typed_date_time_and_datetime_re_materialize_as_themselves() {
    // Issue #105's identity arms: re-materializing a document that's
    // already genuinely `Date`/`Time`/`Datetime`-typed (e.g. read straight
    // from OML's/TOML's own native temporal grammar, or a second
    // `materialize` pass) leaves the value as-is rather than treating it
    // as a type mismatch.
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Date("2024-01-01".into()))),
        ("t", leaf(DocScalar::Time("12:30:00".into()))),
        ("dt", leaf(DocScalar::Datetime("2024-01-01T12:30:00".into()))),
    ]);
    let out = materialize(&node, Some(&schema)).unwrap();
    let RawNode::Edges(out_edges) = out else {
        panic!("expected edges")
    };
    assert_eq!(out_edges[4].1, leaf(DocScalar::Date("2024-01-01".into())));
    assert_eq!(out_edges[5].1, leaf(DocScalar::Time("12:30:00".into())));
    assert_eq!(
        out_edges[6].1,
        leaf(DocScalar::Datetime("2024-01-01T12:30:00".into()))
    );
}

#[test]
fn integer_field_rejects_non_whole_float() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Float(3.5))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let err = materialize(&node, Some(&schema)).unwrap_err();
    assert!(
        err.errors()
            .iter()
            .any(|e| e.code == ErrorCode::TypeMismatch && e.path == "$.i")
    );
}

#[test]
fn number_field_upgrades_int_to_float() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Int((7).into()))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let out = materialize(&node, Some(&schema)).unwrap();
    let RawNode::Edges(out_edges) = out else {
        panic!("expected edges")
    };
    assert_eq!(out_edges[2].1, leaf(DocScalar::Float(7.0)));
}

#[test]
fn number_field_keeps_float_as_is() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(2.5))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let out = materialize(&node, Some(&schema)).unwrap();
    let RawNode::Edges(out_edges) = out else {
        panic!("expected edges")
    };
    assert_eq!(out_edges[2].1, leaf(DocScalar::Float(2.5)));
}

#[test]
fn boolean_field_accepts_bool_as_is() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(false))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let out = materialize(&node, Some(&schema)).unwrap();
    let RawNode::Edges(out_edges) = out else {
        panic!("expected edges")
    };
    assert_eq!(out_edges[3].1, leaf(DocScalar::Bool(false)));
}

#[test]
fn date_field_accepts_valid_iso_date_string() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-02-29".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    materialize(&node, Some(&schema)).unwrap();
}

#[test]
fn date_field_rejects_invalid_calendar_date() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-02-30".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let err = materialize(&node, Some(&schema)).unwrap_err();
    assert!(err.errors().iter().any(|e| e.path == "$.d"));
}

#[test]
fn time_field_accepts_valid_iso_time_string() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("23:59:59".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    materialize(&node, Some(&schema)).unwrap();
}

#[test]
fn datetime_field_rejects_a_bare_date_string() {
    let schema = scalar_schema();
    let node = edges(vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01".into()))), // bare date, not datetime
    ]);
    let err = materialize(&node, Some(&schema)).unwrap_err();
    assert!(err.errors().iter().any(|e| e.path == "$.dt"));
}

#[test]
fn nullable_field_accepts_null_non_nullable_rejects_it() {
    let schema = scalar_schema();
    let mut pairs = vec![
        ("s", leaf(DocScalar::Str("hi".into()))),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ];
    pairs.push(("ns", leaf(DocScalar::Null)));
    let node = edges(pairs);
    materialize(&node, Some(&schema)).unwrap();

    // Now put null in a non-nullable slot.
    let bad = edges(vec![
        ("s", leaf(DocScalar::Null)),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let err = materialize(&bad, Some(&schema)).unwrap_err();
    assert!(
        err.errors()
            .iter()
            .any(|e| e.code == ErrorCode::NullNotAllowed && e.path == "$.s")
    );
}

// ---------------------------------------------------------------------------
// Multi-error collection
// ---------------------------------------------------------------------------

#[test]
fn collects_every_problem_not_just_the_first() {
    let schema = scalar_schema();
    // Three independent problems: missing "s" (cardinality), an
    // unexpected field, and a type-mismatch on "b".
    let node = edges(vec![
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Str("not a bool".into()))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
        ("bogus", leaf(DocScalar::Int((1).into()))),
    ]);
    let err = materialize(&node, Some(&schema)).unwrap_err();
    assert_eq!(err.errors().len(), 3, "{:#?}", err.errors());
    assert!(
        err.errors()
            .iter()
            .any(|e| e.code == ErrorCode::Cardinality)
    );
    assert!(
        err.errors()
            .iter()
            .any(|e| e.code == ErrorCode::UnexpectedField)
    );
    assert!(
        err.errors()
            .iter()
            .any(|e| e.code == ErrorCode::TypeMismatch)
    );
    // Display shows all of them, not just one.
    let text = err.to_string();
    assert!(text.contains("bogus") || text.matches("at $").count() >= 3);
}

#[test]
fn shape_mismatch_when_scalar_expected_but_object_given() {
    let schema = scalar_schema();
    let node = edges(vec![
        (
            "s",
            edges(vec![("nested", leaf(DocScalar::Int((1).into())))]),
        ),
        ("i", leaf(DocScalar::Int((1).into()))),
        ("n", leaf(DocScalar::Float(1.0))),
        ("b", leaf(DocScalar::Bool(true))),
        ("d", leaf(DocScalar::Str("2024-01-01".into()))),
        ("t", leaf(DocScalar::Str("12:30:00".into()))),
        ("dt", leaf(DocScalar::Str("2024-01-01T12:30:00".into()))),
    ]);
    let err = materialize(&node, Some(&schema)).unwrap_err();
    assert!(
        err.errors()
            .iter()
            .any(|e| e.code == ErrorCode::ShapeMismatch && e.path == "$.s")
    );
}

#[test]
fn shape_mismatch_when_object_expected_but_scalar_given() {
    let fields = vec![Field::required("child", Ref::new("Child")).unwrap()];
    let root = Record::new(fields).unwrap();
    let child = Record::new(vec![Field::required("x", STRING).unwrap()]).unwrap();
    let mut env = IndexMap::new();
    env.insert("Root".to_string(), root);
    env.insert("Child".to_string(), child);
    let schema = Schema::new(Ref::new("Root"), env).unwrap();

    let node = edges(vec![("child", leaf(DocScalar::Int((1).into())))]);
    let err = materialize(&node, Some(&schema)).unwrap_err();
    assert!(
        err.errors()
            .iter()
            .any(|e| e.code == ErrorCode::ShapeMismatch && e.path == "$.child")
    );
}

#[test]
fn cardinality_array_field_accepts_repeated_labels() {
    let fields = vec![Field::new("tag", STRING, 0, None).unwrap()];
    let root = Record::new(fields).unwrap();
    let mut env = IndexMap::new();
    env.insert("Root".to_string(), root);
    let schema = Schema::new(Ref::new("Root"), env).unwrap();

    let node = edges(vec![
        ("tag", leaf(DocScalar::Str("a".into()))),
        ("tag", leaf(DocScalar::Str("b".into()))),
        ("tag", leaf(DocScalar::Str("c".into()))),
    ]);
    let out = materialize(&node, Some(&schema)).unwrap();
    let RawNode::Edges(out_edges) = out else {
        panic!("expected edges")
    };
    assert_eq!(out_edges.len(), 3);
}

// ---------------------------------------------------------------------------
// `any` field: passes any node through untouched, no shape check/upgrade.
// ---------------------------------------------------------------------------

#[test]
fn any_field_passes_scalar_and_object_nodes_through_untouched() {
    let fields = vec![Field::required("x", crate::schema::FieldType::Any).unwrap()];
    let root = Record::new(fields).unwrap();
    let mut env = IndexMap::new();
    env.insert("Root".to_string(), root);
    let schema = Schema::new(Ref::new("Root"), env).unwrap();

    let scalar_node = edges(vec![("x", leaf(DocScalar::Int((1).into())))]);
    let out = materialize(&scalar_node, Some(&schema)).unwrap();
    assert_eq!(out, scalar_node);

    let object_node = edges(vec![(
        "x",
        edges(vec![("nested", leaf(DocScalar::Str("v".into())))]),
    )]);
    let out2 = materialize(&object_node, Some(&schema)).unwrap();
    assert_eq!(out2, object_node);
}

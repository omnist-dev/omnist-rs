use super::*;
use crate::document::Value;
use crate::schema::{FieldType, ScalarKind};
use indexmap::IndexMap as Map;

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

fn doc(v: Value) -> Doc {
    Doc::of(&v).unwrap()
}

fn field<'a>(env: &'a IndexMap<String, Record>, rec: &str, label: &str) -> &'a Field {
    env[rec]
        .fields()
        .iter()
        .find(|f| f.label == label)
        .unwrap_or_else(|| panic!("no field {label:?} on {rec:?}"))
}

// ---------------------------------------------------------------------------
// Required / optional / array detection
// ---------------------------------------------------------------------------

#[test]
fn label_present_in_every_sample_once_is_required() {
    let samples = vec![
        doc(obj(&[("name", Value::Str("a".into()))])),
        doc(obj(&[("name", Value::Str("b".into()))])),
    ];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "name");
    assert_eq!((f.min, f.max), (1, Some(1)));
}

#[test]
fn label_absent_in_some_samples_is_optional() {
    let samples = vec![doc(obj(&[("name", Value::Str("a".into()))])), doc(obj(&[]))];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "name");
    assert_eq!((f.min, f.max), (0, Some(1)));
}

#[test]
fn label_repeated_within_a_sample_is_an_array_field() {
    // Simulate repetition by building a raw node directly (Value collapses
    // same-label repeats into an array already, which is exactly the
    // "seen more than once" case).
    let samples = vec![doc(obj(&[(
        "tag",
        Value::Array(vec![Value::Str("a".into()), Value::Str("b".into())]),
    )]))];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "tag");
    assert_eq!((f.min, f.max), (0, None));
}

#[test]
fn array_field_min_reflects_the_smallest_sample_count() {
    let samples = vec![
        doc(obj(&[(
            "tag",
            Value::Array(vec![Value::Str("a".into()), Value::Str("b".into())]),
        )])),
        doc(obj(&[("tag", Value::Str("solo".into()))])),
    ];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "tag");
    assert_eq!((f.min, f.max), (0, None));
}

// ---------------------------------------------------------------------------
// Integer/number collapse
// ---------------------------------------------------------------------------

#[test]
fn integer_and_number_samples_collapse_to_number() {
    let samples = vec![
        doc(obj(&[("x", Value::Int((1).into()))])),
        doc(obj(&[("x", Value::Float(1.5))])),
    ];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "x");
    match &f.ty {
        FieldType::Scalar(s) => assert_eq!(s.kind(), ScalarKind::Number),
        FieldType::Ref(_) => panic!("expected a scalar"),
        FieldType::Any => panic!("expected a scalar"),
    }
}

#[test]
fn all_integer_samples_stay_integer() {
    let samples = vec![
        doc(obj(&[("x", Value::Int((1).into()))])),
        doc(obj(&[("x", Value::Int((2).into()))])),
    ];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "x");
    match &f.ty {
        FieldType::Scalar(s) => assert_eq!(s.kind(), ScalarKind::Integer),
        FieldType::Ref(_) => panic!("expected a scalar"),
        FieldType::Any => panic!("expected a scalar"),
    }
}

#[test]
fn a_genuine_date_time_and_datetime_sample_each_infer_their_own_kind() {
    // Issue #105: a real (non-string) temporal sample -- as a format with
    // native temporal grammar (OML/TOML/YAML) now genuinely produces --
    // infers its own kind, not "string" the way a plain ISO-shaped string
    // would (verified against Python's own strict `value_kind()`, which
    // this function mirrors: see `matches_kind`'s doc comment in
    // `schema.rs`).
    for (value, expected) in [
        (Value::Date("2024-01-01".into()), ScalarKind::Date),
        (Value::Time("12:00:00".into()), ScalarKind::Time),
        (
            Value::Datetime("2024-01-01T12:00:00".into()),
            ScalarKind::Datetime,
        ),
    ] {
        let samples = vec![doc(obj(&[("x", value)]))];
        let schema = infer(&samples, "Root").unwrap();
        let f = field(schema.env(), "Root", "x");
        match &f.ty {
            FieldType::Scalar(s) => assert_eq!(s.kind(), expected),
            FieldType::Ref(_) => panic!("expected a scalar"),
            FieldType::Any => panic!("expected a scalar"),
        }
    }
}

#[test]
fn null_sample_makes_the_scalar_nullable() {
    let samples = vec![
        doc(obj(&[("x", Value::Int((1).into()))])),
        doc(obj(&[("x", Value::Null)])),
    ];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "x");
    match &f.ty {
        FieldType::Scalar(s) => {
            assert_eq!(s.kind(), ScalarKind::Integer);
            assert!(s.is_nullable());
        }
        FieldType::Ref(_) => panic!("expected a scalar"),
        FieldType::Any => panic!("expected a scalar"),
    }
}

#[test]
fn only_null_samples_default_to_nullable_string() {
    let samples = vec![doc(obj(&[("x", Value::Null)]))];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "x");
    match &f.ty {
        FieldType::Scalar(s) => {
            assert_eq!(s.kind(), ScalarKind::String);
            assert!(s.is_nullable());
        }
        FieldType::Ref(_) => panic!("expected a scalar"),
        FieldType::Any => panic!("expected a scalar"),
    }
}

// ---------------------------------------------------------------------------
// Nested record naming
// ---------------------------------------------------------------------------

#[test]
fn object_child_becomes_a_nested_named_record() {
    let samples = vec![doc(obj(&[(
        "address",
        obj(&[("city", Value::Str("NYC".into()))]),
    )]))];
    let schema = infer(&samples, "Root").unwrap();
    let f = field(schema.env(), "Root", "address");
    match &f.ty {
        FieldType::Ref(r) => {
            assert!(schema.env().contains_key(&r.name));
            assert_eq!(r.name, "Address");
        }
        FieldType::Scalar(_) => panic!("expected a ref"),
        FieldType::Any => panic!("expected a ref"),
    }
}

#[test]
fn duplicate_generated_names_are_disambiguated() {
    // Two different labels that both identifier-normalize to "Item" force
    // a numeric suffix on the second.
    let samples = vec![doc(obj(&[
        ("item", obj(&[("a", Value::Int((1).into()))])),
        ("Item", obj(&[("b", Value::Int((1).into()))])),
    ]))];
    let schema = infer(&samples, "Root").unwrap();
    let names: Vec<&String> = schema.env().keys().collect();
    assert!(names.contains(&&"Item".to_string()));
    assert!(names.iter().any(|n| n.as_str() == "Item2"));
}

// ---------------------------------------------------------------------------
// Scalar-shape disagreement error
// ---------------------------------------------------------------------------

#[test]
fn disagreeing_scalar_shapes_raise_a_schema_error() {
    let samples = vec![
        doc(obj(&[("x", Value::Str("a".into()))])),
        doc(obj(&[("x", Value::Bool(true))])),
    ];
    let err = infer(&samples, "Root").unwrap_err();
    assert!(err.to_string().contains("more than one scalar"));
}

// ---------------------------------------------------------------------------
// `allow_any: false` (via `infer`/`infer_with_report`): a hard error, no
// mention of `any` in the message (there's no fallback in this mode).
// ---------------------------------------------------------------------------

#[test]
fn mixing_objects_and_scalars_under_one_label_is_a_schema_error() {
    let samples = vec![
        doc(obj(&[("x", obj(&[("a", Value::Int((1).into()))]))])),
        doc(obj(&[("x", Value::Int((1).into()))])),
    ];
    let err = infer(&samples, "Root").unwrap_err();
    assert!(err.to_string().contains("mixes objects and values"));
}

#[test]
fn disagreeing_scalar_shapes_error_does_not_mention_any_when_allow_any_is_false() {
    let samples = vec![
        doc(obj(&[("x", Value::Str("a".into()))])),
        doc(obj(&[("x", Value::Bool(true))])),
    ];
    let err = infer(&samples, "Root").unwrap_err();
    assert!(err.to_string().contains("more than one scalar"));
    assert!(!err.to_string().contains("any"));
}

// ---------------------------------------------------------------------------
// `allow_any: true` (via `infer_with_report`): real `any` fallback support.
// ---------------------------------------------------------------------------

#[test]
fn allow_any_opens_a_mixed_object_and_scalar_label_as_any() {
    let samples = vec![
        doc(obj(&[("x", obj(&[("a", Value::Int((1).into()))]))])),
        doc(obj(&[("x", Value::Int((1).into()))])),
    ];
    let (schema, fallbacks) = infer_with_report(&samples, "Root", true).unwrap();
    let f = field(schema.env(), "Root", "x");
    assert_eq!(f.ty, FieldType::Any);
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(fallbacks[0].location, "Root.x");
    assert!(fallbacks[0].reason.contains("mixes objects and values"));
}

#[test]
fn allow_any_opens_a_disagreeing_scalar_label_as_any() {
    let samples = vec![
        doc(obj(&[("x", Value::Str("a".into()))])),
        doc(obj(&[("x", Value::Bool(true))])),
    ];
    let (schema, fallbacks) = infer_with_report(&samples, "Root", true).unwrap();
    let f = field(schema.env(), "Root", "x");
    assert_eq!(f.ty, FieldType::Any);
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(fallbacks[0].location, "Root.x");
    assert!(fallbacks[0].reason.contains("more than one scalar kind"));
    assert!(fallbacks[0].reason.contains("boolean"));
    assert!(fallbacks[0].reason.contains("string"));
}

#[test]
fn allow_any_true_reports_no_fallbacks_when_nothing_is_ambiguous() {
    let samples = vec![doc(obj(&[("name", Value::Str("a".into()))]))];
    let (_, fallbacks) = infer_with_report(&samples, "Root", true).unwrap();
    assert!(fallbacks.is_empty());
}

#[test]
fn infer_with_report_allow_any_false_matches_infer_and_has_no_fallbacks() {
    let samples = vec![doc(obj(&[("name", Value::Str("a".into()))]))];
    let (schema, fallbacks) = infer_with_report(&samples, "Root", false).unwrap();
    assert_eq!(schema, infer(&samples, "Root").unwrap());
    assert!(fallbacks.is_empty());
}

// ---------------------------------------------------------------------------
// Misc error paths
// ---------------------------------------------------------------------------

#[test]
fn zero_samples_is_a_schema_error() {
    let err = infer(&[], "Root").unwrap_err();
    assert!(err.to_string().contains("zero samples"));
}

#[test]
fn a_bare_scalar_sample_at_the_root_is_a_schema_error() {
    let samples = vec![doc(Value::Int((1).into()))];
    let err = infer(&samples, "Root").unwrap_err();
    assert!(err.to_string().contains("object (record) samples"));
}

// ---------------------------------------------------------------------------
// identifier / unique_name helpers
// ---------------------------------------------------------------------------

#[test]
fn identifier_substitutes_non_alnum_chars() {
    assert_eq!(identifier("some-label"), "some_label");
}

#[test]
fn identifier_strips_leading_digits_and_underscores() {
    assert_eq!(identifier("_2fast"), "fast");
}

#[test]
fn identifier_falls_back_to_unstripped_when_all_digits() {
    assert_eq!(identifier("123"), "123");
}

#[test]
fn identifier_of_empty_string_is_empty() {
    assert_eq!(identifier(""), "");
}

#[test]
fn unique_name_upper_cases_the_first_letter() {
    let mut used = IndexSet::new();
    assert_eq!(unique_name("address", &mut used), "Address");
}

#[test]
fn unique_name_falls_back_to_rec_for_an_empty_base() {
    let mut used = IndexSet::new();
    assert_eq!(unique_name("", &mut used), "Rec");
}

// ---------------------------------------------------------------------------
// Cross-op characterization: an inferred schema always accepts its own
// source data, via `materialize` (issue #14's tie between both modules).
// ---------------------------------------------------------------------------

#[test]
fn materialize_accepts_a_schemas_own_inferred_source_data() {
    let samples = vec![
        doc(obj(&[
            ("name", Value::Str("Ada".into())),
            ("age", Value::Int((36).into())),
            ("address", obj(&[("city", Value::Str("London".into()))])),
            (
                "tag",
                Value::Array(vec![Value::Str("a".into()), Value::Str("b".into())]),
            ),
        ])),
        doc(obj(&[
            ("name", Value::Str("Grace".into())),
            ("age", Value::Float(40.5)),
            ("address", obj(&[("city", Value::Str("NYC".into()))])),
            ("tag", Value::Str("solo".into())),
        ])),
    ];
    let schema = infer(&samples, "Root").unwrap();
    for s in &samples {
        let raw = s.to_raw();
        let out = crate::materialize::materialize(&raw, Some(&schema))
            .unwrap_or_else(|e| panic!("materialize failed on its own source data: {e}"));
        let rebuilt = Doc::from_raw(out).unwrap();
        assert!(
            schema.accepts(&rebuilt.root()),
            "schema does not accept its own inferred source data"
        );
    }
}

// Note: infer_record's own `depth > MAX_DEPTH` guard is not exercised by a
// test here -- see the module's coverage note in the PR description. Every
// sample is a `Doc`, and `Doc::of`/`build_node` already enforce the same
// `MAX_DEPTH` at construction time (crate::document::check_write_depth), so
// no `Doc` that successfully exists can be deep enough to trip infer's own
// recursion past `MAX_DEPTH` -- this mirrors the Python reference, whose
// `_infer_record` carries the identical defensive (and, for the same
// reason, dead-in-practice) check.

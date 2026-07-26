//! Crate-level tests: version sanity, plus cross-op characterization tying
//! `infer` and `materialize` together (issue #14's "cross-op sanity" test
//! obligation). Kept as a separate `tests.rs` module (matching `oml.rs`/
//! `ops/mod.rs`'s own convention) rather than an inline `mod tests` block in
//! `lib.rs` -- this crate's coverage tooling excludes a module's dedicated
//! `tests.rs` file from that module's own line count, so splitting it out
//! here avoids counting these tests' own source lines against `lib.rs`'s
//! coverage.

use super::*;

#[test]
fn version_matches_cargo_toml() {
    assert_eq!(VERSION, "0.0.1-alpha");
}

// -- cross-op characterization: infer + materialize (issue #14) ------------
//
// An inferred schema is drafted *from* a set of samples, so it should always
// accept its own source data -- this ties `infer` and `materialize` together
// the way neither module's own unit tests do.

#[test]
fn materialize_accepts_infers_own_source_data() {
    use document::Value;
    use indexmap::IndexMap;

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = IndexMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    let samples = vec![
        document::Doc::of(&obj(&[
            ("name", Value::Str("alice".into())),
            ("age", Value::Int(30)),
            (
                "address",
                obj(&[
                    ("city", Value::Str("NYC".into())),
                    ("zip", Value::Str("10001".into())),
                ]),
            ),
            (
                "tags",
                Value::Array(vec![Value::Str("a".into()), Value::Str("b".into())]),
            ),
        ]))
        .unwrap(),
        document::Doc::of(&obj(&[
            ("name", Value::Str("bob".into())),
            // "age" absent in this sample -> optional in the inferred
            // schema.
            (
                "address",
                obj(&[
                    ("city", Value::Str("LA".into())),
                    ("zip", Value::Str("90001".into())),
                ]),
            ),
            ("tags", Value::Array(vec![Value::Str("c".into())])),
        ]))
        .unwrap(),
    ];

    let schema = infer(&samples, "Root").expect("infer should draft a schema for these samples");

    for sample in &samples {
        let raw = sample.root().to_raw();
        materialize(&raw, Some(&schema))
            .expect("an inferred schema must accept its own source data");
    }
}

#[test]
fn materialize_of_infer_upgrades_an_integer_number_mix_to_number() {
    use document::Value;
    use indexmap::IndexMap;

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = IndexMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    let samples = vec![
        document::Doc::of(&obj(&[("price", Value::Int(3))])).unwrap(),
        document::Doc::of(&obj(&[("price", Value::Float(3.5))])).unwrap(),
    ];
    let schema = infer(&samples, "Root").unwrap();
    assert_eq!(
        schema.env()["Root"].field("price").unwrap().ty,
        schema::FieldType::Scalar(schema::NUMBER)
    );

    for sample in &samples {
        let raw = sample.root().to_raw();
        let out = materialize(&raw, Some(&schema)).unwrap();
        // Round-trip through `Doc::from_raw` and read the field back via the
        // ordinary `Cursor` API, rather than destructuring `RawNode`
        // directly -- a manual `Edges`/`Leaf` match here would need an
        // always-false "not a record"/"not a scalar" arm with no reachable
        // input to test (materialize_record/_scalar already cover those
        // shape-mismatch arms directly, see materialize.rs's own tests).
        let rebuilt = document::Doc::from_raw(out).unwrap();
        let price = rebuilt.root().get_one("price").unwrap();
        assert!(matches!(price.value().unwrap(), document::Scalar::Float(_)));
    }
}

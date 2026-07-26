//! Parse an OSD schema, then validate a document against it.
//! `cargo run --example schema_validate`.
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::osd::parse_schema;

fn main() {
    let schema = parse_schema(
        r#"
        record Person {
            "name": string,
            "age": integer,
        }
        root Person
        "#,
    )
    .unwrap();

    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str("Ada".to_string()));
    fields.insert("age".to_string(), Value::Int(37));
    let doc = Doc::of(&Value::Object(fields)).unwrap();

    let result = schema.validate(&doc.root());
    assert!(result.ok(), "{result}");
    println!("{result}");

    // A document that violates the schema collects every problem found.
    let mut bad_fields = IndexMap::new();
    bad_fields.insert("name".to_string(), Value::Str("Ada".to_string()));
    bad_fields.insert("age".to_string(), Value::Str("not a number".to_string()));
    let bad_doc = Doc::of(&Value::Object(bad_fields)).unwrap();
    let bad_result = schema.validate(&bad_doc.root());
    assert!(!bad_result.ok());
    println!("{bad_result}");
}

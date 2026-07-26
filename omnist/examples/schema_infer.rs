//! Draft a schema from example documents. `cargo run --example schema_infer`.
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::infer::infer;
use omnist::osd::to_osd;

fn person(name: &str, age: i64, tags: Vec<&str>) -> Value {
    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str(name.to_string()));
    fields.insert("age".to_string(), Value::Int(age));
    fields.insert(
        "tags".to_string(),
        Value::Array(
            tags.into_iter()
                .map(|t| Value::Str(t.to_string()))
                .collect(),
        ),
    );
    Value::Object(fields)
}

fn main() {
    let samples = vec![
        Doc::of(&person("Ada", 37, vec!["engineer"])).unwrap(),
        Doc::of(&person("Grace", 85, vec!["engineer", "admiral"])).unwrap(),
    ];

    let schema = infer(&samples, "Person").unwrap();
    let text = to_osd(&schema, Some(2));
    println!("{text}");

    // The inferred schema accepts every sample it was drafted from.
    for doc in &samples {
        assert!(schema.accepts(&doc.root()));
    }
    println!("all samples accepted");
}

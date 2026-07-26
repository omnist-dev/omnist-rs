//! Reproduces the small code snippets shown in `docs/guide.md`'s
//! "Documents" section, so its `verified-by` marker points at a real
//! assertion of the exact values shown -- not just "it compiles".

use indexmap::IndexMap;
use omnist::document::{Doc, Scalar, Value};

#[test]
fn documents_section_reproduces_the_shown_values() {
    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str("Ann".to_string()));
    fields.insert(
        "tag".to_string(),
        Value::Array(vec![
            Value::Str("x".to_string()),
            Value::Str("y".to_string()),
        ]),
    );
    let doc = Doc::of(&Value::Object(fields)).unwrap();
    let root = doc.root();

    assert_eq!(root.labels(), vec!["name".to_string(), "tag".to_string()]);
    assert_eq!(root.count("tag"), 2);
    assert_eq!(
        *root.get_one("name").unwrap().value().unwrap(),
        Scalar::Str("Ann".to_string())
    );
    let tags: Vec<Scalar> = root
        .get("tag")
        .iter()
        .map(|c| c.value().unwrap().clone())
        .collect();
    assert_eq!(
        tags,
        vec![Scalar::Str("x".to_string()), Scalar::Str("y".to_string())]
    );
}

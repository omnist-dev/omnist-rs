//! Read/write round trip through YAML. `cargo run --example yaml_roundtrip`.
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::yaml::{read_yaml, write_yaml};

fn main() {
    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str("Ada".to_string()));
    fields.insert("age".to_string(), Value::Int(37.into()));
    let doc = Doc::of(&Value::Object(fields)).unwrap();

    let text = write_yaml(&doc, true, None).unwrap();
    println!("{text}");

    let doc2 = read_yaml(&text).unwrap();
    assert!(doc.eq_doc(&doc2), "round trip must be lossless");
    println!("round trip ok");
}

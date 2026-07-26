//! Read/write round trip through TOML. `cargo run --example toml_roundtrip`.
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::toml::{read_toml, write_toml};

fn main() {
    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str("Ada".to_string()));
    fields.insert("age".to_string(), Value::Int(37));
    let doc = Doc::of(&Value::Object(fields)).unwrap();

    let text = write_toml(&doc, true, None).unwrap();
    println!("{text}");

    let doc2 = read_toml(&text).unwrap();
    assert!(doc.eq_doc(&doc2), "round trip must be lossless");
    println!("round trip ok");
}

//! Read/write round trip through XML. `cargo run --example xml_roundtrip`.
//!
//! XML needs exactly one document element, so (unlike JSON/YAML/TOML) the
//! example document is wrapped under a single top-level `person` edge.
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::xml::{read_xml, write_xml};

fn main() {
    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str("Ada".to_string()));
    fields.insert("age".to_string(), Value::Int(37));
    let mut root = IndexMap::new();
    root.insert("person".to_string(), Value::Object(fields));
    let doc = Doc::of(&Value::Object(root)).unwrap();

    let text = write_xml(&doc, true, None).unwrap();
    println!("{text}");

    let doc2 = read_xml(&text).unwrap();
    assert!(doc.eq_doc(&doc2), "round trip must be lossless");
    println!("round trip ok");
}

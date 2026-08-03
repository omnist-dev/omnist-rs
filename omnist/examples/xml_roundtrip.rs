//! Read/write round trip through XML. `cargo run --example xml_roundtrip`.
//!
//! XML needs exactly one document element, so (unlike JSON/YAML/TOML) the
//! example document is wrapped under a single top-level `person` edge.
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::xml::{read_xml, write_xml};

fn main() {
    // `age` is a `Value::Str`, not `Value::Int`: XML text carries no type
    // information (see `docs/formats/xml.md`'s "Text is untyped" section,
    // and `omnist-rs#86`) -- writing a non-string scalar through XML now
    // honestly reports that it reads back as a string (`value.stringified`),
    // so this example keeps the round trip lossless by not writing a typed
    // scalar in the first place.
    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str("Ada".to_string()));
    fields.insert("age".to_string(), Value::Str("37".to_string()));
    let mut root = IndexMap::new();
    root.insert("person".to_string(), Value::Object(fields));
    let doc = Doc::of(&Value::Object(root)).unwrap();

    let text = write_xml(&doc, true, None).unwrap();
    println!("{text}");

    let doc2 = read_xml(&text).unwrap();
    assert!(doc.eq_doc(&doc2), "round trip must be lossless");
    println!("round trip ok");
}

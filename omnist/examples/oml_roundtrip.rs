//! Read/write round trip through OML, omnist's own lossless format.
//! `cargo run --example oml_roundtrip`.
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::oml::{read_oml, write_oml};

fn main() {
    let mut fields = IndexMap::new();
    fields.insert("name".to_string(), Value::Str("Ada".to_string()));
    fields.insert("age".to_string(), Value::Int(37.into()));
    let doc = Doc::of(&Value::Object(fields)).unwrap();

    let text = write_oml(&doc.to_raw(), 2).unwrap();
    println!("{text}");

    let raw2 = read_oml(&text).unwrap();
    let doc2 = Doc::from_raw(raw2).unwrap();
    assert!(doc.eq_doc(&doc2), "round trip must be lossless");
    println!("round trip ok");
}

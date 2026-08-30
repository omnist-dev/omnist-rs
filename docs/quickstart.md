# Quickstart

```toml
[dependencies]
omnist = "0.2.2-alpha"
```
<!-- doc-illustrative -->

The shortest possible tour -- one round trip, one schema, one validation,
one inference. Each snippet below is a trimmed version of a real file under
[`omnist/examples/`](../omnist/examples/); run it yourself with
`cargo run --example <name>`.

## 1. Read and write a document

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::json::{read_json, write_json};

let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ada".to_string()));
fields.insert("age".to_string(), Value::Int(37));
let doc = Doc::of(&Value::Object(fields)).unwrap();

let text = write_json(&doc, Some(2), true, None).unwrap();
// {
//   "name": "Ada",
//   "age": 37
// }

let doc2 = read_json(&text).unwrap();
assert!(doc.eq_doc(&doc2), "round trip must be lossless");
```
<!-- verified-by: omnist/tests/examples.rs::json_roundtrip -->

## 2. Validate against a schema

```rust
use omnist::osd::parse_schema;

let schema = parse_schema(
    r#"record Person { "name": string, "age": integer } root Person"#,
).unwrap();
// schema.validate(&doc.root()).ok() == true for the document above
```
<!-- verified-by: omnist/tests/examples.rs::schema_validate -->

## 3. Infer a schema from example documents

```rust
use omnist::infer::infer;
use omnist::osd::to_osd;

let schema = infer(&samples, "Person").unwrap();
println!("{}", to_osd(&schema, Some(2)));
// record Person {
//   "name": string,
//   "age": integer,
//   "tags" [0,]: string,
// }
// root Person
```
<!-- verified-by: omnist/tests/examples.rs::schema_infer -->

That's it -- a `Doc`, a `Schema`, `validate()`, and `infer()`. From here:

- [User guide](guide.md) -- the full practical tour, including formats and
  the CLI.
- [CLI reference](cli.md) -- the `omnist` binary's command surface.
- [Per-format pages](formats/) -- adjustment/lossy-conversion behavior for
  JSON, YAML, TOML, XML, and OML.
- [Limitations & stability](limitations.md) -- this port's `0.0.x` alpha
  status and known scoping gaps.

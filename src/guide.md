# Omnist (Rust) -- user guide

Omnist gives you **one canonical data model** for JSON, YAML, TOML, XML, and
its own native [**OML**](formats/oml.md), and a [**schema language**](#schemas-osd)
(OSD) to validate and compare shapes over it. This is the Rust port of
[omnist](https://github.com/omnist-dev/omnist); its public types (`Doc`,
`Schema`, error enums) are not identical to Python's or TypeScript's by
design -- see the workflow playbook's "architecture freedom" note -- but the
model and every format's observable behavior are the same.

- [The two ideas](#the-two-ideas)
- [Documents](#documents)
- [OML -- the native format](#oml-the-native-format)
- [Schemas -- OSD](#schemas-osd)
- [Validation](#validation)
- [Schema algebra](#schema-algebra)
- [Reading & writing other formats](#reading-writing-other-formats)
- [Inferring a schema](#inferring-a-schema)
- [The CLI](#the-cli)

## The two ideas

- A **Document** ([`omnist::document::Doc`]) is a *tree*: a node is either a
  scalar value or an **ordered list of labeled edges**. "Many" is a label
  that repeats, not a field pointing to an array.
- A **Schema** ([`omnist::schema::Schema`]) is built from named `record`
  definitions (a closed set of named fields, each with a cardinality). A
  field's type is exactly one of seven fixed scalar kinds (optionally
  nullable, e.g. `string?`) or a `Ref` to a named record -- never a
  composition of the two. `Ref`s are how reuse and recursion work.

## Documents

[`Doc::of`] builds a `Doc` from a [`Value`] (this crate's plain-value input
type, analogous to a parsed JSON value). An object becomes an edge list; a
key whose value is a `Value::Array` expands into one edge per item (a
repeated label) -- there is no separate array node type.

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};

let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ann".to_string()));
fields.insert(
    "tag".to_string(),
    Value::Array(vec![Value::Str("x".into()), Value::Str("y".into())]),
);
let doc = Doc::of(&Value::Object(fields)).unwrap();
let root = doc.root();

root.labels();                 // ["name", "tag"]
root.count("tag");              // 2 -- "tag" is a repeated label (an array)
root.get_one("name").unwrap();  // a Cursor whose .value() is Scalar::Str("Ann")
root.get("tag");                 // both "tag" cursors, in order
```
<!-- verified-by: omnist/tests/doc_guide.rs::documents_section_reproduces_the_shown_values -->

`Doc::to_grouped()` projects a `Doc` back to a JSON-shaped [`Value`] (same-
label edges become an array); `Doc::to_raw()`/`Doc::from_raw()` go through
[`RawNode`], the interleaving-preserving representation OML and XML need
(see [formats/oml.md](formats/oml.md)).

## OML -- the native format

OML is omnist's own serialization format: every Document round-trips
through it exactly, with no adjustment ever needed (unlike JSON/YAML/TOML/
XML, which each have some lossy corner -- see the per-format pages).

```rust
use omnist::oml::{read_oml, write_oml};

let text = write_oml(&doc.to_raw(), 2).unwrap();
// name: "Ada"
// age: 37
let doc2 = omnist::document::Doc::from_raw(read_oml(&text).unwrap()).unwrap();
assert!(doc.eq_doc(&doc2));
```
<!-- verified-by: omnist/tests/examples.rs::oml_roundtrip -->

## Schemas -- OSD

OSD (Omnist Schema Definition) is the text language for [`Schema`]. A quoted
token is always a field label; an unquoted identifier is always a schema
name (scalar keyword or `Ref`).

```text
record Person {
    "name": string,
    "age": integer,
}
root Person
```
<!-- doc-illustrative -->

`omnist::osd::parse_schema` parses this text into a `Schema`;
`omnist::osd::to_osd` writes one back out.

## Validation

`Schema::validate` checks a document's [`Cursor`] against the schema's root
type and collects *every* problem found, not just the first.

```rust
use omnist::osd::parse_schema;

let schema = parse_schema(
    r#"record Person { "name": string, "age": integer } root Person"#,
).unwrap();
let result = schema.validate(&doc.root());
assert!(result.ok());
```
<!-- verified-by: omnist/tests/examples.rs::schema_validate -->

## Schema algebra

`omnist::ops` implements the schema-algebra operations: `prune` (drop
unreachable/unsatisfiable parts), `normalize` (canonical minimal form),
`extract` (a subschema over a subset of root fields), `lint`, `is_isomorphic`,
`compatible_with`/`equivalent` (subschema relations), and `local_signature`.

```rust
use omnist::ops::prune;

// `Broken` requires itself and can never be satisfied; `Dead` is
// unreachable from `Root`. `prune` drops both, keeping `Root`.
let pruned = prune(&schema);
assert!(pruned.env().contains_key("Root"));
assert!(!pruned.env().contains_key("Dead"));
assert!(!pruned.env().contains_key("Broken"));
```
<!-- verified-by: omnist/tests/examples.rs::schema_algebra -->

## Reading & writing other formats

Every format module (`omnist::formats::{json,yaml,toml,xml}`, plus
`omnist::oml` for OML) exposes a `read_*`/`write_*`/`check_*` triple.
`write_*`/`check_*` return a [`WriteReport`] cataloging every adjustment a
lossy write had to make (dropped nulls, stringified NaN, ...); `strict`
mode turns any adjustment into an error instead of a silent write. See
[formats/](formats/) for what each format actually adjusts, verified
against each codec's own merged PR.

### The format registry

Alongside the per-format functions, [`Doc::from_format`]/`to_format`/
`check_format` dispatch by format **name** through a runtime registry
(`omnist::formats`/`register_format`/`get_format`), so a new format can be
registered under an arbitrary name at runtime and used everywhere a format
name is accepted -- not just the five builtins. `register_format` takes a
[`Format`] (`name` + `read`/`write` closures, plus an optional `check`); a
plugin with no `check` still works for `from_format`/`to_format`, but
`check_format` on it returns a clean error rather than panicking.

```rust
use omnist::document::Doc;
use omnist::{Format, formats, register_format};

let d = Doc::from_format("json", r#"{"a": 1}"#).unwrap();
assert_eq!(d.to_format("yaml").unwrap(), "a: 1");

register_format(Format::new(
    "kv",
    |text| {
        let edges = text
            .split(',')
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap();
                (k.to_string(), omnist::document::RawNode::Leaf(
                    omnist::document::Scalar::Str(v.to_string()),
                ))
            })
            .collect();
        Doc::from_raw(omnist::document::RawNode::Edges(edges)).map_err(Into::into)
    },
    |doc| {
        let omnist::document::RawNode::Edges(edges) = doc.to_raw() else {
            return Ok(String::new());
        };
        Ok(edges.iter().map(|(k, v)| {
            let omnist::document::RawNode::Leaf(omnist::document::Scalar::Str(s)) = v else {
                unreachable!()
            };
            format!("{k}={s}")
        }).collect::<Vec<_>>().join(","))
    },
));
assert!(formats().contains(&"kv".to_string()));
assert_eq!(Doc::from_format("kv", "a=1,b=2").unwrap().to_format("kv").unwrap(), "a=1,b=2");
```
<!-- verified-by: omnist/tests/examples.rs::format_registry -->

## Inferring a schema

`omnist::infer::infer` drafts a `record` schema that accepts a set of sample
`Doc`uments, without an `allow_any` fallback (see
[limitations.md](limitations.md) for why).

```rust
use omnist::infer::infer;

let schema = infer(&samples, "Person").unwrap();
for doc in &samples {
    assert!(schema.accepts(&doc.root()));
}
```
<!-- verified-by: omnist/tests/examples.rs::schema_infer -->

## The CLI

The `omnist` binary (crate `omnist-cli`) wraps this library as a
command-line tool -- `format`, `convert`, `check`, `validate`, `infer`, and
the `schema` subcommand family. See [cli.md](cli.md) for the full command
reference.

# OML

`omnist::oml::{read_oml, write_oml}`. Ported from `~/dev/omnist/omnist/oml.py`
(issue #10); see [`omnist/src/oml.rs`](../../omnist/src/oml.rs)'s module doc
for the full detail. OML (Omnist Markup Language) is omnist's own native
format: every Document round-trips through it exactly, with **no
adjustment ever needed** -- unlike JSON/YAML/TOML/XML, each of which has at
least one lossy corner (see their own pages).

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::oml::{read_oml, write_oml};

let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ada".to_string()));
fields.insert("age".to_string(), Value::Int(37));
let doc = Doc::of(&Value::Object(fields)).unwrap();

let text = write_oml(&doc.to_raw(), 2).unwrap();
let raw2 = read_oml(&text).unwrap();
let doc2 = Doc::from_raw(raw2).unwrap();
assert!(doc.eq_doc(&doc2));
```
<!-- verified-by: omnist/tests/examples.rs::oml_roundtrip -->

## Core vs. Extended

`read_oml` implements the **OML-Core** grammar in full, plus the
**OML-Extended** raw-string (`'...'`) and triple-quoted multiline-string
(`"""..."""`) spellings on read only. `write_oml` only ever emits OML-Core
double-quoted strings, matching the Python reference exactly -- reading
back a raw-string or triple-quoted literal never round-trips to that same
spelling, only to an equivalent Core one.

## `RawNode`, not `Value` -- the only codec (with XML) that needs it

`Value::Object`'s `IndexMap` can only represent "repeated label" as a
*contiguous run* (an array under one key). OML must round-trip arbitrary
**interleaving** of repeated labels losslessly -- its whole reason for
existing -- so `read_oml`/`write_oml` work over `RawNode`'s literal edge
list, never `Value`.

## Temporal literals: shape-checked, then validated

`date`/`time`/`datetime` literals are recognized by shape in this module's
own scanner, then validated by the same shared
`crate::schema::is_iso_date`/`is_iso_time`/`is_iso_datetime` checks
`schema.rs` and every other codec use. A recognized-but-invalid literal
(`2024-02-30`) is a `ParseError`, never silently accepted. Like every other
codec, `Scalar` has no native temporal variant, so a valid literal becomes
a `Scalar::Str` holding its exact source spelling.

## Integer digit cap and `i64`

Same 4300-digit cap (`MAX_INT_DIGITS`) as `json.rs`/`yaml.rs`/`toml.rs`, and
the same `i64`-backed representational ceiling -- see
[formats/json.md](json.md).

## `--arrays` is not yet implemented

Python's `write_oml` supports an `arrays=True` mode that collapses runs of
same-label edges into `[...]` array syntax. This port's `write_oml` has no
such parameter yet -- separate, not-yet-ported library work tracked by the
CLI's own `--arrays` "not supported yet" error (see [cli.md](../cli.md)).

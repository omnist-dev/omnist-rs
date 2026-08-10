# JSON

`omnist::formats::json::{read_json, write_json, check_json}`. Ported from
`~/dev/omnist/omnist/formats.py`'s `read_json`/`write_json`/`check_json`;
see [`omnist/src/formats/json.rs`](../../omnist/src/formats/json.rs)'s
module doc for the full detail behind every claim below.

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::json::{read_json, write_json};

let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ada".to_string()));
fields.insert("age".to_string(), Value::Int(37));
let doc = Doc::of(&Value::Object(fields)).unwrap();

let text = write_json(&doc, Some(2), true, None).unwrap();
let doc2 = read_json(&text).unwrap();
assert!(doc.eq_doc(&doc2));
```
<!-- verified-by: omnist/tests/examples.rs::json_roundtrip -->

## The one lossy JSON adjustment: `NaN`/`Infinity`

JSON's grammar has no token for `NaN`/`Infinity`/`-Infinity`. Writing one of
these floats substitutes `null` and records a `temporal.stringified`-style
adjustment via `WriteReport`; `strict` mode turns this into an error instead
of a silent substitution.

## No native temporal type

JSON has no native `date`/`time`/`datetime` literal, so a genuine
`Scalar::Date`/`Time`/`Datetime` (issue #105) writes the same way a
`Scalar::Str` does -- its raw ISO text, quoted -- and `check_json` records
a `format.temporal-stringified` warning, matching Python (which
stringifies a live `datetime.date`/`time` object and records its own
`temporal.stringified`-equivalent warning). This is a real adjustment now,
not merely unreachable: before issue #105, `Scalar` had no temporal
variant at all, so this codec's decoder path (in the conformance harness)
collapsed such values to a plain string before the writer ever ran,
making the adjustment structurally unreachable in this port
(`formats-json/basic/temporal-leaf-is-stringified-on-write`, previously
skipped, now passes for real).

## Integer digit cap (arbitrary-precision, matching Python -- issue #104)

A JSON integer literal over **4300 digits** is a `ParseError` (mirrors
CPython's `sys.set_int_max_str_digits` guard, which fires inside
`json.loads` before Python ever sees the value). Under that cap, this
port's `Scalar::Int`/`Value::Int` hold the value exactly, at any size --
`num_bigint::BigInt`, not a fixed-width integer -- matching Python's own
arbitrary-precision `int` with no additional ceiling. (Previously
`i64`-backed, ~19 significant digits; that was a real spec-conformance
bug, not a disclosed representational gap -- see
[Limitations](../limitations.md#scalarint-is-arbitrary-precision-issue-104).)

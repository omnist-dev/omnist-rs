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

This port's `Scalar` has no `date`/`time`/`datetime` variant at all (a
Rust-specific architecture consequence, not a JSON limitation) -- a
temporal value is already a `Scalar::Str` holding its ISO spelling by the
time it reaches this codec, so there is nothing left to adjust or report
here, unlike Python (which stringifies a live `datetime.date`/`time` object
and records a `temporal.stringified` warning).

## Integer digit cap and the `i64` representational limit

A JSON integer literal over **4300 digits** is a `ParseError` (mirrors
CPython's `sys.set_int_max_str_digits` guard, which fires inside
`json.loads` before Python ever sees the value). Because this port's
`Scalar::Int` is `i64` (max ~19 significant digits), any literal over 19
digits fails first with "out of range for a 64-bit integer" -- the
4300-digit cap essentially never fires in practice for this port, but is
kept anyway for parity with the same guard `oml.rs`/`yaml.rs`/`toml.rs`
share. Python's reference, by contrast, holds arbitrary-precision `int`s
under the 4300-digit cap with no 64-bit ceiling at all -- a disclosed
Rust-specific representational gap, not a bug.

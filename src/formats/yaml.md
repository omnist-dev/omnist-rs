# YAML

`omnist::formats::yaml::{read_yaml, write_yaml, check_yaml}`. Ported from
`~/dev/omnist/omnist/formats.py`'s `read_yaml`/`write_yaml`/`check_yaml`;
see [`omnist/src/formats/yaml.rs`](../../omnist/src/formats/yaml.rs)'s
module doc for the full detail.

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::yaml::{read_yaml, write_yaml};

let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ada".to_string()));
fields.insert("age".to_string(), Value::Int(37));
let doc = Doc::of(&Value::Object(fields)).unwrap();

let text = write_yaml(&doc, true, None).unwrap();
let doc2 = read_yaml(&text).unwrap();
assert!(doc.eq_doc(&doc2));
```
<!-- verified-by: omnist/tests/examples.rs::yaml_roundtrip -->

## Scalar-tag resolution: PyYAML's rules, not `yaml_rust2`'s

`yaml_rust2`'s own resolver only recognizes `true`/`false` for booleans
(YAML 1.2 core schema). This module ignores that and re-implements PyYAML's
YAML-1.1 implicit-resolver regexes instead, live-checked against PyYAML
(this project's Python reference): `yes`/`no`/`on`/`off` (and case
variants) are also booleans; bare `y`/`n` are **not** and stay strings.
Quoted scalars are never auto-typed, matching PyYAML (implicit resolution
only applies to plain-style scalars).

## Merge keys (`<<`)

`yaml_rust2` has no built-in merge-key support; this module implements the
YAML merge-key spec directly (an unquoted `<<` key's value must be a
mapping or sequence of mappings, merged in order, explicit keys taking
precedence).

## No native temporal type, but a looser input grammar than JSON

Like `json.rs`, `Scalar` has no temporal variant -- a timestamp becomes a
`Scalar::Str` holding its ISO spelling. Unlike JSON, YAML's timestamp
grammar is looser (space-separated date/time, single-digit month/day, a
bare `Z` suffix, no zero-padding); this module normalizes any such spelling
to the same canonical, zero-padded, `T`-joined ISO shape PyYAML's own
`datetime.isoformat()` would produce -- so `2001-12-14 21:59:43.10 -5`
round-trips to `2001-12-14T21:59:43.100000-05:00`, not its original
spelling. A timestamp-shaped string naming a calendar/clock value that
doesn't exist (`2024-13-01`) is a `ParseError`, matching PyYAML's own
construction-time failure.

## Native `NaN`/`Infinity` -- no lossy adjustment here (unlike JSON)

YAML's float grammar has native `.nan`/`.inf`/`-.inf` tokens, so unlike
`write_json`, `write_yaml` never substitutes `null` for a special float.
The only adjustment `check_yaml` ever records is forcing double-quoted
style for a string containing U+0085 (NEL), which PyYAML's default styles
would otherwise normalize away as a line break.

## Legacy sexagesimal integers (`H:M:S`-shaped)

YAML 1.1's implicit-int resolver also recognizes a colon-separated
sexagesimal form -- `12:00:00` resolves to `Scalar::Int(43200)`, not a
string. This module's resolver folds each `:`-separated group as
`acc*60 + group` (checked arithmetic; overflow reports the same
out-of-range `ParseError` as an oversized plain integer), requires no
leading zero on the first group, and constrains later groups to `0..=59`
-- so `01:20`, `1:60`, and `0:0:1` all stay plain strings (each violates
one of those rules), matching PyYAML's own grammar. Confirmed by omnist-rs
issue #87, found while building the conformance harness against
[omnist-spec](../conformance.md).

## Mapping keys are implicitly typed too (the "Norway problem")

Every mapping key is run through the same implicit-type resolver as
values, matching PyYAML: a key like `on:` is rejected (it resolves to
`Bool(true)`, not a string), and so is any other non-string-resolving key
shape (int-, float-, sexagesimal-shaped, `null`-shaped). The rejection is
a `DocumentError` at path `"$"` with a Python-parity message (e.g.
`object key True is not a string`, `object key 1.0 is not a string` --
including keeping the `.0` on whole-number floats). Ordinary string keys,
and non-boolean words like bare `y`/`n`, are unaffected. Confirmed by
omnist-rs issue #88, found and fixed alongside #87 above.

## Integer digit cap

Same 4300-digit cap as `json.rs`/`oml.rs`/`toml.rs`, applied to a plain
decimal integer scalar's digit run before parsing; arbitrary-precision
above that (see [formats/json.md](json.md#integer-digit-cap-arbitrary-precision-matching-python----issue-104)).
The legacy sexagesimal fold (above) enforces the identical cap on its
*folded result*, not any one group -- issue #104: an unbounded
`BigInt` fold with no such check would let a many-group literal build an
arbitrarily large integer, a resource-exhaustion regression the fold's
old `i64` overflow used to prevent as an unplanned side effect.

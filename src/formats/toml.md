# TOML

`omnist::formats::toml::{read_toml, write_toml, check_toml}`. Ported from
`~/dev/omnist/omnist/formats.py`'s `read_toml`/`write_toml`/`check_toml`;
see [`omnist/src/formats/toml.rs`](../../omnist/src/formats/toml.rs)'s
module doc for the full detail.

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::toml::{read_toml, write_toml};

let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ada".to_string()));
fields.insert("age".to_string(), Value::Int(37));
let doc = Doc::of(&Value::Object(fields)).unwrap();

let text = write_toml(&doc, true, None).unwrap();
let doc2 = read_toml(&text).unwrap();
assert!(doc.eq_doc(&doc2));
```
<!-- verified-by: omnist/tests/examples.rs::toml_roundtrip -->

## No `null` -- the one lossy TOML adjustment

TOML has no `null` at all. Writing a null-valued field drops the field
entirely (`{"a": 1, "b": null}` writes as `a = 1\n`, no trace of `b`); a
null inside an array drops just that element, shifting later elements down.
Each drop is recorded as a `null.omitted`/`Severity::Warning` adjustment,
live-confirmed against `tomli_w.dumps` (Python's reference TOML writer) to
match exactly, path-for-path. `strict` mode raises even though the severity
is only `Warning`.

## Integer digit cap, and a real, external `i64` ceiling from `toml_edit`

Live-confirmed against `tomllib.loads`: a **decimal** integer literal
follows the identical 4300-digit `sys.set_int_max_str_digits` cap
`json.rs`/`yaml.rs` already apply. **Hex/octal/binary literals are a
genuine exception in Python** -- `tomllib.loads("x = 0x" + "f" * 10000)`
parses successfully with no error at all, because CPython's digit-limit
guard explicitly exempts power-of-two bases.

This port does **not** replicate that decimal/non-decimal split, but for
a different reason than it used to (issue #104: `Scalar::Int`/`Value::Int`
are arbitrary-precision `BigInt`s now, not `i64`). The underlying
`toml_edit` crate's own `Integer` type is `i64`-backed regardless of
radix (the TOML 1.0 format spec itself documents 64-bit signed
integers), so **reading** a >19-digit literal of any radix from TOML
*source text* fails in `toml_edit`'s own parser, before this codec's
`Scalar` conversion ever runs -- a real, external, still-current
divergence from Python's arbitrary-precision `int`, not something this
port's own representation choice can lift. Live-confirmed:
`convert --from toml` on `n = 99999999999999999999999999` fails with
`"integer literal ... is out of range for a 64-bit integer"`.
**Writing** an oversized `Scalar::Int` *to* TOML, by contrast, succeeds
-- this codec's writer renders integers as plain digit text rather than
going through `toml_edit`'s typed API, so the ceiling is read-side only;
live-confirmed the same 26-digit value round-trips out via
`convert --from oml --to toml` with no error, it just can't be read back
in through TOML afterward.

## Native temporal types, truncated (not rounded) sub-microsecond precision

TOML has **four** first-class temporal literal forms (local date, local
time, local datetime, offset datetime), stricter-shaped than YAML's --
`toml_edit` itself fully validates calendar/clock fields at parse time
(`2024-02-30`, `25:00:00`, and a `+25:00` offset are all parse errors, not
accepted-then-rejected-later). Fractional seconds beyond microsecond
precision are **truncated, not rounded** to six digits, live-confirmed
against `tomllib`: `00:32:00.9999999` (7 nines) reads as
`datetime.time(0, 32, 0, 999999)`, matching this module's integer-
truncating conversion exactly. A numeric UTC offset is preserved exactly
across read+write (no offset-erasure bug).

On write, an ISO-shaped `Scalar::Str` is written as a *native*, unquoted
TOML temporal literal -- this **diverges from Python's own `write_toml`**,
whose document model retains a real `datetime` object end-to-end: a plain
Python string that merely *looks* like a date
(`tomli_w.dumps({'a': '1979-05-27'})`) writes as the quoted string
`a = "1979-05-27"`, not a native date literal. This is the same,
already-accepted consequence of this port's `Scalar`-has-no-temporal-
variant architecture decision that `yaml.rs` documents too, not a new bug.

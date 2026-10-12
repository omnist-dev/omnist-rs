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

TOML has no `null` at all. Writing a node that contains a null fails the write
unconditionally with `write.unsupported-value` (spec sections 8.3.8 and 8.3.9),
whatever `strict` says: dropping the field, as this port once did and recorded
as a `null.omitted` warning, erased the edge's existence with no trace on
read-back (omnist-rs#160 retired that behaviour). The XML writer refuses a
null the same way (spec C-10); see [XML](xml.md#null-has-no-xml-form).

## Integers are arbitrary precision; the digit limit counts decimal digits

A TOML integer is arbitrary precision here, in every radix (spec D-2). The
limit is `Limits::max_int_digits` (default 4300; `read_toml_with` changes it),
and it counts the **decimal digits of the value** (D-28), whatever the
spelling: `0xFFF` is 4095, four digits; underscores are separators, not digits.
A value over the limit is a `DocumentError` with code
`document.limit.int-digits` at the integer's Document path, never a syntax
error. A 3,600-digit hex literal is about 4,335 decimal digits, so it is
refused at the default; a 4,301-digit binary literal is about 1,295, so it
reads.

`toml_edit` holds an integer in 64 bits and refuses a longer one with no value
to read. This reader works around that: it takes the byte range from the
refusal, reads the literal itself (any radix, optional sign, underscores),
swaps in a one-character stand-in of the same width so every other offset keeps
its place, parses again, and puts the real value back by offset when it builds
the Document. A decimal literal already longer than the limit in its own
spelling is not converted at all; a placeholder stands in for it, so it is
refused without paying the superlinear conversion. A syntax error elsewhere
in the text still wins over a limit. The literals are found in one scan, so the
cost stays linear however many there are.

Python's `tomllib` exempts hex, octal and binary literals from CPython's
4,300-digit string-conversion guard; this port does not, because D-28 counts
the value's decimal digits for every radix.

A float whose magnitude overflows (`1e999`, `-1e999`) reads as positive or
negative infinity (D-29), by the same route.

**Writing** an oversized integer to TOML needs no workaround: the writer renders
integers as plain digit text.

## Nesting past about 80 levels

`toml_edit` has a recursion limit of its own, about 80 levels of table header,
inline table, array or dotted key. TOML nested between about 81 levels and the
configured maximum depth (200 by default) therefore fails with
`document.limit.depth` at `$` even though the configured limit has not been
reached; the message names `toml_edit` as the cause. This is a limit of the
parser library, not of the format or of the Document model, and raising
`max_depth` does not lift it.

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

On read, `toml_value_to_value` constructs the real `Value::Date`/`Time`/
`Datetime` variant directly from `toml_edit::Datetime`'s own already-
validated `date`/`time` fields (issue #105) -- previously this discarded
into a plain `Value::Str`. On write, only a genuine `Date`/`Time`/
`Datetime` variant is written as a native, unquoted TOML literal; a plain
`Scalar::Str` that merely *looks* like a date always writes as a quoted
string, matching Python's `write_toml` exactly (a plain Python string that
looks like a date, e.g. `tomli_w.dumps({'a': '1979-05-27'})`, also writes
quoted). This resolves the shape-guessing divergence issue #99 first
introduced for OML and issue #105 now closes here too -- see [Python
divergences](../python-divergences.md#date-shaped-strings-as-native-temporal-literals-tomlyaml-resolved-by-issue-105).
A `Time` value carrying a UTC offset has no native TOML spelling at all
(only *date*time can carry an offset) and always writes double-quoted, with
`format.temporal-stringified` reported at its path (E-37). Every writer writes a
time's seconds (`12:00` is written `12:00:00`) and keeps the offset and every
fractional digit; a datetime uses the `T` separator.

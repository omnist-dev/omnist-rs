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

## Temporal literals: shape-checked, then validated, then canonicalized

`date`/`time`/`datetime` literals are recognized by shape in this module's
own scanner, then validated by the same shared
`crate::schema::is_iso_date`/`is_iso_time`/`is_iso_datetime` checks
`schema.rs` and every other codec use. A recognized-but-invalid literal
(`2024-02-30`) is a `ParseError`, never silently accepted. A valid literal
becomes a genuine `Scalar::Date`/`Time`/`Datetime` (issue #105) -- the
scanner threads which of the three kinds it matched (`TemporalKind`)
through to the parser so it constructs the exact right variant, not a
generic string.

**`time`/`datetime` literal text is canonicalized on read** (issue #90,
fixed while building the [conformance harness](../conformance.md) against
omnist-spec): a missing `:SS` is filled to `:00`, and an under-padded
fractional-second component is zero-padded to 6 digits. `date` literals
have no optional grammar components and pass through unchanged. This
means a bare `12:00` reads as `Scalar::Time("12:00:00")`, **not**
`"12:00"` -- see the corrected note in [Python
divergences](../python-divergences.md#bare-time-literal-round-tripping-oml-pr-11)
for what changed from this port's earlier, stronger byte-for-byte claim.

## Bare vs. quoted on write: real variant, not shape-guessing

A leaf writes bare (no quotes) only when it genuinely holds a
`Scalar::Date`/`Time`/`Datetime` -- never by guessing from a
`Scalar::Str`'s shape (issue #99). The pre-#99 writer wrote *any*
date/time/datetime-*shaped* string bare, regardless of provenance: a plain
JSON string like `"2024-01-01"` got silently promoted to a genuine OML
temporal literal on write, corrupting it on the next read (a different
Document, per [ch.4's grammar](https://github.com/omnist-dev/omnist-spec/blob/main/docs/04-oml-grammar.md)). Confirmed live and fixed.

Issue #99 fixed this with a `RawNode::TemporalLeaf` write-hint tag layered
on top of `Scalar::Str`, since `Scalar` had no temporal variant of its own
at the time. Issue #105 gave `Scalar` real `Date`/`Time`/`Datetime`
variants, making that tag redundant -- it's been removed, and the writer
now just matches on `Scalar`'s own variant directly. The two real sources
that produce a genuine temporal variant are unchanged:

- **OML's own bare-literal grammar.** `read_oml`'s parser constructs a
  `Scalar::Date`/`Time`/`Datetime` for a genuinely-read bare literal
  token; an ordinary quoted string (however it's shaped) stays a plain
  `Scalar::Str`.
- **Schema-directed `materialize`.** Upgrading a field to a
  Date/Time/Datetime-kinded schema constructs the real variant too, so a
  materialized document writes its temporal fields bare through OML.

## Integer digit cap and `i64`

Same 4300-digit cap (`MAX_INT_DIGITS`) as `json.rs`/`yaml.rs`/`toml.rs`, and
the same `i64`-backed representational ceiling -- see
[formats/json.md](json.md).

## `--arrays` is not yet implemented

Python's `write_oml` supports an `arrays=True` mode that collapses runs of
same-label edges into `[...]` array syntax. This port's `write_oml` has no
such parameter yet -- separate, not-yet-ported library work tracked by the
CLI's own `--arrays` "not supported yet" error (see [cli.md](../cli.md)).

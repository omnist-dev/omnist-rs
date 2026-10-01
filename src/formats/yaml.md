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
mapping or sequence of mappings).

The order is normative (omnist-spec `formats/yaml.md`): merged entries come
first, in source order (`<<: [*a, *b]` is `a`'s entries, then `b`'s -- never
reversed), then the mapping's own. A key supplied more than once resolves to
one edge at the position of its first occurrence, carrying the mapping's own
value when it writes one, otherwise the earliest alias's. A merged mapping's
own `<<` is flattened first, so a grandparent's entries arrive before the
parent's.

## Byte-order marks

A leading `U+FEFF` is stripped once, before the YAML library sees the text
(D-15); a second is rejected at `1:1` as `parse.codec-syntax` (D-21), because
`yaml_rust2` would otherwise keep it as part of the first key. The one shape
this costs is an *unquoted* first key beginning with the mark; quote it. The
writer quotes any string that starts with the mark, so written YAML always
reads back.

## Alias expansion limit (D-18, D-19, D-20)

The reader enforces omnist-spec section 2.4.1: for every *candidate node* `a`
it computes `E(a) = W(a) / S(a)`, where `S` is the number of value slots
written in `a`'s own definition (an alias counts as one slot) and `W` the
number of slots materialized when `a` is expanded (an alias contributes its
target's whole `W`). It rejects the input with `document.limit.alias-expansion`
at path `$` when any `E` exceeds the maximum, which is **50 by default**
(`omnist::formats::yaml::DEFAULT_MAX_ALIAS_EXPANSION`).

- **Candidates** are every anchored node, and every mapping and sequence
  whether anchored or not: the document root, an unanchored container that
  merely holds aliases, and a mapping written inline as a merge source
  (`<<: {a: 1}`). Scalars are never checked. Each candidate is measured over
  its own subtree, so one amplifying mapping is rejected where it sits, not
  diluted by a large document around it.
- **A plain alias** (`*b`) is one slot in `S` and contributes `W(b)` to `W`.
  **A merge-key reference** (`<<: *b`) is one slot in `S` and contributes
  `W(b) - 1` (the container is flattened into the referring mapping, not
  reproduced). `<<: [*p, *q]` is one slot in `S`, no slot of its own in `W`,
  and each alias contributes `W - 1`.
- **`W` is a structural upper bound**, deliberately blind to key collisions:
  `q: &q {<<: *p, k: 9}` counts the merged `k` even though the local one
  displaces it. The reader never refines `W` downward.
- **The check runs before anything is materialized** (D-19). It consumes
  `yaml_rust2`'s event stream in one pass with each anchor's `W` memoized, so
  the work is linear in the input however often a definition is referred to;
  the events are buffered and only replayed into the tree builder once every
  candidate has passed. `W` and `S` are `u64` with saturating arithmetic, and
  `E` is checked as each container completes, so no stored `W` exceeds
  `max x S` of the node in hand.
- **A self-referential anchor** (`a: &A {b: *A}`, `a: &A {<<: *A}`, or a cycle
  through other definitions) has unbounded `W` and is rejected under the same
  code (D-20); no finite `E` is computed and nothing cyclic is built.
- **Known edge, same reading as the Go port:** an *anchored* literal merge
  sequence (`<<: &s [*p, *q]`) is treated as an ordinary merge value (a
  container that is flattened), not as the syntactic carrier of the
  unanchored form. Its slot then counts in `S`, so `E` is slightly
  under-counted for that one spelling. Every other candidate is still checked.

### The maximum is configurable

```rust
use omnist::formats::yaml::{YamlReadOptions, read_yaml_with};

let options = YamlReadOptions::default().with_max_alias_expansion(200);
let doc = read_yaml_with("a: &x [1, 2]\nb: *x\n", &options).unwrap();
assert_eq!(doc.root().get("b").len(), 2);
```
<!-- verified-by: omnist/src/formats/yaml/alias_tests.rs::doc_example_raise_the_maximum -->

`read_yaml` uses the default. `YamlReadOptions::max_alias_expansion` is a
`u32`: **`0` selects the default 50** (a zero or unset value never widens the
limit), and **a value above 10000 is refused** by `read_yaml_with` (and by
`YamlReadOptions::validate`) with an uncoded `DocumentError` rather than
clamped. "No limit" is not a legal setting (D-10). The command-line tool
always reads with the default.

### What the default accepts and rejects

Ordinary configuration reads far below the default: a `<<: *defaults` block
reads `E = 1.00` for the anchored defaults, 100 compose services each merging
a 20-key defaults anchor read about 7, and a scalar anchor aliased 500 times is
exactly 1. But a mapping that merges an `n`-key anchor and writes one key of
its own has `W = n + 2` over `S = 3`, so `E = (n + 2) / 3`: **merging a
150-key anchor into such a mapping reads `E ~ 50.67` and is rejected at the
default**, while 148 keys (`E = 50.00`) is accepted (only `E > max` rejects).
Raise `max_alias_expansion` (up to 10000) for configurations that large.

### It is a ratio, not a size cap

`E` bounds the expansion of each node relative to what that node writes. It
bounds the whole document too: `W(root) <= max x S(root)`. It does **not**
bound a scalar-heavy document that is large without any aliasing, and the
node cap below (`MAX_MATERIALIZED_NODES`, 100,000, reported as
`document.limit.nodes`) is a separate, absolute size limit that the reader
still applies after the check has accepted the input. Neither limit stands in
for the other: a document can sit far under the node cap and still be refused
for its ratio (a 100-key block aliased 100 times at the document root), and a
document of unaliased scalars can pass the ratio check at `E = 1.00` and still
be refused for its size. Parse cost of the input text itself is linear and is
paid before and independently of both.

## Native temporal type on read, but no bare-time literal, and a looser input grammar than JSON

A bare YAML timestamp reads as a genuine `Scalar::Date` (no `T`) or
`Scalar::Datetime` (has one) (issue #105) -- never `Scalar::Time`: YAML's
own `normalize_timestamp` grammar always requires a date component, so
there's no bare-time literal to produce one from. YAML's timestamp grammar
is looser than JSON's (space-separated date/time, single-digit month/day,
a bare `Z` suffix, no zero-padding); this module normalizes any such
spelling to the same canonical, zero-padded, `T`-joined ISO shape PyYAML's
own `datetime.isoformat()` would produce -- so `2001-12-14 21:59:43.10 -5`
round-trips to `2001-12-14T21:59:43.100000-05:00`, not its original
spelling. A timestamp-shaped string naming a calendar/clock value that
doesn't exist (`2024-13-01`) is a `ParseError`, matching PyYAML's own
construction-time failure.

On write, only a genuine `Scalar::Date`/`Datetime` writes bare; a plain
`Scalar::Str` that merely looks like one always writes quoted, matching
Python exactly (see [Python
divergences](../python-divergences.md#date-shaped-strings-as-native-temporal-literals-tomlyaml-resolved-by-issue-105)).
A genuine `Scalar::Time` (e.g. from OML's own bare-time grammar, or a
schema-directed upgrade) has no native YAML spelling at all and always
writes quoted.

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

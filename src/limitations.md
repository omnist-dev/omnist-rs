# Limitations & stability

## Alpha status: `0.6.0-alpha`, per this project's versioning rule

The Rust port's first feature-complete milestone (issue #28) plus its own
conformance-test harness against
[omnist-spec](https://github.com/omnist-dev/omnist-spec) (issue #82 --
see [Conformance against omnist-spec](conformance.md) for the real,
measured results) are both now in place, and the maintainer has signed
off on moving past `0.0.x` to mark that milestone. It still ships
`-alpha`, though: there is **no beta** until the maintainer explicitly
signs off on the scoping decisions below (the `any`-type gap chief among
them); accumulating further features or fixes alone never moves it past
`-alpha` on its own. Treat every public API in this crate as subject to
change without a deprecation cycle until that further sign-off happens.

## Programmatic-schema rules from spec v0.28.0-beta (S-8, S-22, S-23, S-24)

These four rules take a schema built through the API, which no conformance
vector can supply (DIV-5), so the unit tests in `omnist/tests/spec_v028.rs` are
the only pin.

- **S-8 and S-24/OSD-16 are implemented.** `Schema::new` reports a bad record
  or `Ref` target name as `schema.invalid-name` at `$`; `osd::to_osd` fails
  with `write.unsupported-value` at the record path on a `max = 0` field. Which
  violation is reported when several exist is unspecified.
- **S-22 (`schema.invalid-label`) is vacuous in Rust.** A field label is a
  `String`, so it cannot hold invalid UTF-8. Every public way to give a label
  takes `impl Into<String>` (`Field::new`, `Field::required`, ...); there is no
  `OsString`, `Vec<u8>` or `Cow<[u8]>` route into a schema, so there is nothing
  to check and no code path emits `schema.invalid-label`. Input bytes are a
  different rule (D-14, `parse.invalid-encoding`).
- **S-23 (`schema.unknown-record`) has no Rust surface.** No public function
  takes a caller-supplied ordering of records: `to_osd` and the other writers
  emit `Schema::env()` in its own order, and `extract`'s `keep` argument is a
  set of field labels, not records. `schema.unknown-record` is therefore never
  emitted.
- **No OSD-OML schema writer.** The port has no OSD-OML schema reader or
  writer, so the OSD-OML half of OSD-16/S-24 does not apply.

## The `any`-type support (landed)

Python's schema model has an `AnyType`/`ANY` type and an `allow_any` option
several APIs (`osd`, schema algebra, inference) use as a fallback when a
precise type can't otherwise be resolved. In this Rust port, `FieldType::Any`
is fully supported across `omnist::schema`, `omnist::osd` parsing (`record X { "a": any }`),
and `omnist::infer` (with `allow_any` fallback mode when schemas have ambiguous
types or mixed structures, also wired into the CLI's `infer --allow-any` flag).

## Safety limits and the YAML alias expansion factor (D-10, D-11)

Every limit below is finite, documented here, and reported with its
`document.limit.*` code (spec section 2.4):

| Limit | Value | Configurable |
|---|---|---|
| Maximum nesting depth | 200 | no (compile-time `MAX_DEPTH`) |
| Maximum node count | 1,000,000 (documents), 100,000 (YAML materialization) | no |
| Maximum integer digits | 4,300 | no |
| Maximum alias expansion factor (YAML) | **50** | yes: `YamlReadOptions::max_alias_expansion` (`0` = default, at most 10000) |
| Maximum expanded size (YAML, inputs with an alias or merge key) | **1,000,000** value slots | yes: `YamlReadOptions::max_expanded_slots` (`0` = default, at most 10,000,000) |

The alias expansion factor bounds the materialized-to-written value-slot
ratio of every anchored node, every other mapping and sequence, and the
document root; see [YAML](formats/yaml.md#alias-expansion-limit-d-18-d-19-d-20)
for the counting rules and for what the default accepts (for example, merging
a 150-key anchor into a mapping that writes one key of its own reads
`E ~ 50.67` and is rejected at the default). It is a ratio, so it does **not**
replace the node cap: a scalar-heavy document is bounded as a whole only by
`max x S(root)`, and size limits such as the YAML node cap stay in force.
Only YAML has an anchor mechanism, so the factor binds no other codec.

The ratio does not bound absolute size, so the reader also enforces a maximum
expanded size (D-22): an input that contains at least one alias or merge key
and whose root materializes more than 1,000,000 value slots is refused with
`document.limit.expanded-size`, after the ratio check (an input that fails
both reports `document.limit.alias-expansion`). Ratio and size are separate
options, and neither implies the other. An input with no alias and no merge
key is exempt however large, which is a cliff by design: a plain file of two
million slots passes, and adding one alias subjects it to the cap. See
[YAML](formats/yaml.md#expanded-size-limit-d-22). Rust's node cap (100,000,
keys included) is lower than D-22's default, so for example the spec's own
1,000-service x 60-key compose example (`W` = 62,063) is refused with
`document.limit.nodes`; this is pre-existing and the node cap is unchanged. A malformed merge (a scalar
merge value, a scalar or sequence member of a merge sequence, an alias to a
sequence of scalars) is `parse.codec-syntax` and wins over both limits.

## `Scalar::Int` is arbitrary-precision (issue #104)

`omnist::document::Scalar::Int` and `Value::Int` are backed by
`num_bigint::BigInt`, not a fixed-width integer -- matching omnist-spec
[section 2.2](https://github.com/omnist-dev/omnist-spec/blob/main/docs/02-document-model.md#22-values)'s
requirement that `integer` be arbitrary-precision, and Python's/Go's own
representations (`int`, `*big.Int`). This was previously `i64` (max ~19
significant decimal digits) -- a real spec-conformance bug, not a
disclosed permitted variation, since a 20+ digit literal under the shared
4,300-digit security cap was rejected outright with no
`declared_max_int_digits` override in play (omnist-spec ledger entry D-9).
Fixed; see each format's own page for anything still worth knowing:

- [formats/toml.md](formats/toml.md) -- **one real, external divergence
  remains**: `toml_edit`, the crate this port's TOML codec is built on,
  has its own `i64`-backed integer type (the TOML 1.0 format spec itself
  documents 64-bit signed integers), so a >19-digit integer literal in
  TOML *source text* is still rejected -- by `toml_edit`'s own parser,
  before this port's `Scalar` is ever involved. Writing an
  arbitrary-precision `Scalar::Int` *to* TOML still succeeds (this
  codec's writer renders integers as plain digit text, not through
  `toml_edit`'s typed API), so the asymmetry is read-side only: such a
  value round-trips out but not back in through TOML specifically. Every
  other format (JSON, YAML, OML) has no such ceiling.

## Temporal kinds have no arithmetic

`Scalar`/`Value` carry real `Date(String)`/`Time(String)`/`Datetime(String)`
variants (added in issue #105), each holding an already shape-validated,
canonical ISO spelling -- but the string is opaque data, not a `chrono`/
`time` value. There is no date arithmetic, comparison, or component
extraction anywhere in this crate; the algebra never needed it (mirroring
the same no-arithmetic reasoning `Scalar::Int`'s `BigInt` backing already
applies to integers). This means:

- `omnist::infer::infer` infers `date`/`time`/`datetime` only from a
  genuinely temporal-kinded sample (one already read as `Scalar::Date`/
  `Time`/`Datetime` -- e.g. from OML's or TOML's own native temporal
  grammar); a plain ISO-shaped *string* sample still infers as `string`,
  matching Python's own strict `value_kind()` exactly.
- `omnist::schema::matches_kind`, by contrast, accepts either a real
  temporal variant *or* a shape-matching plain string for a
  `Date`/`Time`/`Datetime`-typed field -- also matching Python's own
  hybrid `matches_kind` exactly. A schema-directed `materialize` upgrade is
  what promotes a matching string to the real typed variant.
- Formats with a native temporal type on the wire (TOML's four temporal
  literal forms, YAML's looser timestamp grammar) now construct the real
  typed variant directly on read and write it back bare on write -- no
  more silent collapse to `Scalar::Str`; see each format's own page for
  the exact behavior (particularly [formats/toml.md](formats/toml.md),
  whose write-side shape-guessing divergence from Python is now resolved).

## Architecture-freedom disclosures already made per codec

Beyond the two structural gaps above, each format module documents its own
disclosed, live-checked divergences from the Python reference (namespace
resolution in XML, ASCII-only digit parsing in XML's coercion, `strict`
vs. non-`strict` OML-Extended string spellings, and more) -- see
[formats/](formats/) for the specifics, all checked against a live Python
interpreter or the Python reference's own merged PR history, not assumed
from memory.

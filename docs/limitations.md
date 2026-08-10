# Limitations & stability

## Alpha status: `0.1.0-alpha`, per this project's versioning rule

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

## The `any`-type scoping gap (deferred, not forgotten)

Python's schema model has an `AnyType`/`ANY` type and an `allow_any` option
several APIs (`osd`, schema algebra, inference) use as a fallback when a
precise type can't otherwise be resolved. This Rust port's
`omnist::schema` module (issue #6) **deliberately does not implement
`any`** -- its inclusion in the public API is an explicitly deferred design
question pending user sign-off, not an oversight. The same scoping choice
threads through every module that would otherwise need it:

- `omnist::osd` still recognizes `"any"` as a reserved schema-text keyword
  (so it can't be used as a record name), but using it as a field's type
  returns a clear `SchemaError` explaining it isn't supported yet, rather
  than silently misparsing.
- `omnist::infer::infer` has no `allow_any` fallback: a label whose samples
  mix objects and scalars, or whose scalars disagree on kind outside the
  integer/number subset relation, returns a `SchemaError` instead of
  silently degrading to `any`.
- The CLI's `infer --allow-any` flag is accepted by the argument parser
  (matching Python's surface) but returns the same "not supported yet"
  error rather than doing something different from plain `infer`.

Closing this gap is 1.0-gating work, not a bug to fix incrementally --
see the sibling `omnist` project's own `any`-openness decision, which this
port's scoping deliberately mirrors rather than resolves independently.

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

## No native temporal `Scalar` variant

`Scalar` has no `date`/`time`/`datetime` variant at all -- every temporal
value, in every format, is represented as a `Scalar::Str` holding its
canonical ISO spelling, validated by the shared
`crate::schema::is_iso_date`/`is_iso_time`/`is_iso_datetime` checks. This
means:

- `omnist::infer::infer` never infers `date`/`time`/`datetime` from a
  string-shaped sample -- every string sample infers as `string`. A schema
  wanting a temporal field must be authored (or edited in after
  inference), not inferred.
- Formats with a native temporal type on the wire (TOML's four temporal
  literal forms, YAML's looser timestamp grammar) still round-trip through
  a `Scalar::Str` internally; see each format's own page for exactly what
  that means for their write-side behavior (particularly
  [formats/toml.md](formats/toml.md)'s divergence from Python's
  live-`datetime`-object model).

## Architecture-freedom disclosures already made per codec

Beyond the two structural gaps above, each format module documents its own
disclosed, live-checked divergences from the Python reference (namespace
resolution in XML, ASCII-only digit parsing in XML's coercion, `strict`
vs. non-`strict` OML-Extended string spellings, and more) -- see
[formats/](formats/) for the specifics, all checked against a live Python
interpreter or the Python reference's own merged PR history, not assumed
from memory.

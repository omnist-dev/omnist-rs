# Limitations & stability

## Alpha status: `0.0.x`, per this project's versioning rule

This is the Rust port's first feature-complete milestone -- all library
modules, all five format codecs, the CLI, a fuzzing/oracle harness, and
this documentation are now in place (issue #28, the last port-order item
per `docs/workflow-playbook.md` §4). It still ships as `0.0.x`. There is
**no beta** until the project's maintainer explicitly signs off on the
scoping decisions below; accumulating features or fixes alone never moves
the version past `0.0.x`. Treat every public API in this crate as subject
to change without a deprecation cycle until that sign-off happens.

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

## Representational limits from `Scalar::Int` being `i64`

Every format codec shares one representational ceiling this port's Python
and TypeScript siblings don't have: `omnist::document::Scalar::Int` is a
plain `i64` (max ~19 significant decimal digits), not an arbitrary-
precision integer. Each format's own page documents the specific
consequence:

- [formats/json.md](formats/json.md), [formats/yaml.md](formats/yaml.md) --
  a decimal integer literal over 19 digits (but under the shared
  4300-digit security cap) is a `ParseError` ("out of range for a 64-bit
  integer"), where Python holds it as an exact arbitrary-precision `int`.
- [formats/toml.md](formats/toml.md) -- the same cap is applied uniformly
  to hex/octal/binary literals too, a disclosed divergence from Python
  (which exempts those radixes from its digit-limit guard entirely).
- [formats/xml.md](formats/xml.md) -- an over-`i64` numeral falls through
  to `Scalar::Float` instead of erroring, mirroring Python's own
  int-then-float coercion fallback, just with a narrower `Scalar::Int`.

None of this is a bug: it is the direct, disclosed consequence of issue
#4's Document-model architecture decision (`Scalar` has exactly five
variants, `Int` is `i64`), applied consistently across every codec rather
than special-cased away.

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

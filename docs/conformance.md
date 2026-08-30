# Conformance against omnist-spec

This port has its own conformance-test harness (`tools/conformance/`)
against [omnist-spec](https://github.com/omnist-dev/omnist-spec), the
language-agnostic upstream specification. It vendors omnist-spec as a
pinned git submodule (`vendor/omnist-spec`, currently commit `aac3ce0`,
past the `v0.5.0-beta` tag -- pins the fix for
[omnist-spec#52](https://github.com/omnist-dev/omnist-spec/issues/52),
a new Sec8.5.3 rule for XML-whitespace-insensitive conformance-vector
comparison) and
runs entirely against this crate's own library code -- it does not depend
on the Python or TypeScript ports' implementations.

Two tracks, both wired into CI as a dedicated `conformance` job
(`.github/workflows/ci.yml`), gated on real fail count only, never on
skip count, per the spec's [section
8.5.5](https://github.com/omnist-dev/omnist-spec/blob/main/docs/08-conformance-and-errors.md#855-reporting)
reporting rule:

- **Track 1** (`vendor/omnist-spec/conformance/fixtures/`, directory-per-fixture,
  11 operations): **19 passed, 0 failed, 0 skipped**.
- **Track 2** (`vendor/omnist-spec/test-suite/`, JSON-vector suite, 14-operation
  vocabulary): **166 passed, 0 failed, 6 skipped** (of 172 vectors).

Zero real fails on either track as of this writing. Run it yourself:

```
cargo run -p conformance --bin self-test
cargo run -p conformance --bin runner
cargo run -p conformance --bin vector_runner
```
<!-- doc-illustrative -->

## Every Track 2 skip, and why

Every skip below is cited in `tools/conformance/src/bin/vector_runner.rs`'s
own source (either "not yet implemented" or a numbered entry in the
spec's [divergence
ledger](https://github.com/omnist-dev/omnist-spec/blob/main/docs/09-divergence-ledger.md)
section 9.4), per section 8.5.5's requirement that no skip go unexplained.
All 6 remaining skips are now one category:

- **6 `limits.json` vectors**: each expects a *vector-local* configurable
  limit (a specific max-nodes/max-depth/max-int-digits value scoped to
  that one test case). This port's `MAX_NODES`/`MAX_DEPTH` are general,
  crate-wide constants (`omnist::document`), not something the harness can
  override per-vector -- there is no representational path to make these
  pass without adding a runtime-configurable limits API this port doesn't
  otherwise need. Distinct from the divergence ledger's D-1 entry in the
  specific reason (a vector-local knob, not a fixed-ceiling-value
  mismatch), even though both are filed under the same general "limits"
  heading.

The OSD-grammar and OML-grammar skips this section used to describe (~16
of them, syntax the parser didn't yet accept) are gone -- this port's
grammar coverage is complete now; every remaining skip is the
vector-local-limits category above.

## Where this port's real ceiling differs from Python's/TypeScript's --
not implied parity

- **Divergence ledger D-6** (integer/number-kind-collapse) is
  **TypeScript-only** and does not apply to this port -- confirmed
  empirically, not assumed: `Scalar::Int(i64)`/`Scalar::Float(f64)` are
  separate enum variants here, unlike TypeScript's shared `number`, so the
  collapse D-6 describes structurally cannot happen in Rust.
- This port's `ParseError { line, col, message }` is **structured**
  (unlike TypeScript's message-only `ParseError`), which let most
  syntax-failure vectors run for real here instead of blanket-skipping --
  a genuinely favorable per-language difference, found empirically while
  building the harness, not assumed going in.
- Diagnostics are compared in **code-agnostic mode** (path-set only, not
  exact error-code string). This port's `ErrorCode::as_str()` now *does*
  produce the spec's family-namespaced codes (`"validate.type-mismatch"`,
  `"materialize.inexact-conversion"`, per §8.3.1, fixed in issue #152) --
  but the vectors and fixtures are still compared code-agnostically
  regardless, since some still carry the pre-namespacing bare form
  recorded against the reference implementation (omnist-spec D-4, open).
  Same mismatch found independently on the TypeScript port; not
  Rust-specific.

## Real bugs this harness found and fixed

Building this harness against the real spec (rather than trusting the
Python/TypeScript ports as ground truth) found seven real product bugs,
all fixed across this port's `0.1.0-alpha`/`0.1.1-alpha` releases:

- **XML reader was type-coercing leaf text** (int/float/bool) at parse
  time, contradicting the spec -- XML has no typed literals. Fixed:
  `read_xml` always produces string leaves now; see
  [XML](formats/xml.md).
- **YAML's implicit-int resolver was missing the legacy sexagesimal
  form** -- `12:00:00` stayed a string instead of resolving to `43200`.
  Fixed; see [YAML](formats/yaml.md).
- **YAML mapping keys were never run through the implicit-type
  resolver** (the "Norway problem") -- `on:` wasn't rejected as YAML 1.1
  requires. Fixed to match Python's reference behavior exactly (any
  non-string key is rejected, not just bool/null-shaped ones); see
  [YAML](formats/yaml.md).
- **OML's tokenizer wasn't canonicalizing temporal literal text** --
  missing seconds got dropped instead of filled to `:00`, and sub-second
  fractions weren't zero-padded to 6 digits; see [OML](formats/oml.md).
- **OML's writer shape-guessed date/time/datetime from string content**
  to decide bare-vs-quoted, since `Scalar` has no temporal variant (issue
  #16) and thus no real provenance signal -- a plain JSON string that
  merely looked date-shaped got silently promoted to a genuine OML
  temporal literal on write. Found while directly verifying, not just
  trusting, this suite's own reported numbers: the bug had a fully-green
  117/0/22 run despite existing, because no vector at the time tested it.
  Fixed by tagging genuine provenance (OML's own bare-literal grammar, or
  a schema-directed `materialize` upgrade) on `RawNode` instead of
  guessing from shape; see [OML](formats/oml.md#bare-vs-quoted-on-write-real-variant-not-shape-guessing).
  omnist-spec's own `v0.1.1-alpha` adds the 6 vectors
  (`formats-oml/oml.json`) this fix now passes for real.
- One harness-side false fail: a JSON temporal-write-report vector is
  structurally unreachable given this port's `any`-scoping decision (see
  [`limitations.md`](limitations.md#the-any-type-scoping-gap-deferred-not-forgotten));
  reclassified from fail to a cited skip rather than a product fix.
- A separate harness-side skip, since resolved by issue #105: the
  `formats-json/basic/temporal-leaf-is-stringified-on-write` vector was
  structurally unreachable because `Scalar` had no temporal variant to
  preserve through the harness's own vector decoder (issue #16/#89) --
  skipped, cited, not a product bug. Issue #105 gave `Scalar` real
  `Date`/`Time`/`Datetime` variants, the decoder now preserves them, and
  this vector passes for real; its skip detector has been removed from
  `vector_runner.rs`.
- **`Scalar::Int(i64)` rejected valid arbitrary-precision integer
  literals** -- omnist-spec section 2.2 defines `integer` as
  arbitrary-precision (bounded only by the shared 4,300-digit cap), not
  fixed-width; a 20+ digit OML literal was rejected outright with no
  digit-cap override in play, a real grammar-acceptance bug (spec
  section 9.2), not a permitted narrower-limit variation. Not found by
  this harness on its own -- surfaced by a maintainer-prompted
  ledger-legitimacy audit ("is this a genuine language limitation or an
  unexamined shortcut") on the `omnist-spec` side, which added the vector
  this fix now passes for real. Fixed by moving `Scalar::Int`/`Value::Int`
  onto `num_bigint::BigInt`; see
  [Limitations](limitations.md#scalarint-is-arbitrary-precision-issue-104).
  Found and fixed along the way, not assumed mechanical: the YAML legacy
  sexagesimal literal's fold used to rely on `i64` overflow as an
  incidental size bound -- a naive `BigInt` swap would have silently
  removed it, letting a many-`:`-group literal build an arbitrarily large
  integer with nothing stopping it. Fixed by enforcing the existing
  digit cap explicitly on the fold's result instead.

None of these required filing against omnist-spec, Python, or
TypeScript -- every real fail traced back to an omnist-rs bug when
checked against a live Python run first, per this project's own
cross-implementation triage rule.

## Known non-blocking gap in the CI gate itself

`cargo llvm-cov --workspace --fail-under-lines 100`, this project's
coverage gate, has an open, unexplained discrepancy where its exit code
doesn't reliably correlate with its own printed Lines% column across
commits -- see
[omnist-rs#95](https://github.com/omnist-dev/omnist-rs/issues/95). Not
specific to the conformance work; noted here because it surfaced while
landing it.

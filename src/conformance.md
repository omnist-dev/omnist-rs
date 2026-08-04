# Conformance against omnist-spec

This port has its own conformance-test harness (`tools/conformance/`)
against [omnist-spec](https://github.com/omnist-dev/omnist-spec), the
language-agnostic upstream specification. It vendors omnist-spec as a
pinned git submodule (`vendor/omnist-spec`, currently `v0.1.1-alpha`) and
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
  vocabulary): **124 passed, 0 failed, 22 skipped** (of 146 vectors).

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
The two structural categories:

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
- **~16 remaining skips**: OSD-grammar and OML-grammar/format-specific
  vectors exercising syntax this port's parsers don't yet accept, each
  cited individually in-source with the specific grammar gap.

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
  exact error-code string) -- `ErrorCode::as_str()` produces bare codes
  (`"type-mismatch"`) while the spec's vectors expect
  operation-prefixed codes (`"validate.type-mismatch"`). Same mismatch
  found independently on the TypeScript port; not Rust-specific.

## Real bugs this harness found and fixed

Building this harness against the real spec (rather than trusting the
Python/TypeScript ports as ground truth) found six real product bugs,
all fixed across this port's `0.1.0-alpha`/`0.1.1-alpha`:

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
  guessing from shape; see [OML](formats/oml.md#bare-vs-quoted-on-write-rawnodetemporalleaf-not-shape-guessing).
  omnist-spec's own `v0.1.1-alpha` adds the 6 vectors
  (`formats-oml/oml.json`) this fix now passes for real.
- One harness-side false fail: a JSON temporal-write-report vector is
  structurally unreachable given this port's `any`-scoping decision (see
  [`limitations.md`](limitations.md#the-any-type-scoping-gap-deferred-not-forgotten));
  reclassified from fail to a cited skip rather than a product fix.

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

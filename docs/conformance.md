# Conformance against omnist-spec

This port has its own conformance-test harness (`tools/conformance/`)
against [omnist-spec](https://github.com/omnist-dev/omnist-spec), the
language-agnostic upstream specification. It vendors omnist-spec as a
pinned git submodule (`vendor/omnist-spec`, currently commit `64cbb68`,
the `v0.33.0-beta` tag) and
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
  vocabulary): **339 passed, 0 failed, 28 skipped** (of 367 vectors),
  **diagnostics compared as `(path, code)` sets** (section 8.5.2), not in
  code-agnostic mode. The runner exits non-zero on any failing vector and
  never on skips (E-22); it has no list of tolerated failures.

Neither track has a fail. Run it yourself:

```
cargo run -p conformance --bin self-test
cargo run -p conformance --bin runner
cargo run -p conformance --bin vector_runner
```
<!-- doc-illustrative -->

## Every Track 2 skip, and why

All 28 skips are one spec category, E-20 "not yet implemented"; none is an
E-21 documented divergence. The 6 `document-model/limits` vectors were skips
through 0.6.1-alpha (304 / 0 / 34); the limits became runtime-configurable in
0.7.0-alpha and they now run and pass (310 / 0 / 28).

**Adopting v0.33.0-beta (0.9.0-alpha).** The suite grew from 338 to 367 vectors
(7 repeated-label path vectors, 10 input-size vectors, 7 OML-29 vectors, 5 XML
null vectors) and the pin moved from v0.28.0-beta. Before any change the runner
reported 318 pass / 21 fail / 28 skip, and the 21 were: 5 repeated-label paths
(E-10: the first occurrence carried no index, DIV-16), 7 OML-29 (a separator
after the colon, DIV-20), 5 over-cap input-size vectors (D-23, DIV-17) and 4
XML null writes (C-10, DIV-23). The 5 at-cap input-size vectors passed falsely
at that point, because the runner ignored `declared_max_input_bytes`: it passed
the vector against the port's own default, which E-20a forbids. The runner now
passes the declared maximum through `Limits::max_input_bytes`, and a vector
carrying any other `declared_*` key, or one the runner does not honour on its
operation or format, is an E-20 skip rather than a run against the default.
After the adoption: 339 / 0 / 28, and the 28 skips are the same OSD-OML
extension vectors as before. Two further rules have no vector: C-9 (a writer
refuses a string with no UTF-8 encoding) is vacuous here, see
[Limitations](limitations.md#c-9-writers-refuse-a-string-with-no-utf-8-encoding-vacuous-here),
and the two OML reader fixes of omnist-rs#193 and #194 are pinned by unit
tests only (no vector covers them).

Earlier history: (v0.28.0-beta adds no vectors (still 338): its four programmatic-schema
rules, S-8's `$` path, S-22, S-23 and OSD-16/S-24, are pinned by no vector
(DIV-5), only by `omnist/tests/spec_v028.rs`; Track 2 stays 304 / 0 / 34.
v0.27.0-beta (338 vectors) added 7 `alias-expansion` vectors for the empty merge
sequence `<<: []` (D-18a); the port already merged nothing for it, so all 7
passed with no code change. v0.26.0-beta (331 vectors) grew
`alias-expansion.json` to 35 vectors (D-18a, D-22 and the malformed-merge
syntax errors); all run and pass, with `declared_max_alias_expansion` and
`declared_max_expanded_slots` passed through `YamlReadOptions` for the vectors
that carry them. Before this port adopted it the baseline was 289 pass / 8
fail / 34 skip: the anchored merge carrier and the alias to a merge sequence
were wrongly rejected, 4 expanded-size vectors parsed OK, and a merge after a
bomb and a merge sequence of sequences were not reported as syntax errors.
v0.25.0-beta (312 vectors) added the 16 D-18
`alias-expansion` vectors, which now all run and pass with the declared
maximum passed through `YamlReadOptions::max_alias_expansion`, and the 4
E-32 `line:col` placeholder vectors, which pass: a `parse.codec-syntax`
expectation whose path is the placeholder is satisfied by the same code at a
well-formed text position inside the input, and every other path is still
compared byte for byte. Before this change, with the alias skip removed and
nothing implemented, the baseline was 266 pass / 12 fail / 34 skip: the 8
alias vectors that expect a rejection and the 4 placeholders. v0.22.0-beta added 14 vectors (287 in all): 10
OML-26 with-a-separator vectors, the OML-25 scalar counterpart, OML-26's
negative control and 2 E-28 code-point column vectors, all passing; before
this port adopted OML-26 and E-28 the baseline was 238 pass / 9 fail / 40
skip, the failures being 7 OML-26 vectors and the 2 column vectors, see
the CHANGELOG. The suite grew from 249 to 273 vectors across
v0.20.0-beta and v0.21.0-beta -- 5 OML-26/27, 4 OSD-15 canonical-escaping and
1 S-21 `infer` vector at v0.20.0-beta, then 14 new `bytes_hex` D-14 vectors at
v0.21.0-beta -- and this port now runs every one of them for real, including
`bytes_hex` through the CLI, its byte-oriented entry point, per E-27; see
"Byte inputs (D-14, E-27)" below. The 4 OSD-15 vectors were already passing
before the v0.21.0-beta work landed -- the writer was already correct; see
below.) Each skip's reason is generated by
`tools/conformance/src/bin/vector_runner.rs` and checked against the vector's
own input by the `every_skip_reason_is_true_for_its_vector` test, so it
cannot drift from what is actually skipped. A vector whose actual error lacks
a structured `code` or `path` FAILS; it is never skipped for lack of one.

- **28 `extensions-osd-oml/*` vectors.** The OSD-OML extension operations
  (`parse_schema_oml`, `schema_from_document`, `schema_to_document`,
  `write_schema_oml`) are not implemented; tracked in
  [omnist-rs#175](https://github.com/omnist-dev/omnist-rs/issues/175).

## Where this port's real ceiling differs from Python's/TypeScript's --
not implied parity

- **Divergence ledger D-6** (integer/number-kind-collapse) is
  **TypeScript-only** and does not apply to this port -- confirmed
  empirically, not assumed: `Scalar::Int(i64)`/`Scalar::Float(f64)` are
  separate enum variants here, unlike TypeScript's shared `number`, so the
  collapse D-6 describes structurally cannot happen in Rust.
- This port's `ParseError { line, col, code, message }` is **structured**
  (unlike TypeScript's message-only `ParseError`), which let most
  syntax-failure vectors run for real here instead of blanket-skipping --
  a genuinely favorable per-language difference, found empirically while
  building the harness, not assumed going in.
- Diagnostics are compared as **`(path, code)` sets** (section 8.5.2), the
  strict mode. Up to spec v0.9.1-beta this runner compared paths only, which
  is how vectors reported green while the code was wrong (`infer` and
  `extract` failures were not even compared, and `extract` reported a field
  path where the spec wants the record). Every error type now carries what
  the taxonomy needs: `ParseError { line, col, code, message }`,
  `SchemaError { path, code, message }`,
  `DocumentError { path, code: Option<_>, message }`,
  `WriteError { path, code, .. }`, and `WriteReport` adjustments.
  Diagnostics this port cannot yet emit exactly are listed in
  [omnist-rs#182](https://github.com/omnist-dev/omnist-rs/issues/182).

## Byte-order marks (D-15, D-21)

One leading `U+FEFF` is stripped on every read surface (OML, OSD, JSON, YAML,
TOML, XML); a second is rejected at `1:1` (`parse.unexpected-token` on OML and
OSD, `parse.codec-syntax` on the four codecs); a `U+FEFF` anywhere else is
ordinary content. The stripping lives in exactly one helper
(`omnist/src/bom.rs`) that every reader calls before it does anything else,
so no reader or library strips a second one. Writers never emit one. Before
this change (measured, not assumed): OML stripped one and rejected a second at
`1:1`; TOML and XML stripped one (inside their libraries) but rejected a
second at the wrong position (`1:4`); JSON and OSD rejected even a single
mark; YAML kept every mark *as part of the first key*, so one mark silently
renamed the key and two were accepted. Two tests guard it: one scans every tracked text
file's bytes for a raw `EF BB BF` (the mark is written as `\u{FEFF}` in
source, never raw), and `omnist/tests/bom.rs` exercises all six readers and
every writer. The one shape YAML loses under D-21 is an *unquoted* first key
that begins with the mark; the YAML writer quotes any string that starts with
one, so a Document this port writes always reads back.

## Byte inputs (D-14, E-27)

New in v0.21.0-beta: a read-side vector (`parse`, `parse_schema`,
`parse_schema_oml`) may give its input as `bytes_hex` (lowercase hex, two
digits per byte) instead of `text`, so D-14 -- reject invalid UTF-8 with
`parse.invalid-encoding` at `1:1`, before the BOM strip and doubled-BOM check
-- can finally be pinned by a vector; a JSON-vector `text` field cannot hold
ill-formed bytes, so the rule was untestable before this.

This crate's readers all take `&str`, which is always already-valid UTF-8 by
construction (omnist-spec section 2.5 lets a string-typed entry point treat
its input as decoded), so the check has to run earlier, on real bytes.
`omnist-cli` is this port's byte-oriented entry point -- it reads a file or
stdin -- so it is where D-14 is enforced: `omnist_cli::decode_input` decodes
strictly and, on failure, returns the fixed diagnostic (`1:1`,
`parse.invalid-encoding`), never a lossy repair. Every CLI command that reads
document, OML or OSD input (`format`, `convert`, `check`, `validate`,
`infer`, every `schema` subcommand, and `--schema`) goes through it.

The vector runner (`tools/conformance/src/bin/vector_runner.rs`) presents a
`bytes_hex` vector's bytes to that same entry point -- `omnist-cli`'s
`read_document_bytes`/`read_oml_bytes`/`parse_schema_bytes`, added as a small
public surface on the `omnist-cli` crate for exactly this -- **never** by
decoding with `U+FFFD` replacement and running the result through the
ordinary `&str` reader, which the spec calls out by name as the one wrong way
to run these vectors. `tools/conformance/Cargo.toml` depends on `omnist-cli`
for this; it is an in-process function call, not a spawned subprocess, since
Track 2 already links against the library crates directly. A `text` vector on
these three operations still goes straight to the `&str` reader, unchanged.

All 14 `bytes_hex` vectors pass: the 8 invalid-UTF-8 ones (a truncated
sequence, an overlong encoding, a lone continuation byte, an encoded
surrogate, a byte above the Unicode range, one pinning the offset-zero case
and one pinning line 2 -- both still `1:1`) and the 6 valid multi-byte
controls, proving the check does not also reject legitimate multi-byte text.
`omnist-cli`'s own test suite (`omnist-cli/tests/cli.rs`) drives the same
check directly through the compiled binary over stdin and files, on all six
surfaces, plus the D-21 boundary (a truncated BOM is D-14; a valid doubled BOM
is D-21, not D-14) and the "no panic on hostile bytes" sweep described below.

## Canonical OSD escaping and the unwritable label (OSD-15, OSD-14)

New conformance vectors in v0.21.0-beta, but OSD-15's *rule* -- a backslash is
written `\\`, a double quote `\"`, nothing else -- was already satisfied by
this port's `omnist::osd::quote_label` before this sweep. **A draft of this
document previously claimed a fix that did not happen**; verified directly
(`git diff origin/main -- omnist/src/osd.rs` at the v0.19.0-beta -> v0.21.0-beta
boundary never touches `quote_label`, and the pre-sweep code was measured
against the four new `osd-grammar/canonical-output/label-*` vectors on its
own: 219 pass / 14 fail / 40 skip, with all 14 failures being `bytes_hex`
D-14 vectors and zero being OSD-15 vectors).

OSD-14 (a field label with a C0 control character has no OSD spelling) has no
conformance vector -- the suite's `schema` field is always OSD text, and a
schema with such a label has none -- so `to_osd` is unit- and property-tested
directly instead (`omnist/src/osd.rs`, `omnist/tests/fuzz.rs`): a schema built
programmatically with a control-character label fails with
`write.unsupported-value` at the *record's* path, and a `proptest` property
(`osd_write_read_round_trips_for_arbitrary_valid_labels`) checks
`parse_schema(to_osd(s).unwrap()).unwrap() == s` for arbitrary non-empty
labels excluding `[`/`]` and C0 controls, generated with a distribution that
frequently includes backslashes, quotes and non-ASCII text (not just plain
ASCII words), so it actually exercises OSD-15's escaping on every run.
`to_osd` returning `Result` rather than `String` is a breaking API change for
an alpha crate (see the crate's own CHANGELOG); every caller in this
workspace (the CLI, examples, tests, both conformance runners) was updated.

## The conformance harness's own comparison bugs (found by independent review)

Two real gaps in `tools/conformance` itself, found by an independent review
of this PR and fixed with mutation evidence, not just re-reading the code --
and one over-correction in the first attempt at fixing them, caught by a
second independent review and corrected in turn:

- **`run_parse_schema` (Track 2) never compared `expect.schema`.** A vector
  giving `expect.schema` alongside a successful `parse_schema` only ever
  checked `ok: true`, never the schema's canonical text -- proven by
  deleting all escaping from `quote_label` in a scratch copy and observing
  no change in the reported pass count. Fixed: when `expect.schema` is
  present, the parsed schema is rendered through `to_osd(&schema, Some(4))`
  (the canonical pretty form) and compared byte-for-byte, per omnist-spec
  section 8.5.3.
- **Track 2's schema-output comparisons (`normalize`/`prune`/`extract`) were
  structural, not byte-for-byte, when section 8.5.3 requires byte-for-byte.**
  `referee::compare_schema`'s `"exact"` mode re-parsed both OSD texts and
  compared `Schema == Schema`, blind to any writer formatting difference
  that doesn't change the parsed structure -- proven by widening `to_osd`'s
  pretty-mode indent from one space to three and observing both conformance
  tracks stay green.

  **The first fix over-corrected: it made `"exact"` itself byte-for-byte,
  which is right for Track 2 but wrong for Track 1.** Track 1's own spec
  (`vendor/omnist-spec/docs/conformance-harness.md` section 4's reference
  pseudocode, and section 6's self-test fixtures --
  `01-schema-exact-equal-different-field-order`,
  `07-schema-exact-equal-different-env-declaration-order` -- one of them
  literally titled "two structurally identical schemas, written with
  different field order and different indentation, compare equal under
  `exact` mode") defines `"exact"` as **structural**, deliberately, and
  Track 1's own `runner.rs` self-test fixtures failed for real once
  `"exact"` was redefined globally. A second review caught this before it
  shipped.

  The corrected design: `referee::compare_schema` now has **three** modes.
  `"exact"` stays structural (`Schema == Schema`), used only by Track 1's
  `runner.rs`, matching `docs/conformance-harness.md`. A new `"canonical"`
  mode does the real byte-for-byte comparison Track 2 needs, used only by
  `vector_runner.rs` for `normalize`/`prune`/`extract`/`parse_schema`'s
  `expect.schema` (section 8.5.3 explicitly requires it for `normalize` and
  `parse_schema`, and `extract` inherits it by delegating its final step to
  `normalize`, per `docs/06-schema-algebra.md` section 6.9).
  `"isomorphic"` (`infer`) is unaffected in either track -- correctly
  structural, since an inferred schema's record names are
  implementation-derived and never expected to match byte-for-byte.

Both fixes were verified by mutation, twice: once against the (later
corrected) global change, and again against the final three-mode design. A
deliberate `to_osd` spacing change now fails 18 Track 2 vectors while
leaving Track 1 at 19/0 (exactly as `docs/conformance-harness.md` says it
should -- Track 1's `"exact"` is meant to tolerate this); deleting
`quote_label`'s escaping entirely fails the 4 OSD-15 vectors directly.
Neither mutation-and-revert cycle surfaced an actual writer bug -- with the
corrected comparisons in place, both tracks were unchanged at 233/0/40 (of
273, v0.21.0-beta) and 19/0.

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

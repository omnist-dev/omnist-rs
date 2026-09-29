# Changelog

## 0.3.0-alpha (unreleased, continued)

Adopts omnist-spec **v0.21.0-beta** (was v0.19.0-beta), still unreleased on
`main` as 0.3.0-alpha -- these changes are breaking for library users (see
below), but since 0.3.0-alpha has not been published yet, no further version
bump is needed; the breaking surface simply lands in the same unreleased
release.

Conformance, Track 2 (JSON vectors), before and after, `(path, code)` set
comparison:

- v0.19.0-beta suite, previous release: 209 pass, 0 fail, 40 skip of 249.
- v0.21.0-beta suite, this code before any change: 219 pass, **14 fail**, 40
  skip of 273. All 14 failures are `bytes_hex` D-14 vectors the runner did
  not yet know the field for (across all six read surfaces); the 4 new
  OSD-15 canonical-escaping vectors were already passing -- this port's
  `quote_label` already escaped `\\`/`\"` correctly before this PR (see
  "Corrected" below). Measured directly against the unmodified pre-PR code
  at the v0.21.0-beta pin, not assumed.
- v0.21.0-beta suite, after: **233 pass, 0 fail, 40 skip of 273**. Track 1
  unaffected: 19 pass, 0 fail. The runner exits non-zero on any failing
  vector.

Corrected:

- **OSD-15 was already satisfied before this PR** -- an earlier draft of
  this entry wrongly claimed a fix. `omnist::osd::quote_label` already
  escaped a label's backslash as `\\` and double quote as `\"`, and nothing
  else, unchanged by this PR (confirmed: `git diff origin/main -- \
  omnist/src/osd.rs` touches `to_osd`'s signature and OSD-14's new check,
  never `quote_label`). The 4 new OSD-15 vectors pass because the writer was
  already correct, not because of anything landed here. Go and TypeScript
  independently found the same thing in their own v0.21.0-beta sweeps.
- Fixed two real conformance-harness gaps found by independent review, with
  mutation evidence, plus a design correction found by a second review:
  `run_parse_schema` (Track 2) never compared `expect.schema` against the
  parsed schema's canonical OSD text (a vector carrying `expect.schema` only
  ever checked `ok`); and Track 2's `normalize`/`prune`/`extract` used
  `referee::compare_schema`'s `"exact"` mode, which re-parses both sides and
  compares `Schema == Schema` structurally, where omnist-spec section 8.5.3
  requires byte-for-byte (a mutation to `to_osd`'s own indentation left every
  such vector green). The first fix made `"exact"` itself byte-for-byte
  everywhere, which broke Track 1: `vendor/omnist-spec/docs/
  conformance-harness.md` section 4 and its own self-test fixtures
  (`01-schema-exact-equal-different-field-order`,
  `07-schema-exact-equal-different-env-declaration-order`) define `"exact"`
  as deliberately structural for that track. Corrected: `compare_schema` now
  has three modes -- `"exact"` (structural, Track 1 only), a new
  `"canonical"` (byte-for-byte, Track 2 only, per section 8.5.3), and
  `"isomorphic"` (unchanged). Re-verified by mutation against the final
  design: a `to_osd` spacing change fails 18 Track 2 vectors while Track 1
  stays at 19/0 (as `conformance-harness.md` intends); deleting
  `quote_label`'s escaping fails the 4 OSD-15 vectors directly. No
  underlying writer bug was hiding behind the original loose comparison --
  both tracks stay at 233/0/40 (of 273) and 19/0 with the corrected
  comparisons in place.

Breaking:

- `omnist::osd::to_osd` now returns `Result<String, WriteError>` instead of
  `String` (OSD-14: a field label with a C0 control character has no OSD
  spelling and the write fails with `write.unsupported-value` rather than
  emitting text no reader accepts). Every caller in this workspace was
  updated.

Added:

- `omnist_cli::{decode_input, read_document_bytes, read_oml_bytes,
  parse_schema_bytes}`: the CLI's byte-oriented D-14 check (strict UTF-8
  decode, `parse.invalid-encoding` at `1:1`, never a lossy repair), and the
  byte-taking read entry points built on it. Every CLI command that reads a
  document, OML or OSD file (or stdin) now goes through them; `read_input`
  became `read_bytes`.

Changed:

- YAML: block collections (`- - -...` or one more indent per line) are now
  depth-guarded during the event stream itself, not only after a `Document`
  is built -- yaml-rust2's own `Parser::load` recurses with no limit for
  block collections and a ~15 KB input could overflow the stack before this
  crate's depth check ever ran.
- OML-26/OML-27 (`docs/04-oml-grammar.md` section 4.6.1): unchanged behavior,
  carried forward from the v0.19.0-beta sweep and now backed by spec text
  (the port implemented this ahead of the rule landing).
- Conformance runner (Track 2): a read-side vector (`parse`, `parse_schema`)
  may give its input as `bytes_hex` instead of `text` (E-27); the runner
  decodes the hex and hands the bytes to `omnist-cli`'s byte-oriented entry
  points -- an in-process call, not a spawned subprocess -- never by
  decoding with replacement and running the result as text.

## 0.3.0-alpha

Adopts omnist-spec **v0.19.0-beta** (was v0.9.1-beta). Breaking for library users: `ParseError` gains `code`, `DocumentError` gains `code: Option<String>`, `WriteError` gains `path`/`code`, and `ParseError::new` takes a code argument.

Conformance, Track 2 (JSON vectors), before and after:

- v0.9.1-beta pin, path-only comparison: 170 pass, 0 fail, 34 skip of 204.
- v0.19.0-beta suite, this code before any change, path-only: 197 pass, 18 fail, 34 skip of 249.
- v0.19.0-beta suite, after, `(path, code)` set comparison: 209 pass, 0 fail, 40 skip of 249. Track 1: 19 pass, 0 fail. The runner exits non-zero on any failing vector.

Changed:

- D-15/D-21: one leading U+FEFF is stripped on OML, OSD, JSON, YAML, TOML and XML by a single helper (`omnist/src/bom.rs`); a second is rejected at `1:1`; an interior one is preserved. Writers never emit one (the YAML writer now quotes a string that starts with U+FEFF).
- Data-XML profile: DOCTYPE (`format.dtd-forbidden`), non-predefined entities (`format.entity-forbidden`) and mixed content (`format.mixed-content`) are refused at `$`, after well-formedness. Fixes #179.
- XML write: a string with a C0 control character (or U+FFFE/U+FFFF) fails with `write.unsupported-value` instead of being replaced with U+FFFD.
- OML: leftover content after a complete top-level edge is `parse.trailing-content` (omnist-spec#103, settled); a missing separator inside braces or brackets stays `parse.unexpected-token`.
- E-23/E-24/OML-25: OSD string errors report the opening quote as `line:col`; OSD errors carry text-position paths; `parse.trailing-content` for a scalar followed by leftover content; `parse.separator-in-array`; `parse.invalid-date` vs `parse.invalid-time`; OSD escaped control character is `parse.control-character`.
- YAML merge keys: merged entries first in source order, collision rule, nested merge, repeated alias.
- `materialize` reports shape/cardinality problems under `validate.*` codes (spec 8.3.5); `extract` reports a record path; `infer` reports `Record.label` paths (S-21 verified for both `any` openings, nested).
- XML: a non-predefined entity reference inside an attribute value is refused too (`format.entity-forbidden`); predefined entities and numeric character references stay legal there.
- YAML: a self-referential anchor (`a: &A {b: *A}`, `a: &A {<<: *A}`) is rejected with `document.limit.alias-expansion` at `$` (D-20) instead of panicking; `!!int` with non-integer text is `parse.codec-syntax` instead of panicking.
- Conformance runner compares `(path, code)` sets and never skips for lack of structure; skips are E-20 only (6 limits: #181, 6 alias expansion: DIV-3/#180, 28 OSD-OML extension: #175).

Not done, by design: D-18 alias expansion (#180). Open diagnostics gaps: #182.

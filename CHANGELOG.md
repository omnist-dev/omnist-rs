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
- v0.21.0-beta suite, this code before any change: 219 pass, 14 fail, 40 skip
  of 273 (14 new failures: 8 `bytes_hex` D-14 vectors the runner did not yet
  know the field for, plus the 4 new OSD-15 canonical-escaping vectors, plus
  2 more D-14 vectors this runner's own `text`-only dispatch could not reach).
- v0.21.0-beta suite, after: **233 pass, 0 fail, 40 skip of 273**. Track 1
  unaffected: 19 pass, 0 fail. The runner exits non-zero on any failing
  vector.

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

- OSD-15: `to_osd` now escapes a label's backslash as `\\` and double quote
  as `\"`, and nothing else -- previously it escaped nothing, silently
  corrupting a label containing either character on write.
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

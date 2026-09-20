# Changelog

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

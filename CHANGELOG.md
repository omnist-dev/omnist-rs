# Changelog

## 0.4.0-alpha

Adopts omnist-spec **v0.25.0-beta** (was v0.22.0-beta, 312 vectors) and
enforces its YAML alias expansion limit, **D-18/D-19/D-20** (section 2.4.1;
ledger DIV-3, omnist-rs#180). This closes a denial-of-service gap: the old
reader cloned an anchor's subtree at every alias and bounded only the total
size (100,000 nodes), so a 24-line "billion laughs" or a 1.3 MB list of
`{k: *b}` entries made the reader do large amounts of work before refusing.
New public option, so a minor bump of the alpha.

Added:

- **The alias expansion limit.** For every candidate node (every anchored
  node, every mapping and sequence anchored or not, the document root and
  inline merge sources included; scalars are never checked) the reader
  computes `E = W / S` in one memoized pass over `yaml_rust2`'s event stream,
  with saturating `u64` arithmetic, and rejects input where any `E` exceeds the
  maximum with `document.limit.alias-expansion` at `$`. The check runs before
  anything is materialized: events are buffered and replayed into the tree
  builder only after every candidate passed. Self-referential anchors (D-20)
  are rejected under the same code. The reference default is 50.
- **`omnist::formats::yaml::read_yaml_with` and `YamlReadOptions`**, with
  `max_alias_expansion: u32` (`0` selects the default 50; above 10000,
  `MAX_ALIAS_EXPANSION_CEILING`, is refused with an uncoded `DocumentError`)
  and the constants `DEFAULT_MAX_ALIAS_EXPANSION` and
  `MAX_ALIAS_EXPANSION_CEILING`. `read_yaml`, the registry codec and the CLI
  read with the default; the CLI has no flag for it.
- **E-32 `line:col` placeholder** in the Track 2 runner (4 vectors).
- Note: an anchored literal merge sequence (`<<: &s [*p, *q]`) is currently counted as an ordinary merge value, which under-counts E for that spelling; omnist-spec PR #126 (D-18a, the merge carrier rule) will change this.

Changed:

- A YAML input that used to be refused as `document.limit.nodes` because of
  alias expansion is now refused as `document.limit.alias-expansion` (the
  node cap still applies below the expansion limit). Behaviour to know: a
  mapping that merges an `n`-key anchor and writes one key of its own has
  `E = (n + 2) / 3`, so merging a 150-key anchor reads `E ~ 50.67` and is
  rejected at the default; raise `max_alias_expansion` (at most 10000).
- The expansion factor bounds the ratio, not scalar-heavy size: whole-document
  `W` is bounded by `max x S(root)`, and the node cap remains a separate limit.

Conformance, Track 2 (JSON vectors), `(path, code)` set comparison:

- v0.25.0-beta suite, this code before any change: 258 pass, 4 fail, 50 skip
  of 312 (the 4 failures are the E-32 placeholder vectors; 16 alias vectors
  skipped).
- With the alias skip removed and nothing implemented: 266 pass, 12 fail, 34
  skip (the 8 alias vectors that expect a rejection fail, and the 4
  placeholders; the 8 that expect success pass with no check at all).
- After: **278 pass, 0 fail, 34 skip of 312** (skips: 28 OSD-OML, 6 limits).
  Track 1: 19 pass, 0 fail.

## 0.3.1-alpha

Adopts omnist-spec **v0.22.0-beta** (was v0.21.0-beta). 0.3.0-alpha is
published, so this behaviour change ships as a patch bump of the alpha
(`0.3.0-alpha` -> `0.3.1-alpha`); nothing in the public API changes.

Conformance, Track 2 (JSON vectors), `(path, code)` set comparison:

- v0.21.0-beta suite, previous release: 233 pass, 0 fail, 40 skip of 273.
- v0.22.0-beta suite, this code before any change: 238 pass, **9 fail**, 40
  skip of 287. The 9 failures: 7 OML-26 with-a-separator vectors
  (`separator-then-comma`, `-then-closing-bracket`, `-then-a-non-label-token`,
  `-then-nan`, `-then-opening-brace`, `-then-an-array`, `-then-colon`; this
  port reported `parse.unexpected-token`, the position was already right),
  and the two E-28 column vectors (`oml-grammar/errors/column-counts-code-
  points-after-an-astral-character`, expected `1:12`, got `1:15`;
  `osd-grammar/errors/...`, expected `2:18`, got `2:21`: byte columns).
- v0.22.0-beta suite, after: **247 pass, 0 fail, 40 skip of 287** (skips
  unchanged: 28 OSD-OML, 6 alias-expansion, 6 limits). Track 1: 19 pass, 0
  fail. Referee self-test: 10/10.

Changed:

- **OML-26 / OML-25 / OML-27.** After a complete top-level edge (or scalar
  document), any leftover token that cannot continue the edge list is
  `parse.trailing-content` at that token, with or without a separator before
  it. The list continues only when a separator is followed by a STRING or
  IDENT (the next edge, reporting its own error if malformed). Inside `{...}`
  and `[...]` a stray token stays `parse.unexpected-token`. Previously
  every one of those tokens after a separator reported
  `parse.unexpected-token`.
- **E-28 / E-29.** OML and OSD `line:col`: the column now counts Unicode code
  points from the start of the line (was bytes), lines end at LF. For example an emoji inside a string before a bad escape on the same line now
  puts the error at `1:12` (was `1:15`). This changes
  `ParseError::position()`, `ParseError::col`, the `SchemaError` path of OSD
  parse errors, and the `line N, col N` text the CLI prints for OML.
  Computation is linear (measured with the release CLI, before/after: an 8 MB
  single-line string 0.149s/0.125s, a 4 MB astral line with an error at the
  end 0.060s/0.071s, a 200k-token line 0.133s/0.132s, 200k tokens plus an
  error at the end 0.071s/0.072s, 100k edges on one line 0.043s/0.046s).
- Codec (JSON/YAML/TOML/XML) syntax positions are untouched (omnist-spec#114).

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
  already correct, not because of anything landed here.
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

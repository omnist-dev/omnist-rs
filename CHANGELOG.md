# Changelog

## 0.8.0-alpha

Two diagnostics fixes from omnist-rs#182 (items 2 and 3; item 1 shipped in
0.7.0-alpha). Spec pin unchanged (v0.28.0-beta); vectors 310 pass, 0 fail, 28
skip of 338; fixtures 19 / 19.

Minor bump of the alpha: the renamed report codes are user-visible output of
the public `WriteReport` and of the CLI's `--report` and `check`, so a caller
matching the old strings breaks. The previous release took a minor for new
public API and a changed error path; this one changes an output contract and
an error path, and adds no API.

**Breaking output change: two `WriteReport` adjustment codes are renamed to
the codes the spec's section 8.3.8 names.** They appear in
`Adjustment::code`, in the CLI's `--report` output and in `check` output:

| through 0.7.0-alpha | from 0.8.0-alpha |
|---|---|
| `value.stringified` (XML: a non-string scalar written as text) | `format.value-stringified` |
| `string.line-break-char` (YAML: U+0085 in a label or value) | `format.string-line-break-char` |

No vector pins either (a `write` vector comparing `(path, code)` would have
failed). The third non-taxonomy code, **`null.omitted`** (an XML null written
as an empty element), is **unchanged**: section 8.3.8 has no code for it, so
this port does not invent one. It is now documented as a known non-taxonomy
code (`docs/formats/xml.md`), and a spec issue is the next step; the TOML page
no longer claims TOML emits it (TOML null fails the write, since
omnist-rs#160).

Fixed:

- **YAML `document.unlabeled-element` for a non-string mapping key in a nested
  mapping carries that mapping's Document path** (it was `$` at every depth).
  `a: {on: 1}` is `$.a`, `a:\n  b:\n    1: x` is `$.a.b`, and a mapping
  inside a sequence follows E-10 (the index only when the label repeats):
  `a: [{on: 1}]` is `$.a`, `a: [{x: 1}, {on: 1}]` is `$.a[1]`. The top-level
  behaviour (`$`, pinned by the `norway-problem` vector) is unchanged. An
  over-long integer used as a key (`document.limit.int-digits`) carries the
  same path. The path is threaded through the untyped tree as a lazy chain,
  rendered only when an error needs it, so a document that never errs pays no
  allocation for it.

## 0.7.0-alpha

Makes the three safety limits of the spec's section 2.4 runtime-configurable
(omnist-rs#181) and reports every `document.limit.*` violation with a Document
path (omnist-rs#182, item 1; spec E-11). Spec pin unchanged (v0.28.0-beta);
vectors 310 pass, 0 fail, 28 skip of 338 (was 304 / 0 / 34: the six
`document-model/limits` vectors now run and pass); fixtures 19 / 19.

Minor bump of the alpha: a new public configuration surface (`omnist::limits`,
the `*_with` entry points) and a behaviour change in the error type and path
of limit violations, both of which a caller can observe. The previous release
took a patch for a change that added no API and rejected nothing new.

Added:

- **`omnist::limits::Limits`** (`max_depth`, `max_nodes`, `max_int_digits`,
  defaulting to 200, 1,000,000 and 4,300, the spec's reference defaults), in
  the style of `YamlReadOptions`: `#[non_exhaustive]`, `with_*` builders,
  `effective_*`, `validate`. `0` selects the default (a zero or unset value
  never widens a limit); a value above its ceiling (250, 10,000,000 and
  43,000, this port's own, since the spec recommends none for these three) is
  refused, not clamped. The ceilings and defaults are public constants. D-10
  (finite) and D-11 (documented, in `docs/limitations.md`) are met.
- **Explicit-limits entry points**: `read_oml_with`, `read_json_with`,
  `read_toml_with`, `read_xml_with`, `YamlReadOptions::with_limits` (with the
  existing `read_yaml_with`), `Doc::of_with` and `Doc::from_raw_with`. The
  plain `read_*` signatures and defaults are unchanged. A `Doc` remembers the
  limits it was built under, so a later `add` / `set` enforces the same ones.
- **The conformance runner sets the limits from `declared_max_depth`,
  `declared_max_nodes` and `declared_max_int_digits`** and runs the six
  `document-model/limits` vectors instead of skipping them (E-20): each
  at-limit vector is accepted by the reader and by `Doc::from_raw_with`, each
  one-past vector is refused with the right code and path.

Changed:

- **Limit violations are `DocumentError`s with a Document path (E-11), not
  `ParseError`s with a `line:col`** (#182, item 1), in the OML, JSON, TOML and
  YAML readers (XML and `Doc::from_raw` already were). `document.limit.depth`
  and `document.limit.nodes` carry path `$`; `document.limit.int-digits`
  carries the path of the integer (`$.n`, and `$.n[1]` only when the label
  repeats, E-10), as the vectors pin. The code is unchanged. `read_oml` keeps
  its signature and still returns a `ParseError` carrying the position; the
  Document form is `read_oml_with` and the registry's `oml` format. The message
  of the digit-cap error no longer states the literal's own digit count (an
  over-cap literal is replaced by a placeholder and never converted).
  A syntax error anywhere in the input now wins over an over-long integer
  (previously whichever came first in the text).
- **The CLI reads OML with `read_oml_with`**, so a limit violation in OML input
  prints a Document path (`$: nesting exceeds the maximum depth (200)`), like the
  other formats; syntax errors stay `line:col`. The depth ceiling is 250: a 2 MB
  thread stack was measured to overflow at 400 levels in a debug build.
- **A node is a container in `Doc` and in the OML reader**, as D-9 counts them
  (and as the YAML reader has since 0.6.1-alpha): `Doc` counted every leaf,
  and the OML parser counted every scalar value, so `a: 1` / `b: 2` was
  three nodes. The XML reader counted every element; it now counts the document
  root and every element that has a child element, matching `Doc`. A document
  with many scalars (or leaf elements) is accepted where it was refused.
- **`Doc::of` and `Doc::from_raw` refuse an integer over the digit cap**
  (spec: the limits bound every route into the model); programmatic
  construction was unbounded before. A single-element array's path no longer
  carries `[0]` (E-10) in `Doc::of`'s diagnostics.

## 0.6.1-alpha

Fixes the YAML materialization node cap (omnist-rs#189, DIV-11 in the spec's
divergence ledger). Spec pin unchanged (v0.28.0-beta); vectors unchanged
(304 pass, 0 fail, 34 skip of 338; fixtures 19 / 19).

Patch bump of the alpha: the change makes the reader accept more (a limit
raised and a counting rule corrected), adds no public API and no error code,
and rejects nothing that was accepted before. The last adoption took a minor
for new codes and new rejections; this is neither.

Changed:

- **The YAML node cap counts containers only and defaults to 1,000,000.** It
  was 100,000 and counted every key and every scalar value, whereas the spec's
  D-9 counts nodes: mappings and sequences (an edge list; neither a key nor a
  scalar value is a node), with a reference default of 1,000,000. So a document
  the spec's own D-22 rationale says must not be refused was refused with
  `document.limit.nodes`: the 1,000-service compose example, each service
  merging a 60-key defaults block (`W` = 62,063), and a plain file of 160,000
  scalar entries (one node). Both are now accepted. An alias is charged for the
  containers it clones. The error code, path and message are unchanged, and the
  cap is still not configurable. Depth (200) and the D-18 / D-22 limits are
  untouched; a 1,000,000-container document is accepted and the next container
  is refused.

## 0.6.0-alpha

Adopts omnist-spec **v0.28.0-beta** (was v0.27.0-beta; still 338 vectors, none
added: DIV-5). Minor bump of the alpha because validation is stricter and a
writer now fails where it used to emit text: observable behavior changes on
the public API, as at 0.5.0-alpha. Track 2 is **304 pass, 0 fail, 34 skip of
338**; Track 1 19 / 19. The only pin for the rules below is
`omnist/tests/spec_v028.rs`.

Why a minor bump: previously accepted inputs are now rejected (`Schema::new`
on a bad name), a writer refuses a schema it used to write (`to_osd` on
`max = 0`), and `infer` derives record names differently. The spec's new codes
`schema.invalid-label` and `schema.unknown-record` have no Rust surface (below);
`schema.invalid-name` at `$` is new in practice.

Changed:

- **`infer` record names are ASCII (S-8).** A record name is derived from a
  field key: every character outside `[A-Za-z0-9_]` becomes `_`, leading digits
  and underscores are stripped, the first letter is capitalised, `Rec` is used
  if nothing is left, and collisions get `2`, `3` suffixes. So `123`, `9`, `日本`
  -> `Rec`; `éclair` -> `Clair`; `a b` -> `A_b`; ordinary ASCII names are
  unchanged. Without this, `omnist infer` failed on such keys now that
  `Schema::new` enforces S-8 (before, a non-identifier name was kept as is).
  Identical to omnist-py's `_identifier` and omnist-ts.
- **S-8 at construction.** `Schema::new` rejects a record name or `Ref` target
  name (the root's and every field's) that is not `[A-Za-z_][A-Za-z0-9_]*` with
  `schema.invalid-name` at `$`, the name in the message only. Previously such a
  schema was accepted (a bad `Ref` target failed later as
  `schema.unknown-type`, a bad record name not at all). OSD text cannot reach
  this; the OSD parser is unchanged.
- **OSD-16 / S-24.** `osd::to_osd` fails with `write.unsupported-value` at the
  record path `R` for a field with `max = 0` (`[0,0]` stays representable).
  `prune` already removed `max = 0` fields from every record it rebuilds and
  keeps an unsatisfiable root intact, so prune before writing; `normalize`
  never introduces `max = 0`. Both are tested.

Not applicable, documented in `docs/limitations.md`:

- **S-22 `schema.invalid-label`** is vacuous: labels are `String`, and no
  public surface takes bytes.
- **S-23 `schema.unknown-record`** has no surface: no function takes a
  caller-supplied record ordering.
- No OSD-OML schema writer exists, so its half of OSD-16/S-24 does not apply.

## 0.5.1-alpha

Adopts omnist-spec **v0.27.0-beta** (was v0.26.0-beta, 338 vectors, +7). D-18a
now says an empty merge sequence `<<: []` is a well-formed carrier that merges
nothing (W 0, S one slot for the `<<` entry); the same for `s: &s []` then
`<<: *s`. An empty sequence outside merge position is an ordinary node. No
library change: the reader already behaved this way. Track 2 is **304 pass, 0
fail, 34 skip of 338**; Track 1 19 / 19. New tests in `alias_tests.rs` pin the
contract, including the size-cap boundary (W(root) 2, S(root) 3).

## 0.5.0-alpha

Adopts omnist-spec **v0.26.0-beta** (was v0.25.0-beta, 331 vectors): the
merge-carrier rule **D-18a**, the expanded size limit **D-22**, and merge-shape
syntax errors, all in section 2.4.1. New public option and constants, so a
minor bump of the alpha. 0.4.0-alpha is untagged and unpublished.

Added:

- **The expanded size limit (D-22).** An input that contains at least one
  alias or merge key and whose root materializes more than the maximum
  (`W(root)`, default 1,000,000 value slots) is rejected with
  `document.limit.expanded-size` at `$`; equal to the maximum is accepted. It
  is checked at the root after every candidate's ratio check (an input that
  fails both reports `document.limit.alias-expansion`), from the same single
  pass with saturating counts, before anything is materialized. An input with
  no alias and no merge key is exempt however large: a plain file of two
  million slots passes, and one added alias subjects it to the cap.
- **`YamlReadOptions::max_expanded_slots`** (`with_max_expanded_slots`,
  `effective_max_expanded_slots`), `DEFAULT_MAX_EXPANDED_SLOTS` (1,000,000) and
  `MAX_EXPANDED_SLOTS_CEILING` (10,000,000). Same convention as
  `max_alias_expansion`: `0` selects the default, a larger value is refused by
  `read_yaml_with`/`validate` with an uncoded `DocumentError`. The CLI has no
  flag for it.
- **Malformed merge shapes are `parse.codec-syntax`** (D-18a): a scalar merge
  value, a scalar member of a merge sequence, a sequence inside a merge
  sequence (written, or `<<: [*s]` with `s` a sequence), an alias to a scalar,
  and `<<: *s` with `s` holding anything but mappings. The position is the
  offending node's `line:col`. They are found in the checker's pass, win over
  every `document.limit.*` code (the checker keeps reading after a limit
  violation to find them) and need no counting. `<<: []` is accepted.

Changed:

- **Merge carriers (D-18a).** A sequence in merge-value position is a carrier
  whether or not it is anchored: `<<: &s [*p, *q]` holds no slot in `W` or `S`
  and is not a candidate (it used to be counted as an ordinary flattened
  value, under-counting `E`). `<<: *s` contributes the sum over the members of
  `W - 1` and one slot in `S`; a plain alias to a carrier materializes the
  list. This removes the "anchored literal merge sequence" known edge. An
  anchored carrier at the declared limit, and an alias to a merge sequence at
  it, used to be rejected wrongly.
- The tree builder's own merge checks are now unreachable (the checker refuses
  every malformed shape first); a sequence of sequences used to be flattened
  silently and is now a syntax error.
- Runner: `declared_max_expanded_slots` is passed through
  `YamlReadOptions::max_expanded_slots` for the vectors that carry it.

Behaviour to know: the reader's own 100,000-node materialization cap
(`document.limit.nodes`, keys included) is far below the default maximum
expanded size, so an input the size limit accepts can still be refused for its
node count. `W(root) <= max x S(root)` and the `(keys + 2) / 3` merge ratio
are unchanged.

Conformance, Track 2 (JSON vectors), `(path, code)` set comparison:

- v0.26.0-beta suite, this code before any change (runner passing
  `declared_max_expanded_slots`): 289 pass, 8 fail, 34 skip of 331 (the
  anchored carrier and the alias to a merge sequence wrongly rejected; the 4
  expanded-size vectors parsed OK; the malformed merge after a bomb reported
  the limit; a merge sequence of sequences parsed OK).
- After: **297 pass, 0 fail, 34 skip of 331** (skips: 28 OSD-OML, 6 limits).

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

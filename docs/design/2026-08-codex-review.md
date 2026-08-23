# omnist-rs deep-dive audit

Date: 2026-08-20

Scope: correctness, maintainability, security, and performance review of the Rust port, including all six text surfaces (OML, OSD, JSON, YAML, TOML, XML), validation, materialization, writers, dependency risk, and the pinned conformance suite.

This is a review and improvement plan only. No implementation changes are part of this document.

## Executive summary

The port has a strong baseline: the workspace builds cleanly, 960 tests pass, strict Clippy passes, the Lines coverage gate is genuinely 100.00%, RustSec reports no vulnerable dependency, the main conformance runner is 19/19, and the vector runner is 130 passed / 0 failed / 22 skipped. Validation and materialization avoid the per-edge/per-field quadratic scan found in sibling ports, and the OML scanner avoids their per-token suffix-copy lexer bug.

Seven confirmed defects remain:

1. Integer-to-number materialization reports success after losing integer value.
2. The recursive JSON parser can overflow the process stack before the Document depth guard runs.
3. OML parser and writer depth accounting disagree; accepted input makes the CLI panic.
4. The public OML raw-tree path has no finite node-count limit.
5. The XML reader silently accepts and truncates malformed multi-root/trailing-content input.
6. The OSD writer does not escape quote or backslash characters in field labels and can emit an unreadable schema.
7. YAML merge-key de-duplication is quadratic and is reachable below the existing materialized-node cap.

There is also a confirmed structured-diagnostics conformance gap: 16 schema/OSD failure vectors skip because SchemaError has no code or path. Six other skips concern temporary runtime limit overrides; those are a harness/configuration capability gap, not evidence that the compiled-in limits are wrong.

Priority terminology in this report:

- P1: correctness loss, process abort/panic, or practical denial-of-service exposure; address before treating untrusted input as production-safe.
- P2: real interoperability/conformance defect without the same immediate crash or data-corruption impact.
- P3: maintainability or measured-performance work that should follow the defects.

## Audit snapshot and ground truth

- Repository branch: `main`
- Repository commit: `1f806f7c9327081859b2cc72ebff57f247f9d9c7`
- Crate version: `0.1.3-alpha`, read directly from `omnist/Cargo.toml:1-4`
- Pinned spec commit: `f93c56991a16b86470fb9dccb6a3faa1f1c2219c`
- The pin identifies itself as `v0.2.2-alpha-2-gf93c569`.

The pinned submodule is behind two decisions relevant to this audit. I checked the merged spec history/issues rather than reopening them as ambiguities:

- omnist-spec#44, merged as `47cbedd`, permits XML schema-aware pretyping. Rust's missing schema parameter is already tracked as omnist-rs#114. It is not re-reported here.
- omnist-spec#45, merged as `0452cc8`, makes duplicate JSON/YAML mapping keys last-key-wins. The Rust readers already implement that behavior.

The pinned history was still reviewed as the repository's checked-in normative baseline. In particular, `vendor/omnist-spec/docs/02-document-model.md:154-180` requires finite depth, node, and integer-digit limits; lines 191-193 and 221-229 specifically require OML parser/builder depth agreement. Materialization exactness is normative at `vendor/omnist-spec/docs/07-codecs-and-deserialization.md:30-47` and `:155-160`.

## Confirmed defects and vulnerabilities

### P1 — integer-to-number materialization silently loses value

Evidence:

- `omnist/src/materialize.rs:200-230` implements the upgrade table.
- At `:221-228`, every `BigInt` is converted with `to_f64()` and returned as a successful `Float`, without checking finiteness or a round-trip back to the original integer.
- The surrounding code at `:186-197` treats `Some` as a successful exact upgrade and emits no diagnostic.
- The spec requires value-exact conversion at `vendor/omnist-spec/docs/07-codecs-and-deserialization.md:30-47` and says materialization never loses a value at `:155-160`.

Concrete reproduction:

```sh
printf %s "x: 9007199254740993" |
  cargo run -q -p omnist-cli -- convert --from oml --to json \
    --schema <(printf "%s\n" "record R {" '    "x": number,' "}" "root R") -
```

Observed result (exit 0):

```json
{"x": 9007199254740992.0}
```

The input integer and output number are different mathematical values. Larger magnitudes can also become infinities and still pass materialization; a later writer may then substitute or reject them, but the materializer has already violated its success guarantee.

Improvement: convert to `f64`, require a finite result, convert that exact float back to `BigInt`, and return `None` unless it equals the source. Add boundary tests at `2^53`, `2^53+1`, negative equivalents, and beyond finite f64 range.

Cost/benefit: local fix and tests; negligible steady-state cost relative to arbitrary-precision conversion. It prevents silent data corruption. No benchmark is needed before committing to correctness, though a microbenchmark can confirm the extra reverse conversion is irrelevant for ordinary integers.

### P1 — deeply nested JSON aborts the process before the declared limit

Evidence:

- `omnist/src/formats/json.rs:68-81` parses a complete intermediate `Value` and only then calls `Doc::of`.
- `parse_value`, `parse_object`, and `parse_array` recurse at `omnist/src/formats/json.rs:348-444` with no parser depth counter.
- The only shared depth/node enforcement is later in Document construction (`omnist/src/document.rs:268-285` and `:312-328`).
- The module comment at `omnist/src/formats/json.rs:4-14` explicitly relies on this later check, but that cannot protect the parser's call stack.

Concrete reproduction:

```sh
python3 -c 'n=10000; print("{" + "\"a\":{" * n + "\"z\":1" + "}" * (n + 1))' |
  target/debug/omnist convert --from json --to oml --compact -
```

Observed result:

```text
thread 'main' (...) has overflowed its stack
fatal runtime error: stack overflow, aborting
Aborted
```

The input is only a small fraction of the one-million-node limit; depth should be rejected as `document.limit.depth`, not terminate an embedding process.

Improvement: enforce the shared depth constant before recursive descent and account nodes while parsing, before allocating the full tree. Because the public error contract distinguishes parse errors from `document.limit.*`, preserve a structured limit error rather than translating it to a generic syntax error.

Cost/benefit: moderate internal parser refactor (the parser currently returns only `ParseError`). The gain is removal of a reliable untrusted-input process-abort path and earlier refusal of oversized shallow inputs. Correctness takes priority over benchmarking; benchmark representative JSON afterward to ensure counters do not regress throughput materially.

### P1 — OML accepts a tree its writer rejects, and `omnist format` panics

Evidence:

- `omnist/src/oml/parser.rs:99-137` passes the current container depth to `parse_value`.
- `parse_value` at `:167-173` applies no depth check to scalar children.
- `parse_brace_value` checks only `depth + 1` at `:175-201`.
- Consequently, a brace node at depth 200 can contain a scalar at depth 201.
- The writer independently checks every child depth.
- `omnist-cli/src/lib.rs:689-703` asserts that the two checks agree and calls `.expect(...)`, turning the disagreement into a panic.
- The spec expressly forbids parser/builder disagreement at `vendor/omnist-spec/docs/02-document-model.md:191-193` and `:221-229`.

Concrete reproduction:

```sh
python3 -c 'print("a: { " * 200 + "z: 1" + " }" * 200)' |
  cargo run -q -p omnist-cli -- format --compact -
```

Observed result:

```text
thread 'main' (...) panicked at omnist-cli/src/lib.rs:703:6:
read_oml already enforced the depth guard write_oml re-checks:
WriteError { message: "nesting exceeds the maximum depth (200)", report: None }
```

The same input through `convert --from oml` is accepted by `read_oml` and then rejected by `Doc::from_raw` as exceeding depth.

Improvement: define depth in one shared helper and charge every parsed node, including scalar and array-expanded children. Independently remove the CLI `expect` and surface writer failure normally; defensive error propagation remains valuable even after the invariant is fixed.

Cost/benefit: local-to-moderate parser/CLI change with boundary tests. It removes a deterministic panic and restores a normative invariant. No benchmark prerequisite.

### P1 — public OML parsing/formatting has no finite node-count limit

Evidence:

- `omnist/src/oml.rs:74-83` publicly returns an unrestricted `RawNode`.
- `omnist/src/oml/parser.rs:99-137` appends every parsed edge to an unbudgeted `Vec`.
- The format command intentionally bypasses `Doc::from_raw` at `omnist-cli/src/lib.rs:689-697`.
- The one-million-node guard exists only at the Document arena choke point, `omnist/src/document.rs:312-328`; the registry adapter reaches it later at `omnist/src/oml.rs:130-135`.
- The spec says no implementation may be unbounded on node count at `vendor/omnist-spec/docs/02-document-model.md:154-180`.

Concrete failure scenario: a shallow OML document containing more than one million `a: 0` edges is accepted into a larger-than-limit raw tree by `read_oml`; `omnist format` then traverses and serializes it without ever entering the guarded Document arena. Library callers can do the same through the public API. Registry-based conversion eventually rejects, but only after the oversized raw tree has already been allocated.

Improvement: make the OML parser own a shared node budget and reject before pushing the node that crosses it. Count array sugar by expanded child nodes, matching the Document model. Keep `Doc::from_raw`'s guard as defense in depth.

Cost/benefit: moderate parser change because counting semantics must match builder semantics. The gain is a real finite bound for every public OML path. Add at-limit/one-past tests; benchmark wide OML before and after, but do not use benchmark results to justify retaining an unbounded path.

### P1 — XML reader silently truncates malformed documents

Evidence:

- `omnist/src/formats/xml.rs:140-175` skips every non-start event before the first element, including non-whitespace text.
- On the first `Start` or `Empty`, it immediately returns at `:151-163`; it never consumes the remainder of the XML stream.
- The pinned XML mapping says an XML document has exactly one top-level element at `vendor/omnist-spec/docs/formats/xml.md:19-22`.

Concrete reproduction:

```sh
printf %s 'garbage<a/><b/>trailing' |
  cargo run -q -p omnist-cli -- convert --from xml --to oml --compact -
```

Observed result (exit 0):

```oml
a: ""
```

Leading garbage, a second root, and trailing garbage are all silently discarded. This is data truncation, not merely liberal XML acceptance.

Improvement: accept only XML-legal prolog/epilog events, reject non-whitespace top-level text, continue after the root until EOF, and reject a second root or any other trailing content that makes the document ill-formed.

Cost/benefit: local reader state-machine change and tests. It prevents silent data loss and improves parser trustworthiness. No benchmark prerequisite.

### P2 — OSD writer fails to escape arbitrary field labels

Evidence:

- OSD string decoding is `\X -> X`; the normative quote/backslash spellings are described at `vendor/omnist-spec/docs/05-osd-grammar.md:54-63`.
- The parser implements that rule at `omnist/src/osd.rs:134-150`.
- The writer inserts `f.label` verbatim between quotes at `omnist/src/osd.rs:437-444`.
- Field labels originate in arbitrary Document keys (for example through inference), so quote/backslash values are reachable through public API, not only hand-built invalid state.

Concrete reproduction:

```sh
printf %s '{"a\"b\\c":1}' |
  cargo run -q -p omnist-cli -- infer --from json --compact -
```

Observed output:

```osd
record Root { "a"b\c": integer } root Root
```

Piping that output to `omnist schema format --compact -` fails with:

```text
error: unexpected character '\\' at 18
```

Improvement: escape `\` as `\\` and `"` as `\"` before quoting. Do not reuse OML/JSON's named escape table because OSD deliberately has different semantics.

Cost/benefit: small local fix plus round-trip tests over quotes, backslashes, and combinations. It restores the writer's basic round-trip contract with negligible cost.

### P1 — YAML merge-key de-duplication is quadratic

Evidence:

- `omnist/src/formats/yaml.rs:391-433` resolves merge mappings.
- It builds `own_labels: Vec<&str>` and `merged_seen: Vec<&str>` at `:419-422`.
- Every merged key performs linear `.contains` scans at `:423-430`.
- With N distinct merged labels and no explicit keys, `merged_seen` grows from 0 to N and performs Θ(N²) string comparisons.
- The alias/materialization cap at `omnist/src/formats/yaml.rs:111-137` limits expansion to 100,000 nodes but still permits tens of thousands of merged keys, enough for a practical CPU-amplification input.

Concrete benchmark probe, debug binary, one warmed WSL process:

```text
merged keys   elapsed   user CPU
4,000         0.531 s   0.371 s
8,000         1.405 s   1.315 s
16,000        3.429 s   3.333 s
24,000        6.874 s   6.704 s
```

The input is an anchored mapping with N distinct keys and one `<<: *base` target. The 24,000-key case remains under the 100,000 materialized-node cap.

Improvement: retain the output `Vec` for order, but use hash sets for membership in explicit and already-merged labels. Rust's standard `HashSet<&str>` is sufficient because set iteration order is never observed.

Cost/benefit: small local change; memory adds O(N) hash-table overhead while CPU drops to expected O(N). The measured scaling already justifies work, but add a reproducible Criterion or ignored scaling benchmark (for example 2k/4k/8k) before merging so the improvement and memory trade-off remain visible.

### P2 — schema parse failures lack normative structured diagnostics

Evidence:

- The spec requires every diagnostic to carry code, path, message, and severity at `vendor/omnist-spec/docs/08-conformance-and-errors.md:48-67`.
- `SchemaError` is only a string newtype at `omnist/src/error.rs:38-49`.
- The conformance runner documents the gap at `tools/conformance/src/bin/vector_runner.rs:65-84` and skips diagnostic-bearing schema failures at `:395-406`.
- The current run skipped 16 OSD grammar/schema-wellformedness vectors for exactly `SchemaError carries no structured path/code`.

This is not a claim that those inputs are accepted: the parser does reject them. The defect is that callers and conformance tooling cannot distinguish `schema.no-root`, `schema.duplicate-field`, `schema.invalid-cardinality`, and the other stable taxonomy entries without parsing unstable message text.

Improvement: introduce a structured schema diagnostic/error representation and populate it at construction and OSD parse sites. Preserve a human-readable Display implementation and plan the public API migration because `SchemaError(pub String)` is exposed.

Cost/benefit: cross-cutting public-API refactor, not a local patch. It enables full error-code conformance, stable machine handling, and removes 16 skips. Design the migration first; benchmark impact is immaterial, but compatibility impact needs explicit review.

## Security assessment beyond the findings

### Positive controls

- No production `unsafe` blocks were found. The only source match is explanatory prose in `document.rs`.
- No generic object deserialization, type-name dispatch, or reflection is used on production input. Serde JSON parsing appears only in tests/conformance tooling.
- XML external entities are not resolved. `quick_xml` produces explicit general-reference events and the code resolves only recognized built-ins/numeric references in `omnist/src/formats/xml.rs:296-335`; unknown references error.
- YAML uses an event receiver rather than an unsafe object constructor. Alias expansion is charged before cloning, capped at 100,000 materialized nodes at `omnist/src/formats/yaml.rs:111-137` and `:201-335`. A 10,000-level flow-sequence probe was rejected by `yaml-rust2` at its own recursion limit before the recursive alias subtree counter ran.
- Integer digit caps are checked before arbitrary-precision conversion in the hand-written JSON/OML/YAML paths.
- `cargo audit` found no vulnerable dependency (full raw output below).

### Remaining hardening/design debt

1. JSON, TOML, and XML build intermediate trees before the guarded Document arena. JSON is a confirmed vulnerability above because recursion aborts the process. XML has its own depth check but no raw-tree node budget (`omnist/src/formats/xml.rs:183-260`); TOML parses a complete `toml_edit::DocumentMut` and converts it before `Doc::of` (`omnist/src/formats/toml.rs:228-233`). A one-past-limit shallow input is ultimately rejected, but only after a large intermediate allocation.

   Cost/benefit: JSON should be fixed regardless. XML can add a local node counter during event processing. TOML is harder because the third-party parser owns the first tree; a streaming replacement would be a substantial refactor. Benchmark peak RSS and throughput on wide 100k/500k/1m-node documents before choosing a TOML replacement; the gain would be predictable early refusal, while the cost is parser complexity and possible TOML compatibility regressions.

2. Limit constants are compile-time only. Six conformance vectors skip because the harness cannot temporarily supply their tiny declared limits (`tools/conformance/src/bin/vector_runner.rs:49-63` and `:283-288`). The spec explicitly does not require a public configuration surface, so this is not a conformance defect by itself.

   Cost/benefit: adding a public limits object would touch every reader/builder API and increase configuration burden. The gain is embedders choosing smaller limits and full execution of six scaling vectors. Treat this as an API-design decision, not an automatic fix; prototype an internal/test-only limit context first and benchmark whether passing it through hot paths has measurable cost.

## Performance assessment

### Confirmed favorable paths

- The sibling-port lexer suffix-copy defect is absent. The OML scanner stores byte offsets into borrowed source and decodes lazily at `omnist/src/oml/scanner.rs:65-93`; token payloads copy only their own spans, for example `:455-466`.
- OSD tokenization passes a borrowed suffix to the regex engine at `omnist/src/osd.rs:78-131`; it does not allocate a fresh remaining-input string per token. The unanchored regex is undesirable clarity-wise, but a valid token matches at offset zero and an invalid offset errors on that iteration, so the sibling Θ(n²) suffix-copy pattern is not present.
- Record lookup is indexed by label at `omnist/src/schema.rs:495-542`.
- Validation counts edges once and performs indexed field lookup at `omnist/src/schema.rs:915-950`.
- Materialization uses the same one-pass counts/indexed lookup shape at `omnist/src/materialize.rs:121-159`.
- Therefore the sibling per-edge/per-field Θ(E×F) scan is absent; normal record validation/materialization is O(E+F), excluding recursive descent into child records.

### Benchmark-first optimization opportunities

1. JSON and YAML writing construct the complete grouped shadow tree twice. `write_json` calls `check_json` and then `to_grouped` again at `omnist/src/formats/json.rs:93-116`; YAML repeats the same shape at `omnist/src/formats/yaml.rs:900-920`. Each `to_grouped` clones labels/scalars and allocates maps/arrays recursively at `omnist/src/document.rs:449-483`.

   Cost/benefit: a local first step can build one grouped tree and let both scan and emit consume it, roughly halving shadow-tree allocations in those writers. A larger cursor-streaming writer refactor could avoid the shadow tree entirely but is more complex because repeated labels must be grouped. Benchmark wall time and peak RSS on wide unique-key, repeated-key, and deep documents before choosing between reuse and streaming. The likely gain is material for library embedding; the risk is duplicating grouping logic or changing stable output ordering.

2. XML writing clones the whole Document to `RawNode` before scanning/emitting at `omnist/src/formats/xml.rs:345-365`. TOML necessarily builds and then transforms a grouped tree at `omnist/src/formats/toml.rs:403-418`.

   Cost/benefit: cursor-based traversal could reduce peak memory but is a multi-codec refactor, and TOML's table ordering rules make direct streaming less simple. Benchmark first. Prefer codec-local reuse improvements before introducing a shared visitor abstraction unless profiles show the allocation cost is important.

3. `Record::partial_eq` sorts both field lists on every equality at `omnist/src/schema.rs:501-513`. That is O(F log F) and can recur in algebra operations.

   Cost/benefit: comparing through the existing `by_label` index could be O(F), but equality is not proven hot. Benchmark large-schema normalize/equivalent/minimize operations first. The gain may be negligible compared with graph refinement, while changing equality deserves care around declaration-order independence.

## Quality and maintainability

### Stale or contradictory documentation

These are not runtime defects, but they directly misdescribe public behavior and security controls:

- `docs/limitations.md:3` says `0.1.0-alpha`; the crate is `0.1.3-alpha`.
- `docs/limitations.md:17-41` says `any` is deliberately unimplemented. `FieldType::Any`, OSD `any`, inference `allow_any`, validation, and algebra support now exist.
- `omnist/src/document.rs:23-31` still says integers are `i64` and no integer-digit guard is needed, contradicting `:58-66` and the current `BigInt` implementation.
- `omnist/src/formats/json.rs:16-44` says there are no temporal variants and integers are `i64`; both were superseded by issues #104/#105.
- `omnist/src/formats/toml.rs:93-100` reasons from an `i64`-backed `Scalar`, which is no longer true (the `toml_edit` read-side ceiling remains, but for a different reason).
- `omnist/src/oml.rs:31-33` says the scanner uses `Vec<char>`; the scanner is now byte-indexed and borrowed.
- `omnist/src/oml/writer.rs:8-22` describes the removed `RawNode::TemporalLeaf`; the writer now matches real temporal `Scalar` variants.
- `omnist/src/schema.rs:714-715` duplicates the same public doc line.
- `omnist-cli/src/lib.rs:689-697` contains a detailed proof that parser/writer depth disagreement is impossible; the audit reproduction disproves it.

Improvement: perform one focused documentation sweep tied to the BigInt, temporal, `any`, scanner, and depth-limit migrations. Add a workflow checklist item requiring comments/limitations/API docs to be searched for removed type names and old representation claims when a core model type changes.

Cost/benefit: local documentation-only work, except the CLI comment should change with its defect fix. It prevents future security review and user decisions from relying on false architecture claims. Automated checks can catch removed identifiers such as `TemporalLeaf`; semantic claims still need review.

### API consistency observations

- The format registry presents all document codecs as `Doc` readers/writers, but public OML uniquely exposes `RawNode`. That is justified by lossless repeated/interleaved edges, yet it also bypasses Document invariants. Keep the raw API only if it enforces the same depth/node limits, or name/document it explicitly as a bounded parsed representation.
- Error types are inconsistent in structure: validation has codes/paths, parse errors have line/column, while schema errors are message-only. The structured-diagnostics plan above should define one stable diagnostic shape without erasing useful format-specific positions.
- The source is generally well-factored: shared string escaping, float formatting, write reports, format registry, indexed schema records, and arena construction reduce duplication and centralize invariants.

## Dependency vulnerability audit — exact command and full raw output

Command:

```sh
cd ~/dev/omnist-rs && cargo audit
```

Full raw output:

```text
    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 1217 security advisories (from /home/claude/.cargo/advisory-db)
    Updating crates.io index
    Scanning Cargo.lock for vulnerabilities (87 crate dependencies)
```

Exit status: 0. No RustSec vulnerability, warning, or informational advisory was reported.

Production dependencies are a small set (`omnist/Cargo.toml:10-18`): indexmap, thiserror, regex, yaml-rust2 (default features disabled), toml_edit (parse/display only), quick-xml, num-bigint, and num-traits.

## Verification results

All commands were run inside Debian WSL from `~/dev/omnist-rs`.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo build --workspace` | pass |
| `cargo test --workspace` | pass; 960 tests, 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo llvm-cov --workspace --fail-under-lines 100` | pass |
| `cargo run -q -p conformance --bin self-test` | 10/10 passed |
| `cargo run -q -p conformance --bin runner` | 19 passed, 0 failed, 0 skipped |
| `cargo run -q -p conformance --bin vector_runner` | 130 passed, 0 failed, 22 skipped |
| `cargo audit` | pass; no advisories |

Coverage summary (read from the correct columns):

```text
TOTAL  Regions 21866, Missed Regions 119, Cover 99.46%
       Functions 1243, Missed Functions 0, Executed 100.00%
       Lines 11395, Missed Lines 0, Cover 100.00%
```

The gate is Lines, and Lines are 100.00%. Regions are 99.46%; that is not a failure and must not be confused with line coverage.

Vector-runner skip breakdown:

- 6 document-limit vectors: compile-time constants cannot be overridden to each vector's tiny declared test limit. This is not a claim that the real constants fail.
- 16 OSD/schema failure vectors: `SchemaError` lacks structured code/path fields, so expected diagnostics cannot be compared. This is the confirmed diagnostics gap above.

## Prioritized improvement plan

No step below should begin implementation without explicit approval.

### Phase 1 — stop corruption and process termination

1. Add failing tests and fix value-exact BigInt-to-f64 materialization.
2. Add parser-time JSON depth/node accounting and verify deep input returns a structured limit error rather than aborting.
3. Unify OML node/depth accounting across parser, raw API, Document builder, and writers; replace the CLI `expect` with normal error propagation.
4. Add XML full-stream validation so leading/trailing content and second roots are rejected.

Acceptance: all reproductions in this report fail safely; existing gates and conformance remain green; new limit failures use the required `document.limit.*` categories.

### Phase 2 — restore serialization and untrusted YAML guarantees

5. Escape OSD field labels according to OSD's deliberately minimal `\X` semantics and add generated-schema round-trip tests.
6. Replace YAML merge membership vectors with sets; add a stable scaling benchmark and adversarial regression test below the materialized-node cap.

Acceptance: inferred schemas with arbitrary quote/backslash labels parse back identically; YAML merge scaling is approximately linear and preserves first-source/explicit-key precedence and output order.

### Phase 3 — close structured conformance gaps

7. Design the public structured `SchemaError` migration (code, path, message, severity), then populate all OSD and schema construction sites.
8. Enable the 16 currently skipped schema diagnostic vectors.
9. Decide separately whether an internal or public configurable-limits context is worth the API cost; do not couple that optional decision to the structured-error work.

Acceptance: schema failure vectors compare stable codes/paths without message parsing; any public API break is documented for the alpha release.

### Phase 4 — benchmark-led embedding improvements

10. Benchmark JSON/YAML writer allocation and peak RSS; first reuse one grouped projection for scan+emit, then consider cursor streaming only if the profile justifies it.
11. Benchmark XML/TOML intermediate-tree peak memory on wide documents before choosing local counters versus parser refactors.
12. Benchmark schema equality/algebra on large field sets before changing `Record::partial_eq`.

Acceptance: every optimization has before/after time and allocation/RSS evidence, preserves canonical output and reports, and does not trade away limits or error fidelity.

### Phase 5 — documentation truth sweep

13. Update stale version, `any`, BigInt, temporal, scanner, TOML, and removed-`TemporalLeaf` documentation.
14. Add migration-oriented stale-identifier/comment review to the workflow playbook.

Acceptance: public docs and module comments describe the current types and controls, and no old representation claim survives a targeted search.

## Explicitly not re-reported

- XML schema-aware pretyping is already omnist-rs#114, based on omnist-spec#44 / `47cbedd`.
- Duplicate JSON/YAML key semantics were resolved by omnist-spec#45 / `0452cc8`; Rust already uses last-key-wins.
- The sibling OML/OSD suffix-copy lexer defect is absent.
- The sibling per-edge/per-field validation/materialization scan is absent.
- Region coverage below 100% is not a coverage-gate failure; Lines are 100.00%.

Stop here pending explicit approval. This plan does not authorize implementation, commits, issues, or pull requests.

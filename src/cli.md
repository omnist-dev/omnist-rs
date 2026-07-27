# CLI reference

The `omnist` binary (`omnist-cli`, issue #24) wraps the library crate as a
command-line tool. This page mirrors the actual `clap` command surface in
[`omnist-cli/src/lib.rs`](../omnist-cli/src/lib.rs); every example below is
drawn from a real assertion in [`omnist-cli/tests/cli.rs`](../omnist-cli/tests/cli.rs).

```text
omnist <COMMAND>

Commands:
  format    Canonicalize an OML document (the only format with no other tool for this)
  convert   Convert a document between formats (one in, one out)
  check     Report what writing as --to would adjust, without ever writing
  validate  Check a document against a schema (no schema-directed upgrading)
  infer     Draft a schema from example documents (all the same format)
  schema    Operate on a Schema (OSD)
```
<!-- doc-illustrative -->

Every subcommand accepts `-` for its input file meaning stdin, `--output`
(`-o`)/stdout for where the result goes, and `--json` to wrap CLI-level
errors as structured JSON on stderr instead of plain text. Exit codes: `0`
success, `1` a conformance/adjustment failure the command itself reports,
`2` a usage or parse error.

## `format`

Canonicalizes an OML document -- the only format with no other
canonicalization tool.

```bash
$ omnist format doc.oml
a: 1
b: "x"
```
<!-- verified-by: omnist-cli/tests/cli.rs::format_golden_path_pretty_prints_oml -->

`--compact` prints single-line OML (`; `-separated) instead of one edge per
line.

## `convert`

Converts one format to another in a single pass (`--from`/`--to` are
required; `json`, `yaml`, `toml`, `xml`, `oml`).

```bash
$ omnist convert doc.json --from json --to oml
a: 1
b: "x"
```
<!-- verified-by: omnist-cli/tests/cli.rs::convert_golden_path_json_to_oml -->

`--schema <file>` materializes the document against an OSD schema before
writing (schema-directed deserialization, e.g. numeric-string coercion);
`--strict` turns any lossy-write adjustment into an error instead of a
silent write; `--report` prints the adjustment report to stderr;
`--result-format {text,json,oml}` controls how a report (or `--schema`
outcome) is rendered.

## `check`

Like `convert`, but never writes -- reports what a `--to` write *would*
adjust.

```bash
$ omnist check doc.json --from json --to toml
no adjustments
```
<!-- verified-by: omnist-cli/tests/cli.rs::check_golden_path_never_writes_default_exit_0 -->

## `validate`

Checks a document against an OSD schema (`--schema <file>`, required). No
schema-directed upgrading happens here -- `validate` only checks, `convert
--schema` is what materializes.

```bash
$ omnist validate doc.json --from json --schema person.osd
valid
```
<!-- verified-by: omnist-cli/tests/cli.rs::validate_golden_path_is_valid_exit_0 -->

An invalid document exits `1` and prints every collected `ValidationError`;
`--json` emits `{"ok": false, "errors": [{"path", "message", "code"}, ...]}`
with the same stable `code` strings `schema::ErrorCode::as_str` returns
(`"type-mismatch"`, `"cardinality"`, ...).

## `infer`

Drafts a schema from one or more sample documents (`--from`, required; all
samples must be the same format).

```bash
$ omnist infer doc.json --from json
record Root {
    "a": integer,
    "b": string,
}
root Root
```
<!-- verified-by: omnist-cli/tests/doc_cli.rs::infer_golden_path_emits_exact_osd_text -->

`--allow-any` is accepted by the argument parser (matching Python's
surface) but returns a clear "not supported yet" error -- this port's
[`omnist::infer::infer`] has no `any`-fallback (see
[limitations.md](limitations.md)).

## `schema <subcommand>`

Operates on an OSD schema file (all subcommands take a `.osd` file, `-` for
stdin):

- `format` -- canonicalize OSD text.
- `normalize` -- canonical minimal form (structurally identical records
  collapsed to one).
- `prune` -- drop unreachable/unsatisfiable records and fields.
- `is-empty` -- exit `0` if the schema's language is empty, `1` otherwise
  (and prints `true`/`false`).
- `extract --keep <labels>` -- a subschema over a subset of root fields.
- `lint [--severity {info,warning}]` -- structural lint findings.
- `compatible-with <a> <b>` / `equivalent <a> <b>` -- subschema relations
  between two schema files.

```bash
$ omnist schema prune schema.osd   # Dead (unreachable) record is gone
record Root {
    "a": string,
}
root Root
```
<!-- verified-by: omnist-cli/tests/doc_cli.rs::schema_prune_emits_exact_osd_text -->

## Known scope gaps (library limitations, not CLI bugs)

- **`--arrays`**: accepted wherever the Python CLI accepts it (OML output
  paths), but this port's `omnist::oml::write_oml` has no `arrays` mode yet
  -- passing it where it would apply returns a clear "not supported yet"
  error (exit `2`) rather than being silently ignored or faked. Where
  `--arrays` has no effect per spec, it is accepted and silently ignored,
  matching Python.
- **schema-directed `--schema` on `convert`**: implemented via
  `omnist::materialize::materialize` on the raw node after a schema-less
  read, mirroring Python's own `_materialize(node, schema)` call inside
  each reader.

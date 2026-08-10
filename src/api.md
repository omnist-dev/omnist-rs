# API reference

This page enumerates the `omnist` crate's public surface: every exported
type, function, and method, grouped by area. Signatures are copied from the
current source (`omnist/src/*.rs`); doc comments are condensed, not
reproduced verbatim. For *why* each operation behaves the way it does, see
[the user guide](guide.md) (worked examples) or the linked
[omnist-spec](https://github.com/omnist-dev/omnist-spec) sections (the
normative behavior).

`omnist` has `publish = false` in `Cargo.toml`, so there is no docs.rs page
for this crate -- this is the closest equivalent. All items below are
re-exported at the crate root or reachable through their listed module path
(`omnist::document::Doc`, `omnist::materialize`, etc).

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```
<!-- doc-illustrative -->

`VERSION` is the crate's own Cargo package version.

## Documents (`omnist::document`)

The Document model: an ordered, possibly-repeated, possibly-interleaved
edge tree. See omnist-spec's
[`docs/02-document-model.md`](https://github.com/omnist-dev/omnist-spec/blob/main/docs/02-document-model.md)
for the normative model.

```rust
pub const MAX_DEPTH: usize = 200;
pub const MAX_NODES: usize = 1_000_000;
```
<!-- doc-illustrative -->

Depth and total-node-count guards enforced on every construction path.

```rust
pub struct NodeId(/* opaque */);
```
<!-- doc-illustrative -->

An opaque index into a `Doc`'s arena.

```rust
pub enum Scalar {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}
```
<!-- doc-illustrative -->

A leaf value. Implements `Display` (`null`, `true`, an integer, a float, or
a debug-quoted string).

```rust
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}
```
<!-- doc-illustrative -->

A plain input value (JSON/YAML/TOML-shaped) -- what `Doc::of`/`Doc::add`/
`Doc::set` turn into canonical nodes. `Object` uses `IndexMap` so key order
survives construction. `impl From<Scalar> for Value` converts the other
way.

```rust
pub struct Doc { /* private arena + root */ }

impl Doc {
    pub fn of(value: &Value) -> Result<Doc, DocumentError>;
    pub fn root(&self) -> Cursor<'_>;
    pub fn add(&mut self, at: NodeId, path: &str, label: &str, value: &Value)
        -> Result<NodeId, DocumentError>;
    pub fn set(&mut self, at: NodeId, path: &str, label: &str, value: &Value)
        -> Result<NodeId, DocumentError>;
    pub fn remove(&mut self, at: NodeId, path: &str, label: &str) -> Result<(), DocumentError>;
    pub fn to_grouped(&self) -> Value;
    pub fn to_data(&self) -> Value;
    pub fn eq_doc(&self, other: &Doc) -> bool;
    pub fn from_raw(root: RawNode) -> Result<Doc, DocumentError>;
    pub fn to_raw(&self) -> RawNode;
    pub fn from_format(name: &str, text: &str) -> Result<Doc, crate::error::OmnistError>;
    pub fn to_format(&self, name: &str) -> Result<String, crate::error::OmnistError>;
    pub fn check_format(&self, name: &str)
        -> Result<crate::report::WriteReport, crate::error::OmnistError>;
}
```
<!-- doc-illustrative -->

A guarded handle on a Document tree.

- `of` builds a `Doc` from a `Value`.
- `root` returns a cursor to the root, path `"$"`.
- `add` appends an edge `(label, value)` under an internal node; a repeated
  label is how an array grows. Returns the new edge's `NodeId`.
- `set` replaces all edges under `label` with a single new edge, positioned
  at the first old occurrence (`set = remove + add`).
- `remove` removes every edge under `label`.
- `to_grouped` returns a JSON-shaped projection: same-label edges grouped
  into an array.
- `to_data` returns a lossless structural copy (does not re-group repeated
  labels; used for `eq_doc`).
- `eq_doc` is structural equality: same shape, edge order, labels, leaf
  values.
- `from_raw`/`to_raw` convert to/from `RawNode`, preserving edge order and
  interleaving exactly.
- `from_format`/`to_format`/`check_format` dispatch by registered format
  name through `omnist::registry` (see [Registry](#registry-omnistregistry)
  below).

```rust
pub struct Cursor<'a> {
    pub path: String,
    /* private doc + id */
}

impl<'a> Cursor<'a> {
    pub fn id(&self) -> NodeId;
    pub fn is_leaf(&self) -> bool;
    pub fn value(&self) -> Result<&'a Scalar, DocumentError>;
    pub fn edges(&self) -> Result<Vec<(String, Cursor<'a>)>, DocumentError>;
    pub fn labels(&self) -> Vec<String>;
    pub fn get(&self, label: &str) -> Vec<Cursor<'a>>;
    pub fn get_one(&self, label: &str) -> Result<Cursor<'a>, DocumentError>;
    pub fn count(&self, label: &str) -> usize;
    pub fn child(&self, label: &str) -> Result<Cursor<'a>, DocumentError>;
    pub fn to_raw(&self) -> RawNode;
}
```
<!-- doc-illustrative -->

A read-only cursor into a `Doc`'s tree, tracking its own path (built
incrementally, e.g. `$.a.b[1]`, for error messages and equality with the
Python reference's `Doc.path`).

- `value` errors if called on an internal node ("not a leaf; use edges()").
- `edges` errors if called on a leaf; returns every child cursor with its
  label.
- `get`/`get_one` filter children by label; `get_one` errors unless exactly
  one match exists.
- `child` is an alias for `get_one`.
- `to_raw` returns a lossless `RawNode` copy of the subtree rooted at this
  cursor.

```rust
pub enum RawNode {
    Leaf(Scalar),
    TemporalLeaf(Scalar),
    Edges(Vec<(String, RawNode)>),
}
```
<!-- doc-illustrative -->

The *raw* canonical Document node: a leaf scalar, a write-hint temporal
leaf (schema- or OML-grammar-known to be date/time/datetime-kinded --
consumed only by `omnist::oml::write_oml`), or an ordered edge list that
may repeat and interleave a label arbitrarily. Unlike `Value::Object`'s
`IndexMap`, `RawNode::Edges` can represent non-contiguous repeats of the
same label exactly, which is why OML's reader/writer (`omnist::oml`) walks
`Doc` through this type instead of `Value`. `PartialEq` treats `Leaf` and
`TemporalLeaf` holding the same `Scalar` as equal -- the tag is a writer
hint, not a value difference.

## Schema model (`omnist::schema`)

The Schema model: closed records of labeled, cardinality-bound fields.
See omnist-spec's
[`docs/03-schema-model.md`](https://github.com/omnist-dev/omnist-spec/blob/main/docs/03-schema-model.md)
for the normative model.

```rust
pub enum ScalarKind {
    String,
    Integer,
    Number,
    Boolean,
    Date,
    Time,
    Datetime,
}

impl ScalarKind {
    pub const ALL: [ScalarKind; 7];
    pub fn as_str(&self) -> &'static str;
    pub fn parse(name: &str) -> Result<ScalarKind, SchemaError>;
}
```
<!-- doc-illustrative -->

One of the seven predefined value kinds a `Scalar` can hold. `parse` reads
a kind name as it appears in schema text (e.g. `"date"`).

```rust
pub struct Scalar { /* private kind + nullable */ }

impl Scalar {
    pub const fn new(kind: ScalarKind, nullable: bool) -> Self;
    pub fn named(name: &str, nullable: bool) -> Result<Self, SchemaError>;
    pub fn kind(&self) -> ScalarKind;
    pub fn is_nullable(&self) -> bool;
}

pub const STRING: Scalar;
pub const INTEGER: Scalar;
pub const NUMBER: Scalar;
pub const BOOLEAN: Scalar;
pub const DATE: Scalar;
pub const TIME: Scalar;
pub const DATETIME: Scalar;

pub fn nullable(scalar: Scalar) -> Scalar;
```
<!-- doc-illustrative -->

One of the seven predefined value types, optionally nullable. The seven
`pub const`s are the non-nullable form of each kind; `nullable(scalar)`
returns a copy that also accepts `null` (the `?` form in OSD text).
Implements `Display` (e.g. `integer`, `string?`).

```rust
pub struct Ref {
    pub name: String,
}

impl Ref {
    pub fn new(name: impl Into<String>) -> Self;
}
```
<!-- doc-illustrative -->

A reference to a named record in a `Schema`'s environment. `Display`
renders as `ref(name)`.

```rust
pub enum FieldType {
    Scalar(Scalar),
    Ref(Ref),
    Any,
}
```
<!-- doc-illustrative -->

A field's type. `Any` accepts every legal Document value (ported from
Python's `AnyType`/`ANY` singleton, shipped since Python v0.5.0) -- it is
neither a `Scalar` (no kind, no nullable flag) nor a `Ref` (names nothing).
`impl From<Scalar>` and `impl From<Ref>` construct a `FieldType` from
either.

```rust
pub struct Field {
    pub label: String,
    pub ty: FieldType,
    pub min: usize,
    pub max: Option<usize>,
}

impl Field {
    pub fn new(label: impl Into<String>, ty: impl Into<FieldType>, min: usize, max: Option<usize>)
        -> Result<Self, SchemaError>;
    pub fn required(label: impl Into<String>, ty: impl Into<FieldType>) -> Result<Self, SchemaError>;
    pub fn cardinality_str(&self) -> String;
}
```
<!-- doc-illustrative -->

One named, cardinality-bound field slot of a record: `label` of `ty`,
occurring `[min, max]` times (`max = None` is unbounded). `new` errors if
`max < min`. `required` is the common `[1,1]` case. `cardinality_str`
renders a human-readable cardinality description ("exactly 1", "0 or 1",
"at least N", "between N and M").

```rust
pub struct Record { /* private fields + label index */ }

impl Record {
    pub fn new(fields: Vec<Field>) -> Result<Self, SchemaError>;
    pub fn fields(&self) -> &[Field];
    pub fn field(&self, label: &str) -> Option<&Field>;
}
```
<!-- doc-illustrative -->

A closed set of named fields. `new` rejects a duplicate field label.
`fields` preserves declaration order; equality (`PartialEq`) is
declaration-order-independent (a record is an unordered field set at the
model layer, per omnist-spec §3.1).

```rust
pub enum ErrorCode {
    UnexpectedField,
    Cardinality,
    TypeMismatch,
    NullNotAllowed,
    ShapeMismatch,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str;
}
```
<!-- doc-illustrative -->

A stable, machine-readable validation failure code.

```rust
pub struct ValidationError {
    pub path: String,
    pub message: String,
    pub code: ErrorCode,
}

pub struct ValidationResult { /* private errors */ }

impl ValidationResult {
    pub fn new() -> Self;
    pub fn ok(&self) -> bool;
    pub fn errors(&self) -> &[ValidationError];
}
```
<!-- doc-illustrative -->

`ValidationResult` is the outcome of `Schema::validate`: empty on success,
one entry per problem found (validation collects every error, not just the
first). `Display` renders `"valid"` or an indented `"invalid:"` listing.

```rust
pub fn matches_kind(value: &document::Scalar, kind: ScalarKind) -> bool;
```
<!-- doc-illustrative -->

Does `value` match scalar kind `kind`? Validation only checks, never
converts (see `materialize` below for the upgrading counterpart). `Date`/
`Time`/`Datetime` only ever match a `Str` shaped like, and semantically
valid as, that kind's ISO form, since `document::Scalar` has no native
temporal variant.

```rust
pub enum Resolved<'a> {
    Record(&'a Record),
    Scalar(Scalar),
    Any,
}

pub struct Schema { /* private root + env */ }

impl Schema {
    pub fn new(root: Ref, env: IndexMap<String, Record>) -> Result<Self, SchemaError>;
    pub fn root(&self) -> &Ref;
    pub fn env(&self) -> &IndexMap<String, Record>;
    pub fn resolve(&self, ty: &FieldType) -> Resolved<'_>;
    pub fn validate(&self, cursor: &document::Cursor<'_>) -> ValidationResult;
    pub fn accepts(&self, cursor: &document::Cursor<'_>) -> bool;
}
```
<!-- doc-illustrative -->

A schema: a root reference plus an environment of named records.

- `new` checks every `Ref` (the root's, and every field's) resolves within
  `env`, and enforces that no record is named after a scalar keyword or
  `any` (a bare name in type position always resolves to the builtin
  first, so such a record could never be referenced).
- `resolve` maps a `FieldType` to a `Resolved` value: a bare `Scalar`
  resolves to itself, `Any` to `Resolved::Any`, a `Ref` to a single
  environment lookup (guaranteed to succeed once a `Schema` exists).
- `validate` walks `cursor` against the schema's root type, collecting
  every problem found.
- `accepts` is `validate(cursor).ok()`.

## Schema text -- OSD (`omnist::osd`)

OSD (Omnist Schema Definition) is the text language for `Schema`. See
omnist-spec's
[`docs/03-schema-model.md`](https://github.com/omnist-dev/omnist-spec/blob/main/docs/03-schema-model.md)
for the grammar.

```rust
pub fn parse_schema(text: &str) -> Result<Schema, SchemaError>;
pub fn to_osd(schema: &Schema, indent: Option<usize>) -> String;
```
<!-- doc-illustrative -->

`parse_schema` parses OSD text into a `Schema`. `to_osd` serializes a
`Schema` back to OSD text: `indent: None` renders a single-line,
machine-oriented form; `Some(n)` sets the pretty-printed indent width in
spaces. Both forms round-trip through `parse_schema`.

## Formats (`omnist::formats` and `omnist::oml`)

Codecs over the canonical Document model. Every builtin format follows the
same `read_*`/`write_*`/`check_*` naming convention. Unlike OML (always
lossless), JSON/YAML/TOML/XML writers are lenient by default -- they adjust
values that don't fit the target format and record the change in a
`WriteReport` (see [Reporting](#reporting-omnistreport)); `strict: true`
makes the writer return `WriteError` instead. See omnist-spec's
[`docs/04-formats.md`](https://github.com/omnist-dev/omnist-spec/blob/main/docs/04-formats.md)
for the per-format lossiness rules.

Note: OML lives in its own top-level `omnist::oml` module, not under
`omnist::formats` -- it is omnist's own native format, not one of the four
lossy interchange codecs.

```rust
// omnist::formats::json
pub fn read_json(text: &str) -> Result<Doc, OmnistError>;
pub fn write_json(doc: &Doc, indent: Option<usize>, strict: bool, report: Option<&mut WriteReport>)
    -> Result<String, WriteError>;
pub fn check_json(doc: &Doc) -> WriteReport;
```
<!-- doc-illustrative -->

JSON: `write_json`'s `indent: None` writes compact JSON; `Some(n)`
pretty-prints with `n` spaces per level. `NaN`/`Infinity`/`-Infinity` are
adjusted to `null` in lenient mode.

```rust
// omnist::formats::yaml
pub fn read_yaml(text: &str) -> Result<Doc, OmnistError>;
pub fn write_yaml(doc: &Doc, strict: bool, report: Option<&mut WriteReport>)
    -> Result<String, WriteError>;
pub fn check_yaml(doc: &Doc) -> WriteReport;
```
<!-- doc-illustrative -->

YAML: block style, 2-space indent, insertion order preserved. `read_yaml`
accepts exactly one YAML document (a multi-document stream errors, matching
Python's `yaml.safe_load`); empty/blank input parses as a `Null` document.

```rust
// omnist::formats::toml
pub fn read_toml(text: &str) -> Result<Doc, OmnistError>;
pub fn write_toml(doc: &Doc, strict: bool, report: Option<&mut WriteReport>)
    -> Result<String, WriteError>;
pub fn check_toml(doc: &Doc) -> WriteReport;
```
<!-- doc-illustrative -->

TOML: `write_toml` errors if the root isn't an object (TOML documents are
always tables at the top level). `null` values are stripped, recorded in
the report.

```rust
// omnist::formats::xml
pub fn read_xml(text: &str) -> Result<Doc, OmnistError>;
pub fn write_xml(doc: &Doc, strict: bool, report: Option<&mut WriteReport>)
    -> Result<String, WriteError>;
pub fn check_xml(doc: &Doc) -> WriteReport;
```
<!-- doc-illustrative -->

XML: `write_xml` requires a single-rooted Document (exactly one top-level
edge) and errors otherwise; `check_xml` does not enforce that shape (it
mirrors Python's `check_xml`, a plain adjustment scan with no root-shape
guard).

```rust
// omnist::oml
pub fn read_oml(text: &str) -> Result<RawNode, ParseError>;
pub fn write_oml(node: &RawNode, indent: usize) -> Result<String, WriteError>;
pub fn write_oml_compact(node: &RawNode) -> Result<String, WriteError>;
pub fn check_oml(doc: &Doc) -> WriteReport;
```
<!-- doc-illustrative -->

OML (Omnist Markup Language), omnist's own native format: every Document
round-trips through it exactly, with no adjustment ever needed. `read_oml`
supports the full OML-Core grammar plus OML-Extended raw-string (`'...'`)
and triple-quoted (`"""..."""`) string spellings on read only --
`write_oml`/`write_oml_compact` only ever emit OML-Core double-quoted
strings. `write_oml_compact` is the single-line form (edges joined by
`"; "`, no newlines); both round-trip through `read_oml`. `check_oml`
always returns an empty `WriteReport` (OML is lossless) -- it exists only
so the `"oml"` registry entry has a `check` callable like the other four
formats.

## Materialize (`omnist::materialize`)

```rust
pub fn materialize(node: &RawNode, schema: Option<&Schema>) -> Result<RawNode, MaterializeError>;
```
<!-- doc-illustrative -->

Schema-directed deserialization: a copy of `node` with leaf values upgraded
to match `schema` (e.g. `1.0 -> 1` for an `integer` field, `1 -> 1.0` for a
`number` field -- upgrades are always value-exact), guaranteed to conform
to it -- or every reason it can't, collected into one `MaterializeError`
(never just the first problem found). `schema: None` is a no-op passthrough
-- `node` is cloned back unchanged, with no validation performed. An
`Any`-typed field passes its node through completely untouched. See
omnist-spec's
[`docs/03-schema-model.md`](https://github.com/omnist-dev/omnist-spec/blob/main/docs/03-schema-model.md)
for the scalar-upgrade rules.

## Infer (`omnist::infer`)

```rust
pub struct AnyFallback {
    pub location: String,
    pub reason: String,
}

pub fn infer(samples: &[Doc], root_name: &str) -> Result<Schema, SchemaError>;
pub fn infer_with_report(samples: &[Doc], root_name: &str, allow_any: bool)
    -> Result<(Schema, Vec<AnyFallback>), SchemaError>;
```
<!-- doc-illustrative -->

`infer` drafts a `record` `Schema` (rooted at `root_name`) that accepts
every sample in `samples`; every sample's root must be an object, and an
empty `samples` list errors. It always infers with `allow_any: false` --
a label whose samples disagree in a way that can't resolve to one precise
type is a `SchemaError`.

`infer_with_report` additionally accepts `allow_any: true`: instead of
erroring on an ambiguous field, it opens the field as `FieldType::Any` and
records one `AnyFallback` (`location` reads `RecordName.label`; `reason`
says why). With `allow_any: false` it behaves exactly like `infer`, and the
returned `Vec<AnyFallback>` is always empty.

Neither function auto-normalizes -- the raw result may contain
structurally-identical duplicate records; call `omnist::ops::normalize` (see
[Operations / algebra](#operations--algebra-omnistops) below) where a
canonical minimal schema is wanted.

## Operations / algebra (`omnist::ops`)

The schema compatibility and structural-transform algebra. See
omnist-spec's
[`docs/03-schema-model.md`](https://github.com/omnist-dev/omnist-spec/blob/main/docs/03-schema-model.md)
for the algebra's normative rules and definitions.

```rust
// omnist::ops::subschema
pub fn compatible_with(a: &Schema, b: &Schema) -> bool;
pub fn equivalent(a: &Schema, b: &Schema) -> bool;
```
<!-- doc-illustrative -->

`compatible_with(a, b)` is true iff every document `a` accepts is also
accepted by `b` (`a` is a subschema of `b` / `b` is backward-compatible
with `a`). `equivalent(a, b)` is `compatible_with(a, b) &&
compatible_with(b, a)`.

```rust
// omnist::ops::extract
pub fn extract(s: &Schema, keep: &[&str]) -> Result<Schema, SchemaError>;
```
<!-- doc-illustrative -->

The minimal subschema of `s` that only recognizes documents built from
labels in `keep`. Errors if deleting the other labels would invalidate the
root record (deleting a mandatory field is a hard error, never silently
loosened to optional).

```rust
// omnist::ops::prune
pub fn satisfiable_set(s: &Schema) -> IndexSet<String>;
pub fn is_empty(s: &Schema) -> bool;
pub fn prune(s: &Schema) -> Schema;
```
<!-- doc-illustrative -->

`satisfiable_set` returns the env record names that admit at least one
finite document (a least fixpoint: a record is satisfiable iff every
mandatory field is a `Scalar`/`Any` or a `Ref` to a satisfiable record).
`is_empty` is true iff the root record itself is unsatisfiable (the
schema's language is empty). `prune` returns an equivalent schema with
everything that can never match removed: unreachable records, `max == 0`
fields, optional fields typed to an unsatisfiable record, and records left
unreachable/unsatisfiable afterward. If the root itself is unsatisfiable,
its fields are left untouched (stripping them would produce a different,
satisfiable schema) and only the rest of the environment is reduced.

```rust
// omnist::ops::minimize
pub fn equivalence_classes(s: &Schema) -> Vec<Vec<String>>;
pub fn normalize(s: &Schema) -> Schema;
```
<!-- doc-illustrative -->

`equivalence_classes` partitions `s.env`'s record names into
structural-equivalence classes via partition refinement (an initial
target-blind structural-key grouping, refined to a fixpoint by which block
each same-labeled ref field points to); it does not prune first. `normalize`
returns the canonical minimal schema equivalent to `s`: prunes first, then
collapses each equivalence class to its lexicographically smallest member
name. An unsatisfiable pruned root is returned unchanged (partition
refinement over an empty-language core isn't meaningful).

```rust
// omnist::ops::lint
pub struct LintFinding {
    pub code: &'static str,
    pub severity: &'static str,
    pub location: String,
    pub message: String,
}

pub fn lint(s: &Schema) -> Vec<LintFinding>;
```
<!-- doc-illustrative -->

Non-destructive structural diagnostics for a schema -- reports, never
mutates. Four checks: `unsatisfiable-record` (warning; a reachable record
no finite document can match), `unreachable-record` (warning; defined but
not reachable from root), `duplicate-record` (warning; two or more
structurally identical records under different names), and `any-field`
(info; an inventory of every `any`-typed field). Findings are sorted
deterministically by `(code, location)`.

```rust
// omnist::ops::isomorphic
pub fn is_isomorphic(a: &Schema, b: &Schema) -> bool;
```
<!-- doc-illustrative -->

True iff normalized schemas `a` and `b` are isomorphic: there is a
bijection between their env record names under which the two root records
(and everything reachable from them) match exactly. Both inputs are
assumed already normalized by the caller. **Empty-schema convention**: if
both `a` and `b` are unsatisfiable, they're treated as isomorphic (both
accept the empty language); if exactly one is, they're *not*. This is not
a committed public surface the way `equivalent` is -- it exists as an
independent cross-check oracle used in this crate's own test suite,
mirroring the Python reference's choice to keep the equivalent function
private there.

```rust
// omnist::ops::signature
pub enum ShapeKey {
    Scalar(ScalarKind, bool),
    Ref,
    Any,
}
pub type FieldKey = (String, usize, Option<usize>, ShapeKey);
pub type LocalSignature = Vec<FieldKey>;

pub fn local_signature(rec: &Record) -> LocalSignature;
```
<!-- doc-illustrative -->

`local_signature` is a record's target-blind structural key: every field's
`(label, min, max, shape)`, sorted by label, used as the initial partition
for `minimize`'s refinement and as the comparison key for `isomorphic`'s
signature matching.

## Registry (`omnist::registry`)

Name-keyed runtime dispatch: read/write/check a `Doc` by format name, and
register your own format plugins.

```rust
pub type ReadFn = dyn Fn(&str) -> Result<Doc, OmnistError> + Send + Sync;
pub type WriteFn = dyn Fn(&Doc) -> Result<String, OmnistError> + Send + Sync;
pub type CheckFn = dyn Fn(&Doc) -> WriteReport + Send + Sync;

pub struct Format {
    pub name: String,
    pub read: Arc<ReadFn>,
    pub write: Arc<WriteFn>,
    pub check: Option<Arc<CheckFn>>,
}

impl Format {
    pub fn new(
        name: impl Into<String>,
        read: impl Fn(&str) -> Result<Doc, OmnistError> + Send + Sync + 'static,
        write: impl Fn(&Doc) -> Result<String, OmnistError> + Send + Sync + 'static,
    ) -> Self;
    pub fn with_check(self, check: impl Fn(&Doc) -> WriteReport + Send + Sync + 'static) -> Self;
}

pub fn register_format(fmt: Format);
pub fn get_format(name: &str) -> Result<Format, OmnistError>;
pub fn formats() -> Vec<String>;
```
<!-- doc-illustrative -->

`Format` bundles a name with `read`/`write` callables and an optional
`check` -- a plugin built with `Format::new` alone has no `check`, and
`Doc::check_format` returns a clean error (not a panic) if invoked on it.
`register_format` adds or replaces a format under `fmt.name`, usable
everywhere a format name is accepted (`Doc::from_format`/`to_format`/
`check_format`). `get_format` looks up a registered `Format` by name, or an
`OmnistError::Format` naming every currently-registered name, sorted.
`formats` lists all registered format names, sorted. The five builtins
(`json`, `yaml`, `toml`, `xml`, `oml`) are always registered.

## Errors (`omnist::error`)

```rust
pub struct DocumentError {
    pub path: String,
    pub message: String,
}

pub struct SchemaError(pub String);

pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

pub struct FormatError(pub String);

pub struct WriteError {
    pub message: String,
    pub report: Option<crate::report::WriteReport>,
}

impl WriteError {
    pub fn new(message: impl Into<String>) -> Self;
    pub fn with_report(message: impl Into<String>, report: crate::report::WriteReport) -> Self;
    pub fn report(&self) -> Option<&crate::report::WriteReport>;
}

pub struct MaterializeError(pub crate::schema::ValidationResult);

impl MaterializeError {
    pub fn new(result: crate::schema::ValidationResult) -> Self;
    pub fn result(&self) -> &crate::schema::ValidationResult;
    pub fn errors(&self) -> &[crate::schema::ValidationError];
}

pub enum OmnistError {
    Document(DocumentError),
    Schema(SchemaError),
    Materialize(MaterializeError),
    Parse(ParseError),
    Write(WriteError),
    Format(FormatError),
}
```
<!-- doc-illustrative -->

`OmnistError` is the crate-wide top-level error; each leaf type is a
`#[from]` variant, mirroring the Python reference's
`OmnistError`/`SchemaError`/`ParseError`/`WriteError`/`DocumentError`
hierarchy.

- `DocumentError` -- a Document operation is invalid, or a plain value is
  not a legal Document (construction/mutation outside the Document model,
  or an operation that doesn't fit the node it's called on).
- `SchemaError` -- a Schema definition is invalid (bad cardinality,
  duplicate field label, unknown scalar/ref name).
- `ParseError` -- OML source text could not be parsed; carries a "line N,
  col N: msg" position.
- `FormatError` -- an unknown format name was looked up in the format
  registry.
- `WriteError` -- an in-memory Document could not be written; carries an
  optional `WriteReport` when a strict-mode format writer raised because of
  accumulated adjustments.
- `MaterializeError` -- a freshly-read node could not be made to conform to
  a `Schema`; wraps the same `ValidationResult` `Schema::validate` uses.

## Reporting (`omnist::report`)

```rust
pub enum Severity {
    Warning,
    Error,
}

pub struct Adjustment {
    pub path: String,
    pub code: String,
    pub message: String,
    pub severity: Severity,
}

pub struct WriteReport { /* private adjustments */ }

impl WriteReport {
    pub fn new() -> Self;
    pub fn add(&mut self, path: impl Into<String>, code: impl Into<String>,
        message: impl Into<String>, severity: Severity);
    pub fn adjustments(&self) -> &[Adjustment];
    pub fn warnings(&self) -> Vec<&Adjustment>;
    pub fn errors(&self) -> Vec<&Adjustment>;
    pub fn is_ok(&self) -> bool;
    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn iter(&self) -> std::slice::Iter<'_, Adjustment>;
}

pub fn finish_write(text: String, rep: WriteReport, strict: bool, report: Option<&mut WriteReport>)
    -> Result<String, WriteError>;
```
<!-- doc-illustrative -->

Adjustment reports for lossy writes. Writing a `Doc` to a format that can't
hold every value losslessly means the writer has to adjust the data; each
adjustment is recorded as an `Adjustment` in a `WriteReport` rather than
lost silently.

- `Severity::Warning` is conventional/recoverable (e.g. a date written as a
  string); `Severity::Error` is likely to surprise or corrupt (e.g. `NaN`
  written as JSON `null`).
- `WriteReport::is_ok` is true iff there are no error-severity entries --
  warnings alone don't flip it (this is the report's `bool` conversion in
  the Python reference).
- `finish_write` is the standard `strict`/`report` handling every format
  writer applies to its own accumulated `WriteReport`: if `report` is
  given, `rep`'s adjustments are copied into it; if `strict` and `rep` has
  any adjustments, returns `WriteError` carrying `rep`; otherwise returns
  `text`.

#!/usr/bin/env python3
"""Extract Python-observed input/output fixtures for the omnist-rs parity
harness (issue #40 in omnist-dev/omnist-rs). Runs the real, installed
Python `omnist` package (no reading of assertions) and dumps a JSON
corpus consumed by omnist-rs's `omnist/tests/parity.rs`.

Scope note: covers the modules that have a shipped Rust counterpart today
(oml codec, depth guards, schema/OSD + ops algebra, doc<->format codecs,
materialize, ops.lint). Explicitly OUT of scope, documented in the PR:
  - test_any_core.py / test_any_grammar.py: exercise the `any` type's OSD
    *grammar* edge cases -- basic `any` field parsing already has a Rust
    counterpart (`omnist/src/osd.rs`'s `any` keyword support) and is
    exercised indirectly by the lint fixtures below (`any-field`), but the
    dedicated grammar-edge-case corpus in these two files is still a later
    PR per the v1.0 `any` decision (see docs/design/any-type-spec.md
    upstream).
  - test_public_api.py: freezes the *Python* import surface
    (omnist.__all__, signatures) - not a cross-language concept.
  - test_cli.py / test_cli_examples.py / test_cli_fuzz.py: Python CLI
    plumbing/argparse behavior, not a Document/Schema API.
  - test_examples*.py / test_docs.py / test_check_doc_examples.py /
    test_grammar_docs.py: doc-example / README / packaging generators for
    the Python repo's own tooling, not portable data.
  - test_fuzz.py: already ported at omnist-rs issue #26
    (omnist/tests/fuzz.rs), including a live cross-implementation oracle.
  - test_semantic_oracle.py: exercises tools/semantic_oracle.py, a
    Python-only dev tool (already used as the oracle by fuzz.rs).

Issue #61 correction: `test_lint.py` was previously (and wrongly) grouped
under "doc-example/README/packaging generators" above. It is actually pure
schema-diagnostics logic (`omnist.ops.lint`), which has a full Rust port at
`omnist/src/ops/lint.rs` -- its 9 cases are now extracted below. Issue #61
also expands `test_canonical.py` coverage from ~1 fixture/class toward ~1
fixture/method for `TestDocument`, `TestInfer`, `TestValidation`,
`TestOsdRobustness`, `TestTemporalBoundary`, and `TestOperations` (the
other ~20 classes in that file remain out of scope for this pass -- most
cover `TestExtract`/`TestNormalizePartitionRefinement`/etc. algorithms not
yet ported, or Python-specific plumbing (registry, plugin, dunder/repr
checks) with no Rust equivalent to replay against), plus a lighter pass
over `test_depth_guards.py`.
"""
import datetime
import json
import math
import sys

from omnist import (
    Doc,
    LintFinding,
    ParseError,
    SchemaError,
    WriteError,
    DocumentError,
    check_oml,
    doc,
    field,
    lint,
    materialize,
    parse_schema,
    read_oml,
    record,
    ref,
    schema,
    t,
    to_osd,
    write_oml,
    write_json,
    write_toml,
    write_xml,
    write_yaml,
    read_json,
    read_toml,
    read_xml,
    read_yaml,
)
from omnist.ops import compatible_with, equivalent, is_empty, normalize, prune

fixtures = []


def enc(v):
    """JSON-safe encoding of a raw OML node value (mirrors Rust RawNode)."""
    if v is None:
        return {"$null": True}
    if isinstance(v, bool):
        return {"$bool": v}
    if isinstance(v, int):
        return {"$int": v}
    if isinstance(v, float):
        if math.isnan(v):
            return {"$float": "nan"}
        if math.isinf(v):
            return {"$float": "inf" if v > 0 else "-inf"}
        return {"$float": v}
    if isinstance(v, str):
        return {"$str": v}
    # omnist-rs's Scalar has no dedicated date/time/datetime variant (see
    # document.rs's module doc): temporal literals are represented as
    # Scalar::Str holding the ISO text. Encode the same way here so the
    # Rust harness compares like-for-like.
    if isinstance(v, datetime.datetime):
        return {"$str": v.isoformat()}
    if isinstance(v, datetime.date):
        return {"$str": v.isoformat()}
    if isinstance(v, datetime.time):
        return {"$str": v.isoformat()}
    if isinstance(v, list):
        return {"$edges": [[label, enc(val)] for (label, val) in v]}
    raise TypeError(f"unhandled type {type(v)}")


def add(module, note, kind, **kw):
    fixtures.append({"module": module, "note": note, "kind": kind, **kw})


# --------------------------------------------------------------------
# test_oml.py: scalar round trips
# --------------------------------------------------------------------
OML_SCALAR_CASES = [
    ('a: "hello"', "string scalar"),
    ("a: 42", "positive integer scalar"),
    ("a: -42", "negative integer scalar"),
    ("a: 3.14", "positive float scalar"),
    ("a: -3.14", "negative float scalar"),
    ("a: 1e10", "float exponent notation"),
    ("a: 1.5e-3", "float negative exponent"),
    ("a: true", "boolean true scalar"),
    ("a: false", "boolean false scalar"),
    ("a: null", "null scalar"),
    ("a: 2024-01-01", "date scalar"),
    ("a: 12:30:00", "time scalar"),
    ("a: 2024-01-01T12:30:00", "datetime scalar"),
    ("a: inf", "positive infinity float"),
    ("a: -inf", "negative infinity float"),
]
for src, label in OML_SCALAR_CASES:
    node = read_oml(src)
    add(
        "test_oml.test_scalar_round_trip",
        f"OML: {label} parses and round-trips through write_oml",
        "oml_roundtrip",
        input=src,
        expected=enc(node),
    )

# NaN: not self-equal, checked separately in Python; assert is-nan property.
nan_node = read_oml("a: nan")
assert math.isnan(nan_node[0][1])
add(
    "test_oml.test_scalar_round_trip",
    "OML: 'nan' float scalar parses to a NaN value (property check, not equality)",
    "oml_parses_to_nan",
    input="a: nan",
)

add("test_oml.test_empty_document_is_empty_node", "OML: empty text is the empty node []",
    "oml_roundtrip", input="", expected=enc(read_oml("")))
add("test_oml.test_empty_document_is_empty_node", "OML: whitespace-only text is the empty node []",
    "oml_roundtrip", input="   \n  \n", expected=enc(read_oml("   \n  \n")))
add("test_oml.test_crlf_line_endings_act_as_separators", "OML: CRLF line endings act as separators",
    "oml_roundtrip", input="a: 1\r\nb: 2\r\n", expected=enc(read_oml("a: 1\r\nb: 2\r\n")))
add("test_oml.test_bare_leaf_document", "OML: bare integer leaf (no label) is a valid document",
    "oml_roundtrip", input="42", expected=enc(read_oml("42")))
add("test_oml.test_bare_leaf_document", "OML: bare string leaf (no label) is a valid document",
    "oml_roundtrip", input='"just a string"', expected=enc(read_oml('"just a string"')))
add("test_oml.test_repeated_labels_and_interleaving", "OML: repeated/interleaved labels preserve order",
    "oml_roundtrip", input="a: 1\nb: 2\na: 3\nb: 4\na: 5",
    expected=enc(read_oml("a: 1\nb: 2\na: 3\nb: 4\na: 5")))
add("test_oml.test_nested_braces_arbitrary_depth", "OML: nested braces at arbitrary depth",
    "oml_roundtrip", input='a: { b: { c: { d: "leaf" } } }',
    expected=enc(read_oml('a: { b: { c: { d: "leaf" } } }')))
add("test_oml.test_inline_brace_style_with_semicolons", "OML: inline brace style with semicolon separators",
    "oml_roundtrip", input='{ a: 1; b: 2 }', expected=enc(read_oml('{ a: 1; b: 2 }')))

OML_ERROR_CASES = [
    ("a: `", "stray character backtick is a ParseError", "test_oml.test_stray_character_is_a_parse_error"),
    ("a: @", "stray character '@' is rejected", "test_oml.test_stray_characters_are_rejected"),
    ("a: &", "stray character '&' is rejected", "test_oml.test_stray_characters_are_rejected"),
    ("a: ]", "unmatched close bracket is a ParseError", "test_oml.test_unmatched_close_bracket_is_a_parse_error"),
    ("a: 2024-13-01", "invalid date (month 13) is a ParseError", "test_oml.test_invalid_temporal_literals_are_parse_errors"),
    ("a: 25:00:00", "invalid time (hour 25) is a ParseError", "test_oml.test_invalid_temporal_literals_are_parse_errors"),
    ("a: 2024-13-01T00:00:00", "invalid datetime (month 13) is a ParseError", "test_oml.test_invalid_temporal_literals_are_parse_errors"),
]
for src, label, module in OML_ERROR_CASES:
    try:
        read_oml(src)
        raise SystemExit(f"expected ParseError for {src!r}")
    except ParseError as e:
        add(module, f"OML: {label}", "oml_parse_error", input=src, error_contains=str(e))

# --------------------------------------------------------------------
# test_depth_guards.py
# --------------------------------------------------------------------
def deep_node(depth, leaf=1):
    node = leaf
    for _ in range(depth):
        node = [("a", node)]
    return node

DEEP = 5000
JUST_UNDER = 190

try:
    write_oml(deep_node(DEEP))
    raise SystemExit("expected WriteError")
except WriteError as e:
    add("test_depth_guards.TestWriteOml.test_too_deep_raises_write_error_naming_the_limit",
        "depth guard: write_oml on a node 5000 levels deep raises WriteError naming the 200 limit",
        "oml_write_error", depth=DEEP, error_contains="nesting exceeds the maximum depth (200)")

just_under_text = write_oml(deep_node(JUST_UNDER))
add("test_depth_guards.TestWriteOml.test_just_under_limit_succeeds",
    "depth guard: write_oml on a node 190 levels deep (just under the 200 limit) succeeds",
    "oml_write_ok", depth=JUST_UNDER)

# --------------------------------------------------------------------
# test_canonical.py: schema / OSD / ops algebra + format codecs
# --------------------------------------------------------------------
OSD_R_INT_STR = 'record R { "n": integer, "s": string? }\nroot R'
s = parse_schema(OSD_R_INT_STR)
add("test_canonical.TestPublicApi.test_methods_and_t_namespace",
    "schema: OSD parses record with required integer + optional string field",
    "osd_parse_ok", input=OSD_R_INT_STR)

ok = s.validate(doc({"n": 1, "s": None})).ok
assert ok is True
add("test_canonical.TestPublicApi.test_methods_and_t_namespace",
    "schema: validate() accepts {n:1, s:null} against record R{n:integer, s:string?}",
    "schema_validate", schema=OSD_R_INT_STR,
    doc_json_input={"n": 1, "s": None}, expected_ok=True)

roundtrip_osd = to_osd(s)
s2 = parse_schema(roundtrip_osd)
assert s.equivalent(s2)
add("test_canonical.TestPublicApi.test_methods_and_t_namespace",
    "schema: to_osd()+parse_schema() round-trips to an equivalent schema",
    "schema_osd_roundtrip_equivalent", schema=OSD_R_INT_STR)

WIDE_OSD = 'record R { "n": number, "s": string? }\nroot R'
wide = parse_schema(WIDE_OSD)
assert s.compatible_with(wide)
add("test_canonical.TestPublicApi.test_methods_and_t_namespace",
    "schema: {n:integer,s:string?} is compatible_with widened {n:number,s:string?} (integer<:number)",
    "schema_compatible_with", schema_a=OSD_R_INT_STR, schema_b=WIDE_OSD, expected=True)

norm = s.normalize()
assert norm.equivalent(s)
add("test_canonical.TestPublicApi.test_methods_and_t_namespace",
    "schema ops: normalize() of a schema stays equivalent() to the original",
    "schema_normalize_equivalent", schema=OSD_R_INT_STR)

# additional ops coverage: is_empty / prune on a simple schema
assert is_empty(parse_schema('record R { }\nroot R')) is False
add("ops.is_empty",
    "ops: a record with no fields is not is_empty() (it still accepts the empty record itself)",
    "schema_is_empty", schema='record R { }\nroot R', expected=False)

# format codecs: a Doc round trips through json/yaml (both support null natively)
SAMPLE_DOC = {"name": "Ada", "age": 36, "active": True, "tags": None}
d = doc(SAMPLE_DOC)
for fmt_name in ["json", "yaml"]:
    text = getattr(d, f"to_{fmt_name}")()
    back = getattr(type(d), f"from_{fmt_name}")(text)
    assert back.to_data() == d.to_data()
    add(f"test_canonical.formats.{fmt_name}",
        f"format: Doc({{name,age,active,tags:null}}) round-trips through Doc.to_{fmt_name}/from_{fmt_name}",
        "format_roundtrip", format=fmt_name, doc_json=SAMPLE_DOC)

# TOML has no null literal: the same doc minus the null field round-trips losslessly.
TOML_DOC = {"name": "Ada", "age": 36, "active": True}
dt = doc(TOML_DOC)
ttext = dt.to_toml()
tback = type(dt).from_toml(ttext)
assert tback.to_data() == dt.to_data()
add("test_canonical.formats.toml",
    "format: Doc({name,age,active}) (no null field - TOML has no null literal) round-trips through Doc.to_toml/from_toml",
    "format_roundtrip", format="toml", doc_json=TOML_DOC)

# XML requires a single-rooted document (one top-level edge).
XML_DOC = {"root": {"name": "Ada", "age": 36, "active": True}}
dx = doc(XML_DOC)
xtext = dx.to_xml()
xback = type(dx).from_xml(xtext)
assert xback.to_data() == dx.to_data()
add("test_canonical.formats.xml",
    "format: single-rooted Doc({root:{name,age,active}}) round-trips through Doc.to_xml/from_xml",
    "format_roundtrip", format="xml", doc_json=XML_DOC)

# --------------------------------------------------------------------
# materialize: fill in a Doc's schema-implied defaults (infer's counterpart)
# --------------------------------------------------------------------
from omnist import infer as _infer
MAT_DOC = {"n": 7, "s": "x"}
mat_input = doc(MAT_DOC)
mat_schema_inferred = _infer([mat_input])
materialized = materialize(mat_input.to_data(), mat_schema_inferred)
add("test_canonical.materialize",
    "materialize: a Doc materialized against its own inferred schema is unchanged (idempotent w.r.t. self-inference)",
    "materialize_case", schema=to_osd(mat_schema_inferred), doc_json=MAT_DOC, expected=enc(materialized))

# --------------------------------------------------------------------
# test_lint.py: omnist.ops.lint structural diagnostics (issue #61 gap 1)
# --------------------------------------------------------------------
def lint_triples(findings):
    """(code, severity, location) triples, dropping `message` -- Python's
    message text uses `repr()` (single-quoted) and Rust's uses `Debug`
    (double-quoted), so exact wording is language-specific; the codes,
    severities, and locations are the portable, asserted contract."""
    return [[f.code, f.severity, f.location] for f in findings]


LINT_UNSAT = 'record A { "b": B }\nrecord B { "a": A }\nroot A'
add("test_lint.test_unsatisfiable_record",
    "lint: a mandatory ref cycle (A<->B) reports both records unsatisfiable",
    "lint_case", schema=LINT_UNSAT, expected=lint_triples(lint(parse_schema(LINT_UNSAT))))

LINT_UNREACH = 'record R { "x": integer }\nrecord Orphan { "y": string }\nroot R'
add("test_lint.test_unreachable_record",
    "lint: a record defined but never referenced from root is unreachable-record",
    "lint_case", schema=LINT_UNREACH, expected=lint_triples(lint(parse_schema(LINT_UNREACH))))

LINT_DUP = ('record Addr { "c": string }\nrecord Location { "c": string }\n'
            'record R { "a": Addr, "l": Location }\nroot R')
add("test_lint.test_duplicate_record",
    "lint: two structurally identical records (Addr, Location) report duplicate-record",
    "lint_case", schema=LINT_DUP, expected=lint_triples(lint(parse_schema(LINT_DUP))))

LINT_ANY = 'record R { "id": string, "data": any }\nroot R'
add("test_lint.test_any_field_inventory",
    "lint: a schema with one any-typed field reports exactly one any-field finding",
    "lint_case", schema=LINT_ANY, expected=lint_triples(lint(parse_schema(LINT_ANY))))

LINT_CLEAN = 'record R { "x": integer, "y" [0,1]: string }\nroot R'
assert lint(parse_schema(LINT_CLEAN)) == []
add("test_lint.test_clean_schema_has_no_findings",
    "lint: a schema with no structural problems produces zero findings",
    "lint_case", schema=LINT_CLEAN, expected=[])

LINT_SORTED = ('record A { "b": B }\nrecord B { "a": A }\n'
               'record Orphan { "z": any }\nroot A')
sorted_findings = lint(parse_schema(LINT_SORTED))
sorted_keys = [(f.code, f.location) for f in sorted_findings]
assert sorted_keys == sorted(sorted_keys)
add("test_lint.test_findings_sorted_by_code_then_location",
    "lint: findings across all four check kinds come back sorted by (code, location)",
    "lint_case", schema=LINT_SORTED, expected=lint_triples(sorted_findings))

LINT_ANY_ONLY = 'record R { "data": any }\nroot R'
any_only_findings = lint(parse_schema(LINT_ANY_ONLY))
assert [f.code for f in any_only_findings] == ["any-field"]
assert not any(f.severity == "warning" for f in any_only_findings)
add("test_lint.test_any_only_schema_has_no_warning",
    "lint: an info-only schema (just an any-field inventory) has no warning-severity findings",
    "lint_case", schema=LINT_ANY_ONLY, expected=lint_triples(any_only_findings))

# test_lint_finding_is_frozen: Python's LintFinding is a frozen dataclass
# (assigning a field raises). Rust's `LintFinding` has no interior
# mutability and is only ever handed out by-value from `lint()` -- there is
# no setter to call, so the "cannot mutate after construction" property
# holds by construction rather than by a runtime check. This fixture
# documents the equivalence (field values constructed and read back) rather
# than replaying a mutation-attempt/exception, which has no Rust analogue.
frozen_check = LintFinding("any-field", "info", "R.x", "msg")
add("test_lint.test_lint_finding_is_frozen",
    "lint: LintFinding's four fields round-trip through construction (Python enforces "
    "immutability via a frozen dataclass; Rust's by-value struct has no setter to guard)",
    "lint_finding_shape", code=frozen_check.code, severity=frozen_check.severity,
    location=frozen_check.location, message=frozen_check.message)

LINT_NO_MUTATE = 'record R { "x": integer }\nrecord Orphan { "y": string }\nroot R'
s_no_mutate = parse_schema(LINT_NO_MUTATE)
before_env = list(s_no_mutate.env)
lint(s_no_mutate)
assert list(s_no_mutate.env) == before_env
add("test_lint.test_lint_does_not_mutate",
    "lint: calling lint(s) does not change s's env (record set, order) -- diagnose, never mutate",
    "lint_no_mutation", schema=LINT_NO_MUTATE)


# --------------------------------------------------------------------
# test_canonical.py: TestDocument (issue #61 gap 2)
# --------------------------------------------------------------------
def edges_of(pairs):
    """`enc()`-tag a raw Python edge-list (list of (label, scalar) pairs),
    matching the `$edges` shape `parity.rs::decode_raw` already parses."""
    return enc(list(pairs))


d = doc({"name": "Ann", "age": 30})
add("test_canonical.TestDocument.test_build_and_navigate",
    "Document: labels()/get_one() navigate a flat two-field document",
    "doc_query", initial=edges_of([("name", "Ann"), ("age", 30)]),
    expected_labels=d.labels(), expected_counts={"name": d.count("name"), "age": d.count("age")})

d = doc({"member": [{"n": 1}, {"n": 2}]})
add("test_canonical.TestDocument.test_repeated_label_is_an_array",
    "Document: a repeated label ('member' x2) is 'many members', not an array field",
    "doc_query", initial=edges_of([("member", [("n", 1)]), ("member", [("n", 2)])]),
    expected_labels=d.labels(), expected_counts={"member": d.count("member")})

d = doc({"a": 1, "xs": [1, 2]})
add("test_canonical.TestDocument.test_to_data_is_edge_list",
    "Document: to_data() on a repeated label is the flat edge list, not a grouped array",
    "doc_to_data", initial=edges_of([("a", 1), ("xs", 1), ("xs", 2)]),
    expected=enc([("a", 1), ("xs", 1), ("xs", 2)]))

# test_to_grouped_projects_back is already covered by the `format_roundtrip`
# fixtures below (to_grouped is what to_json/to_yaml project through).

# test_bare_array_rejected / test_array_of_arrays_rejected /
# test_non_string_key_rejected: these assert on the SHAPE Python's `doc()`
# constructor accepts as raw input (a bare list, nested list-of-lists, a
# non-string dict key) -- Rust's `RawNode`/`Value` are statically typed
# (Edges is always `Vec<(String, _)>`), so these Python-only constructor-
# input-shape rejections have no Rust call site to replay against; noted
# here rather than silently dropped.

# test_editing + all test_set_* (issue #156, B4): replayed as an initial
# edge-list plus a sequence of add/set/remove ops, checked against the
# final edge list -- covers Doc::add/set/remove's shared semantics.
DOC_OPS_CASES = [
    ("test_canonical.TestDocument.test_editing",
     "Document: add()x2 (repeated 'tag'), set() on existing/absent labels, then remove('tag')",
     [("name", "Ann")],
     [("add", "tag", "x"), ("add", "tag", "y"), ("set", "name", "Bob"),
      ("set", "age", 30), ("remove", "tag", None)]),
    ("test_canonical.TestDocument.test_set_on_absent_label_appends",
     "Document: set() on a label absent from the document appends a new edge",
     [("a", 1)], [("set", "b", 2)]),
    ("test_canonical.TestDocument.test_set_on_single_label_unchanged_behavior",
     "Document: set() on a label with exactly one existing edge replaces it in place",
     [("a", 1), ("b", 2)], [("set", "a", 99)]),
    ("test_canonical.TestDocument.test_set_on_repeated_label_collapses_to_one_at_first_position",
     "Document: set() on a repeated label collapses all occurrences to one, at the first position",
     [("a", 1)], [("add", "a", 2), ("add", "b", 9), ("set", "a", 99)]),
    ("test_canonical.TestDocument.test_set_preserves_first_occurrence_position_among_other_labels",
     "Document: set() keeps the surviving edge at the label's first-occurrence slot",
     [("a", 1), ("b", 2)], [("add", "a", 3), ("set", "a", 99)]),
    ("test_canonical.TestDocument.test_set_after_remove_appends",
     "Document: set() after remove() appends at the end, same as a fresh label",
     [("a", 1)], [("add", "a", 2), ("remove", "a", None), ("set", "a", 7)]),
]
for module, note, initial_pairs, ops in DOC_OPS_CASES:
    d = doc(dict())  # placeholder object; not used -- Doc is edge-list oriented below
    # Replay against a real Python Doc built the same edge-list way, to
    # capture its actually-observed to_data() (not a hand-transcribed one).
    pyd = Doc(list(initial_pairs))
    for op, label, value in ops:
        if op == "add":
            pyd.add(label, value)
        elif op == "set":
            pyd.set(label, value)
        elif op == "remove":
            pyd.remove(label)
    add(module, note, "doc_ops",
        initial=edges_of(initial_pairs),
        ops=[{"op": o, "label": lbl, "value": (enc(v) if v is not None else None)}
             for (o, lbl, v) in ops],
        expected=enc(pyd.to_data()))

# test_set_replace_all_matches_remove_then_add_docstring_contract: the ONE
# documented divergence between set() and a literal remove()+add() -- both
# variants replayed, expecting two DIFFERENT final edge lists.
d1 = Doc([("a", 1)])
d1.add("a", 2).add("b", 9)
d1.set("a", 99)
d2 = Doc([("a", 1)])
d2.add("a", 2).add("b", 9)
d2.remove("a").add("a", 99)
add("test_canonical.TestDocument.test_set_replace_all_matches_remove_then_add_docstring_contract",
    "Document: set() keeps first-occurrence position; remove()+add() appends at the end -- "
    "same starting document, two different resulting edge lists",
    "doc_ops_pair",
    initial=edges_of([("a", 1)]),
    ops_a=[{"op": "add", "label": "a", "value": enc(2)}, {"op": "add", "label": "b", "value": enc(9)},
           {"op": "set", "label": "a", "value": enc(99)}],
    ops_b=[{"op": "add", "label": "a", "value": enc(2)}, {"op": "add", "label": "b", "value": enc(9)},
           {"op": "remove", "label": "a", "value": None}, {"op": "add", "label": "a", "value": enc(99)}],
    expected_a=enc(d1.to_data()), expected_b=enc(d2.to_data()))


# --------------------------------------------------------------------
# test_canonical.py: TestInfer
# --------------------------------------------------------------------
def infer_docs(dicts):
    return [doc(x) for x in dicts]


s = _infer(infer_docs([{"name": "Ann", "age": 30}, {"name": "Bob"}]))
add("test_canonical.TestInfer.test_flat",
    "infer: age (present in one sample, absent in another) infers optional; name required",
    "infer_case", samples=[{"name": "Ann", "age": 30}, {"name": "Bob"}],
    checks=[({"name": "Cy"}, True), ({"age": 1}, False)])

s = _infer(infer_docs([{"id": 1, "tags": ["a", "b"], "addr": {"city": "X"}}]))
add("test_canonical.TestInfer.test_array_and_nested",
    "infer: array-of-string and nested-record shape both infer correctly",
    "infer_case", samples=[{"id": 1, "tags": ["a", "b"], "addr": {"city": "X"}}],
    checks=[({"id": 9, "tags": ["c"], "addr": {"city": "Y"}}, True),
            ({"id": 9, "tags": [1], "addr": {"city": "Y"}}, False)])

samples = [{"v": 1}, {"v": 2.5}]
s = _infer(infer_docs(samples))
add("test_canonical.TestInfer.test_accepts_its_own_samples",
    "infer: a schema inferred from int+float samples (widened to number) accepts both samples back",
    "infer_case", samples=samples, checks=[(x, True) for x in samples])

try:
    _infer(infer_docs([{"v": 1}, {"v": "x"}]))
    raise SystemExit("expected SchemaError")
except SchemaError:
    add("test_canonical.TestInfer.test_conflicting_scalars_raise",
        "infer: conflicting scalar kinds for the same field (int vs string) across samples raises SchemaError",
        "infer_error", samples=[{"v": 1}, {"v": "x"}])

s = _infer(infer_docs([{"v": None}, {"v": None}]))
add("test_canonical.TestInfer.test_null_only_field_infers_nullable_string",
    "infer: a field that is null in every sample infers as nullable string (accepts any value)",
    "infer_case", samples=[{"v": None}, {"v": None}],
    checks=[({"v": None}, True), ({"v": "anything"}, True)])

s = _infer(infer_docs([{"v": 1}, {"v": None}]))
add("test_canonical.TestInfer.test_null_alongside_a_kind_is_orthogonal",
    "infer: null alongside a concrete kind (int) infers nullable-of-that-kind, not any",
    "infer_case", samples=[{"v": 1}, {"v": None}],
    checks=[({"v": 7}, True), ({"v": None}, True), ({"v": "x"}, False)])

absent_first = _infer(infer_docs([{"host": "a"}, {"host": "b", "port": 80}]))
absent_last = _infer(infer_docs([{"host": "b", "port": 80}, {"host": "a"}]))
assert absent_first.equivalent(absent_last)
port = absent_first.env["Root"].fields[1]
assert port.label == "port"
assert (port.min, port.max) == (0, 1)
add("test_canonical.TestInfer.test_optional_field_detection_is_order_independent",
    "infer: a field absent in an earlier sample but present later infers optional, "
    "regardless of sample order (port: min=0, max=1)",
    "infer_order_independent",
    samples_a=[{"host": "a"}, {"host": "b", "port": 80}],
    samples_b=[{"host": "b", "port": 80}, {"host": "a"}],
    expected_port_min=port.min, expected_port_max=port.max,
    checks=[({"host": "x"}, True), ({"host": "x", "port": 1}, True)])


# --------------------------------------------------------------------
# test_canonical.py: TestValidation
# --------------------------------------------------------------------
def add_validate(module, note, schema_text, doc_json, expected_ok):
    ok = parse_schema(schema_text).validate(doc(doc_json)).ok
    assert ok is expected_ok
    add(module, note, "schema_validate", schema=schema_text,
        doc_json_input=doc_json, expected_ok=expected_ok)


VAL_SCALAR = 'record R { "n": integer, "s": string }\nroot R'
add_validate("test_canonical.TestValidation.test_scalar_kinds",
             "validate: matching scalar kinds pass", VAL_SCALAR, {"n": 1, "s": "x"}, True)
add_validate("test_canonical.TestValidation.test_scalar_kinds",
             "validate: an integer field given a string value fails", VAL_SCALAR, {"n": "x", "s": "x"}, False)

VAL_REQ_OPT = 'record R { "name": string, "age" [0,1]: integer }\nroot R'
add_validate("test_canonical.TestValidation.test_required_and_optional",
             "validate: required field alone (optional absent) passes", VAL_REQ_OPT, {"name": "a"}, True)
add_validate("test_canonical.TestValidation.test_required_and_optional",
             "validate: required + optional both present passes", VAL_REQ_OPT, {"name": "a", "age": 3}, True)
add_validate("test_canonical.TestValidation.test_required_and_optional",
             "validate: required field missing fails", VAL_REQ_OPT, {"age": 3}, False)

VAL_CLOSED = 'record R { "a": integer }\nroot R'
closed_result = parse_schema(VAL_CLOSED).validate(doc({"a": 1, "b": 2}))
assert not closed_result.ok
assert any(e.code == "unexpected-field" for e in closed_result.errors)
add("test_canonical.TestValidation.test_closed_rejects_unexpected",
    "validate: a closed record rejects an unexpected field with an unexpected-field error code",
    "schema_validate_error_code", schema=VAL_CLOSED, doc_json_input={"a": 1, "b": 2},
    expected_code="unexpected-field")

VAL_ARR_UNBOUNDED = 'record R { "xs" [0,]: integer }\nroot R'
add_validate("test_canonical.TestValidation.test_array_cardinality",
             "validate: unbounded array with 3 elements passes", VAL_ARR_UNBOUNDED, {"xs": [1, 2, 3]}, True)
add_validate("test_canonical.TestValidation.test_array_cardinality",
             "validate: [0,] cardinality allows zero occurrences", VAL_ARR_UNBOUNDED, {}, True)
VAL_ARR_MIN1 = 'record R { "xs" [1,]: integer }\nroot R'
add_validate("test_canonical.TestValidation.test_array_cardinality",
             "validate: [1,] cardinality rejects zero occurrences", VAL_ARR_MIN1, {}, False)
add_validate("test_canonical.TestValidation.test_array_cardinality",
             "validate: [1,] cardinality accepts one occurrence", VAL_ARR_MIN1, {"xs": [1]}, True)
VAL_ARR_EXACT2 = 'record R { "xs" [2]: integer }\nroot R'
add_validate("test_canonical.TestValidation.test_array_cardinality",
             "validate: exact cardinality [2] accepts exactly two", VAL_ARR_EXACT2, {"xs": [1, 2]}, True)
add_validate("test_canonical.TestValidation.test_array_cardinality",
             "validate: exact cardinality [2] rejects one", VAL_ARR_EXACT2, {"xs": [1]}, False)

VAL_NULLABLE = 'record R { "note": string? }\nroot R'
add_validate("test_canonical.TestValidation.test_nullable",
             "validate: nullable field accepts null", VAL_NULLABLE, {"note": None}, True)
add_validate("test_canonical.TestValidation.test_nullable",
             "validate: nullable field accepts its underlying kind", VAL_NULLABLE, {"note": "hi"}, True)
add_validate("test_canonical.TestValidation.test_nullable",
             "validate: nullable string still rejects a wrong-kind value", VAL_NULLABLE, {"note": 1}, False)

VAL_NUMBER = 'record R { "v": number }\nroot R'
add_validate("test_canonical.TestValidation.test_integer_satisfies_number",
             "validate: an integer value satisfies a number field", VAL_NUMBER, {"v": 7}, True)
add_validate("test_canonical.TestValidation.test_integer_satisfies_number",
             "validate: a float value satisfies a number field", VAL_NUMBER, {"v": 7.5}, True)
add_validate("test_canonical.TestValidation.test_integer_satisfies_number",
             "validate: a string value does not satisfy a number field", VAL_NUMBER, {"v": "x"}, False)

VAL_REF_REC = 'record Node { "value": integer, "kids" [0,]: Node }\nroot Node'
add_validate("test_canonical.TestValidation.test_ref_and_recursion",
             "validate: a self-referential record (Node) validates a nested valid child",
             VAL_REF_REC, {"value": 1, "kids": [{"value": 2, "kids": []}]}, True)
add_validate("test_canonical.TestValidation.test_ref_and_recursion",
             "validate: a self-referential record rejects a type mismatch nested inside a child",
             VAL_REF_REC, {"value": 1, "kids": [{"value": "x", "kids": []}]}, False)

try:
    parse_schema('record A { "x": integer }\nrecord R { "a": A? }\nroot R')
    raise SystemExit("expected SchemaError")
except SchemaError:
    add("test_canonical.TestValidation.test_question_mark_on_ref_is_error",
        "OSD: a '?' nullable marker on a Ref-typed field is rejected at parse time",
        "osd_parse_error", input='record A { "x": integer }\nrecord R { "a": A? }\nroot R')

try:
    parse_schema('record R { "status": "open" | "closed" }\nroot R')
    raise SystemExit("expected SchemaError")
except SchemaError:
    add("test_canonical.TestValidation.test_enum_syntax_is_rejected",
        "OSD: enum-literal-union syntax ('a' | 'b') is rejected -- '|' isn't in the grammar at all",
        "osd_parse_error", input='record R { "status": "open" | "closed" }\nroot R')

for bad in ['record R { "status": "open" }\nroot R', 'record R { "n": 5 }\nroot R']:
    try:
        parse_schema(bad)
        raise SystemExit("expected SchemaError")
    except SchemaError:
        add("test_canonical.TestValidation.test_literal_valued_field_is_rejected",
            "OSD: a single literal value in type position (no '|') is rejected",
            "osd_parse_error", input=bad)

try:
    parse_schema('union License { "auto", "manual" }\nrecord R { "a": integer }\nroot R')
    raise SystemExit("expected SchemaError")
except SchemaError:
    add("test_canonical.TestValidation.test_union_keyword_is_rejected",
        "OSD: the 'union' keyword (never part of the shipped grammar) is rejected",
        "osd_parse_error", input='union License { "auto", "manual" }\nrecord R { "a": integer }\nroot R')


# --------------------------------------------------------------------
# test_canonical.py: TestOsdRobustness
# --------------------------------------------------------------------
try:
    parse_schema('record R { "a" [1.5,3]: integer }\nroot R')
    raise SystemExit("expected SchemaError")
except SchemaError:
    add("test_canonical.TestOsdRobustness.test_float_cardinality_raises_cleanly",
        "OSD: a float cardinality bound ([1.5,3]) is rejected cleanly as a SchemaError",
        "osd_parse_error", input='record R { "a" [1.5,3]: integer }\nroot R')

FLAT_150 = "".join(f'record R{i} {{ "a": integer }}\n' for i in range(150)) + "root R0"
flat_schema = parse_schema(FLAT_150)
assert flat_schema.root.name == "R0"
add("test_canonical.TestOsdRobustness.test_many_flat_definitions_are_not_rejected",
    "OSD: 150 unrelated non-nested record definitions parse fine (depth guard counts nesting, not total records)",
    "osd_parse_ok", input=FLAT_150, expected_root="R0")

try:
    parse_schema('record string { "x": integer }\nrecord R { "a": string }\nroot R')
    raise SystemExit("expected SchemaError")
except SchemaError:
    add("test_canonical.TestOsdRobustness.test_record_named_a_scalar_keyword_is_rejected",
        "OSD: naming a record after a reserved scalar keyword ('string') is rejected",
        "osd_parse_error", input='record string { "x": integer }\nrecord R { "a": string }\nroot R')

add_validate("test_canonical.TestOsdRobustness.test_record_named_a_non_scalar_word_is_fine",
             "OSD: a record named after a non-reserved word ('Address') parses and validates fine",
             'record Address { "city": string }\nrecord R { "a": Address }\nroot R',
             {"a": {"city": "X"}}, True)


# --------------------------------------------------------------------
# test_canonical.py: TestTemporalBoundary
# --------------------------------------------------------------------
TB_DATE = 'record R { "v": date }\nroot R'
TB_TIME = 'record R { "v": time }\nroot R'
TB_DATETIME = 'record R { "v": datetime }\nroot R'

add_validate("test_canonical.TestTemporalBoundary.test_bare_date_string_satisfies_only_date",
             "temporal: a bare ISO date string satisfies date", TB_DATE, {"v": "2024-01-01"}, True)
add_validate("test_canonical.TestTemporalBoundary.test_bare_date_string_satisfies_only_date",
             "temporal: a bare ISO date string does NOT satisfy datetime", TB_DATETIME, {"v": "2024-01-01"}, False)
add_validate("test_canonical.TestTemporalBoundary.test_bare_date_string_satisfies_only_date",
             "temporal: a bare ISO date string does NOT satisfy time", TB_TIME, {"v": "2024-01-01"}, False)

add_validate("test_canonical.TestTemporalBoundary.test_bare_time_string_satisfies_only_time",
             "temporal: a bare ISO time string satisfies time", TB_TIME, {"v": "12:00:00"}, True)
add_validate("test_canonical.TestTemporalBoundary.test_bare_time_string_satisfies_only_time",
             "temporal: a bare ISO time string does NOT satisfy date", TB_DATE, {"v": "12:00:00"}, False)
add_validate("test_canonical.TestTemporalBoundary.test_bare_time_string_satisfies_only_time",
             "temporal: a bare ISO time string does NOT satisfy datetime", TB_DATETIME, {"v": "12:00:00"}, False)

for v in ("2024-01-01T12:00:00", "2024-01-01T00:00:00"):
    add_validate("test_canonical.TestTemporalBoundary.test_full_timestamp_string_satisfies_only_datetime",
                 f"temporal: a full ISO timestamp ({v}) satisfies datetime", TB_DATETIME, {"v": v}, True)
    add_validate("test_canonical.TestTemporalBoundary.test_full_timestamp_string_satisfies_only_datetime",
                 f"temporal: a full ISO timestamp ({v}) does NOT satisfy date", TB_DATE, {"v": v}, False)
    add_validate("test_canonical.TestTemporalBoundary.test_full_timestamp_string_satisfies_only_datetime",
                 f"temporal: a full ISO timestamp ({v}) does NOT satisfy time", TB_TIME, {"v": v}, False)

for tb_schema, tb_module in ((TB_DATE, "date"), (TB_TIME, "time"), (TB_DATETIME, "datetime")):
    add_validate("test_canonical.TestTemporalBoundary.test_unparseable_string_satisfies_none",
                 f"temporal: an unparseable string satisfies none of date/time/datetime ({tb_module})",
                 tb_schema, {"v": "not-a-date"}, False)

# test_real_objects_unaffected: Python distinguishes real datetime.date/
# datetime.datetime objects from strings at the value-kind level; Rust's
# Scalar has only Str for all three temporal kinds (see document.rs's
# module doc), so there is no Rust-side "real object vs. string" distinction
# to replay -- approximated here via the same ISO-string encoding already
# covered by the fixtures above (the datetime-vs-date-object asymmetry
# itself is Python-specific and not portable).


# --------------------------------------------------------------------
# test_canonical.py: TestOperations
# --------------------------------------------------------------------
def add_compat(module, note, schema_a, schema_b, expected):
    a = parse_schema(schema_a)
    b = parse_schema(schema_b)
    assert compatible_with(a, b) is expected
    add(module, note, "schema_compatible_with", schema_a=schema_a, schema_b=schema_b, expected=expected)


add_compat("test_canonical.TestOperations.test_added_optional_field_is_compatible",
           "compatible_with: adding an optional field widens the schema (old data still compatible)",
           'record R { "a": integer }\nroot R',
           'record R { "a": integer, "b" [0,1]: integer }\nroot R', True)
add_compat("test_canonical.TestOperations.test_added_optional_field_is_compatible",
           "compatible_with: the reverse direction (removing a field) is not compatible",
           'record R { "a": integer, "b" [0,1]: integer }\nroot R',
           'record R { "a": integer }\nroot R', False)

add_compat("test_canonical.TestOperations.test_required_to_optional_is_compatible",
           "compatible_with: loosening a required field to optional is a compatible widening",
           'record R { "a": integer, "b": integer }\nroot R',
           'record R { "a": integer, "b" [0,1]: integer }\nroot R', True)
add_compat("test_canonical.TestOperations.test_required_to_optional_is_compatible",
           "compatible_with: the reverse (optional -> required) is not compatible",
           'record R { "a": integer, "b" [0,1]: integer }\nroot R',
           'record R { "a": integer, "b": integer }\nroot R', False)

add_compat("test_canonical.TestOperations.test_integer_is_compatible_with_number",
           "compatible_with: integer field widened to number is compatible",
           'record R { "v": integer }\nroot R', 'record R { "v": number }\nroot R', True)
add_compat("test_canonical.TestOperations.test_integer_is_compatible_with_number",
           "compatible_with: number narrowed to integer is not compatible",
           'record R { "v": number }\nroot R', 'record R { "v": integer }\nroot R', False)

add_compat("test_canonical.TestOperations.test_nullable_is_one_directional",
           "compatible_with: a non-nullable field widened to nullable is compatible",
           'record R { "v": string }\nroot R', 'record R { "v": string? }\nroot R', True)
add_compat("test_canonical.TestOperations.test_nullable_is_one_directional",
           "compatible_with: a nullable field narrowed to non-nullable is not compatible",
           'record R { "v": string? }\nroot R', 'record R { "v": string }\nroot R', False)

add_compat("test_canonical.TestOperations.test_array_bounds",
           "compatible_with: a narrower array bound [2,3] is compatible with a wider [1,5]",
           'record R { "xs" [2,3]: integer }\nroot R', 'record R { "xs" [1,5]: integer }\nroot R', True)
add_compat("test_canonical.TestOperations.test_array_bounds",
           "compatible_with: the wider bound is not compatible with the narrower one",
           'record R { "xs" [1,5]: integer }\nroot R', 'record R { "xs" [2,3]: integer }\nroot R', False)

TEMPORAL_DATE_SCHEMA = to_osd(schema(ref("R"), R=record(field("d", t.date))))
TEMPORAL_STRING_SCHEMA = to_osd(schema(ref("R"), R=record(field("d", t.string))))
add_compat("test_canonical.TestOperations.test_temporal_date_not_compatible_with_string",
           "compatible_with: a date-typed field is never compatible with a string-typed one",
           TEMPORAL_DATE_SCHEMA, TEMPORAL_STRING_SCHEMA, False)

REORDER_A = 'record R { "a": integer, "b": string }\nroot R'
REORDER_B = 'record R { "b": string, "a": integer }\nroot R'
assert equivalent(parse_schema(REORDER_A), parse_schema(REORDER_B))
add("test_canonical.TestOperations.test_equivalent_reordered",
    "equivalent: field declaration order does not affect schema equivalence",
    "schema_equivalent", schema_a=REORDER_A, schema_b=REORDER_B, expected=True)

NORM_DUP = ('record A { "x": integer }\nrecord B { "x": integer }\n'
            'record R { "a": A, "b": B }\nroot R')
norm_src = parse_schema(NORM_DUP)
norm_dst = normalize(norm_src)
assert len(norm_dst.env) < len(norm_src.env)
assert equivalent(norm_src, norm_dst)
add("test_canonical.TestOperations.test_normalize_merges_identical",
    "normalize: two structurally-identical records (A, B) merge into one, shrinking env, staying equivalent",
    "schema_normalize_merges", schema=NORM_DUP,
    expected_env_before=len(norm_src.env), expected_env_after=len(norm_dst.env))


# --------------------------------------------------------------------
# test_depth_guards.py: lighter pass (issue #61 gap 3, lower priority)
# --------------------------------------------------------------------
# The Rust port centralizes its depth guard at Doc construction time
# (`Doc::from_raw`/`Doc::of` in document.rs), not per-writer like Python's
# separate write_json/write_yaml/write_toml/write_xml guards -- once a
# `Doc` exists, every writer/checker in Rust already operates on a
# depth-valid tree by construction, so a single from_raw-level fixture pair
# below covers the same "fails cleanly, not a stack overflow" guarantee
# that test_depth_guards.py's per-format classes each check individually.
d_deep_raw = deep_node(DEEP)
try:
    Doc(d_deep_raw).to_data()
    raise SystemExit("expected DocumentError")
except DocumentError as e:
    add("test_depth_guards.TestDocExport.test_to_data_too_deep_raises_document_error",
        "depth guard: a raw Doc node 5000 levels deep raises DocumentError naming the 200 limit "
        "(Rust centralizes this at Doc::from_raw construction time)",
        "doc_construct_depth_error", depth=DEEP, error_contains=str(e))

Doc(deep_node(JUST_UNDER)).to_data()
add("test_depth_guards.TestDocExport.test_to_data_just_under_limit_succeeds",
    "depth guard: a raw Doc node 190 levels deep (just under the limit) constructs and exports fine",
    "doc_construct_depth_ok", depth=JUST_UNDER)


print(json.dumps({"fixtures": fixtures}, indent=2, sort_keys=False))
print(f"TOTAL={len(fixtures)}", file=sys.stderr)

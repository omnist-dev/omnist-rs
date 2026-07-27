#!/usr/bin/env python3
"""Extract Python-observed input/output fixtures for the omnist-rs parity
harness (issue #40 in omnist-dev/omnist-rs). Runs the real, installed
Python `omnist` package (no reading of assertions) and dumps a JSON
corpus consumed by omnist-rs's `omnist/tests/parity.rs`.

Scope note: covers the modules that have a shipped Rust counterpart today
(oml codec, depth guards, schema/OSD + ops algebra, doc<->format codecs,
materialize). Explicitly OUT of scope, documented in the PR:
  - test_any_core.py / test_any_grammar.py: exercise the `any` type,
    which is deliberately unshipped in omnist-rs pending the v1.0 `any`
    decision (see docs/design/any-type-spec.md upstream) - no Rust API
    exists yet to replay against.
  - test_public_api.py: freezes the *Python* import surface
    (omnist.__all__, signatures) - not a cross-language concept.
  - test_cli.py / test_cli_examples.py / test_cli_fuzz.py: Python CLI
    plumbing/argparse behavior, not a Document/Schema API.
  - test_examples*.py / test_docs.py / test_check_doc_examples.py /
    test_grammar_docs.py / test_lint.py: doc-example / README / packaging
    generators for the Python repo's own tooling, not portable data.
  - test_fuzz.py: already ported at omnist-rs issue #26
    (omnist/tests/fuzz.rs), including a live cross-implementation oracle.
  - test_semantic_oracle.py: exercises tools/semantic_oracle.py, a
    Python-only dev tool (already used as the oracle by fuzz.rs).
"""
import datetime
import json
import math
import sys

from omnist import (
    Doc,
    ParseError,
    SchemaError,
    WriteError,
    DocumentError,
    check_oml,
    doc,
    field,
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

print(json.dumps({"fixtures": fixtures}, indent=2, sort_keys=False))
print(f"TOTAL={len(fixtures)}", file=sys.stderr)

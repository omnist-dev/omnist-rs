"""Cross-implementation oracle helper (issue #32).

Invoked as a subprocess by `omnist/tests/fuzz.rs`'s
`cross_implementation_oracle_bounded_sample` test: reads two OSD schema
texts (the same text format both `omnist::osd` and `omnist.osd` parse --
see `omnist/src/osd.rs`'s doc comment) from the two file paths given as
argv[1]/argv[2], computes the same three schema-algebra queries the Rust
side computes for the identical parsed schemas, and prints the result as
one line of JSON so the Rust side can compare field-for-field.

Kept as a standalone script (not a pytest test) because it's driven from
the *other* repo's test suite, not this one's -- this file is data/tooling
for omnist-rs's CI, not part of omnist's own test suite.
"""

import json
import sys

from omnist import osd
from omnist.ops.minimize import normalize
from omnist.ops.prune import is_empty, prune
from omnist.ops.subschema import compatible_with


def main() -> None:
    a_path, b_path = sys.argv[1], sys.argv[2]
    with open(a_path, encoding="utf-8") as f:
        a = osd.parse_schema(f.read())
    with open(b_path, encoding="utf-8") as f:
        b = osd.parse_schema(f.read())

    result = {
        "compatible_a_b": compatible_with(a, b),
        "is_empty_a": is_empty(a),
        "normalize_prune_is_empty_a": is_empty(prune(normalize(a))),
    }
    print(json.dumps(result))


if __name__ == "__main__":
    main()

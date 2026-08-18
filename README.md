# omnist-rs

From-scratch Rust port of the [Omnist data-interchange specification](https://github.com/omnist-dev/omnist-spec).

Docs: [rs.omnist.dev](https://rs.omnist.dev) — includes generated [rustdoc API reference](https://rs.omnist.dev/api/omnist/index.html)

## Methodology

This repository follows a strict spec-first methodology. `vendor/omnist-spec` is pinned as a git submodule and serves as the primary normative contract.

## Sibling Ports

- **Specification**: [omnist-spec](https://github.com/omnist-dev/omnist-spec)
- **Python**: [omnist](https://github.com/omnist-dev/omnist)
- **TypeScript**: [omnist-ts](https://github.com/omnist-dev/omnist-ts)
- **Java**: [omnist-j](https://github.com/omnist-dev/omnist-j)
- **Go**: [omnist-go](https://github.com/omnist-dev/omnist-go)

## Documentation

- [rs.omnist.dev](https://rs.omnist.dev): mdBook user guide — quickstart, formats, CLI reference, conformance, limitations.
- [Rustdoc API reference](https://rs.omnist.dev/api/omnist/index.html): generated straight from source.

## Status

**`v0.1.3-alpha`.**

- **Conformance Harness**: Track 1 (CLI fixtures) **19 / 19 (100%) PASS**. Track 2 (JSON test vectors) **130 / 152 PASS, 0 real fails, 22 skips**.
- **Testing**: **936 tests passing**, 0 failures — unit tests plus `proptest`-based property fuzzing across every format reader.
- **Code Coverage**: **100% lines, 100% branches** (`cargo llvm-cov --workspace --fail-under-lines 100`, gated in CI).

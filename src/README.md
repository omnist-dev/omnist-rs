# Introduction

<img src="./assets/logo.svg" alt="omnist logo" width="120" style="display:block;margin:0 auto 1.5rem;">

**omnist** is one canonical data model for JSON, YAML, TOML, XML, and its
own native OML, plus a schema language (OSD) to validate and compare
shapes over that model. This book is the Rust port's documentation
(the crate is `omnist`, CLI is `omnist-cli`).

Start with the [Quickstart](./quickstart.md), then read the
[user guide](./guide.md) for the full model. The [CLI reference](./cli.md)
covers the `omnist` binary, and each entry under [Formats](./formats/index.md)
documents one codec's specific round-trip behavior.

This is the Rust port of [omnist](https://github.com/omnist-dev/omnist).
See [Python divergences](./python-divergences.md) for where the two
implementations differ, and [Limitations & stability](./limitations.md)
for the current alpha-status caveats.

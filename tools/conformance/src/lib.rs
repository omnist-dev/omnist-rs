//! omnist-rs's own conformance-test harness against omnist-spec (issue
//! #82): the referee and its self-test, plus Track 1
//! (`conformance/fixtures/`) and Track 2 (`test-suite/`) runners.
//!
//! Consumes the pinned `vendor/omnist-spec` git submodule (see
//! `docs/conformance.md` for the currently-pinned commit -- kept there,
//! not duplicated here, so it doesn't go stale in two places) directly
//! via `omnist` library calls -- never depends on Python's or
//! TypeScript's implementation, matching the design already proven twice
//! (Python's `omnist/tools/conformance/`, omnist-ts's
//! `tools/conformance/`).

pub mod referee;
mod version_check;

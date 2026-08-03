//! omnist-rs's own conformance-test harness against omnist-spec (issue
//! #82). Step 1 delivers the referee and its self-test only; Track 1
//! (`conformance/fixtures/`) and Track 2 (`test-suite/`) runners are later
//! steps.
//!
//! Consumes the pinned `vendor/omnist-spec` git submodule (tag
//! `v0.1.0-alpha`, commit `b5232e9edb8f40119d0514c7a5a7fc0830be1bf3`)
//! directly via `omnist` library calls -- never depends on Python's or
//! TypeScript's implementation, matching the design already proven twice
//! (Python's `omnist/tools/conformance/`, omnist-ts's
//! `tools/conformance/`).

pub mod referee;

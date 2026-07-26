//! Runs every file under `examples/` and asserts its exact stdout, so the
//! doc-example CI gate's `verified-by` markers (see `docs/*.md`) can point
//! at a real assertion of literal output -- not just "it compiles". Each
//! function name matches the example file it covers.

use std::process::Command;

/// Runs `cargo run --quiet --example <name>` from this crate's manifest
/// directory (so there is no ambiguity with the sibling `omnist-cli`
/// package) and returns its stdout as a `String`.
fn run_example(name: &str) -> String {
    let output = Command::new(env!("CARGO"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["run", "--quiet", "--example", name])
        .output()
        .expect("cargo run --example should spawn");
    assert!(
        output.status.success(),
        "example {name} exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("example stdout is utf-8")
}

#[test]
fn json_roundtrip() {
    let out = run_example("json_roundtrip");
    assert_eq!(
        out,
        "{\n  \"name\": \"Ada\",\n  \"age\": 37\n}\nround trip ok\n"
    );
}

#[test]
fn oml_roundtrip() {
    let out = run_example("oml_roundtrip");
    assert_eq!(out, "name: \"Ada\"\nage: 37\nround trip ok\n");
}

#[test]
fn toml_roundtrip() {
    let out = run_example("toml_roundtrip");
    assert_eq!(out, "name = \"Ada\"\nage = 37\n\nround trip ok\n");
}

#[test]
fn xml_roundtrip() {
    let out = run_example("xml_roundtrip");
    assert_eq!(
        out,
        "<person>\n  <name>Ada</name>\n  <age>37</age>\n</person>\n\nround trip ok\n"
    );
}

#[test]
fn yaml_roundtrip() {
    let out = run_example("yaml_roundtrip");
    assert_eq!(out, "name: Ada\nage: 37\nround trip ok\n");
}

#[test]
fn schema_validate() {
    let out = run_example("schema_validate");
    assert_eq!(
        out,
        "valid\ninvalid:\n  at $.age: expected integer, got string (\"not a number\")\n"
    );
}

#[test]
fn schema_infer() {
    let out = run_example("schema_infer");
    assert_eq!(
        out,
        "record Person {\n  \"name\": string,\n  \"age\": integer,\n  \"tags\" [0,]: string,\n}\nroot Person\n\nall samples accepted\n"
    );
}

#[test]
fn schema_algebra() {
    let out = run_example("schema_algebra");
    assert_eq!(out, "record Root {\n  \"name\": string,\n}\nroot Root\n\n");
}

#[test]
fn format_registry() {
    let out = run_example("format_registry");
    assert_eq!(out, "json, kv, oml, toml, xml, yaml\n");
}

//! Integration tests for the `omnist` CLI (issue #24). Spawns the real
//! compiled binary via `Command::new(env!("CARGO_BIN_EXE_omnist"))`, per
//! the pattern #2/PR #3 established with `prints_version_line` -- this is
//! also how `main.rs`/`lib.rs`'s glue code gets exercised for coverage
//! purposes, not just each command's logic.

use std::io::Write as _;
use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_omnist"))
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(args: &[&str]) -> Run {
    run_stdin(args, None)
}

fn run_stdin(args: &[&str], stdin: Option<&str>) -> Run {
    let mut cmd = bin();
    cmd.args(args);
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn omnist binary");
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().expect("failed to wait on child");
    Run {
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
        code: output.status.code().unwrap(),
    }
}

/// A fresh scratch file under the test binary's own temp dir, so parallel
/// tests never collide, holding `content`.
fn fixture(name: &str, content: &str) -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "omnist-cli-test-{}-{}-{}",
        std::process::id(),
        name,
        n
    ));
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

const DOC_JSON: &str = r#"{"a": 1, "b": "x"}"#;
const SCHEMA_OSD: &str = r#"record Root { "a": integer, "b": string } root Root"#;

// --------------------------------------------------------------------- version

#[test]
fn prints_version_with_version_flag() {
    let r = run(&["--version"]);
    assert!(r.code == 0);
    assert!(r.stdout.trim().starts_with("omnist "));
}

#[test]
fn no_subcommand_is_a_usage_error() {
    let r = run(&[]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Usage"));
}

// --------------------------------------------------------------------- format

#[test]
fn format_golden_path_pretty_prints_oml() {
    let input = fixture("format_in", "a: 1; b: \"x\"\n");
    let r = run(&["format", &input]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "a: 1\nb: \"x\"\n");
}

#[test]
fn format_compact_single_lines() {
    let input = fixture("format_compact_in", "a: 1\nb: \"x\"\n");
    let r = run(&["format", &input, "--compact"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout, "a: 1; b: \"x\"\n");
}

#[test]
fn format_stdin_and_stdout_dash_plumbing() {
    let r = run_stdin(&["format", "-"], Some("a: 1\n"));
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout, "a: 1\n");
}

#[test]
fn format_reports_a_parse_error_with_exit_2() {
    let input = fixture("format_bad", "not valid oml {{{");
    let r = run(&["format", &input]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.starts_with("error: "), "stderr: {}", r.stderr);
}

#[test]
fn format_arrays_is_a_clear_not_yet_supported_error() {
    let input = fixture("format_arrays", "a: 1\n");
    let r = run(&["format", &input, "--arrays"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("not yet supported"));
}

#[test]
fn format_json_error_shape() {
    let input = fixture("format_bad_json", "not valid oml {{{");
    let r = run(&["format", &input, "--json"]);
    assert_eq!(r.code, 2);
    assert!(r.stdout.contains("\"ok\": false"));
    assert!(r.stdout.contains("\"errors\": []"));
    assert_eq!(r.stderr, "");
}

#[test]
fn format_writes_to_output_file() {
    let input = fixture("format_o_in", "a: 1\n");
    let mut out = std::env::temp_dir();
    out.push(format!("omnist-cli-test-format-out-{}", std::process::id()));
    let out_str = out.to_string_lossy().into_owned();
    let r = run(&["format", &input, "-o", &out_str]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout, "");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "a: 1\n");
}

// --------------------------------------------------------------------- convert

#[test]
fn convert_golden_path_json_to_oml() {
    let input = fixture("convert_in", DOC_JSON);
    let r = run(&["convert", &input, "--from", "json", "--to", "oml"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "a: 1\nb: \"x\"\n");
}

#[test]
fn convert_oml_to_oml_is_rejected() {
    let input = fixture("convert_oml_oml", "a: 1\n");
    let r = run(&["convert", &input, "--from", "oml", "--to", "oml"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("use `omnist format` instead"));
}

#[test]
fn convert_schema_directed_materialization() {
    let input = fixture("convert_schema_in", r#"{"a": 1.0, "b": "x"}"#);
    let schema = fixture("convert_schema", SCHEMA_OSD);
    let r = run(&[
        "convert", &input, "--from", "json", "--to", "json", "--schema", &schema,
    ]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("\"a\": 1"));
}

#[test]
fn convert_schema_conformance_failure_exits_2() {
    let input = fixture("convert_schema_bad", r#"{"a": "nope", "b": "x"}"#);
    let schema = fixture("convert_schema_bad_osd", SCHEMA_OSD);
    let r = run(&[
        "convert", &input, "--from", "json", "--to", "json", "--schema", &schema,
    ]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.starts_with("error: "));
}

#[test]
fn convert_strict_refuses_a_lossy_write_with_exit_1() {
    let input = fixture("convert_strict_in", r#"{"a": NaN}"#);
    let r = run(&[
        "convert", &input, "--from", "json", "--to", "json", "--strict",
    ]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.starts_with("error: "));
}

#[test]
fn convert_report_prints_adjustments_to_stderr_and_still_writes() {
    let input = fixture("convert_report_in", r#"{"a": NaN}"#);
    let r = run(&[
        "convert", &input, "--from", "json", "--to", "json", "--report",
    ]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("null"));
    assert!(!r.stderr.is_empty());
}

#[test]
fn convert_result_format_json_encodes_the_report() {
    let input = fixture("convert_report_json_in", r#"{"a": NaN}"#);
    let r = run(&[
        "convert",
        &input,
        "--from",
        "json",
        "--to",
        "json",
        "--report",
        "--result-format",
        "json",
    ]);
    assert_eq!(r.code, 0);
    assert!(r.stderr.trim_end().starts_with('['));
}

#[test]
fn convert_arrays_on_oml_output_is_a_clear_not_yet_supported_error() {
    let input = fixture("convert_arrays_in", DOC_JSON);
    let r = run(&[
        "convert", &input, "--from", "json", "--to", "oml", "--arrays",
    ]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("not yet supported"));
}

#[test]
fn convert_stdin_stdout_dash_plumbing() {
    let r = run_stdin(
        &["convert", "-", "--from", "json", "--to", "oml"],
        Some(DOC_JSON),
    );
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout, "a: 1\nb: \"x\"\n");
}

#[test]
fn convert_missing_input_file_exits_2() {
    let r = run(&[
        "convert",
        "/no/such/file.json",
        "--from",
        "json",
        "--to",
        "oml",
    ]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.starts_with("error: "));
}

// --------------------------------------------------------------------- check

#[test]
fn check_golden_path_never_writes_default_exit_0() {
    let input = fixture("check_in", DOC_JSON);
    let r = run(&["check", &input, "--from", "json", "--to", "toml"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "no adjustments");
}

#[test]
fn check_strict_exits_1_when_something_would_be_adjusted() {
    let input = fixture("check_strict_in", r#"{"a": NaN}"#);
    let r = run(&[
        "check", &input, "--from", "json", "--to", "json", "--strict",
    ]);
    assert_eq!(r.code, 1);
}

#[test]
fn check_json_result_format() {
    let input = fixture("check_json_in", r#"{"a": NaN}"#);
    let r = run(&[
        "check",
        &input,
        "--from",
        "json",
        "--to",
        "json",
        "--result-format",
        "json",
    ]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.trim_end().starts_with('['));
}

// --------------------------------------------------------------------- validate

#[test]
fn validate_golden_path_is_valid_exit_0() {
    let input = fixture("validate_in", DOC_JSON);
    let schema = fixture("validate_schema", SCHEMA_OSD);
    let r = run(&["validate", &input, "--from", "json", "--schema", &schema]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "valid");
}

#[test]
fn validate_conformance_failure_exit_1() {
    let input = fixture("validate_bad_in", r#"{"a": "nope", "b": "x"}"#);
    let schema = fixture("validate_bad_schema", SCHEMA_OSD);
    let r = run(&["validate", &input, "--from", "json", "--schema", &schema]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.contains("invalid"));
}

#[test]
fn validate_json_success_shape() {
    let input = fixture("validate_json_ok_in", DOC_JSON);
    let schema = fixture("validate_json_ok_schema", SCHEMA_OSD);
    let r = run(&[
        "validate", &input, "--from", "json", "--schema", &schema, "--json",
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "{\"ok\": true}");
}

#[test]
fn validate_json_failure_shape_has_structured_errors() {
    let input = fixture("validate_json_bad_in", r#"{"a": "nope", "b": "x"}"#);
    let schema = fixture("validate_json_bad_schema", SCHEMA_OSD);
    let r = run(&[
        "validate", &input, "--from", "json", "--schema", &schema, "--json",
    ]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.contains("\"code\": \"type-mismatch\""));
}

#[test]
fn validate_result_format_oml() {
    let input = fixture("validate_oml_in", DOC_JSON);
    let schema = fixture("validate_oml_schema", SCHEMA_OSD);
    let r = run(&[
        "validate",
        &input,
        "--from",
        "json",
        "--schema",
        &schema,
        "--result-format",
        "oml",
    ]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("ok: true"));
}

// --------------------------------------------------------------------- infer

#[test]
fn infer_golden_path_emits_osd() {
    let input = fixture("infer_in", DOC_JSON);
    let r = run(&["infer", &input, "--from", "json"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("root Root"));
    assert!(r.stdout.contains("\"a\": integer"));
}

#[test]
fn infer_compact_single_line() {
    let input = fixture("infer_compact_in", DOC_JSON);
    let r = run(&["infer", &input, "--from", "json", "--compact"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.lines().count(), 1);
}

#[test]
fn infer_arrays_flag_is_rejected() {
    let input = fixture("infer_arrays_in", DOC_JSON);
    let r = run(&["infer", &input, "--from", "json", "--arrays"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("applies only to OML output"));
}

#[test]
fn infer_from_a_scalar_root_is_a_schema_error() {
    let input = fixture("infer_scalar_root", "42");
    let r = run(&["infer", &input, "--from", "json"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.starts_with("error: "));
}

// --------------------------------------------------------------------- schema *

#[test]
fn schema_format_golden_path() {
    let schema = fixture("schema_format_in", SCHEMA_OSD);
    let r = run(&["schema", "format", &schema]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("root Root"));
}

#[test]
fn schema_format_arrays_flag_is_rejected() {
    let schema = fixture("schema_format_arrays_in", SCHEMA_OSD);
    let r = run(&["schema", "format", &schema, "--arrays"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("applies only to OML output"));
}

#[test]
fn schema_format_parse_error_exits_2() {
    let schema = fixture("schema_format_bad", "not an osd schema {{{");
    let r = run(&["schema", "format", &schema]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_normalize_golden_path() {
    let schema = fixture(
        "schema_normalize_in",
        r#"record A { "x": string } record B { "x": string }
           record Root { "a": A, "b": B } root Root"#,
    );
    let r = run(&["schema", "normalize", &schema]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    // A and B are structurally identical -- normalize should collapse them
    // to a single record.
    assert_eq!(r.stdout.matches("record ").count(), 2);
}

#[test]
fn schema_prune_golden_path() {
    let schema = fixture(
        "schema_prune_in",
        r#"record Dead { "x": string } record Root { "a": string } root Root"#,
    );
    let r = run(&["schema", "prune", &schema]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(!r.stdout.contains("Dead"));
}

#[test]
fn schema_is_empty_false_exits_1_true_exits_0() {
    let schema = fixture("schema_is_empty_in", SCHEMA_OSD);
    let r = run(&["schema", "is-empty", &schema]);
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout.trim(), "false");

    let empty_schema = fixture(
        "schema_is_empty_empty_in",
        r#"record Root { "self": Root } root Root"#,
    );
    let r2 = run(&["schema", "is-empty", &empty_schema]);
    assert_eq!(r2.code, 0);
    assert_eq!(r2.stdout.trim(), "true");
}

#[test]
fn schema_extract_golden_path() {
    let schema = fixture("schema_extract_in", SCHEMA_OSD);
    let r = run(&["schema", "extract", &schema, "--keep", "a,b"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("root Root"));
}

#[test]
fn schema_extract_no_valid_subschema_exits_1() {
    let schema = fixture("schema_extract_bad_in", SCHEMA_OSD);
    let r = run(&["schema", "extract", &schema, "--keep", "a"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no valid subschema"));
}

#[test]
fn schema_lint_no_findings() {
    let schema = fixture("schema_lint_in", SCHEMA_OSD);
    let r = run(&["schema", "lint", &schema]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "no findings");
}

#[test]
fn schema_lint_warning_severity_exits_1() {
    let schema = fixture(
        "schema_lint_warn_in",
        r#"record Dead { "x": string } record Root { "a": string } root Root"#,
    );
    let r = run(&["schema", "lint", &schema]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.contains("warning"));
}

#[test]
fn schema_lint_json_shape() {
    let schema = fixture("schema_lint_json_in", SCHEMA_OSD);
    let r = run(&["schema", "lint", &schema, "--json"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("\"findings\""));
}

#[test]
fn schema_compatible_with_true_and_false() {
    let a = fixture("schema_compat_a", SCHEMA_OSD);
    let b = fixture(
        "schema_compat_b",
        r#"record Root { "a": integer, "b": string, "c" [0,1]: string } root Root"#,
    );
    let r = run(&["schema", "compatible-with", &a, &a]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "true");

    let r2 = run(&["schema", "compatible-with", &b, &a]);
    assert_eq!(r2.code, 1);
    assert_eq!(r2.stdout.trim(), "false");
}

#[test]
fn schema_equivalent_true_and_false() {
    let a = fixture("schema_equiv_a", SCHEMA_OSD);
    let b = fixture(
        "schema_equiv_b",
        r#"record Root { "a": integer, "b": string, "c" [0,1]: string } root Root"#,
    );
    let r = run(&["schema", "equivalent", &a, &a]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "true");

    let r2 = run(&["schema", "equivalent", &a, &b]);
    assert_eq!(r2.code, 1);
    assert_eq!(r2.stdout.trim(), "false");
}

#[test]
fn schema_pair_stdin_dash_plumbing() {
    let b = fixture("schema_pair_stdin_b", SCHEMA_OSD);
    let r = run_stdin(&["schema", "equivalent", "-", &b], Some(SCHEMA_OSD));
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "true");
}

// --------------------------------------------------------------------- every
// format arm (read/write/check), coverage-closing per the issue's 100%
// obligation -- `read_by_fmt`/`write_by_fmt`/`check_by_fmt` dispatch to a
// codec per `Fmt` variant, so each one needs at least one exercise.

#[test]
fn convert_round_trips_every_non_oml_format_pair() {
    // XML needs a single top-level document element, so this uses a
    // single-field document (unlike `DOC_JSON`'s two fields) to stay valid
    // across every pairing, including the `--to xml` one.
    const SINGLE_FIELD_JSON: &str = r#"{"a": 1}"#;
    for (from, to) in [
        ("json", "yaml"),
        ("yaml", "toml"),
        ("toml", "xml"),
        ("xml", "json"),
    ] {
        let input = fixture(&format!("roundtrip_{from}_{to}_in"), SINGLE_FIELD_JSON);
        // Seed via a json->from conversion so every `from` gets a
        // same-shaped valid document in its own syntax.
        let seeded = run(&["convert", &input, "--from", "json", "--to", from]);
        assert_eq!(seeded.code, 0, "seeding {from}: {}", seeded.stderr);
        let seeded_file = fixture(&format!("roundtrip_{from}_{to}_seeded"), &seeded.stdout);
        let r = run(&["convert", &seeded_file, "--from", from, "--to", to]);
        assert_eq!(r.code, 0, "{from}->{to}: {}", r.stderr);
        assert!(!r.stdout.is_empty());
    }
}

#[test]
fn check_every_non_oml_to_format() {
    for to in ["json", "yaml", "toml", "xml"] {
        let input = fixture(&format!("check_to_{to}_in"), DOC_JSON);
        let r = run(&["check", &input, "--from", "json", "--to", to]);
        assert_eq!(r.code, 0, "--to {to}: {}", r.stderr);
    }
}

#[test]
fn check_to_oml() {
    let input = fixture("check_to_oml_in", DOC_JSON);
    let r = run(&["check", &input, "--from", "json", "--to", "oml"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "no adjustments");
}

#[test]
fn convert_to_oml_compact() {
    let input = fixture("convert_to_oml_compact_in", DOC_JSON);
    let r = run(&[
        "convert",
        &input,
        "--from",
        "json",
        "--to",
        "oml",
        "--compact",
    ]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "a: 1; b: \"x\"\n");
}

#[test]
fn convert_report_with_a_warning_severity_adjustment_json_and_oml() {
    // TOML has no `null` -- writing one is a `Severity::Warning`
    // (`null.omitted`) adjustment, unlike JSON's NaN->null (`Error`).
    let input = fixture("convert_warning_report_in", r#"{"a": null}"#);
    let r_json = run(&[
        "convert",
        &input,
        "--from",
        "json",
        "--to",
        "toml",
        "--report",
        "--result-format",
        "json",
    ]);
    assert_eq!(r_json.code, 0, "stderr: {}", r_json.stderr);
    assert!(r_json.stderr.contains("\"severity\": \"warning\""));

    let r_oml = run(&[
        "convert",
        &input,
        "--from",
        "json",
        "--to",
        "toml",
        "--report",
        "--result-format",
        "oml",
    ]);
    assert_eq!(r_oml.code, 0, "stderr: {}", r_oml.stderr);
    assert!(r_oml.stderr.contains("severity: \"warning\""));
}

#[test]
fn check_result_format_oml_with_a_warning_adjustment() {
    let input = fixture("check_warning_oml_in", r#"{"a": null}"#);
    let r = run(&[
        "check",
        &input,
        "--from",
        "json",
        "--to",
        "toml",
        "--result-format",
        "oml",
    ]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("severity: \"warning\""));
}

// --------------------------------------------------------------------- I/O
// failure paths -- missing input file / unwritable output path -- for
// every command that reads/writes via `read_input`/`write_output`.

#[test]
fn format_missing_input_file_exits_2() {
    let r = run(&["format", "/no/such/file.oml"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.starts_with("error: "));
}

#[test]
fn format_write_output_failure_exits_2() {
    let input = fixture("format_write_fail_in", "a: 1\n");
    let r = run(&["format", &input, "-o", "/no/such/dir/out.oml"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.starts_with("error: "));
}

#[test]
fn convert_write_output_failure_exits_2() {
    let input = fixture("convert_write_fail_in", DOC_JSON);
    let r = run(&[
        "convert",
        &input,
        "--from",
        "json",
        "--to",
        "oml",
        "-o",
        "/no/such/dir/out.oml",
    ]);
    assert_eq!(r.code, 2);
}

#[test]
fn check_missing_input_file_exits_2() {
    let r = run(&[
        "check",
        "/no/such/file.json",
        "--from",
        "json",
        "--to",
        "toml",
    ]);
    assert_eq!(r.code, 2);
}

#[test]
fn check_read_by_fmt_parse_error_exits_2() {
    let input = fixture("check_bad_json_in", "not json {{{");
    let r = run(&["check", &input, "--from", "json", "--to", "toml"]);
    assert_eq!(r.code, 2);
}

#[test]
fn validate_missing_input_file_exits_2() {
    let schema = fixture("validate_missing_schema", SCHEMA_OSD);
    let r = run(&[
        "validate",
        "/no/such/file.json",
        "--from",
        "json",
        "--schema",
        &schema,
    ]);
    assert_eq!(r.code, 2);
}

#[test]
fn validate_read_by_fmt_parse_error_exits_2() {
    let input = fixture("validate_bad_json_in", "not json {{{");
    let schema = fixture("validate_bad_json_schema", SCHEMA_OSD);
    let r = run(&["validate", &input, "--from", "json", "--schema", &schema]);
    assert_eq!(r.code, 2);
}

#[test]
fn validate_result_format_json_non_flag_variant() {
    let input = fixture("validate_rf_json_in", DOC_JSON);
    let schema = fixture("validate_rf_json_schema", SCHEMA_OSD);
    let r = run(&[
        "validate",
        &input,
        "--from",
        "json",
        "--schema",
        &schema,
        "--result-format",
        "json",
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "{\"ok\": true, \"errors\": []}");
}

#[test]
fn validate_result_format_oml_with_errors_exercises_the_error_list() {
    let input = fixture("validate_rf_oml_err_in", r#"{"a": "nope", "b": "x"}"#);
    let schema = fixture("validate_rf_oml_err_schema", SCHEMA_OSD);
    let r = run(&[
        "validate",
        &input,
        "--from",
        "json",
        "--schema",
        &schema,
        "--result-format",
        "oml",
    ]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.contains("path: \"$.a\""));
}

#[test]
fn infer_missing_input_file_exits_2() {
    let r = run(&["infer", "/no/such/file.json", "--from", "json"]);
    assert_eq!(r.code, 2);
}

#[test]
fn infer_read_by_fmt_parse_error_exits_2() {
    let input = fixture("infer_bad_json_in", "not json {{{");
    let r = run(&["infer", &input, "--from", "json"]);
    assert_eq!(r.code, 2);
}

#[test]
fn infer_write_output_failure_exits_2() {
    let input = fixture("infer_write_fail_in", DOC_JSON);
    let r = run(&[
        "infer",
        &input,
        "--from",
        "json",
        "-o",
        "/no/such/dir/out.osd",
    ]);
    assert_eq!(r.code, 2);
}

// --------------------------------------------------------------------- schema
// commands: missing/malformed schema file, write-output failure, second
// (`b`) file variants, for every `schema *` subcommand.

#[test]
fn schema_format_missing_file_exits_2() {
    let r = run(&["schema", "format", "/no/such/schema.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_format_write_output_failure_exits_2() {
    let schema = fixture("schema_format_write_fail_in", SCHEMA_OSD);
    let r = run(&["schema", "format", &schema, "-o", "/no/such/dir/out.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_normalize_missing_file_exits_2() {
    let r = run(&["schema", "normalize", "/no/such/schema.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_normalize_arrays_flag_is_rejected() {
    let schema = fixture("schema_normalize_arrays_in", SCHEMA_OSD);
    let r = run(&["schema", "normalize", &schema, "--arrays"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("applies only to OML output"));
}

#[test]
fn schema_normalize_write_output_failure_exits_2() {
    let schema = fixture("schema_normalize_write_fail_in", SCHEMA_OSD);
    let r = run(&["schema", "normalize", &schema, "-o", "/no/such/dir/out.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_prune_missing_file_exits_2() {
    let r = run(&["schema", "prune", "/no/such/schema.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_prune_write_output_failure_exits_2() {
    let schema = fixture("schema_prune_write_fail_in", SCHEMA_OSD);
    let r = run(&["schema", "prune", &schema, "-o", "/no/such/dir/out.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_is_empty_missing_file_exits_2() {
    let r = run(&["schema", "is-empty", "/no/such/schema.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_is_empty_result_format_json() {
    let schema = fixture("schema_is_empty_rf_json_in", SCHEMA_OSD);
    let r = run(&["schema", "is-empty", &schema, "--result-format", "json"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout.trim(), "{\"empty\": false}");
}

#[test]
fn schema_extract_missing_file_exits_2() {
    let r = run(&["schema", "extract", "/no/such/schema.osd", "--keep", "a"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_extract_write_output_failure_exits_2() {
    let schema = fixture("schema_extract_write_fail_in", SCHEMA_OSD);
    let r = run(&[
        "schema",
        "extract",
        &schema,
        "--keep",
        "a,b",
        "-o",
        "/no/such/dir/out.osd",
    ]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_lint_missing_file_exits_2() {
    let r = run(&["schema", "lint", "/no/such/schema.osd"]);
    assert_eq!(r.code, 2);
}

#[test]
fn schema_lint_severity_warning_json_with_findings() {
    let schema = fixture(
        "schema_lint_severity_warning_json_in",
        r#"record Dead { "x": string } record Root { "a": string } root Root"#,
    );
    let r = run(&["schema", "lint", &schema, "--severity", "warning", "--json"]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.contains("unreachable-record"));
}

#[test]
fn schema_compatible_with_missing_a_and_b_files() {
    let b = fixture("schema_compat_missing_b_ok", SCHEMA_OSD);
    let r = run(&["schema", "compatible-with", "/no/such/a.osd", &b]);
    assert_eq!(r.code, 2);

    let a = fixture("schema_compat_missing_a_ok", SCHEMA_OSD);
    let r2 = run(&["schema", "compatible-with", &a, "/no/such/b.osd"]);
    assert_eq!(r2.code, 2);
}

#[test]
fn schema_compatible_with_result_format_json() {
    let schema = fixture("schema_compat_rf_json_in", SCHEMA_OSD);
    let r = run(&[
        "schema",
        "compatible-with",
        &schema,
        &schema,
        "--result-format",
        "json",
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "{\"compatible\": true}");
}

#[test]
fn schema_equivalent_missing_a_and_b_files() {
    let b = fixture("schema_equiv_missing_b_ok", SCHEMA_OSD);
    let r = run(&["schema", "equivalent", "/no/such/a.osd", &b]);
    assert_eq!(r.code, 2);

    let a = fixture("schema_equiv_missing_a_ok", SCHEMA_OSD);
    let r2 = run(&["schema", "equivalent", &a, "/no/such/b.osd"]);
    assert_eq!(r2.code, 2);
}

#[test]
fn schema_equivalent_result_format_json() {
    let schema = fixture("schema_equiv_rf_json_in", SCHEMA_OSD);
    let r = run(&[
        "schema",
        "equivalent",
        &schema,
        &schema,
        "--result-format",
        "json",
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "{\"equivalent\": true}");
}

#[test]
fn schema_is_empty_result_format_oml() {
    let schema = fixture("schema_is_empty_rf_oml_in", SCHEMA_OSD);
    let r = run(&["schema", "is-empty", &schema, "--result-format", "oml"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout.trim(), "empty: false");
}

#[test]
fn schema_compatible_with_result_format_oml() {
    let schema = fixture("schema_compat_rf_oml_in", SCHEMA_OSD);
    let r = run(&[
        "schema",
        "compatible-with",
        &schema,
        &schema,
        "--result-format",
        "oml",
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "compatible: true");
}

#[test]
fn schema_equivalent_result_format_oml() {
    let schema = fixture("schema_equiv_rf_oml_in", SCHEMA_OSD);
    let r = run(&[
        "schema",
        "equivalent",
        &schema,
        &schema,
        "--result-format",
        "oml",
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "equivalent: true");
}

// --------------------------------------------------------------------- a
// few more coverage-closing cases: `convert --from oml` to a non-oml
// target, a report encoding an `Error`-severity (not just `Warning`)
// adjustment via `--result-format oml`, bad-syntax (not just missing-file)
// input on `convert`, a bad `--schema` path on `convert`, a structural
// (non-strict, no-report) write failure, a bad `--schema` path on
// `validate`, and `infer --allow-any`.

#[test]
fn convert_from_oml_to_a_non_oml_format() {
    let input = fixture("convert_from_oml_in", "a: 1\n");
    let r = run(&["convert", &input, "--from", "oml", "--to", "json"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "{\"a\": 1}");
}

#[test]
fn convert_report_result_format_oml_with_an_error_severity_adjustment() {
    // JSON has no NaN -- writing one back to JSON is `Severity::Error`
    // (`nan.to_null`), unlike TOML's null-omission `Warning`.
    let input = fixture("convert_error_report_oml_in", r#"{"a": NaN}"#);
    let r = run(&[
        "convert",
        &input,
        "--from",
        "json",
        "--to",
        "json",
        "--report",
        "--result-format",
        "oml",
    ]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stderr.contains("severity: \"error\""));
}

#[test]
fn convert_bad_syntax_input_exits_2() {
    let input = fixture("convert_bad_syntax_in", "not json {{{");
    let r = run(&["convert", &input, "--from", "json", "--to", "oml"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.starts_with("error: "));
}

#[test]
fn convert_missing_schema_file_exits_2() {
    let input = fixture("convert_missing_schema_in", DOC_JSON);
    let r = run(&[
        "convert",
        &input,
        "--from",
        "json",
        "--to",
        "json",
        "--schema",
        "/no/such/schema.osd",
    ]);
    assert_eq!(r.code, 2);
}

#[test]
fn convert_structural_write_failure_exits_2_without_a_report() {
    // XML needs exactly one top-level document element -- a two-field
    // document has no valid single-root XML rendering, a structural
    // `WriteError` with no `report` attached (unlike a strict-mode
    // refusal), so it takes the generic exit-2 path, not exit 1.
    let input = fixture("convert_structural_fail_in", DOC_JSON);
    let r = run(&["convert", &input, "--from", "json", "--to", "xml"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("document element"));
}

#[test]
fn validate_missing_schema_file_exits_2() {
    let input = fixture("validate_missing_schema_in", DOC_JSON);
    let r = run(&[
        "validate",
        &input,
        "--from",
        "json",
        "--schema",
        "/no/such/schema.osd",
    ]);
    assert_eq!(r.code, 2);
}

#[test]
fn infer_allow_any_is_a_clear_not_supported_error() {
    let input = fixture("infer_allow_any_in", DOC_JSON);
    let r = run(&["infer", &input, "--from", "json", "--allow-any"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("--allow-any"));
}

// --------------------------------------------------------------------- the
// shared `--json` flag's success-result encoding: `check`/`schema
// is-empty`/`schema compatible-with`/`schema equivalent` each pick
// `ResultFormat::Json` when `--json` is passed (overriding
// `--result-format`, same as Python's `_cmd_*` `"json" if args.json else
// args.result_format` -- these are distinct from `--result-format json`,
// which exercises the `else` arm instead).

#[test]
fn check_json_flag_picks_json_result_encoding() {
    let input = fixture("check_json_flag_in", DOC_JSON);
    let r = run(&["check", &input, "--from", "json", "--to", "toml", "--json"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "[]");
}

#[test]
fn schema_is_empty_json_flag_picks_json_result_encoding() {
    let schema = fixture("schema_is_empty_json_flag_in", SCHEMA_OSD);
    let r = run(&["schema", "is-empty", &schema, "--json"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout.trim(), "{\"empty\": false}");
}

#[test]
fn schema_compatible_with_json_flag_picks_json_result_encoding() {
    let schema = fixture("schema_compat_json_flag_in", SCHEMA_OSD);
    let r = run(&["schema", "compatible-with", &schema, &schema, "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "{\"compatible\": true}");
}

#[test]
fn schema_equivalent_json_flag_picks_json_result_encoding() {
    let schema = fixture("schema_equiv_json_flag_in", SCHEMA_OSD);
    let r = run(&["schema", "equivalent", &schema, &schema, "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "{\"equivalent\": true}");
}

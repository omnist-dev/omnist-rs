//! The `omnist` command-line interface.
//!
//! Ported from `~/dev/omnist/omnist/cli.py` (issue #24) into a `clap`-based
//! command tree over the existing `omnist` library crate. Per issue #1's
//! "architecture freedom" policy, `clap`'s derive-based subcommand tree is
//! not required to mirror Python's `argparse` structure line-for-line --
//! only the resulting CLI surface (flags, subcommands, exit codes, output
//! shapes) has to match. See `docs/design/cli-spec.md` (in the Python
//! reference repo) for the authoritative command-surface spec this module
//! is checked against.
//!
//! ## Known scope gaps (library limitations, not CLI bugs)
//!
//! - **`--arrays`**: the Python reference's `write_oml` supports an
//!   `arrays=True` mode that collapses runs of same-label edges into
//!   `[...]` array syntax. This port's [`omnist::oml::write_oml`] has no
//!   `arrays` parameter yet (separate, not-yet-ported library work).
//!   Passing `--arrays` where it would apply to OML output (`format`,
//!   `convert --to oml`) is accepted by the argument parser (matching the
//!   Python surface) but reported as a clear, non-panicking "not supported
//!   yet" error (exit 2) rather than silently ignored or faked. Wherever
//!   `--arrays` has no effect per spec (OSD output, or a `--to` other than
//!   `oml`), it is accepted and silently ignored, matching Python exactly.
//!   (`infer`'s `--allow-any` used to be scoped out the same way -- it is
//!   now fully wired to [`omnist::infer_with_report`]'s `allow_any`, issue
//!   #29.)
//! - **schema-directed `--schema` on `convert`**: implemented via
//!   [`omnist::materialize::materialize`] on the raw node after a
//!   schema-less read, exactly mirroring Python's `_materialize(node,
//!   schema)` call inside each reader.

use std::io::{self, Read, Write};

use clap::{Parser, Subcommand, ValueEnum};
use omnist::document::{Doc, Value};
use omnist::schema::Schema;
use omnist::{OmnistError, WriteError, WriteReport};

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "omnist",
    version = omnist::VERSION,
    about = "One canonical data model for JSON, YAML, TOML, XML, and OML -- \
             read, validate, and write any of them."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Canonicalize an OML document (the only format with no other tool for this).
    Format(FormatArgs),
    /// Convert a document between formats (one in, one out).
    Convert(ConvertArgs),
    /// Report what writing as --to would adjust, without ever writing.
    Check(CheckArgs),
    /// Check a document against a schema (no schema-directed upgrading).
    Validate(ValidateArgs),
    /// Draft a schema from example documents (all the same format).
    Infer(InferArgs),
    /// Operate on a Schema (OSD).
    #[command(subcommand)]
    Schema(SchemaCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Fmt {
    Json,
    Yaml,
    Toml,
    Xml,
    Oml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Default)]
pub enum ResultFormat {
    #[default]
    Text,
    Json,
    Oml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Default)]
pub enum LintSeverity {
    #[default]
    Info,
    Warning,
}

#[derive(clap::Args, Debug)]
pub struct FormatArgs {
    /// OML file, or - for stdin
    pub input: String,
    #[arg(long)]
    pub compact: bool,
    #[arg(long)]
    pub arrays: bool,
    #[arg(short, long)]
    pub output: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct ConvertArgs {
    pub input: String,
    #[arg(long = "from", value_enum)]
    pub from: Fmt,
    #[arg(long = "to", value_enum)]
    pub to: Fmt,
    #[arg(long)]
    pub schema: Option<String>,
    #[arg(long)]
    pub strict: bool,
    #[arg(long)]
    pub report: bool,
    #[arg(long = "result-format", value_enum, default_value_t = ResultFormat::Text)]
    pub result_format: ResultFormat,
    #[arg(long)]
    pub compact: bool,
    #[arg(long)]
    pub arrays: bool,
    #[arg(short, long)]
    pub output: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct CheckArgs {
    pub input: String,
    #[arg(long = "from", value_enum)]
    pub from: Fmt,
    #[arg(long = "to", value_enum)]
    pub to: Fmt,
    #[arg(long)]
    pub strict: bool,
    #[arg(long = "result-format", value_enum, default_value_t = ResultFormat::Text)]
    pub result_format: ResultFormat,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct ValidateArgs {
    pub input: String,
    #[arg(long = "from", value_enum)]
    pub from: Fmt,
    #[arg(long)]
    pub schema: String,
    #[arg(long = "result-format", value_enum, default_value_t = ResultFormat::Text)]
    pub result_format: ResultFormat,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct InferArgs {
    #[arg(required = true)]
    pub input: Vec<String>,
    #[arg(long = "from", value_enum)]
    pub from: Fmt,
    #[arg(long)]
    pub compact: bool,
    #[arg(long)]
    pub arrays: bool,
    #[arg(long = "allow-any")]
    pub allow_any: bool,
    #[arg(short, long)]
    pub output: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum SchemaCommand {
    Format(SchemaFormatArgs),
    Normalize(SchemaFormatArgs),
    Prune(SchemaPruneArgs),
    #[command(name = "is-empty")]
    IsEmpty(SchemaResultArgs),
    Extract(SchemaExtractArgs),
    Lint(SchemaLintArgs),
    #[command(name = "compatible-with")]
    CompatibleWith(SchemaPairArgs),
    Equivalent(SchemaPairArgs),
}

#[derive(clap::Args, Debug)]
pub struct SchemaFormatArgs {
    pub schema_file: String,
    #[arg(long)]
    pub compact: bool,
    #[arg(long)]
    pub arrays: bool,
    #[arg(short, long)]
    pub output: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct SchemaPruneArgs {
    pub schema_file: String,
    #[arg(long)]
    pub compact: bool,
    #[arg(short, long)]
    pub output: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct SchemaResultArgs {
    pub schema_file: String,
    #[arg(long = "result-format", value_enum, default_value_t = ResultFormat::Text)]
    pub result_format: ResultFormat,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct SchemaExtractArgs {
    pub schema_file: String,
    #[arg(long)]
    pub keep: String,
    #[arg(long)]
    pub compact: bool,
    #[arg(short, long)]
    pub output: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct SchemaLintArgs {
    pub schema_file: String,
    #[arg(long, value_enum, default_value_t = LintSeverity::Info)]
    pub severity: LintSeverity,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct SchemaPairArgs {
    pub a: String,
    pub b: String,
    #[arg(long = "result-format", value_enum, default_value_t = ResultFormat::Text)]
    pub result_format: ResultFormat,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// I/O plumbing
// ---------------------------------------------------------------------------

/// Read `path` (or stdin for `-`). On failure, the message names the path
/// (mirroring Python's `OSError` string, which embeds the filename) rather
/// than a bare OS message with no context.
fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut s = String::new();
        io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("{e} (reading stdin)"))?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
    }
}

fn write_output(path: Option<&str>, mut text: String) -> Result<(), String> {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    match path {
        None | Some("-") => {
            print!("{text}");
            // Not a `Result`-returning error path: `io::stdout()` is
            // unconditionally line-buffered (`LineWriter`), regardless of
            // whether it's a tty, per `std::io::Stdout`'s own docs. `text`
            // always ends in `\n` (enforced above), so the `print!` call
            // just above has *already* pushed every byte through to the
            // OS-level write by the time it returns -- that's what
            // `LineWriter` does on seeing a trailing newline. By the time
            // control reaches here there is nothing left buffered for
            // `flush` to push, so it degenerates to `StdoutRaw::flush`,
            // which is a hard no-op (unbuffered raw fds have nothing to
            // flush). A real write failure (e.g. broken pipe, `ENOSPC` from
            // `/dev/full`) panics *inside* `print!` itself (`io::Write`'s
            // `print!`/`println!` macros `.unwrap()` internally, they don't
            // propagate `Result`) before this line is ever reached --
            // confirmed empirically: forcing broken-pipe and `/dev/full`
            // stdout in integration tests both panic at the `print!` call,
            // never here. So this call cannot observably fail; `.expect`
            // documents that invariant instead of carrying a dead
            // `Result`-returning branch.
            io::stdout().flush().expect(
                "stdout is line-buffered and `text` ends in '\\n', so `print!` already flushed; a real write failure would have already panicked inside `print!` itself",
            );
            Ok(())
        }
        Some(p) => std::fs::write(p, text).map_err(|e| format!("{p}: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Uniform error shape
// ---------------------------------------------------------------------------

/// One structured error entry, shared by every `--json` failure payload.
/// Mirrors Python's `ParseError.errors`: only schema-conformance failures
/// via [`omnist::materialize::materialize`] ever populate this --
/// format-syntax failures always report `[]`.
fn extract_errors(e: &OmnistError) -> Vec<(String, String, String)> {
    match e {
        OmnistError::Materialize(me) => me
            .errors()
            .iter()
            .map(|ve| {
                (
                    ve.path.clone(),
                    ve.code.as_str().to_string(),
                    ve.message.clone(),
                )
            })
            .collect(),
        _ => vec![],
    }
}

/// Serialize a [`serde_json::Value`] using Python `json.dumps`'s default
/// separators (`", "`/`": "`) rather than serde_json's default compact
/// (no-space) style, so `--json`/`--result-format json` output matches
/// Python's byte-for-byte (the CLI's own JSON payloads are always small,
/// hand-built shapes -- this is not a general-purpose formatter). Key
/// order is preserved by `serde_json`'s `preserve_order` feature, matching
/// the insertion order Python's dict literals give `json.dumps`.
fn py_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(m) => {
            let items: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{}: {}", serde_json::to_string(k).unwrap(), py_json(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        serde_json::Value::Array(a) => {
            let items: Vec<String> = a.iter().map(py_json).collect();
            format!("[{}]", items.join(", "))
        }
        other => serde_json::to_string(other).unwrap(),
    }
}

fn json_error_payload(message: &str, errors: &[(String, String, String)]) -> String {
    let errs: Vec<serde_json::Value> = errors
        .iter()
        .map(|(p, c, m)| serde_json::json!({"path": p, "code": c, "message": m}))
        .collect();
    py_json(&serde_json::json!({"ok": false, "message": message, "errors": errs}))
}

/// Uniform in-command error emission, mirroring Python's `_fail`. Under
/// `--json`, prints a machine-readable error object to stdout; otherwise
/// the free-text `error: ...` to stderr. Returns `code` unchanged either
/// way.
fn fail(json: bool, message: &str, errors: &[(String, String, String)], code: i32) -> i32 {
    if json {
        println!("{}", json_error_payload(message, errors));
    } else {
        eprintln!("error: {message}");
    }
    code
}

/// The generic uncaught-error path, mirroring Python `main()`'s top-level
/// exception handler: exit 2 always.
fn generic_fail(json: bool, e: &OmnistError) -> i32 {
    fail(json, &e.to_string(), &extract_errors(e), 2)
}

fn io_fail(json: bool, message: &str) -> i32 {
    fail(json, message, &[], 2)
}

// ---------------------------------------------------------------------------
// Format dispatch helpers
// ---------------------------------------------------------------------------

fn read_by_fmt(fmt: Fmt, text: &str) -> Result<Doc, OmnistError> {
    match fmt {
        Fmt::Json => omnist::formats::json::read_json(text),
        Fmt::Yaml => omnist::formats::yaml::read_yaml(text),
        Fmt::Toml => omnist::formats::toml::read_toml(text),
        Fmt::Xml => omnist::formats::xml::read_xml(text),
        Fmt::Oml => {
            let raw = omnist::oml::read_oml(text)?;
            Ok(Doc::from_raw(raw)?)
        }
    }
}

const ARRAYS_UNSUPPORTED_MSG: &str = "--arrays is not yet supported by this port's OML writer (a separate library gap, see \
     omnist-cli's crate doc comment)";

const ARRAYS_OSD_ONLY_MSG: &str = "--arrays applies only to OML output (format, convert --to oml)";

enum WriteOutcome {
    Text(String),
    ArraysUnsupported,
}

fn write_by_fmt(
    fmt: Fmt,
    doc: &Doc,
    strict: bool,
    report: Option<&mut WriteReport>,
    compact: bool,
    arrays: bool,
) -> Result<WriteOutcome, WriteError> {
    match fmt {
        // `--compact` applies only to OML output (per cli-spec.md §3's
        // `convert` entry: "no effect otherwise") -- JSON's own writer
        // default (`indent: None`, matching Python's `write_json`'s
        // `indent: Optional[int] = None` default) always applies here,
        // regardless of `--compact`.
        Fmt::Json => {
            omnist::formats::json::write_json(doc, None, strict, report).map(WriteOutcome::Text)
        }
        Fmt::Yaml => omnist::formats::yaml::write_yaml(doc, strict, report).map(WriteOutcome::Text),
        Fmt::Toml => omnist::formats::toml::write_toml(doc, strict, report).map(WriteOutcome::Text),
        Fmt::Xml => omnist::formats::xml::write_xml(doc, strict, report).map(WriteOutcome::Text),
        Fmt::Oml => {
            if arrays {
                return Ok(WriteOutcome::ArraysUnsupported);
            }
            let raw = doc.to_raw();
            if compact {
                omnist::oml::write_oml_compact(&raw).map(WriteOutcome::Text)
            } else {
                omnist::oml::write_oml(&raw, 2).map(WriteOutcome::Text)
            }
        }
    }
}

fn check_by_fmt(fmt: Fmt, doc: &Doc) -> Result<WriteReport, WriteError> {
    match fmt {
        Fmt::Json => Ok(omnist::formats::json::check_json(doc)),
        Fmt::Yaml => Ok(omnist::formats::yaml::check_yaml(doc)),
        Fmt::Toml => Ok(omnist::formats::toml::check_toml(doc)),
        Fmt::Xml => Ok(omnist::formats::xml::check_xml(doc)),
        Fmt::Oml => {
            // OML is always lossless; the only possible failure is the
            // depth guard, surfaced the same way a real write would be.
            omnist::oml::write_oml(&doc.to_raw(), 2)?;
            Ok(WriteReport::new())
        }
    }
}

fn encode_write_report(rep: &WriteReport, fmt: ResultFormat) -> String {
    match fmt {
        ResultFormat::Text => rep.to_string(),
        ResultFormat::Json => {
            let adjustments: Vec<serde_json::Value> = rep
                .iter()
                .map(|a| {
                    let sev = match a.severity {
                        omnist::Severity::Warning => "warning",
                        omnist::Severity::Error => "error",
                    };
                    serde_json::json!({
                        "path": a.path, "code": a.code, "message": a.message, "severity": sev
                    })
                })
                .collect();
            py_json(&serde_json::Value::Array(adjustments))
        }
        ResultFormat::Oml => {
            let adjustments: Vec<Value> = rep
                .iter()
                .map(|a| {
                    let sev = match a.severity {
                        omnist::Severity::Warning => "warning",
                        omnist::Severity::Error => "error",
                    };
                    Value::Object(indexmap::IndexMap::from([
                        ("path".to_string(), Value::Str(a.path.clone())),
                        ("code".to_string(), Value::Str(a.code.clone())),
                        ("message".to_string(), Value::Str(a.message.clone())),
                        ("severity".to_string(), Value::Str(sev.to_string())),
                    ]))
                })
                .collect();
            let payload = Value::Object(indexmap::IndexMap::from([(
                "adjustments".to_string(),
                Value::Array(adjustments),
            )]));
            let doc = Doc::of(&payload).expect("a report payload is always a legal Document");
            omnist::oml::write_oml(&doc.to_raw(), 2)
                .expect("report payloads never nest past MAX_DEPTH")
        }
    }
}

fn encode_bool_result(key: &str, value: bool, fmt: ResultFormat) -> String {
    match fmt {
        ResultFormat::Text => if value { "true" } else { "false" }.to_string(),
        ResultFormat::Json => py_json(&serde_json::json!({key: value})),
        ResultFormat::Oml => {
            let payload = Value::Object(indexmap::IndexMap::from([(
                key.to_string(),
                Value::Bool(value),
            )]));
            let doc = Doc::of(&payload).expect("a bool-result payload is always a legal Document");
            omnist::oml::write_oml(&doc.to_raw(), 2).expect("bool-result payloads never nest")
        }
    }
}

fn encode_validation_result(
    result: &omnist::schema::ValidationResult,
    fmt: ResultFormat,
) -> String {
    match fmt {
        ResultFormat::Text => result.to_string(),
        ResultFormat::Json => {
            let errors: Vec<serde_json::Value> = result
                .errors()
                .iter()
                .map(|e| serde_json::json!({"path": e.path, "message": e.message}))
                .collect();
            py_json(&serde_json::json!({"ok": result.ok(), "errors": errors}))
        }
        ResultFormat::Oml => {
            let errors: Vec<Value> = result
                .errors()
                .iter()
                .map(|e| {
                    Value::Object(indexmap::IndexMap::from([
                        ("path".to_string(), Value::Str(e.path.clone())),
                        ("message".to_string(), Value::Str(e.message.clone())),
                    ]))
                })
                .collect();
            let payload = Value::Object(indexmap::IndexMap::from([
                ("ok".to_string(), Value::Bool(result.ok())),
                ("errors".to_string(), Value::Array(errors)),
            ]));
            let doc = Doc::of(&payload).expect("a validation-result payload is always legal");
            omnist::oml::write_oml(&doc.to_raw(), 2).expect("validation-result payloads never nest")
        }
    }
}

fn to_osd_text(schema: &Schema, compact: bool) -> String {
    omnist::osd::to_osd(schema, if compact { None } else { Some(4) })
}

/// Read + parse an OSD schema file (or `-`), producing the uniform failure
/// exit code (`2`, matching Python's generic parse-error path) on either an
/// I/O error or a [`omnist::SchemaError`].
fn parse_schema_file(path: &str, json: bool) -> Result<Schema, i32> {
    let text = read_input(path).map_err(|e| io_fail(json, &e))?;
    omnist::osd::parse_schema(&text).map_err(|e| generic_fail(json, &e.into()))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_format(args: FormatArgs) -> i32 {
    let text = match read_input(&args.input) {
        Ok(t) => t,
        Err(e) => return io_fail(args.json, &e),
    };
    let raw = match omnist::oml::read_oml(&text) {
        Ok(r) => r,
        Err(e) => return generic_fail(args.json, &e.into()),
    };
    if args.arrays {
        return fail(args.json, ARRAYS_UNSUPPORTED_MSG, &[], 2);
    }
    // No `Doc::from_raw`/`.to_raw()` round-trip here: `format` reads and
    // re-writes OML only, so `raw` (already the exact shape `write_oml`
    // wants) goes straight back out. `read_oml` already enforces the same
    // `MAX_DEPTH` guard `write_oml` re-checks (see `oml.rs`'s module doc),
    // so re-serializing what was *just* read successfully can never itself
    // hit the depth guard -- confirmed the same way `infer.rs` confirmed
    // its own now-absent depth check was unreachable (see that module's
    // doc comment): there is no way to get a `raw` here that didn't just
    // pass this exact check a few lines up.
    let text_out = if args.compact {
        omnist::oml::write_oml_compact(&raw)
    } else {
        omnist::oml::write_oml(&raw, 2)
    }
    .expect("read_oml already enforced the depth guard write_oml re-checks");
    if let Err(e) = write_output(args.output.as_deref(), text_out) {
        return io_fail(args.json, &e);
    }
    0
}

fn cmd_convert(args: ConvertArgs) -> i32 {
    if args.from == Fmt::Oml && args.to == Fmt::Oml {
        return fail(
            args.json,
            "--from oml --to oml is not supported here; use `omnist format` instead",
            &[],
            2,
        );
    }
    let text = match read_input(&args.input) {
        Ok(t) => t,
        Err(e) => return io_fail(args.json, &e),
    };
    let mut doc = match read_by_fmt(args.from, &text) {
        Ok(d) => d,
        Err(e) => return generic_fail(args.json, &e),
    };
    if let Some(schema_path) = &args.schema {
        let schema = match parse_schema_file(schema_path, args.json) {
            Ok(s) => s,
            Err(code) => return code,
        };
        let materialized = match omnist::materialize::materialize(&doc.to_raw(), Some(&schema)) {
            Ok(r) => r,
            Err(e) => return generic_fail(args.json, &OmnistError::Materialize(e)),
        };
        // `materialize` only ever upgrades leaf scalars in place (see its
        // module doc's scalar-upgrade table) -- it never adds nesting, so
        // its output can't be any deeper than `doc.to_raw()` already was,
        // which itself already passed this exact depth guard when `doc`
        // was first built by `read_by_fmt` above. Not reachable via any
        // real input, same class as `cmd_format`'s now-absent equivalent.
        doc = Doc::from_raw(materialized)
            .expect("materialize cannot deepen a Document past what read_by_fmt already allowed");
    }
    let mut report = if args.report {
        Some(WriteReport::new())
    } else {
        None
    };
    let write_result = write_by_fmt(
        args.to,
        &doc,
        args.strict,
        report.as_mut(),
        args.compact,
        args.arrays,
    );
    let text_out = match write_result {
        Ok(WriteOutcome::Text(s)) => s,
        Ok(WriteOutcome::ArraysUnsupported) => {
            return fail(args.json, ARRAYS_UNSUPPORTED_MSG, &[], 2);
        }
        Err(e) => {
            if let Some(rep) = e.report() {
                // A definite "no" -- a strict-mode refusal, not a usage/parse
                // failure -- exit 1, matching Python's `_cmd_convert`.
                return fail(args.json, &rep.to_string(), &[], 1);
            }
            return generic_fail(args.json, &e.into());
        }
    };
    if let Err(e) = write_output(args.output.as_deref(), text_out) {
        return io_fail(args.json, &e);
    }
    if let Some(rep) = &report {
        eprintln!("{}", encode_write_report(rep, args.result_format));
    }
    0
}

fn cmd_check(args: CheckArgs) -> i32 {
    let text = match read_input(&args.input) {
        Ok(t) => t,
        Err(e) => return io_fail(args.json, &e),
    };
    let doc = match read_by_fmt(args.from, &text) {
        Ok(d) => d,
        Err(e) => return generic_fail(args.json, &e),
    };
    // `check_by_fmt`'s only failure mode (the `--to oml` arm) is the same
    // depth guard `doc` already passed when `read_by_fmt` built it above --
    // not reachable via any real input, same class as `cmd_format`'s and
    // `cmd_convert`'s equivalents.
    let rep = check_by_fmt(args.to, &doc)
        .expect("check_by_fmt's only failure mode is a depth guard read_by_fmt already enforced");
    let fmt = if args.json {
        ResultFormat::Json
    } else {
        args.result_format
    };
    println!("{}", encode_write_report(&rep, fmt));
    if args.strict && !rep.is_empty() { 1 } else { 0 }
}

fn cmd_validate(args: ValidateArgs) -> i32 {
    let text = match read_input(&args.input) {
        Ok(t) => t,
        Err(e) => return io_fail(args.json, &e),
    };
    let doc = match read_by_fmt(args.from, &text) {
        Ok(d) => d,
        Err(e) => return generic_fail(args.json, &e),
    };
    let schema = match parse_schema_file(&args.schema, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let result = schema.validate(&doc.root());
    if args.json {
        if result.ok() {
            println!("{}", py_json(&serde_json::json!({"ok": true})));
            return 0;
        }
        let errors: Vec<serde_json::Value> = result
            .errors()
            .iter()
            .map(|e| serde_json::json!({"path": e.path, "code": e.code.as_str(), "message": e.message}))
            .collect();
        println!(
            "{}",
            py_json(
                &serde_json::json!({"ok": false, "message": result.to_string(), "errors": errors})
            )
        );
        return 1;
    }
    println!("{}", encode_validation_result(&result, args.result_format));
    if result.ok() { 0 } else { 1 }
}

fn cmd_infer(args: InferArgs) -> i32 {
    if args.arrays {
        return fail(args.json, ARRAYS_OSD_ONLY_MSG, &[], 2);
    }
    let mut docs = Vec::with_capacity(args.input.len());
    for path in &args.input {
        let text = match read_input(path) {
            Ok(t) => t,
            Err(e) => return io_fail(args.json, &e),
        };
        match read_by_fmt(args.from, &text) {
            Ok(d) => docs.push(d),
            Err(e) => return generic_fail(args.json, &e),
        }
    }
    let (schema, fallbacks) = match omnist::infer_with_report(&docs, "Root", args.allow_any) {
        Ok(r) => r,
        Err(e) => return generic_fail(args.json, &e.into()),
    };
    for fb in &fallbacks {
        eprintln!("warning: {} opened as `any` ({})", fb.location, fb.reason);
    }
    let text_out = to_osd_text(&schema, args.compact);
    if let Err(e) = write_output(args.output.as_deref(), text_out) {
        return io_fail(args.json, &e);
    }
    0
}

fn cmd_schema_format(args: SchemaFormatArgs) -> i32 {
    if args.arrays {
        return fail(args.json, ARRAYS_OSD_ONLY_MSG, &[], 2);
    }
    let schema = match parse_schema_file(&args.schema_file, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let out = to_osd_text(&schema, args.compact);
    if let Err(e) = write_output(args.output.as_deref(), out) {
        return io_fail(args.json, &e);
    }
    0
}

fn cmd_schema_normalize(args: SchemaFormatArgs) -> i32 {
    if args.arrays {
        return fail(args.json, ARRAYS_OSD_ONLY_MSG, &[], 2);
    }
    let schema = match parse_schema_file(&args.schema_file, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let out = to_osd_text(&omnist::ops::normalize(&schema), args.compact);
    if let Err(e) = write_output(args.output.as_deref(), out) {
        return io_fail(args.json, &e);
    }
    0
}

fn cmd_schema_prune(args: SchemaPruneArgs) -> i32 {
    let schema = match parse_schema_file(&args.schema_file, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let out = to_osd_text(&omnist::ops::prune(&schema), args.compact);
    if let Err(e) = write_output(args.output.as_deref(), out) {
        return io_fail(args.json, &e);
    }
    0
}

fn cmd_schema_is_empty(args: SchemaResultArgs) -> i32 {
    let schema = match parse_schema_file(&args.schema_file, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let result = omnist::ops::is_empty(&schema);
    let fmt = if args.json {
        ResultFormat::Json
    } else {
        args.result_format
    };
    println!("{}", encode_bool_result("empty", result, fmt));
    if result { 0 } else { 1 }
}

fn cmd_schema_extract(args: SchemaExtractArgs) -> i32 {
    let schema = match parse_schema_file(&args.schema_file, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let labels: Vec<&str> = args.keep.split(',').filter(|s| !s.is_empty()).collect();
    let extracted = match omnist::ops::extract(&schema, &labels) {
        Ok(s) => s,
        Err(e) => {
            // A definite "no valid subschema" -- exit 1, like
            // `compatible-with`'s `false`, not the generic parse/usage 2.
            return fail(args.json, &e.to_string(), &[], 1);
        }
    };
    let out = to_osd_text(&extracted, args.compact);
    if let Err(e) = write_output(args.output.as_deref(), out) {
        return io_fail(args.json, &e);
    }
    0
}

fn cmd_schema_lint(args: SchemaLintArgs) -> i32 {
    let schema = match parse_schema_file(&args.schema_file, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let threshold = match args.severity {
        LintSeverity::Info => 0,
        LintSeverity::Warning => 1,
    };
    let sev_rank = |s: &str| if s == "warning" { 1 } else { 0 };
    let findings: Vec<_> = omnist::ops::lint(&schema)
        .into_iter()
        .filter(|f| sev_rank(f.severity) >= threshold)
        .collect();
    let has_warning = findings.iter().any(|f| f.severity == "warning");
    if args.json {
        let findings_json: Vec<serde_json::Value> = findings
            .iter()
            .map(|f| {
                serde_json::json!({
                    "code": f.code, "severity": f.severity,
                    "location": f.location, "message": f.message
                })
            })
            .collect();
        println!(
            "{}",
            py_json(&serde_json::json!({"ok": !has_warning, "findings": findings_json}))
        );
    } else if findings.is_empty() {
        println!("no findings");
    } else {
        for f in &findings {
            println!("{}: {}: {}: {}", f.severity, f.code, f.location, f.message);
        }
    }
    if has_warning { 1 } else { 0 }
}

fn cmd_schema_compatible_with(args: SchemaPairArgs) -> i32 {
    let a = match parse_schema_file(&args.a, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let b = match parse_schema_file(&args.b, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let result = omnist::ops::compatible_with(&a, &b);
    let fmt = if args.json {
        ResultFormat::Json
    } else {
        args.result_format
    };
    println!("{}", encode_bool_result("compatible", result, fmt));
    if result { 0 } else { 1 }
}

fn cmd_schema_equivalent(args: SchemaPairArgs) -> i32 {
    let a = match parse_schema_file(&args.a, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let b = match parse_schema_file(&args.b, args.json) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let result = omnist::ops::equivalent(&a, &b);
    let fmt = if args.json {
        ResultFormat::Json
    } else {
        args.result_format
    };
    println!("{}", encode_bool_result("equivalent", result, fmt));
    if result { 0 } else { 1 }
}

/// Run the CLI given already-parsed [`Cli`] arguments, returning the
/// process exit code. Split from `main` so integration tests exercising
/// the real compiled binary (per the issue's test-obligation) and any
/// future in-process test can share one entry point.
pub fn run(cli: Cli) -> i32 {
    match cli.command {
        Command::Format(a) => cmd_format(a),
        Command::Convert(a) => cmd_convert(a),
        Command::Check(a) => cmd_check(a),
        Command::Validate(a) => cmd_validate(a),
        Command::Infer(a) => cmd_infer(a),
        Command::Schema(sub) => match sub {
            SchemaCommand::Format(a) => cmd_schema_format(a),
            SchemaCommand::Normalize(a) => cmd_schema_normalize(a),
            SchemaCommand::Prune(a) => cmd_schema_prune(a),
            SchemaCommand::IsEmpty(a) => cmd_schema_is_empty(a),
            SchemaCommand::Extract(a) => cmd_schema_extract(a),
            SchemaCommand::Lint(a) => cmd_schema_lint(a),
            SchemaCommand::CompatibleWith(a) => cmd_schema_compatible_with(a),
            SchemaCommand::Equivalent(a) => cmd_schema_equivalent(a),
        },
    }
}

/// Kept for backward compatibility with issue #2/PR #3's placeholder --
/// no longer used by `main`, but harmless to keep as a small library
/// entry point (and its existing test).
pub fn version_line() -> String {
    format!("omnist {}", omnist::VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_includes_crate_version() {
        assert_eq!(version_line(), format!("omnist {}", omnist::VERSION));
    }
}

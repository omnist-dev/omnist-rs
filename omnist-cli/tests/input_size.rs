//! `--max-input-bytes` (spec D-23): every subcommand that reads a Document
//! refuses an input of more than N bytes with `document.limit.input-size` at
//! `$`, before decoding; reads stop at N + 1 bytes; an input of exactly N
//! bytes is accepted; the refusal says how to raise the limit.

use std::io::Write as _;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run_stdin_bytes(args: &[&str], stdin: &[u8]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_omnist"));
    cmd.args(args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn omnist binary");
    let mut pipe = child.stdin.take().unwrap();
    // The child may stop reading early (that is the point of the test): a
    // broken pipe on the write is expected, not a failure.
    let _ = pipe.write_all(stdin);
    drop(pipe);
    let output = child.wait_with_output().expect("failed to wait on child");
    Run {
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
        code: output.status.code().unwrap(),
    }
}

fn fixture(name: &str, content: &[u8]) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!("omnist-cli-size-{}-{}", std::process::id(), name));
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().to_string()
}

/// `doc` padded with trailing spaces to exactly `bytes` bytes.
fn padded(doc: &str, bytes: usize) -> Vec<u8> {
    format!("{doc}{}", " ".repeat(bytes - doc.len())).into_bytes()
}

/// (subcommand args that read one Document from `-`, a valid input for it).
fn reading_commands() -> Vec<(Vec<&'static str>, &'static str)> {
    vec![
        (vec!["format", "-"], "a: 1"),
        (
            vec!["convert", "-", "--from", "json", "--to", "yaml"],
            "{\"a\": 1}",
        ),
        (
            vec!["convert", "-", "--from", "oml", "--to", "json"],
            "a: 1",
        ),
        (vec!["check", "-", "--from", "yaml", "--to", "json"], "a: 1"),
        (vec!["infer", "-", "--from", "toml"], "a = 1"),
    ]
}

fn with_max<'a>(args: &[&'a str], max: &'a str) -> Vec<&'a str> {
    let mut v: Vec<&str> = args.to_vec();
    v.extend(["--max-input-bytes", max]);
    v
}

#[test]
fn at_the_maximum_is_accepted_and_one_over_is_refused_on_every_reading_subcommand() {
    for (args, doc) in reading_commands() {
        let at = run_stdin_bytes(&with_max(&args, "40"), &padded(doc, 40));
        assert_eq!(at.code, 0, "{args:?} at max: {}{}", at.stdout, at.stderr);
        let over = run_stdin_bytes(&with_max(&args, "40"), &padded(doc, 41));
        assert_eq!(
            over.code, 2,
            "{args:?} over: {}{}",
            over.stdout, over.stderr
        );
        assert!(
            over.stderr
                .contains("exceeds the maximum input size (40 bytes)"),
            "{args:?}: {}",
            over.stderr
        );
        assert!(
            over.stderr.contains("--max-input-bytes"),
            "the refusal says how to raise it: {}",
            over.stderr
        );
    }
}

#[test]
fn validate_checks_the_document_not_the_schema_against_the_maximum() {
    let schema = fixture(
        "schema.osd",
        b"record R {\n    \"a\": integer,\n}\nroot R\n",
    );
    // The schema file (more than 8 bytes) is not a Document: unbounded here.
    let ok = run_stdin_bytes(
        &[
            "validate",
            "-",
            "--from",
            "oml",
            "--schema",
            &schema,
            "--max-input-bytes",
            "8",
        ],
        &padded("a: 1", 8),
    );
    assert_eq!(ok.code, 0, "{}{}", ok.stdout, ok.stderr);
    let over = run_stdin_bytes(
        &[
            "validate",
            "-",
            "--from",
            "oml",
            "--schema",
            &schema,
            "--max-input-bytes",
            "8",
        ],
        &padded("a: 1", 9),
    );
    assert_eq!(over.code, 2, "{}{}", over.stdout, over.stderr);
}

#[test]
fn json_output_carries_the_code_and_path() {
    let r = run_stdin_bytes(
        &["format", "-", "--json", "--max-input-bytes", "4"],
        b"a: 1 \n",
    );
    assert_eq!(r.code, 2);
    assert!(
        r.stdout.contains("\"code\": \"document.limit.input-size\""),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("\"path\": \"$\""), "{}", r.stdout);
}

#[test]
fn the_size_refusal_precedes_invalid_encoding() {
    // Invalid UTF-8 and oversized: the size is checked first (D-23).
    let r = run_stdin_bytes(&["format", "-", "--max-input-bytes", "3"], &[0xff; 10]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("input size"), "{}", r.stderr);
    assert!(!r.stderr.contains("UTF-8"), "{}", r.stderr);
    // Within the limit it is the encoding failure.
    let r = run_stdin_bytes(&["format", "-", "--max-input-bytes", "10"], &[0xff; 10]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("UTF-8"), "{}", r.stderr);
}

#[test]
fn a_bom_is_counted_and_the_unit_is_bytes() {
    let with_bom = "\u{feff}a: 1".as_bytes();
    assert_eq!(with_bom.len(), 7);
    let ok = run_stdin_bytes(&["format", "-", "--max-input-bytes", "7"], with_bom);
    assert_eq!(ok.code, 0, "{}", ok.stderr);
    let over = run_stdin_bytes(&["format", "-", "--max-input-bytes", "6"], with_bom);
    assert_eq!(over.code, 2);
    // Two-byte characters: 9 bytes, 7 characters.
    let doc = "a: \"\u{e9}\u{e9}\"".as_bytes();
    assert_eq!(doc.len(), 9);
    let ok = run_stdin_bytes(&["format", "-", "--max-input-bytes", "9"], doc);
    assert_eq!(ok.code, 0, "{}", ok.stderr);
    let over = run_stdin_bytes(&["format", "-", "--max-input-bytes", "8"], doc);
    assert_eq!(over.code, 2);
}

#[test]
fn a_file_is_bounded_like_stdin() {
    let path = fixture("big.oml", &padded("a: 1", 30));
    let run = |max: &str| run_stdin_bytes(&["format", &path, "--max-input-bytes", max], b"");
    assert_eq!(run("30").code, 0);
    let over = run("29");
    assert_eq!(over.code, 2);
    assert!(over.stderr.contains("29 bytes"), "{}", over.stderr);
    // A missing file is still an I/O error naming the path.
    let missing = run_stdin_bytes(&["format", "/nonexistent/size.oml"], b"");
    assert_eq!(missing.code, 2);
    assert!(
        missing.stderr.contains("/nonexistent/size.oml"),
        "{}",
        missing.stderr
    );
}

#[test]
fn reading_stops_at_the_maximum_plus_one() {
    // A writer offering 256 MiB on stdin: a CLI that buffered the whole
    // input would take all of it, one that stops at max + 1 closes the pipe
    // after a few pipe buffers, so the writer gets a broken pipe early.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_omnist"));
    cmd.args(["format", "-", "--max-input-bytes", "1000"]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn omnist binary");
    let mut pipe = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let chunk = vec![b' '; 64 * 1024];
        let mut written = 0usize;
        while written < 256 * 1024 * 1024 {
            if pipe.write_all(&chunk).is_err() {
                break;
            }
            written += chunk.len();
        }
        written
    });
    let output = child.wait_with_output().expect("failed to wait on child");
    let written = writer.join().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("1000 bytes"), "{stderr}");
    assert!(
        written < 8 * 1024 * 1024,
        "the CLI kept reading: {written} bytes were consumed"
    );
}

#[test]
fn the_default_maximum_is_64_mib() {
    // One byte over the default through a sparse file: refused without
    // reading more than 64 MiB + 1.
    let path = std::env::temp_dir().join(format!("omnist-cli-size-{}-default", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    let r = run_stdin_bytes(&["format", path.to_str().unwrap()], b"");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("67108864 bytes"), "{}", r.stderr);
}

#[test]
fn the_flag_is_validated() {
    for bad in [
        "0",
        "-1",
        "abc",
        "1.5",
        "99999999999999999999",
        "1073741825",
    ] {
        let r = run_stdin_bytes(
            &["format", "-", &format!("--max-input-bytes={bad}")],
            b"a: 1",
        );
        assert_eq!(r.code, 2, "{bad}: {}", r.stderr);
        assert!(r.stderr.contains("max-input-bytes"), "{bad}: {}", r.stderr);
    }
    // The ceiling itself is accepted.
    let r = run_stdin_bytes(&["format", "-", "--max-input-bytes=1073741824"], b"a: 1");
    assert_eq!(r.code, 0, "{}", r.stderr);
}

#[test]
fn schema_commands_take_no_such_flag() {
    let schema = fixture("s2.osd", b"record R {\n    \"a\": integer,\n}\nroot R\n");
    let r = run_stdin_bytes(
        &["schema", "format", &schema, "--max-input-bytes", "9"],
        b"",
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("--max-input-bytes"), "{}", r.stderr);
}

#[test]
fn an_unreadable_schema_on_stdin_is_still_an_io_error() {
    // Schema files are read whole by `read_bytes` (not bounded by D-23):
    // stdin as a directory makes the read fail with EISDIR.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_omnist"));
    cmd.args(["schema", "format", "-"]);
    cmd.stdin(std::fs::File::open(std::env::temp_dir()).unwrap());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let output = cmd.output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("(reading stdin)"), "{stderr}");
}

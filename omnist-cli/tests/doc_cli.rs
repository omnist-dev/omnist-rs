//! Locks the exact literal CLI output shown in `docs/cli.md`'s `infer` and
//! `schema prune` examples. `cli.rs`'s own golden-path tests only assert
//! substrings (they predate this doc page); these tests assert the full
//! literal text so the doc's `verified-by` markers are honest.

use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_omnist"))
}

fn run(args: &[&str]) -> (i32, String) {
    let mut cmd = bin();
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().expect("failed to spawn omnist binary");
    let output = child.wait_with_output().expect("failed to wait on child");
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

fn fixture(name: &str, content: &str) -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "omnist-cli-doc-test-{}-{}-{}",
        std::process::id(),
        name,
        n
    ));
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn infer_golden_path_emits_exact_osd_text() {
    let input = fixture("doc_infer_in", r#"{"a": 1, "b": "x"}"#);
    let (code, stdout) = run(&["infer", &input, "--from", "json"]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "record Root {\n    \"a\": integer,\n    \"b\": string,\n}\nroot Root\n"
    );
}

#[test]
fn schema_prune_emits_exact_osd_text() {
    let schema = fixture(
        "doc_schema_prune_in",
        r#"record Dead { "x": string } record Root { "a": string } root Root"#,
    );
    let (code, stdout) = run(&["schema", "prune", &schema]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "record Root {\n    \"a\": string,\n}\nroot Root\n");
}

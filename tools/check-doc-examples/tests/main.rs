//! Spawns the real compiled `check-doc-examples` binary (exercising
//! `main.rs`'s glue, not just the library), mirroring the pattern
//! `omnist-cli/tests/cli.rs` already established for the `omnist` binary.

use std::fs;
use std::process::Command;

fn git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git must be installed and on PATH");
    assert!(status.success());
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path();
    git(path, &["init", "-q"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "user.name", "Test"]);
    fs::create_dir(path.join("docs")).unwrap();
    fs::write(path.join("docs/guide.md"), "# Guide\n").unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "-q", "-m", "initial"]);
    let sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let remotes = path.join(".git/refs/remotes/origin");
    fs::create_dir_all(&remotes).unwrap();
    fs::write(remotes.join("master"), sha).unwrap();
    dir
}

#[test]
fn binary_passes_with_no_changes_using_default_base_ref() {
    let repo = init_repo();
    let output = Command::new(env!("CARGO_BIN_EXE_check-doc-examples"))
        .current_dir(repo.path())
        .output()
        .expect("failed to spawn check-doc-examples binary");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("passed"));
}

#[test]
fn binary_fails_on_an_unmarked_block_with_explicit_base_ref() {
    let repo = init_repo();
    let guide = repo.path().join("docs/guide.md");
    let mut text = fs::read_to_string(&guide).unwrap();
    text.push_str("\n```python\nprint(1)\n```\n");
    fs::write(&guide, text).unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "add unmarked block"]);

    let output = Command::new(env!("CARGO_BIN_EXE_check-doc-examples"))
        .args(["--base-ref", "origin/master"])
        .current_dir(repo.path())
        .output()
        .expect("failed to spawn check-doc-examples binary");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("failed"));
}

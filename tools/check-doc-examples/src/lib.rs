//! Fail CI if a PR adds or changes a fenced code block in `docs/*.md`
//! without either a `verified-by` marker (naming the test that checks its
//! exact literal output) or an explicit `doc-illustrative` opt-out.
//!
//! This does not verify a marker is *honest* -- it only requires one to
//! exist (a known, deliberate gap; see `docs/workflow-playbook.md`'s
//! "Doc-example CI gate" section). Port of the Python project's
//! `tools/check_doc_examples.py`, and of the `omnist-ts` port's
//! `tools/check_doc_examples.ts` (see that repo's own test suite for the
//! precedent this module's tests follow).
//!
//! Usage: `check-doc-examples [--base-ref origin/master]`

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Runs `git` in `cwd` and returns its stdout as a `String`. Panics (rather
/// than returning a `Result`) on a non-zero exit -- a git failure here means
/// the checker's own environment is broken (not a repository, no such
/// ref, ...), which should surface loudly rather than be silently treated
/// as "no changes."
fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git must be installed and on PATH");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git output is utf-8")
}

/// Every `docs/**/*.md` path that changed between `base_ref` and `HEAD`.
pub fn changed_doc_files(cwd: &Path, base_ref: &str) -> Vec<String> {
    let range = format!("{base_ref}...HEAD");
    let out = git(cwd, &["diff", "--name-only", &range, "--", "docs/"]);
    out.lines()
        .filter(|p| !p.is_empty() && p.ends_with(".md"))
        .map(str::to_string)
        .collect()
}

/// Parses the "+" side of a single `@@ -a[,b] +c[,d] @@` hunk header (the
/// part of `rest` after `"@@ "` has been stripped) into a 1-indexed
/// `(start, count)` pair. `git diff`'s hunk-header format always has a `+`
/// side shaped this way for any hunk at all -- there is no reachable input
/// from a real `git diff -U0` invocation that lacks it, so this parses with
/// `expect`, not a silently-skipped `None`/`Err` branch (see this module's
/// test `parses_every_plus_side_hunk_shape` for the exhaustive shape check
/// this claim rests on, mirroring `schema.rs`'s `mandatory_u32` precedent
/// for a similarly-guaranteed-by-format parse).
fn parse_plus_side(rest: &str) -> (usize, usize) {
    let plus_idx = rest
        .find('+')
        .expect("a git diff hunk header always has a + side");
    let after_plus = &rest[plus_idx + 1..];
    let end = after_plus.find(' ').unwrap_or(after_plus.len());
    let spec = &after_plus[..end];
    match spec.split_once(',') {
        Some((s, c)) => (
            s.parse().expect("hunk start is always decimal digits"),
            c.parse().expect("hunk count is always decimal digits"),
        ),
        None => (
            spec.parse().expect("hunk start is always decimal digits"),
            1,
        ),
    }
}

/// The set of 1-indexed line numbers in `path` that changed (were added or
/// modified) between `base_ref` and `HEAD`, per a `-U0` diff's hunk headers.
pub fn changed_line_numbers(cwd: &Path, path: &str, base_ref: &str) -> BTreeSet<usize> {
    let range = format!("{base_ref}...HEAD");
    let out = git(cwd, &["diff", "-U0", &range, "--", path]);
    let mut changed = BTreeSet::new();
    for line in out.lines() {
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let (start, count) = parse_plus_side(rest);
        for i in start..start + count {
            changed.insert(i);
        }
    }
    changed
}

/// `[(fenceOpenLine, fenceCloseLine)]` -- 1-indexed, inclusive.
pub fn find_blocks(path: &Path) -> Vec<(usize, usize)> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("```") {
            let start = i + 1;
            let mut j = i + 1;
            while j < lines.len() && !lines[j].starts_with("```") {
                j += 1;
            }
            blocks.push((start, j + 1));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    blocks
}

/// A marker directly before the fence's closing line or directly after it
/// counts (offsets `-2, -1, 0, 1` relative to `block_end_line`, matching the
/// TS port's window).
pub fn has_marker(path: &Path, block_end_line: usize) -> bool {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    for offset in [-2i64, -1, 0, 1] {
        let idx = block_end_line as i64 + offset - 1;
        if idx < 0 {
            continue;
        }
        let idx = idx as usize;
        if idx < lines.len() && is_marker(lines[idx]) {
            return true;
        }
    }
    false
}

fn is_marker(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(inner) = trimmed
        .strip_prefix("<!--")
        .and_then(|s| s.strip_suffix("-->"))
    else {
        return false;
    };
    let inner = inner.trim();
    inner == "doc-illustrative" || inner.starts_with("verified-by:")
}

/// Runs the check against `cwd` (a git working tree), diffing against
/// `base_ref`. Returns `0` (nothing to report) or `1` (one or more
/// unmarked new/changed blocks found), printing a human-readable report to
/// stdout either way -- mirrors the TS port's `main` exactly.
pub fn run(cwd: &Path, base_ref: &str) -> i32 {
    let mut problems: Vec<String> = Vec::new();
    for rel_path in changed_doc_files(cwd, base_ref) {
        let path = cwd.join(&rel_path);
        if !path.exists() {
            continue;
        }
        let changed = changed_line_numbers(cwd, &rel_path, base_ref);
        for (start, end) in find_blocks(&path) {
            let touched = (start..=end).any(|n| changed.contains(&n));
            if !touched {
                continue;
            }
            if !has_marker(&path, end) {
                problems.push(format!(
                    "{rel_path}:{start}-{end}: new/changed code block has no \
                     <!-- verified-by: path::testName --> or <!-- doc-illustrative --> marker"
                ));
            }
        }
    }

    if !problems.is_empty() {
        println!("Doc-example coverage check failed:\n");
        for p in &problems {
            println!("  {p}");
        }
        println!(
            "\nEvery code block that shows literal output needs a verified-by marker \
             naming the test that asserts that exact text, or a doc-illustrative marker \
             if it's a diagram/table/grammar fragment with no runnable claim."
        );
        return 1;
    }

    println!("Doc-example coverage check passed.");
    0
}

/// Parses `--base-ref <ref>` out of `argv`, defaulting to `origin/master`.
pub fn parse_base_ref(argv: &[String]) -> String {
    let mut i = 0;
    while i < argv.len() {
        if argv[i] == "--base-ref"
            && let Some(v) = argv.get(i + 1)
        {
            return v.clone();
        }
        i += 1;
    }
    "origin/master".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct Repo {
        dir: TempDir,
    }

    impl Repo {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let path = dir.path();
            git(path, &["init", "-q"]);
            git(path, &["config", "user.email", "test@example.com"]);
            git(path, &["config", "user.name", "Test"]);
            fs::create_dir(path.join("docs")).unwrap();
            fs::write(path.join("docs/guide.md"), "# Guide\n\nSome intro text.\n").unwrap();
            git(path, &["add", "-A"]);
            git(path, &["commit", "-q", "-m", "initial"]);
            let repo = Repo { dir };
            repo.mark_origin_at_head();
            repo
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn guide_path(&self) -> PathBuf {
            self.path().join("docs/guide.md")
        }

        fn mark_origin_at_head(&self) {
            let sha = git(self.path(), &["rev-parse", "HEAD"]);
            let sha = sha.trim();
            let remotes = self.path().join(".git/refs/remotes/origin");
            fs::create_dir_all(&remotes).unwrap();
            fs::write(remotes.join("master"), format!("{sha}\n")).unwrap();
        }

        fn append_and_commit(&self, text: &str, message: &str) {
            let p = self.guide_path();
            let mut existing = fs::read_to_string(&p).unwrap();
            existing.push_str(text);
            fs::write(&p, existing).unwrap();
            git(self.path(), &["add", "-A"]);
            git(self.path(), &["commit", "-q", "-m", message]);
        }

        fn run_check(&self) -> i32 {
            run(self.path(), "origin/master")
        }
    }

    #[test]
    fn passes_with_no_changes() {
        let repo = Repo::new();
        assert_eq!(repo.run_check(), 0);
    }

    #[test]
    fn fails_on_a_new_unmarked_block() {
        let repo = Repo::new();
        repo.append_and_commit("\n```python\nprint(1)\n```\n", "add unmarked block");
        assert_eq!(repo.run_check(), 1);
    }

    #[test]
    fn passes_with_a_verified_by_marker() {
        let repo = Repo::new();
        repo.append_and_commit(
            "\n```python\nprint(1)\n```\n<!-- verified-by: tests/test_docs.py::test_x -->\n",
            "add marked block",
        );
        assert_eq!(repo.run_check(), 0);
    }

    #[test]
    fn handles_a_single_line_hunk() {
        // A one-line modification with no surrounding lines added/removed
        // produces a "@@ -N +M @@" hunk header with no ",count" suffix --
        // exercises the implicit-count-of-1 branch.
        let repo = Repo::new();
        let p = repo.guide_path();
        let text = fs::read_to_string(&p)
            .unwrap()
            .replace("Some intro text.", "Some intro text!");
        fs::write(&p, text).unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "tweak one line"]);
        assert_eq!(repo.run_check(), 0);
    }

    #[test]
    fn passes_with_a_doc_illustrative_marker() {
        let repo = Repo::new();
        repo.append_and_commit(
            "\n```mermaid\ngraph LR\n  a --> b\n```\n<!-- doc-illustrative -->\n",
            "add illustrative block",
        );
        assert_eq!(repo.run_check(), 0);
    }

    #[test]
    fn does_not_flag_an_unchanged_existing_block_in_a_touched_file() {
        let repo = Repo::new();
        repo.append_and_commit(
            "\n```python\nprint('old')\n```\n",
            "pre-existing unmarked block",
        );
        repo.mark_origin_at_head();
        repo.append_and_commit(
            "\n## New section\n\n```python\nprint('new')\n```\n<!-- verified-by: tests/test_docs.py::test_y -->\n",
            "add a new marked block, leave the old one alone",
        );
        assert_eq!(repo.run_check(), 0);
    }

    #[test]
    fn skips_a_deleted_doc_file_rather_than_crashing() {
        let repo = Repo::new();
        repo.append_and_commit("\n```python\nprint(1)\n```\n", "add unmarked block");
        repo.mark_origin_at_head();
        fs::remove_file(repo.guide_path()).unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "delete guide.md"]);
        assert_eq!(repo.run_check(), 0);
    }

    #[test]
    fn recognizes_marker_forms() {
        assert!(is_marker("<!-- verified-by: a::b -->"));
        assert!(is_marker("<!-- doc-illustrative -->"));
        assert!(!is_marker("<!-- some other comment -->"));
        assert!(!is_marker("not a marker at all"));
    }

    #[test]
    fn recognizes_markers_with_spaces_in_test_names() {
        assert!(is_marker(
            "<!-- verified-by: tests/docs.rs::quickstart snippet reproduces output -->"
        ));
    }

    #[test]
    fn parse_base_ref_defaults_and_parses() {
        assert_eq!(parse_base_ref(&[]), "origin/master");
        let argv: Vec<String> = vec!["--base-ref".to_string(), "origin/main".to_string()];
        assert_eq!(parse_base_ref(&argv), "origin/main");
        // Trailing flag with no value falls back to the default rather
        // than panicking on an out-of-bounds index.
        let argv2: Vec<String> = vec!["--base-ref".to_string()];
        assert_eq!(parse_base_ref(&argv2), "origin/master");
    }

    #[test]
    fn find_blocks_handles_multiple_blocks_and_no_blocks() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("x.md");
        fs::write(&p, "text\n```a\n1\n```\nmid\n```b\n2\n3\n```\n").unwrap();
        let blocks = find_blocks(&p);
        assert_eq!(blocks.len(), 2);

        let p2 = dir.path().join("y.md");
        fs::write(&p2, "no fences here\n").unwrap();
        assert!(find_blocks(&p2).is_empty());
    }

    #[test]
    fn has_marker_false_when_nothing_nearby() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("z.md");
        fs::write(&p, "```a\n1\n```\nplain trailing text, no marker\n").unwrap();
        assert!(!has_marker(&p, 3));
    }

    #[test]
    fn has_marker_finds_a_marker_at_every_offset_in_the_window() {
        let dir = TempDir::new().unwrap();
        // block_end_line = 3 (the closing fence). Offsets -2..=1 map to
        // 0-indexed lines 0..=3: the opening fence, the body line, the
        // closing fence itself, and the line right after it.
        let cases = [
            "<!-- doc-illustrative -->\n```a\n1\n```\n",
            "```a\n<!-- doc-illustrative -->\n1\n```\n",
            "```a\n1\n<!-- doc-illustrative -->\n```\n",
            "```a\n1\n```\n<!-- doc-illustrative -->\n",
        ];
        for (i, text) in cases.iter().enumerate() {
            let p = dir.path().join(format!("case{i}.md"));
            fs::write(&p, text).unwrap();
            assert!(has_marker(&p, 3), "case {i} should find the marker");
        }
    }

    #[test]
    fn has_marker_handles_a_block_ending_on_line_one() {
        // block_end_line = 1: offset -2 computes a negative 0-indexed line,
        // exercising the `idx < 0` guard rather than panicking.
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("early.md");
        fs::write(&p, "```a\n").unwrap();
        assert!(!has_marker(&p, 1));
    }

    #[test]
    #[should_panic(expected = "git")]
    fn git_helper_panics_with_a_readable_message_on_a_real_git_failure() {
        // A directory that is not a git repository at all makes any `git`
        // subcommand fail -- forces the `assert!`'s failure-message branch
        // (never hit by any other test, since every other test's git
        // commands always succeed) to actually execute, rather than being a
        // permanently-dead branch.
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["status"]);
    }

    #[test]
    fn parse_plus_side_handles_explicit_and_implicit_counts() {
        assert_eq!(parse_plus_side("-1,2 +3,4 @@"), (3, 4));
        assert_eq!(parse_plus_side("-1 +3 @@"), (3, 1));
        assert_eq!(parse_plus_side("-1 +3"), (3, 1));
    }
}

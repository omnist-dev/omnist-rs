//! Guards against the exact class of doc drift found in `docs/conformance.md`
//! and this crate's own `lib.rs` on 2026-08-30: both cited a specific
//! `vendor/omnist-spec` commit SHA in prose, and both were stale by the time
//! anyone noticed -- one by a single fix, one by many months. Mirrors
//! `omnist-go`'s `TestSpecVersionMatchesSubmodule` (issue #75 there), the
//! only one of the 5 ports that already had this check before this fix.

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn conformance_doc_cites_the_current_submodule_pin() {
        // No "submodule missing" guard: every other test in this crate
        // already depends on vendor/omnist-spec being checked out
        // unconditionally, so a missing submodule is a real environment
        // problem to surface loudly, not a case to skip past quietly.
        let submodule = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/omnist-spec");
        let out = Command::new("git")
            .args([
                "-C",
                submodule.to_str().unwrap(),
                "rev-parse",
                "--short",
                "HEAD",
            ])
            .output()
            .expect("git rev-parse failed to run");
        assert!(out.status.success(), "git rev-parse failed: {out:?}");
        let short_sha = String::from_utf8_lossy(&out.stdout).trim().to_string();

        let doc_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/conformance.md");
        let doc = std::fs::read_to_string(&doc_path).expect("failed to read docs/conformance.md");
        assert!(
            doc.contains(&short_sha),
            "docs/conformance.md does not cite the current vendor/omnist-spec commit {short_sha} \
             -- it's citing a stale SHA left over from a previous submodule bump"
        );
    }

    /// `book.toml` builds the published site from `src/`, which duplicates
    /// `docs/`; the two drifted once (a change updated `docs/` and forgot the
    /// twin). Every markdown page under `docs/` must be byte-identical to its
    /// `src/` twin.
    #[test]
    fn book_sources_match_docs() {
        fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "md") {
                    out.push(p);
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut pages = Vec::new();
        walk(&root.join("docs"), &mut pages);
        assert!(!pages.is_empty());
        for page in pages {
            let rel = page.strip_prefix(root.join("docs")).unwrap();
            let twin = root.join("src").join(rel);
            let a = std::fs::read(&page).unwrap();
            // A missing twin reads as empty, which differs from any real page.
            let b = std::fs::read(&twin).unwrap_or_default();
            let msg = format!("src/{0} differs from docs/{0}: copy it over", rel.display());
            assert!(a == b, "{msg}");
        }
    }
}

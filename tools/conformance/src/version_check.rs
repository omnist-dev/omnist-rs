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
        let doc = std::fs::read_to_string(&doc_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", doc_path.display()));
        assert!(
            doc.contains(&short_sha),
            "docs/conformance.md does not cite the current vendor/omnist-spec commit {short_sha} \
             -- it's citing a stale SHA left over from a previous submodule bump"
        );
    }
}

//! Runs vendor/omnist-spec's `conformance/fixtures/_referee-self-test/` --
//! proves the referee's own comparison logic is trustworthy before it
//! judges any real implementation output. omnist-spec's
//! `docs/conformance-harness.md` §6.
//!
//! Ported in spirit from Python's `omnist`'s `tools/conformance/self_test.py`
//! and omnist-ts's `tools/conformance/selfTest.ts` (same architecture);
//! only `FIXTURES_DIR` now points at this repo's own pinned submodule.
//!
//! Usage:
//!
//!     cargo run -p conformance --bin self-test

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use conformance::referee::{compare_document, compare_schema};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("vendor")
        .join("omnist-spec")
        .join("conformance")
        .join("fixtures")
        .join("_referee-self-test")
}

struct CaseResult {
    passed: bool,
    message: String,
}

fn read(dir: &Path, name: &str) -> std::io::Result<String> {
    std::fs::read_to_string(dir.join(name))
}

/// Runs one self-test case directory, returning whether the referee's
/// judgment matched the fixture's `expect.txt`.
fn run_case(case_dir: &Path) -> CaseResult {
    let kind = match read(case_dir, "kind.txt") {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            return CaseResult {
                passed: false,
                message: format!("missing kind.txt: {e}"),
            };
        }
    };
    let expect = match read(case_dir, "expect.txt") {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            return CaseResult {
                passed: false,
                message: format!("missing expect.txt: {e}"),
            };
        }
    };
    let expect_equal = match expect.as_str() {
        "equal" => true,
        "not-equal" => false,
        other => {
            return CaseResult {
                passed: false,
                message: format!("bad expect.txt value {other:?}"),
            };
        }
    };

    let actual_equal: bool = match kind.as_str() {
        "document" => {
            let a = match read(case_dir, "a.oml") {
                Ok(s) => s,
                Err(e) => {
                    return CaseResult {
                        passed: false,
                        message: format!("{e}"),
                    };
                }
            };
            let b = match read(case_dir, "b.oml") {
                Ok(s) => s,
                Err(e) => {
                    return CaseResult {
                        passed: false,
                        message: format!("{e}"),
                    };
                }
            };
            match compare_document(&a, &b) {
                Ok(v) => v,
                Err(e) => {
                    return CaseResult {
                        passed: false,
                        message: e,
                    };
                }
            }
        }
        "schema" => {
            let mode = match read(case_dir, "mode.txt") {
                Ok(s) => s.trim().to_string(),
                Err(e) => {
                    return CaseResult {
                        passed: false,
                        message: format!("missing mode.txt: {e}"),
                    };
                }
            };
            let a = match read(case_dir, "a.osd") {
                Ok(s) => s,
                Err(e) => {
                    return CaseResult {
                        passed: false,
                        message: format!("{e}"),
                    };
                }
            };
            let b = match read(case_dir, "b.osd") {
                Ok(s) => s,
                Err(e) => {
                    return CaseResult {
                        passed: false,
                        message: format!("{e}"),
                    };
                }
            };
            match compare_schema(&a, &b, &mode) {
                Ok(v) => v,
                Err(e) => {
                    return CaseResult {
                        passed: false,
                        message: e,
                    };
                }
            }
        }
        other => {
            return CaseResult {
                passed: false,
                message: format!("bad kind.txt value {other:?}"),
            };
        }
    };

    if actual_equal == expect_equal {
        CaseResult {
            passed: true,
            message: "ok".to_string(),
        }
    } else {
        CaseResult {
            passed: false,
            message: format!(
                "expected {}, got {}",
                if expect_equal { "equal" } else { "not-equal" },
                if actual_equal { "equal" } else { "not-equal" },
            ),
        }
    }
}

/// Runs every case directory under `dir`, printing PASS/FAIL per case plus
/// a summary line. Returns the process exit code (0 all-pass, 1 any
/// failure, 2 no fixtures found) -- takes the directory as a parameter
/// (rather than hard-coding it) so tests can point it at a scratch
/// directory to exercise the not-a-directory/no-cases branches without
/// touching the real submodule checkout.
fn main_with_dir(dir: &Path) -> u8 {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        eprintln!("no self-test fixtures found at {}", dir.display());
        return 2;
    };

    let mut cases: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();

    if cases.is_empty() {
        eprintln!("no self-test fixtures found at {}", dir.display());
        return 2;
    }

    let mut failures = 0u32;
    for case_dir in &cases {
        let purpose = read(case_dir, "purpose.txt")
            .ok()
            .and_then(|s| s.lines().next().map(str::to_string))
            .unwrap_or_default();
        let name = case_dir.file_name().unwrap().to_string_lossy();
        let result = run_case(case_dir);
        let status = if result.passed { "PASS" } else { "FAIL" };
        println!("[{status}] {name} ({purpose}): {}", result.message);
        if !result.passed {
            failures += 1;
        }
    }

    println!(
        "\n{}/{} self-test cases passed",
        cases.len() as u32 - failures,
        cases.len()
    );
    if failures > 0 { 1 } else { 0 }
}

fn main() -> ExitCode {
    ExitCode::from(main_with_dir(&fixtures_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn all_ten_real_fixtures_pass() {
        let code = main_with_dir(&fixtures_dir());
        assert_eq!(code, 0);
    }

    #[test]
    fn real_fixture_count_is_ten() {
        let count = fs::read_dir(fixtures_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .count();
        assert_eq!(count, 10);
    }

    #[test]
    fn missing_directory_returns_two() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing");
        let _ = fs::remove_dir_all(&tmp);
        assert_eq!(main_with_dir(&tmp), 2);
    }

    #[test]
    fn empty_directory_returns_two() {
        let tmp = std::env::temp_dir().join("conformance-self-test-empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        assert_eq!(main_with_dir(&tmp), 2);
    }

    #[test]
    fn a_genuinely_failing_case_returns_one() {
        let tmp = std::env::temp_dir().join("conformance-self-test-fail-case");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-broken");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "document\n");
        write(&case, "expect.txt", "equal\n");
        write(&case, "a.oml", "a: 1\n");
        write(&case, "b.oml", "a: 2\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn bad_expect_value_fails_the_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-bad-expect");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-bad-expect");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "document\n");
        write(&case, "expect.txt", "maybe\n");
        write(&case, "a.oml", "a: 1\n");
        write(&case, "b.oml", "a: 1\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn bad_kind_value_fails_the_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-bad-kind");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-bad-kind");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "nonsense\n");
        write(&case, "expect.txt", "equal\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn missing_kind_file_fails_the_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing-kind");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-missing-kind");
        fs::create_dir_all(&case).unwrap();
        write(&case, "expect.txt", "equal\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn missing_expect_file_fails_the_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing-expect");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-missing-expect");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "document\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn missing_a_oml_file_fails_a_document_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing-a-oml");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-missing-a-oml");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "document\n");
        write(&case, "expect.txt", "equal\n");
        write(&case, "b.oml", "a: 1\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn missing_b_oml_file_fails_a_document_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing-b-oml");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-missing-b-oml");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "document\n");
        write(&case, "expect.txt", "equal\n");
        write(&case, "a.oml", "a: 1\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn missing_a_osd_file_fails_a_schema_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing-a-osd");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-missing-a-osd");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "schema\n");
        write(&case, "expect.txt", "equal\n");
        write(&case, "mode.txt", "exact\n");
        write(
            &case,
            "b.osd",
            "record R {\n    \"x\": string,\n}\nroot R\n",
        );
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn missing_b_osd_file_fails_a_schema_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing-b-osd");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-missing-b-osd");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "schema\n");
        write(&case, "expect.txt", "equal\n");
        write(&case, "mode.txt", "exact\n");
        write(
            &case,
            "a.osd",
            "record R {\n    \"x\": string,\n}\nroot R\n",
        );
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn missing_mode_file_fails_a_schema_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-missing-mode");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-missing-mode");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "schema\n");
        write(&case, "expect.txt", "equal\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn bad_oml_fails_the_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-bad-oml");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-bad-oml");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "document\n");
        write(&case, "expect.txt", "equal\n");
        write(&case, "a.oml", "[[[not valid\n");
        write(&case, "b.oml", "a: 1\n");
        assert_eq!(main_with_dir(&tmp), 1);
    }

    #[test]
    fn bad_osd_fails_a_schema_case() {
        let tmp = std::env::temp_dir().join("conformance-self-test-bad-osd");
        let _ = fs::remove_dir_all(&tmp);
        let case = tmp.join("01-bad-osd");
        fs::create_dir_all(&case).unwrap();
        write(&case, "kind.txt", "schema\n");
        write(&case, "expect.txt", "equal\n");
        write(&case, "mode.txt", "exact\n");
        write(&case, "a.osd", "not valid osd\n");
        write(
            &case,
            "b.osd",
            "record R {\n    \"x\": string,\n}\nroot R\n",
        );
        assert_eq!(main_with_dir(&tmp), 1);
    }
}

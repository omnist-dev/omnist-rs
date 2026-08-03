//! Spawns the real compiled `self-test` and `runner` binaries (exercising
//! each one's `main()` glue, not just `main_with_dir`/`main_with_args`),
//! mirroring the pattern `tools/check-doc-examples/tests/main.rs` already
//! established.

use std::process::Command;

#[test]
fn binary_passes_against_the_real_pinned_submodule_fixtures() {
    let output = Command::new(env!("CARGO_BIN_EXE_self-test"))
        .output()
        .expect("failed to spawn self-test binary");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("10/10 self-test cases passed"));
}

#[test]
fn runner_binary_passes_against_the_real_pinned_submodule_fixtures() {
    let output = Command::new(env!("CARGO_BIN_EXE_runner"))
        .output()
        .expect("failed to spawn runner binary");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("19 passed, 0 failed, 0 skipped"));
}

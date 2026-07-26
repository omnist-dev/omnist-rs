use std::process::Command;

#[test]
fn prints_version_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_omnist"))
        .output()
        .expect("failed to run omnist-cli binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), format!("omnist {}", omnist::VERSION));
}

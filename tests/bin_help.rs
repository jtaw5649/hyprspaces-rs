use std::process::Command;

#[test]
fn prints_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_hyprspaces"))
        .arg("--help")
        .output()
        .expect("run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hyprspaces"));
}

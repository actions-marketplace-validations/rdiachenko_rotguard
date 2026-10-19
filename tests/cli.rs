use std::process::Command;

fn rotguard() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rotguard"))
}

#[test]
fn version_prints_name_and_version() {
    let out = rotguard().arg("--version").output().expect("binary runs");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "rotguard 0.1.0"
    );
}

#[test]
fn unknown_flag_exits_2() {
    let out = rotguard().arg("--bogus").output().expect("binary runs");
    assert_eq!(out.status.code(), Some(2));
}

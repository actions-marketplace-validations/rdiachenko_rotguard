//! `scripts/release-publish.sh` when `gh release edit --draft=false` fails: the script must not
//! delete a release that may already be published.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VIEW: &str = "gh release view v0.1.0 --json isDraft --jq .isDraft";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// Stub `gh`: logs every call, fails `release edit`, and answers `release view` from `GH_VIEW`
/// (`true`, `false`, or anything else to fail).
const GH_STUB: &str = r#"#!/bin/sh
echo "gh $*" >> "$STUB_LOG"
case "$2" in
    edit) exit 1 ;;
    view)
        if [ "$GH_VIEW" = fail ]; then exit 1; fi
        echo "$GH_VIEW"
        ;;
esac
"#;

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

fn run_publish(view: &str) -> (Output, Vec<String>) {
    let dir = tempfile::tempdir().expect("temp dir");
    let dist = dir.path().join("dist");
    let stubs = dir.path().join("stubs");
    fs::create_dir_all(&dist).expect("create dist");
    fs::create_dir_all(&stubs).expect("create stubs");
    for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        fs::write(
            dist.join(format!("rotguard-0.1.0-{target}")),
            "fake binary\n",
        )
        .expect("write binary");
    }
    let notes = dir.path().join("notes.md");
    fs::write(&notes, "- Added a thing.\n").expect("write notes");
    write_executable(&stubs.join("gh"), GH_STUB);
    write_executable(
        &stubs.join("git"),
        "#!/bin/sh\necho \"git $*\" >> \"$STUB_LOG\"\n",
    );
    let log_path = dir.path().join("stub.log");
    let system_path = std::env::var("PATH").expect("PATH is set");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/release-publish.sh");
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-eo", "pipefail"])
        .arg(script)
        .arg("v0.1.0")
        .arg(&dist)
        .arg(&notes)
        .current_dir(dir.path())
        .env("PATH", format!("{}:{system_path}", stubs.display()))
        .env("GITHUB_SHA", SHA)
        .env("STUB_LOG", &log_path)
        .env("GH_VIEW", view)
        .output()
        .expect("run bash");
    let log = fs::read_to_string(&log_path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    (output, log)
}

fn has(log: &[String], call: &str) -> bool {
    log.iter().any(|line| line == call)
}

fn calls_git(log: &[String]) -> bool {
    log.iter().any(|line| line.starts_with("git "))
}

#[test]
fn edit_failure_on_a_draft_deletes_it_and_touches_no_tag() {
    let (output, log) = run_publish("true");
    assert!(!output.status.success(), "log: {log:#?}");
    assert!(has(&log, VIEW), "log: {log:#?}");
    assert!(has(&log, "gh release delete v0.1.0 --yes"), "log: {log:#?}");
    assert!(!calls_git(&log), "log: {log:#?}");
}

#[test]
fn edit_failure_on_a_published_release_keeps_it_and_moves_the_major_tag() {
    let (output, log) = run_publish("false");
    assert!(output.status.success(), "log: {log:#?}");
    assert!(
        !log.iter().any(|line| line.starts_with("gh release delete")),
        "log: {log:#?}"
    );
    assert!(has(&log, &format!("git tag -f v0 {SHA}")), "log: {log:#?}");
    assert!(has(&log, "git push -f origin v0"), "log: {log:#?}");
}

#[test]
fn edit_failure_with_an_unknown_state_deletes_nothing_and_names_the_tag() {
    let (output, log) = run_publish("fail");
    assert!(!output.status.success(), "log: {log:#?}");
    assert!(
        !log.iter().any(|line| line.starts_with("gh release delete")),
        "log: {log:#?}"
    );
    assert!(!calls_git(&log), "log: {log:#?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("v0.1.0 may be published"),
        "stderr: {stderr}"
    );
}

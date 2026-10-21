//! The build cache: the composite action's key, restore and save steps.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use serde_norway::Value;

const PATHS: [&str; 4] = [
    "${{ github.action_path }}/target",
    "~/.cargo/registry/index",
    "~/.cargo/registry/cache",
    "~/.cargo/git/db",
];

fn read_repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} is missing or unreadable: {e}"))
}

fn action() -> Value {
    let text = read_repo_file("action.yml");
    serde_norway::from_str(&text).unwrap_or_else(|e| panic!("action.yml is not valid YAML: {e}"))
}

fn steps(action: &Value) -> &[Value] {
    action["runs"]["steps"]
        .as_sequence()
        .expect("action.yml has runs.steps")
}

fn run_text(step: &Value) -> &str {
    step["run"].as_str().expect("step has a run script")
}

fn bash() -> &'static str {
    if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "bash"
    }
}

fn str_of<'a>(value: &'a Value, what: &str) -> &'a str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{what} is missing or not a string"))
}

/// The lines of a block-string `with.path`, trimmed, without trailing empty lines.
fn path_lines(step: &Value) -> Vec<&str> {
    let mut lines: Vec<&str> = str_of(&step["with"]["path"], "with.path")
        .lines()
        .map(str::trim)
        .collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

#[test]
fn build_cache_s1_action_yml_restores_before_the_build_and_saves_after_the_run() {
    let action = action();
    let steps = steps(&action);
    assert_eq!(steps.len(), 5, "action.yml has five steps");

    let key = &steps[0];
    assert_eq!(key["id"].as_str(), Some("key"), "step 0 id");
    assert_eq!(key["shell"].as_str(), Some("bash"), "step 0 shell");

    let cache = &steps[1];
    assert_eq!(cache["id"].as_str(), Some("cache"), "step 1 id");
    assert_eq!(
        cache["uses"].as_str(),
        Some("actions/cache/restore@v6"),
        "step 1 uses"
    );
    assert_eq!(
        cache["with"]["key"].as_str(),
        Some("${{ steps.key.outputs.key }}"),
        "restore key"
    );
    assert_eq!(
        cache["with"]["restore-keys"].as_str().map(str::trim),
        Some("rotguard-${{ runner.os }}-${{ steps.key.outputs.rustc }}-"),
        "restore-keys"
    );
    assert_eq!(path_lines(cache), PATHS, "restore paths");

    assert!(
        run_text(&steps[2]).contains("cargo +stable build"),
        "step 2 is the build step"
    );
    assert_eq!(steps[3]["id"].as_str(), Some("run"), "step 3 id");

    let save = &steps[4];
    assert_eq!(
        save["uses"].as_str(),
        Some("actions/cache/save@v6"),
        "step 4 uses"
    );
    assert_eq!(
        save["if"].as_str(),
        Some("steps.cache.outputs.cache-hit != 'true'"),
        "save condition"
    );
    assert_eq!(
        save["with"]["key"].as_str(),
        Some("${{ steps.key.outputs.key }}"),
        "save key"
    );
    assert_eq!(path_lines(save), path_lines(cache), "save paths");

    for (i, step) in steps.iter().enumerate() {
        if step.get("run").is_some() {
            assert!(
                !run_text(step).contains("${{"),
                "step {i}'s script has no expression"
            );
        }
    }
}

#[test]
fn build_cache_s2_key_step_writes_the_rustc_version_and_the_key() {
    let action = action();
    let key_step = steps(&action)
        .iter()
        .find(|s| s["id"].as_str() == Some("key"))
        .expect("action.yml has a step with id: key");
    let script = run_text(key_step);

    let work = tempfile::tempdir().expect("temp dir");
    let script_path = work.path().join("step.sh");
    fs::write(&script_path, script).expect("write step script");

    let action_dir = work.path().join("action");
    fs::create_dir(&action_dir).expect("create action dir");
    fs::write(action_dir.join("Cargo.lock"), "x\n").expect("write Cargo.lock");

    let stubs = work.path().join("stubs");
    fs::create_dir(&stubs).expect("create stub dir");
    for (name, body) in [
        ("rustup", "#!/bin/sh\nexit 0\n"),
        (
            "rustc",
            "#!/bin/sh\necho 'rustc 1.95.0 (0123abcd 2026-09-15)'\n",
        ),
    ] {
        let path = stubs.join(name);
        fs::write(&path, body).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
    }

    let github_output = work.path().join("output");
    fs::write(&github_output, "").expect("create GITHUB_OUTPUT");

    let system_path = std::env::var("PATH").expect("PATH is set");
    let output = Command::new(bash())
        .args(["--noprofile", "--norc", "-eo", "pipefail"])
        .arg(&script_path)
        .env("GITHUB_ACTION_PATH", &action_dir)
        .env("RUNNER_OS", "Linux")
        .env("GITHUB_OUTPUT", &github_output)
        .env("PATH", format!("{}:{system_path}", stubs.display()))
        .output()
        .expect("run bash");

    assert!(
        output.status.success(),
        "key step exited with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let written = fs::read_to_string(&github_output).expect("read GITHUB_OUTPUT");
    let lines: Vec<&str> = written.lines().collect();
    for expected in ["rustc=1.95.0", "key=rotguard-Linux-1.95.0-73cb3858a687a849"] {
        assert!(
            lines.contains(&expected),
            "GITHUB_OUTPUT lacks {expected}: {written:?}"
        );
    }
}

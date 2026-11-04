//! Cutting a release: the verify and publish scripts and the release workflow.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_norway::Value;

const TARGETS: [&str; 2] = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

fn repo_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn bash() -> &'static str {
    if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "bash"
    }
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

fn bash_command(script: &Path, stubs: Option<&Path>) -> Command {
    let mut command = Command::new(bash());
    command
        .args(["--noprofile", "--norc", "-eo", "pipefail"])
        .arg(script);
    if let Some(stubs) = stubs {
        let system_path = std::env::var("PATH").expect("PATH is set");
        command.env("PATH", format!("{}:{system_path}", stubs.display()));
    }
    command
}

// ---------------------------------------------------------------------------------------------
// The verify script
// ---------------------------------------------------------------------------------------------

/// A temp repo with `Cargo.toml` at 0.1.0 and the given changelog.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new(changelog: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"rotguard\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nversion = \"9.9.9\"\n",
        )
        .expect("write Cargo.toml");
        fs::write(dir.path().join("CHANGELOG.md"), changelog).expect("write CHANGELOG.md");
        Self { dir }
    }

    fn notes(&self) -> PathBuf {
        self.dir.path().join("notes.md")
    }

    fn verify(&self, tag: &str) -> Output {
        bash_command(&repo_path("scripts/release-verify.sh"), None)
            .arg(self.notes())
            .current_dir(self.dir.path())
            .env("GITHUB_REF_NAME", tag)
            .output()
            .expect("run bash")
    }
}

const CHANGELOG: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - 2026-11-05\n\n\
                         - Added a thing.\n- Fixed another thing.\n\n## [0.0.9] - 2026-10-01\n\n\
                         - Older entry.\n";

#[test]
fn release_s1_verify_writes_the_changelog_section_as_notes() {
    let repo = Repo::new(CHANGELOG);

    let output = repo.verify("v0.1.0");

    assert!(
        output.status.success(),
        "verify exited with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let notes = fs::read_to_string(repo.notes()).expect("notes file");
    assert_eq!(
        notes.strip_suffix('\n').unwrap_or(&notes),
        "- Added a thing.\n- Fixed another thing."
    );
}

#[test]
fn release_s2_verify_rejects_a_tag_that_differs_from_the_cargo_version() {
    let repo = Repo::new(CHANGELOG);

    let output = repo.verify("v0.1.1");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "verify must fail");
    assert!(stderr.contains("v0.1.1"), "stderr: {stderr}");
    assert!(stderr.contains("0.1.0"), "stderr: {stderr}");
    assert!(!repo.notes().exists(), "no notes file on failure");
}

#[test]
fn release_s3_verify_rejects_a_changelog_without_the_version_heading() {
    let repo = Repo::new("# Changelog\n\n## [Unreleased]\n\n- Something.\n");

    let output = repo.verify("v0.1.0");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "verify must fail");
    assert!(stderr.contains("## [0.1.0]"), "stderr: {stderr}");
    assert!(!repo.notes().exists(), "no notes file on failure");
}

// ---------------------------------------------------------------------------------------------
// The workflow
// ---------------------------------------------------------------------------------------------

fn workflow() -> Value {
    let text = fs::read_to_string(repo_path(".github/workflows/release.yml"))
        .unwrap_or_else(|e| panic!("release.yml is missing or unreadable: {e}"));
    serde_norway::from_str(&text).unwrap_or_else(|e| panic!("release.yml is not valid YAML: {e}"))
}

/// A string or a one-element list of strings, as `needs:` may be written.
fn names(value: &Value) -> Vec<&str> {
    match value {
        Value::String(s) => vec![s.as_str()],
        Value::Sequence(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn job_runs(job: &Value) -> Vec<&str> {
    job["steps"]
        .as_sequence()
        .expect("job has steps")
        .iter()
        .filter_map(|s| s["run"].as_str())
        .collect()
}

#[test]
#[ignore = "pending: S4"]
fn release_s4_workflow_runs_on_tags_and_only_publish_can_write() {
    let workflow = workflow();

    // YAML 1.1 readers turn the bare key `on` into a boolean.
    let on = workflow
        .get("on")
        .or_else(|| workflow.get(Value::Bool(true)))
        .expect("workflow has an on: trigger");
    let triggers = on.as_mapping().expect("on: is a mapping");
    assert_eq!(triggers.len(), 1, "only one trigger: {triggers:?}");
    let push = on["push"].as_mapping().expect("on.push");
    assert_eq!(push.len(), 1, "push has only tags: {push:?}");
    assert_eq!(
        names(&on["push"]["tags"]),
        ["v[0-9]+.[0-9]+.[0-9]+"],
        "tag filter"
    );

    let jobs = &workflow["jobs"];
    assert!(
        job_runs(&jobs["verify"])
            .iter()
            .any(|r| r.contains("cargo test --locked")),
        "verify runs cargo test --locked"
    );

    let build = &jobs["build"];
    assert_eq!(names(&build["needs"]), ["verify"], "build needs");
    let entries = build["strategy"]["matrix"]["include"]
        .as_sequence()
        .expect("build matrix has include entries");
    assert_eq!(entries.len(), 2, "matrix entries");
    for (runner, target) in [
        ("ubuntu-latest", TARGETS[0]),
        ("ubuntu-24.04-arm", TARGETS[1]),
    ] {
        let has = |entry: &Value, text: &str| {
            entry
                .as_mapping()
                .is_some_and(|m| m.values().any(|v| v.as_str() == Some(text)))
        };
        assert!(
            entries.iter().any(|e| has(e, runner) && has(e, target)),
            "matrix lacks {runner} with {target}"
        );
    }
    let build_text = serde_norway::to_string(build).expect("serialize build job");
    assert!(
        build_text.contains("GITHUB_REF_NAME") || build_text.contains("ref_name"),
        "build job reads the tag"
    );
    assert!(
        job_runs(build).iter().any(|r| r.contains("--version")
            && r.contains("rotguard ")
            && (r.contains('=') || r.contains("test "))),
        "build compares the binary's --version with rotguard and the tag version"
    );

    assert_eq!(names(&jobs["publish"]["needs"]), ["build"], "publish needs");

    let can_write = |permissions: &Value| {
        permissions.as_str() == Some("write-all")
            || permissions["contents"].as_str() == Some("write")
    };
    assert_eq!(
        workflow["permissions"]["contents"].as_str(),
        Some("read"),
        "top-level permissions.contents"
    );
    assert!(
        !can_write(&workflow["permissions"]),
        "top-level permissions"
    );
    for (name, job) in jobs.as_mapping().expect("jobs mapping") {
        let name = name.as_str().expect("job name");
        assert_eq!(
            can_write(&job["permissions"]),
            name == "publish",
            "contents: write on job {name}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The publish script
// ---------------------------------------------------------------------------------------------

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// A dist folder with both binaries, stub `gh` and `git`, and a notes file.
struct Publish {
    dir: tempfile::TempDir,
}

impl Publish {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let publish = Self { dir };
        fs::create_dir_all(publish.dist()).expect("create dist");
        fs::create_dir_all(publish.stubs()).expect("create stubs");
        for target in TARGETS {
            fs::write(
                publish.dist().join(format!("rotguard-0.1.0-{target}")),
                format!("fake binary for {target}\n"),
            )
            .expect("write binary");
        }
        fs::write(publish.notes(), "- Added a thing.\n").expect("write notes");
        write_executable(
            &publish.stubs().join("gh"),
            "#!/bin/sh\necho \"gh $*\" >> \"$STUB_LOG\"\n\
             if [ -n \"$GH_FAIL_ON\" ] && [ \"$2\" = \"$GH_FAIL_ON\" ]; then exit 1; fi\n",
        );
        write_executable(
            &publish.stubs().join("git"),
            "#!/bin/sh\necho \"git $*\" >> \"$STUB_LOG\"\n",
        );
        publish
    }

    fn dist(&self) -> PathBuf {
        self.dir.path().join("dist")
    }

    fn stubs(&self) -> PathBuf {
        self.dir.path().join("stubs")
    }

    fn notes(&self) -> PathBuf {
        self.dir.path().join("notes.md")
    }

    fn log_path(&self) -> PathBuf {
        self.dir.path().join("stub.log")
    }

    /// Runs the script for `v0.1.0`; the `gh` stub fails on `fail_on` when it is not empty.
    fn run(&self, fail_on: &str) -> Output {
        bash_command(
            &repo_path("scripts/release-publish.sh"),
            Some(&self.stubs()),
        )
        .args(["v0.1.0"])
        .arg(self.dist())
        .arg(self.notes())
        .current_dir(self.dir.path())
        .env("GITHUB_SHA", SHA)
        .env("STUB_LOG", self.log_path())
        .env("GH_FAIL_ON", fail_on)
        .output()
        .expect("run bash")
    }

    fn log(&self) -> Vec<String> {
        fs::read_to_string(self.log_path())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// The sha256 of `path`, from the system tool.
fn sha256(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .or_else(|_| {
            Command::new("shasum")
                .args(["-a", "256"])
                .arg(path)
                .output()
        })
        .expect("run sha256sum or shasum");
    String::from_utf8(output.stdout)
        .expect("utf-8 sum")
        .split_whitespace()
        .next()
        .expect("a sum")
        .to_owned()
}

#[test]
#[ignore = "pending: S5"]
fn release_s5_publish_creates_uploads_publishes_then_moves_the_major_tag() {
    let publish = Publish::new();

    let output = publish.run("");

    assert!(
        output.status.success(),
        "publish exited with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let log = publish.log();
    let position = |prefix: &str| {
        log.iter()
            .position(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("log lacks a line starting {prefix:?}: {log:#?}"))
    };
    let create = position("gh release create v0.1.0 --draft --verify-tag");
    assert!(
        log[create].contains(&publish.notes().display().to_string()),
        "create passes the notes file: {}",
        log[create]
    );
    let upload = position("gh release upload v0.1.0");
    for asset in TARGETS
        .iter()
        .map(|t| format!("rotguard-0.1.0-{t}"))
        .chain(["SHA256SUMS".to_owned()])
    {
        assert!(
            log[upload].contains(&asset),
            "upload lacks {asset}: {}",
            log[upload]
        );
    }
    let edit = position("gh release edit v0.1.0 --draft=false");
    let tag = log
        .iter()
        .position(|l| *l == format!("git tag -f v0 {SHA}"))
        .unwrap_or_else(|| panic!("log lacks the exact tag line: {log:#?}"));
    let push = log
        .iter()
        .position(|l| l == "git push -f origin v0")
        .unwrap_or_else(|| panic!("log lacks the exact push line: {log:#?}"));
    assert!(
        create < upload && upload < edit && edit < tag && tag < push,
        "order: {log:#?}"
    );

    let sums = fs::read_to_string(publish.dist().join("SHA256SUMS")).expect("SHA256SUMS");
    let mut listed: Vec<(String, String)> = sums
        .lines()
        .map(|line| {
            let (sum, name) = line.split_once(char::is_whitespace).expect("sum and name");
            (
                name.trim().trim_start_matches('*').to_owned(),
                sum.to_owned(),
            )
        })
        .collect();
    listed.sort();
    let mut expected: Vec<(String, String)> = TARGETS
        .iter()
        .map(|t| {
            let name = format!("rotguard-0.1.0-{t}");
            let sum = sha256(&publish.dist().join(&name));
            (name, sum)
        })
        .collect();
    expected.sort();
    assert_eq!(listed, expected, "SHA256SUMS");
}

#[test]
#[ignore = "pending: S6"]
fn release_s6_publish_stops_before_git_when_create_fails() {
    let publish = Publish::new();

    let output = publish.run("create");

    assert!(!output.status.success(), "publish must fail");
    let log = publish.log();
    assert!(
        log.iter().any(|l| l.starts_with("gh release create")),
        "create was tried: {log:#?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("git ")),
        "no git call: {log:#?}"
    );
}

#[test]
#[ignore = "pending: S17"]
fn release_s17_publish_deletes_the_draft_when_the_upload_fails() {
    let publish = Publish::new();

    let output = publish.run("upload");

    assert!(!output.status.success(), "publish must fail");
    let log = publish.log();
    let last_gh = log
        .iter()
        .rfind(|l| l.starts_with("gh "))
        .expect("a gh call");
    assert_eq!(last_gh, "gh release delete v0.1.0 --yes", "log: {log:#?}");
    assert!(
        !log.iter().any(|l| l.starts_with("git ")),
        "no git call: {log:#?}"
    );
}

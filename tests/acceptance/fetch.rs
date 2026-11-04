//! The action's fetch step: download the release binary for a tag ref, build from source otherwise.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_norway::Value;

const RELEASE_URL_TAIL: &str = "github.com/rdiachenko/rotguard/releases/download/v0.1.0/";
const X64_ASSET: &str = "rotguard-0.1.0-x86_64-unknown-linux-musl";
const ARM_ASSET: &str = "rotguard-0.1.0-aarch64-unknown-linux-musl";
const FAKE_BINARY: &str = "#!/bin/sh\necho fake rotguard\n";

fn action() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("action.yml");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("action.yml is unreadable: {e}"));
    serde_norway::from_str(&text).unwrap_or_else(|e| panic!("action.yml is not valid YAML: {e}"))
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

/// The sha256 of `bytes`, from the system tool.
fn sha256(bytes: &str) -> String {
    let file = tempfile::NamedTempFile::new().expect("temp file");
    fs::write(file.path(), bytes).expect("write temp file");
    let output = Command::new("sha256sum")
        .arg(file.path())
        .output()
        .or_else(|_| {
            Command::new("shasum")
                .args(["-a", "256"])
                .arg(file.path())
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

/// What the stub `curl` serves.
#[derive(Clone, Copy)]
enum Serve {
    /// The fake binary with a matching `SHA256SUMS`.
    Good,
    /// A `SHA256SUMS` whose sum belongs to other bytes.
    WrongSum,
    /// `curl` exits 22, as `--fail` does on an HTTP 404.
    NotFound,
}

/// One run of the `id: fetch` step.
struct Fetch {
    dir: tempfile::TempDir,
    action_dir: PathBuf,
    output: Output,
}

impl Fetch {
    fn run(action_ref: &str, os: &str, arch: &str, serve: Serve) -> Self {
        let action = action();
        let step = action["runs"]["steps"]
            .as_sequence()
            .expect("action.yml has runs.steps")
            .iter()
            .find(|s| s["id"].as_str() == Some("fetch"))
            .expect("action.yml has a step with id: fetch");
        let script = step["run"].as_str().expect("fetch step has a run script");

        let dir = tempfile::tempdir().expect("temp dir");
        let work = dir.path();
        let script_path = work.join("step.sh");
        fs::write(&script_path, script).expect("write step script");

        let action_dir = work.join("_actions/o/r").join(action_ref);
        fs::create_dir_all(&action_dir).expect("create action dir");
        fs::write(
            action_dir.join("Cargo.toml"),
            "[package]\nname = \"rotguard\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo.toml");

        let asset = if arch == "ARM64" {
            ARM_ASSET
        } else {
            X64_ASSET
        };
        let bytes_for_sum = match serve {
            Serve::WrongSum => "other bytes\n",
            Serve::Good | Serve::NotFound => FAKE_BINARY,
        };
        fs::write(work.join("fixture-binary"), FAKE_BINARY).expect("write fixture binary");
        fs::write(
            work.join("fixture-sums"),
            format!("{}  {asset}\n", sha256(bytes_for_sum)),
        )
        .expect("write fixture sums");

        let stubs = work.join("stubs");
        fs::create_dir(&stubs).expect("create stub dir");
        write_executable(
            &stubs.join("curl"),
            "#!/bin/sh\n\
             prev=''\nout=''\nlast=''\n\
             for arg in \"$@\"; do\n\
             \x20 if [ \"$prev\" = '--output' ]; then out=$arg; fi\n\
             \x20 prev=$arg\n\x20 last=$arg\n\
             done\n\
             echo \"$last\" >> \"$CURL_LOG\"\n\
             if [ \"${CURL_EXIT:-0}\" != 0 ]; then exit \"$CURL_EXIT\"; fi\n\
             case \"$last\" in\n\
             \x20 *SHA256SUMS) cp \"$FIXTURE_SUMS\" \"$out\" ;;\n\
             \x20 *) cp \"$FIXTURE_BINARY\" \"$out\" ;;\n\
             esac\n",
        );

        let runner_temp = work.join("runner-temp");
        fs::create_dir(&runner_temp).expect("create RUNNER_TEMP");
        let github_output = work.join("output");
        fs::write(&github_output, "").expect("create GITHUB_OUTPUT");

        let system_path = std::env::var("PATH").expect("PATH is set");
        let output = Command::new(bash())
            .args(["--noprofile", "--norc", "-eo", "pipefail"])
            .arg(&script_path)
            .env("GITHUB_ACTION_PATH", &action_dir)
            .env("RUNNER_OS", os)
            .env("RUNNER_ARCH", arch)
            .env("RUNNER_TEMP", &runner_temp)
            .env("GITHUB_OUTPUT", &github_output)
            .env("CURL_LOG", work.join("curl.log"))
            .env(
                "CURL_EXIT",
                if matches!(serve, Serve::NotFound) {
                    "22"
                } else {
                    "0"
                },
            )
            .env("FIXTURE_BINARY", work.join("fixture-binary"))
            .env("FIXTURE_SUMS", work.join("fixture-sums"))
            .env("PATH", format!("{}:{system_path}", stubs.display()))
            .output()
            .expect("run bash");
        Self {
            dir,
            action_dir,
            output,
        }
    }

    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    fn assert_success(&self) {
        assert!(
            self.output.status.success(),
            "fetch exited with {:?}: {}{}",
            self.output.status.code(),
            self.stdout(),
            self.stderr()
        );
    }

    fn github_output(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("output"))
            .expect("read GITHUB_OUTPUT")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn assert_mode(&self, mode: &str) {
        let expected = format!("mode={mode}");
        assert!(
            self.github_output().contains(&expected),
            "GITHUB_OUTPUT lacks {expected}: {:?}",
            self.github_output()
        );
    }

    /// The URLs the stub `curl` was asked for, in order.
    fn curl_urls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("curl.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn binary(&self) -> PathBuf {
        self.action_dir.join("target/release/rotguard")
    }

    fn assert_error_line_containing(&self, text: &str) {
        let out = format!("{}\n{}", self.stdout(), self.stderr());
        assert!(
            out.lines()
                .any(|l| l.starts_with("::error::") && l.contains(text)),
            "no ::error:: line containing {text:?} in stdout/stderr: {out}"
        );
    }
}

#[test]
#[ignore = "pending: S7"]
fn fetch_s7_tag_ref_on_x64_installs_the_verified_binary() {
    let fetch = Fetch::run("v0", "Linux", "X64", Serve::Good);

    fetch.assert_success();
    fetch.assert_mode("release");
    assert_eq!(
        fs::read_to_string(fetch.binary()).expect("installed binary"),
        FAKE_BINARY
    );
    let mode = fs::metadata(fetch.binary())
        .expect("binary metadata")
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "binary is executable, mode {mode:o}");
    let urls = fetch.curl_urls();
    assert!(
        urls.iter()
            .any(|u| u.ends_with(&format!("{RELEASE_URL_TAIL}{X64_ASSET}"))),
        "binary URL not requested: {urls:?}"
    );
}

#[test]
#[ignore = "pending: S8"]
fn fetch_s8_arm_runner_downloads_the_aarch64_asset() {
    let fetch = Fetch::run("v0.1.0", "Linux", "ARM64", Serve::Good);

    fetch.assert_success();
    fetch.assert_mode("release");
    let urls = fetch.curl_urls();
    assert!(
        urls.iter()
            .any(|u| u.ends_with(&format!("{RELEASE_URL_TAIL}{ARM_ASSET}"))),
        "aarch64 URL not requested: {urls:?}"
    );
}

#[test]
#[ignore = "pending: S9"]
fn fetch_s9_action_yml_skips_the_build_steps_in_release_mode() {
    let action = action();
    let steps = action["runs"]["steps"]
        .as_sequence()
        .expect("action.yml has runs.steps");
    let by_id = |id: &str| {
        steps
            .iter()
            .find(|s| s["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("no step with id {id}"))
    };
    let build = steps
        .iter()
        .find(|s| {
            s["run"]
                .as_str()
                .is_some_and(|r| r.contains("cargo +stable build"))
        })
        .expect("the build step");
    let save = steps
        .iter()
        .find(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|u| u.starts_with("actions/cache/save"))
        })
        .expect("the save step");

    assert_eq!(steps[0]["id"].as_str(), Some("fetch"), "first step id");
    for (name, step) in [
        ("key", by_id("key")),
        ("cache", by_id("cache")),
        ("build", build),
    ] {
        assert_eq!(
            step["if"].as_str(),
            Some("steps.fetch.outputs.mode == 'build'"),
            "{name} condition"
        );
    }
    assert_eq!(
        save["if"].as_str(),
        Some("steps.fetch.outputs.mode == 'build' && steps.cache.outputs.cache-hit != 'true'"),
        "save condition"
    );
    assert!(by_id("run").get("if").is_none(), "the run step has no if");
}

#[test]
#[ignore = "pending: S10"]
fn fetch_s10_checksum_mismatch_fails_and_installs_nothing() {
    let fetch = Fetch::run("v0", "Linux", "X64", Serve::WrongSum);

    assert!(!fetch.output.status.success(), "fetch must fail");
    fetch.assert_error_line_containing(X64_ASSET);
    assert!(!fetch.binary().exists(), "no binary installed");
}

#[test]
#[ignore = "pending: S11"]
fn fetch_s11_download_failure_names_the_url_and_installs_nothing() {
    let fetch = Fetch::run("v0", "Linux", "X64", Serve::NotFound);

    assert!(!fetch.output.status.success(), "fetch must fail");
    fetch.assert_error_line_containing(&format!("{RELEASE_URL_TAIL}{X64_ASSET}"));
    assert!(!fetch.binary().exists(), "no binary installed");
}

/// S12, S13, S16 and S18: refs and platforms that build from source and download nothing.
fn assert_builds_from_source(action_ref: &str, os: &str, arch: &str) {
    let fetch = Fetch::run(action_ref, os, arch, Serve::Good);

    fetch.assert_success();
    fetch.assert_mode("build");
    assert!(
        fetch.curl_urls().is_empty(),
        "curl was called for {action_ref} on {os}: {:?}",
        fetch.curl_urls()
    );
}

#[test]
#[ignore = "pending: S12"]
fn fetch_s12_main_ref_builds_from_source() {
    assert_builds_from_source("main", "Linux", "X64");
}

#[test]
#[ignore = "pending: S13"]
fn fetch_s13_macos_runner_builds_from_source() {
    assert_builds_from_source("v0", "macOS", "ARM64");
}

#[test]
#[ignore = "pending: S16"]
fn fetch_s16_branch_named_like_a_tag_prefix_builds_from_source() {
    assert_builds_from_source("v0-fix", "Linux", "X64");
}

#[test]
#[ignore = "pending: S18"]
fn fetch_s18_slashed_ref_builds_from_source() {
    assert_builds_from_source("release/v0.2.0", "Linux", "X64");
}

//! The weekly fix PR: the composite action's run step and the documented site workflow.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_norway::Value;

use crate::support::{Routes, World, redirect, status};

/// What one run of the action's run step produced.
struct Step {
    code: i32,
    stdout: String,
    github_output: String,
    report_path: PathBuf,
    /// The report file's text, read before the temp folders go away.
    report: Option<String>,
}

fn repo_file(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read_repo_file(rel: &str) -> String {
    let path = repo_file(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} is missing or unreadable: {e}"))
}

fn parse_yaml(text: &str, what: &str) -> Value {
    serde_norway::from_str(text).unwrap_or_else(|e| panic!("{what} is not valid YAML: {e}"))
}

fn action() -> Value {
    parse_yaml(&read_repo_file("action.yml"), "action.yml")
}

fn steps(action: &Value) -> &[Value] {
    action["runs"]["steps"]
        .as_sequence()
        .expect("action.yml has runs.steps")
}

fn run_step_index(steps: &[Value]) -> usize {
    steps
        .iter()
        .position(|s| s["id"].as_str() == Some("run"))
        .expect("action.yml has a step with id: run")
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

/// Runs the `id: run` step's script the way GitHub does, in the site folder.
async fn run_step(w: &World) -> Step {
    let action = action();
    let script = run_text(&steps(&action)[run_step_index(steps(&action))]).to_owned();

    let script_dir = tempfile::tempdir().expect("temp dir");
    let script_path = script_dir.path().join("step.sh");
    fs::write(&script_path, script).expect("write step script");

    let action_dir = tempfile::tempdir().expect("temp dir");
    let bin_dir = action_dir.path().join("target/release");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    symlink(env!("CARGO_BIN_EXE_rotguard"), bin_dir.join("rotguard")).expect("symlink binary");

    let output_dir = tempfile::tempdir().expect("temp dir");
    let github_output = output_dir.path().join("output");
    fs::write(&github_output, "").expect("create GITHUB_OUTPUT");

    let runner_temp = tempfile::tempdir().expect("temp dir");

    let site = w.site.path().to_owned();
    let (code, stdout) = {
        let action_path = action_dir.path().to_owned();
        let github_output = github_output.clone();
        let runner_temp = runner_temp.path().to_owned();
        tokio::task::spawn_blocking(move || {
            let output = Command::new(bash())
                .args(["--noprofile", "--norc", "-eo", "pipefail"])
                .arg(&script_path)
                .current_dir(&site)
                .env("GITHUB_ACTION_PATH", action_path)
                .env("GITHUB_OUTPUT", github_output)
                .env("RUNNER_TEMP", runner_temp)
                .output()
                .expect("bash runs");
            (
                output.status.code().expect("exited normally"),
                String::from_utf8_lossy(&output.stdout).into_owned(),
            )
        })
        .await
        .expect("join")
    };

    let report_path = runner_temp.path().join("rotguard-report.md");
    Step {
        code,
        stdout,
        github_output: fs::read_to_string(&github_output).expect("read GITHUB_OUTPUT"),
        report: fs::read_to_string(&report_path).ok(),
        report_path,
    }
}

impl Step {
    fn output_lines(&self) -> Vec<&str> {
        self.github_output.lines().collect()
    }

    fn report_text(&self) -> &str {
        self.report
            .as_deref()
            .unwrap_or_else(|| panic!("report file {} is missing", self.report_path.display()))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn action_s1_fix_is_written_and_reported() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, "/new"))
            .fixed("/new", status(200)),
    )
    .await;
    let (old, new) = (w.url("/old"), w.url("/new"));
    w.site.write("post.md", &format!("[x]({old})\n"));

    let step = run_step(&w).await;

    assert_eq!(step.code, 0, "stdout:\n{}", step.stdout);
    assert_eq!(w.site.read("post.md"), format!("[x]({new})\n"));
    assert!(
        step.output_lines().contains(&"exit-code=0"),
        "{}",
        step.github_output
    );
    let report_line = format!("report={}", step.report_path.display());
    assert!(
        step.output_lines().contains(&report_line.as_str()),
        "{}",
        step.github_output
    );
    assert_eq!(
        step.report_text(),
        format!(
            "## Fixed: 1 link in 1 place\n\n\
             | Page | Lines | Old link | New link | Why |\n\
             |---|---|---|---|---|\n\
             | post.md | 1 | <{old}> | <{new}> | moved permanently (301 200) |\n\n\
             ## Needs a hand fix\n\nNothing.\n\n\
             1 link checked: 0 ok, 1 moved permanently.\n"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn action_s2_hand_fix_exits_one_and_writes_nothing() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/gone")));
    let before = w.site.snapshot();

    let step = run_step(&w).await;

    assert_eq!(step.code, 0, "stdout:\n{}", step.stdout);
    assert!(
        step.output_lines().contains(&"exit-code=1"),
        "{}",
        step.github_output
    );
    assert!(
        step.stdout
            .contains("no archived copy in the Wayback Machine"),
        "stdout:\n{}",
        step.stdout
    );
    let report = step.report_text();
    assert!(
        report.contains("no archived copy in the Wayback Machine"),
        "report:\n{report}"
    );
    assert_eq!(w.site.snapshot(), before, "the site folder changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn action_s3_failed_run_exits_zero_with_code_two() {
    let w = World::new().await;
    w.site.write("post.md", "no links\n");
    let config = w.site.read("rotguard.toml");
    w.site
        .write("rotguard.toml", &format!("{config}\ncolour = \"red\"\n"));

    let step = run_step(&w).await;

    assert_eq!(step.code, 0, "stdout:\n{}", step.stdout);
    assert!(
        step.output_lines().contains(&"exit-code=2"),
        "{}",
        step.github_output
    );
}

#[test]
fn action_s4_action_yml_has_the_agreed_shape() {
    let action = action();
    let steps = steps(&action);
    let run_at = run_step_index(steps);

    assert_eq!(action["runs"]["using"].as_str(), Some("composite"));
    assert!(action.get("inputs").is_none(), "action.yml has inputs");

    let build = run_text(&steps[run_at.checked_sub(1).expect("a step before id: run")]);
    assert!(
        build.contains("rustup toolchain install stable --profile minimal"),
        "{build}"
    );
    assert!(
        build.contains(
            "cargo +stable build --release --locked --manifest-path \"$GITHUB_ACTION_PATH/Cargo.toml\""
        ),
        "{build}"
    );

    let run = run_text(&steps[run_at]);
    assert!(
        run.contains("\"$GITHUB_ACTION_PATH/target/release/rotguard\" --apply"),
        "{run}"
    );

    assert_eq!(
        action["outputs"]["exit-code"]["value"].as_str(),
        Some("${{ steps.run.outputs.exit-code }}")
    );
    assert_eq!(
        action["outputs"]["report"]["value"].as_str(),
        Some("${{ steps.run.outputs.report }}")
    );

    for step in steps.iter().filter(|step| step.get("uses").is_none()) {
        assert_eq!(step["shell"].as_str(), Some("bash"), "step: {step:?}");
        assert!(!run_text(step).contains("${{"), "step: {step:?}");
    }
}

/// The first block of `docs/github-actions.md` that opens with a `yaml` fence.
fn first_yaml_block(md: &str) -> String {
    let mut lines = md.lines().skip_while(|l| *l != "```yaml").skip(1);
    let block: Vec<&str> = lines.by_ref().take_while(|l| *l != "```").collect();
    assert!(
        !block.is_empty(),
        "docs/github-actions.md has no yaml block"
    );
    block.join("\n")
}

#[test]
fn action_s5_documented_workflow_has_the_agreed_shape() {
    let doc = read_repo_file("docs/github-actions.md");
    let workflow = parse_yaml(&first_yaml_block(&doc), "the first yaml block");

    assert_eq!(
        workflow["on"]["schedule"][0]["cron"].as_str(),
        Some("0 6 * * 1")
    );
    assert!(
        workflow["on"]
            .as_mapping()
            .expect("on is a mapping")
            .contains_key("workflow_dispatch"),
        "on has no workflow_dispatch"
    );

    let steps = workflow["jobs"]["fix"]["steps"]
        .as_sequence()
        .expect("job fix has steps");
    let rotguard_at = steps
        .iter()
        .position(|s| s["id"].as_str() == Some("rotguard"))
        .expect("a step with id: rotguard");
    assert_eq!(
        steps[rotguard_at]["uses"].as_str(),
        Some("rdiachenko/rotguard@main")
    );

    let pr_at = steps
        .iter()
        .position(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|u| u.starts_with("peter-evans/create-pull-request@"))
        })
        .expect("a create-pull-request step");
    assert!(
        pr_at > rotguard_at,
        "the PR step comes before the rotguard step"
    );
    let pr = &steps[pr_at];
    assert_eq!(
        pr["if"].as_str(),
        Some("steps.rotguard.outputs.exit-code == '0' || steps.rotguard.outputs.exit-code == '1'")
    );
    assert_eq!(pr["with"]["branch"].as_str(), Some("rotguard/fixes"));
    assert_eq!(pr["with"]["delete-branch"].as_bool(), Some(true));
    assert_eq!(
        pr["with"]["body-path"].as_str(),
        Some("${{ steps.rotguard.outputs.report }}")
    );

    let fail = steps[pr_at + 1..]
        .iter()
        .find(|s| s["if"].as_str() == Some("steps.rotguard.outputs.exit-code != '0'"))
        .expect("a failing step after the PR step");
    let script = fail["run"].as_str().expect("the failing step has a run");
    assert!(script.contains("a link needs a hand fix"), "{script}");
    assert!(script.contains("rotguard failed"), "{script}");
    assert!(script.trim_end().ends_with("exit 1"), "{script}");

    let notify = &workflow["jobs"]["notify"];
    assert_eq!(
        notify["needs"].as_sequence().map(Vec::as_slice),
        Some([Value::from("fix")].as_slice())
    );
    assert_eq!(
        notify["if"].as_str(),
        Some("always() && contains(needs.*.result, 'failure')")
    );
    assert_eq!(
        notify["uses"].as_str(),
        Some("./.github/workflows/telegram-notify.yml")
    );
    assert_eq!(
        notify["with"]["title"].as_str(),
        Some("rotguard: weekly run failed")
    );
}

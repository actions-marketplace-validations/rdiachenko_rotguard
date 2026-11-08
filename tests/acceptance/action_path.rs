//! S11-S13: the action's `path` input and its documentation.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_norway::Value;

use crate::support::{Routes, World, redirect, status};

fn read_repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} is missing or unreadable: {e}"))
}

fn parse_yaml(text: &str, what: &str) -> Value {
    serde_norway::from_str(text).unwrap_or_else(|e| panic!("{what} is not valid YAML: {e}"))
}

fn action() -> Value {
    parse_yaml(&read_repo_file("action.yml"), "action.yml")
}

fn steps(doc: &Value) -> &[Value] {
    doc["runs"]["steps"]
        .as_sequence()
        .expect("action.yml has runs.steps")
}

fn run_step(action: &Value) -> &Value {
    steps(action)
        .iter()
        .find(|s| s["id"].as_str() == Some("run"))
        .expect("action.yml has a step with id: run")
}

fn bash() -> &'static str {
    if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "bash"
    }
}

/// What running the `id: run` step produced.
struct Step {
    code: i32,
    stdout: String,
    github_output: String,
}

/// Runs the `id: run` step's script the way GitHub does, in `workspace`, with the `path` input
/// as `ROTGUARD_PATH`.
async fn run_in(workspace: PathBuf, path: &str) -> Step {
    let action = action();
    let script = run_step(&action)["run"]
        .as_str()
        .expect("step has a run script")
        .to_owned();

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

    let action_path = action_dir.path().to_owned();
    let github_output_path = github_output.clone();
    let runner_temp = runner_temp.path().to_owned();
    let path = path.to_owned();
    let (code, stdout) = tokio::task::spawn_blocking(move || {
        let output = Command::new(bash())
            .args(["--noprofile", "--norc", "-eo", "pipefail"])
            .arg(&script_path)
            .current_dir(&workspace)
            .env("GITHUB_ACTION_PATH", action_path)
            .env("GITHUB_OUTPUT", github_output_path)
            .env("RUNNER_TEMP", runner_temp)
            .env("ROTGUARD_PATH", path)
            .output()
            .expect("bash runs");
        (
            output.status.code().expect("exited normally"),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    })
    .await
    .expect("join");

    Step {
        code,
        stdout,
        github_output: fs::read_to_string(&github_output).expect("read GITHUB_OUTPUT"),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S11"]
async fn s11_run_step_works_on_a_subfolder() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, "/new"))
            .fixed("/new", status(200)),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/old")));
    // The workspace holds the site under `blog/` and nothing else. The fixture site is flat.
    let workspace = tempfile::tempdir().expect("temp dir");
    let blog = workspace.path().join("blog");
    fs::create_dir(&blog).expect("create blog dir");
    for entry in fs::read_dir(w.site.path()).expect("read site") {
        let from = entry.expect("site entry").path();
        let to = blog.join(from.file_name().expect("file name"));
        fs::copy(&from, &to).expect("copy site file");
    }

    let step = run_in(workspace.path().to_owned(), "blog").await;

    assert_eq!(step.code, 0, "stdout:\n{}", step.stdout);
    assert_eq!(
        fs::read_to_string(blog.join("post.md")).expect("read post.md"),
        format!("[x]({})\n", w.url("/new"))
    );
    assert!(
        step.github_output.lines().any(|l| l == "exit-code=0"),
        "{}",
        step.github_output
    );
}

#[test]
#[ignore = "pending: S12"]
fn s12_action_declares_the_path_input_and_keeps_it_out_of_scripts() {
    let action = action();

    let inputs = action["inputs"]
        .as_mapping()
        .expect("action.yml has inputs");
    assert_eq!(inputs.len(), 1, "inputs: {inputs:?}");
    let path = &action["inputs"]["path"];
    assert_eq!(path["default"].as_str(), Some("."), "{path:?}");
    assert_eq!(path["required"].as_bool(), Some(false), "{path:?}");

    let run = run_step(&action);
    assert_eq!(
        run["env"]["ROTGUARD_PATH"].as_str(),
        Some("${{ inputs.path }}"),
        "{run:?}"
    );
    let after_separator = Regex::new(r" -- .*ROTGUARD_PATH").expect("test regex");
    let script = run["run"].as_str().expect("step has a run script");
    assert!(after_separator.is_match(script), "script:\n{script}");

    for step in steps(&action) {
        if let Some(script) = step["run"].as_str() {
            assert!(
                !script.contains("${{"),
                "script with an expression:\n{script}"
            );
        }
    }
}

/// The text of each ```yaml fenced block.
fn yaml_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        match &mut current {
            None if line.trim_end() == "```yaml" => current = Some(String::new()),
            None => {}
            Some(_) if line.trim_end() == "```" => {
                blocks.extend(current.take());
            }
            Some(text) => {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    blocks
}

/// The steps of every job in a workflow that use `rdiachenko/rotguard@v0`.
fn rotguard_steps(workflow: &Value) -> Vec<&Value> {
    let Some(jobs) = workflow["jobs"].as_mapping() else {
        return Vec::new();
    };
    jobs.values()
        .filter_map(|job| job["steps"].as_sequence())
        .flatten()
        .filter(|step| step["uses"].as_str() == Some("rdiachenko/rotguard@v0"))
        .collect()
}

#[test]
#[ignore = "pending: S13"]
fn s13_docs_show_a_monorepo_workflow() {
    let docs = read_repo_file("docs/github-actions.md");
    let workflows: Vec<Value> = yaml_blocks(&docs)
        .iter()
        .enumerate()
        .map(|(i, text)| parse_yaml(text, &format!("yaml block {i} of github-actions.md")))
        .collect();
    assert!(!workflows.is_empty(), "no yaml block in github-actions.md");

    // The first block is the plain workflow: a rotguard step with no inputs, then the PR step.
    let first = rotguard_steps(&workflows[0]);
    assert_eq!(first.len(), 1, "first block: {:?}", workflows[0]);
    assert!(first[0]["with"].is_null(), "{:?}", first[0]);

    let subfolder = workflows.iter().flat_map(rotguard_steps).find_map(|step| {
        step["with"]["path"]
            .as_str()
            .filter(|path| !path.is_empty() && *path != ".")
    });
    assert!(
        subfolder.is_some(),
        "no rotguard@v0 step sets with.path to a subfolder"
    );
}

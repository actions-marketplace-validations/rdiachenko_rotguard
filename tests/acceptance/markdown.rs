//! The markdown report: `--format markdown` and the action's run step that publishes it.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::support::{Routes, World, redirect, status};

/// A site whose `post.md` links `/old` on line 1 and line 3, with `/old` moved to `/new`.
async fn moved_world() -> World {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, "/new"))
            .fixed("/new", status(200)),
    )
    .await;
    let link = format!("[a]({})", w.url("/old"));
    w.site.write("post.md", &format!("{link}\n\n{link}\n"));
    w
}

/// A site whose `post.md` is the one line `[x](M/old)`, with `/old` moved to `/new`.
async fn one_line_world() -> World {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, "/new"))
            .fixed("/new", status(200)),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/old")));
    w
}

/// S8's report text for `one_line_world`.
fn one_line_report(w: &World) -> String {
    format!(
        "## Fixed: 1 link in 1 place\n\n\
         | Page | Lines | Old link | New link | Why |\n\
         |---|---|---|---|---|\n\
         | post.md | 1 | <{}> | <{}> | moved permanently (301 200) |\n\n\
         ## Needs a hand fix\n\n\
         Nothing.\n\n\
         1 link checked: 0 ok, 1 moved permanently.\n",
        w.url("/old"),
        w.url("/new")
    )
}

/// What one run of the action's run step produced.
struct Step {
    code: i32,
    stdout: String,
    github_output: String,
    report_path: PathBuf,
    /// The report file's text, read before the temp folders go away.
    report: Option<String>,
    /// The job summary file's text, when the run had one.
    summary: Option<String>,
}

fn bash() -> &'static str {
    if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "bash"
    }
}

/// The script of the `id: run` step of `action.yml`.
fn run_script() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("action.yml");
    let text = fs::read_to_string(&path).expect("action.yml is readable");
    let action: serde_norway::Value = serde_norway::from_str(&text).expect("action.yml is YAML");
    let steps = action["runs"]["steps"]
        .as_sequence()
        .expect("action.yml has runs.steps");
    let step = steps
        .iter()
        .find(|s| s["id"].as_str() == Some("run"))
        .expect("action.yml has a step with id: run");
    step["run"]
        .as_str()
        .expect("step has a run script")
        .to_owned()
}

/// Runs the `id: run` step's script the way GitHub does, in the site folder. With `summary`
/// set, `GITHUB_STEP_SUMMARY` names that file; otherwise the variable is removed.
async fn run_step(w: &World, summary: Option<&Path>) -> Step {
    let script_dir = tempfile::tempdir().expect("temp dir");
    let script_path = script_dir.path().join("step.sh");
    fs::write(&script_path, run_script()).expect("write step script");

    let action_dir = tempfile::tempdir().expect("temp dir");
    let bin_dir = action_dir.path().join("target/release");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    symlink(env!("CARGO_BIN_EXE_rotguard"), bin_dir.join("rotguard")).expect("symlink binary");

    let output_dir = tempfile::tempdir().expect("temp dir");
    let github_output = output_dir.path().join("output");
    fs::write(&github_output, "").expect("create GITHUB_OUTPUT");

    let runner_temp = tempfile::tempdir().expect("temp dir");

    let site = w.site.path().to_owned();
    let summary = summary.map(Path::to_owned);
    let (code, stdout) = {
        let action_path = action_dir.path().to_owned();
        let github_output = github_output.clone();
        let runner_temp = runner_temp.path().to_owned();
        let summary = summary.clone();
        tokio::task::spawn_blocking(move || {
            let mut command = Command::new(bash());
            command
                .args(["--noprofile", "--norc", "-eo", "pipefail"])
                .arg(&script_path)
                .current_dir(&site)
                .env("GITHUB_ACTION_PATH", action_path)
                .env("GITHUB_OUTPUT", github_output)
                .env("RUNNER_TEMP", runner_temp);
            match summary {
                Some(path) => command.env("GITHUB_STEP_SUMMARY", path),
                None => command.env_remove("GITHUB_STEP_SUMMARY"),
            };
            let output = command.output().expect("bash runs");
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
        summary: summary.and_then(|p| fs::read_to_string(p).ok()),
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
async fn markdown_s1_fix_is_one_row_with_both_lines() {
    let w = moved_world().await;
    let (old, new) = (w.url("/old"), w.url("/new"));

    let out = w.run(&["--apply", "--format", "markdown"]).await;

    let link = format!("[a]({new})");
    assert_eq!(w.site.read("post.md"), format!("{link}\n\n{link}\n"));
    out.assert_code(0);
    assert_eq!(
        out.stdout,
        format!(
            "## Fixed: 1 link in 2 places\n\n\
             | Page | Lines | Old link | New link | Why |\n\
             |---|---|---|---|---|\n\
             | post.md | 1, 3 | <{old}> | <{new}> | moved permanently (301 200) |\n\n\
             ## Needs a hand fix\n\n\
             Nothing.\n\n\
             1 link checked: 0 ok, 1 moved permanently.\n"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_s2_proposed_fix_is_not_written() {
    let w = moved_world().await;
    let before = w.site.snapshot();

    let out = w.run(&["--format", "markdown"]).await;

    assert_eq!(w.site.snapshot(), before, "the site folder changed");
    let head = "## Fixes to apply: 1 link in 2 places\n\n\
                Run `rotguard --apply` to write them.\n\n\
                | Page | Lines | Old link | New link | Why |\n";
    assert!(out.stdout.starts_with(head), "stdout:\n{}", out.stdout);
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_s3_dead_link_needs_a_hand_fix() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    let gone = w.url("/gone");
    w.site.write("post.md", &format!("[x]({gone})\n"));
    let before = w.site.snapshot();

    let out = w.run(&["--apply", "--format", "markdown"]).await;

    out.assert_code(1);
    assert_eq!(w.site.snapshot(), before, "the site folder changed");
    assert_eq!(
        out.stdout,
        format!(
            "## Needs a hand fix: 1 link in 1 place\n\n\
             | Page | Lines | Link | Problem | Why rotguard couldn't fix it |\n\
             |---|---|---|---|---|\n\
             | post.md | 1 | <{gone}> | dead (404) | no archived copy in the Wayback Machine |\n\n\
             1 link checked: 0 ok, 1 dead.\n"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_s4_temporary_redirect_is_folded_away() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/t", redirect(302, "/end"))
            .fixed("/end", status(200)),
    )
    .await;
    let (t, end) = (w.url("/t"), w.url("/end"));
    w.site.write("post.md", &format!("[t]({t})\n"));

    let out = w.run(&["--format", "markdown"]).await;

    out.assert_code(0);
    assert_eq!(
        out.stdout,
        format!(
            "## Needs a hand fix\n\n\
             Nothing.\n\n\
             <details><summary>1 more link to know about, no action needed</summary>\n\n\
             | Page | Lines | Link | What rotguard saw |\n\
             |---|---|---|---|\n\
             | post.md | 1 | <{t}> | moved temporarily (302 200) to <{end}> |\n\n\
             </details>\n\n\
             1 link checked: 0 ok, 1 moved temporarily.\n"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_s5_clean_site_says_no_broken_links() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/a", status(200))).await;
    w.site.write("post.md", &format!("[x]({})\n", w.url("/a")));

    let out = w.run(&["--format", "markdown"]).await;

    assert_eq!(out.stdout, "No broken links.\n\n1 link checked: 1 ok.\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_s6_text_from_the_site_is_escaped() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/a|b", status(404))).await;
    w.site.write("a_b.md", &format!("[x]({}|b)\n", w.url("/a")));

    let out = w.run(&["--format", "markdown"]).await;

    let row = format!(
        "| a\\_b.md | 1 | {}\\|b | dead (404) | no archived copy in the Wayback Machine |",
        w.url("/a")
    );
    assert!(
        out.stdout.lines().any(|l| l == row),
        "stdout:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_s7_strict_lists_a_bot_wall_in_the_folded_block() {
    let w = World::new().await;
    w.serve(Routes::new().fixed_re("^/wall/", status(403)))
        .await;
    let page = w.url("/wall/page");
    w.site.write("post.md", &format!("[x]({page})\n"));

    let out = w.run(&["--strict", "--format", "markdown"]).await;

    let row = format!("| post.md | 1 | <{page}> | blocked by a bot wall (403) |");
    let (_, details) = out
        .stdout
        .split_once("<details>")
        .unwrap_or_else(|| panic!("no <details> block in stdout:\n{}", out.stdout));
    let (inside, _) = details
        .split_once("</details>")
        .unwrap_or_else(|| panic!("unclosed <details> block in stdout:\n{}", out.stdout));
    assert!(inside.lines().any(|l| l == row), "stdout:\n{}", out.stdout);
    let counts = out.stdout.trim_end().lines().last().expect("a counts line");
    assert!(counts.ends_with("1 blocked."), "counts line: {counts}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S8"]
async fn markdown_s8_run_step_writes_the_report_and_the_summary() {
    let w = one_line_world().await;
    let summary_dir = tempfile::tempdir().expect("temp dir");
    let summary_path = summary_dir.path().join("summary.md");
    fs::write(&summary_path, "").expect("create GITHUB_STEP_SUMMARY");

    let step = run_step(&w, Some(&summary_path)).await;

    assert_eq!(step.code, 0, "stdout:\n{}", step.stdout);
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
    assert_eq!(w.site.read("post.md"), format!("[x]({})\n", w.url("/new")));
    let report = step.report_text();
    assert_eq!(report, one_line_report(&w));
    assert!(
        step.stdout.contains(report),
        "the log lacks the report:\n{}",
        step.stdout
    );
    assert_eq!(step.summary.as_deref(), Some(report));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S9"]
async fn markdown_s9_run_step_needs_no_summary_file() {
    let w = one_line_world().await;

    let step = run_step(&w, None).await;

    assert_eq!(step.code, 0, "stdout:\n{}", step.stdout);
    assert_eq!(step.report_text(), one_line_report(&w));
}

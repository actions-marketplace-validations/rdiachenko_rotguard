//! `--apply` on a file it can't write: that file's fixes become hand fixes in every report.

use serde_json::{Value, json};

use crate::support::{Routes, World, redirect, status};

/// A site whose `a.md` and `post.md` link `/old`, moved to `/new`, with a leftover
/// `post.md.rotguard.tmp` that makes writing `post.md` fail.
async fn blocked_write_world() -> World {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, "/new"))
            .fixed("/new", status(200)),
    )
    .await;
    let link = format!("[a]({})\n", w.url("/old"));
    w.site.write("a.md", &link);
    w.site.write("post.md", &link.replace("[a]", "[b]"));
    w.site.write("post.md.rotguard.tmp", "x");
    w
}

/// The `fix` of the one finding at `file` and `line` in the JSON report.
fn fix_at(report: &Value, file: &str, line: u64) -> Value {
    let rows = report["findings"].as_array().expect("findings is a list");
    let row = rows
        .iter()
        .find(|r| r["file"] == file && r["line"] == line)
        .unwrap_or_else(|| panic!("no finding at {file}:{line} in {report}"));
    row["fix"].clone()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S1"]
async fn apply_failed_s1_markdown_lists_the_unwritten_file_as_a_hand_fix() {
    let w = blocked_write_world().await;
    let (old, new) = (w.url("/old"), w.url("/new"));

    let out = w.run(&["--apply", "--format", "markdown"]).await;

    out.assert_code(2);
    let expected = format!(
        "## Fixed: 1 link in 1 place\n\n\
         | Page | Lines | Old link | New link | Why |\n\
         |---|---|---|---|---|\n\
         | a.md | 1 | <{old}> | <{new}> | moved permanently (301 200) |\n\n\
         ## Needs a hand fix: 1 link in 1 place\n\n\
         | Page | Lines | Link | Problem | Why rotguard couldn't fix it |\n\
         |---|---|---|---|---|\n\
         | post.md | 1 | <{old}> | moved permanently (301 200) | \
         rotguard couldn't write the file; its error message says why |\n\n\
         1 link checked: 0 ok, 1 moved permanently.\n"
    );
    assert_eq!(out.stdout, expected, "stderr:\n{}", out.stderr);
    assert_eq!(w.site.read("a.md"), format!("[a]({new})\n"));
    assert_eq!(w.site.read("post.md"), format!("[b]({old})\n"));
    assert_eq!(w.site.read("post.md.rotguard.tmp"), "x");
    assert!(
        out.stderr.contains("rotguard: applying fixes: post.md"),
        "stderr:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("remove it and run again"),
        "stderr:\n{}",
        out.stderr
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S2"]
async fn apply_failed_s2_text_says_write_failed() {
    let w = blocked_write_world().await;

    let out = w.run(&["--apply"]).await;

    out.assert_code(2);
    let entries = out.entries();
    let post = entries
        .iter()
        .find(|e| e.loc == "post.md:1")
        .unwrap_or_else(|| panic!("no post.md:1 entry:\n{}", out.stdout));
    assert_eq!(post.detail.as_deref(), Some("manual fix: write failed"));
    assert!(
        entries
            .iter()
            .filter(|e| e.loc.starts_with("post.md:"))
            .all(|e| e.detail.as_deref().is_none_or(|d| !d.starts_with("fix:"))),
        "a post.md entry has a fix line:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S3"]
async fn apply_failed_s3_json_marks_the_fix_manual() {
    let w = blocked_write_world().await;
    let new = w.url("/new");

    let out = w.run(&["--apply", "--format", "json"]).await;

    out.assert_code(2);
    let report: Value = serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\nstdout:\n{}", out.stdout));
    assert_eq!(
        fix_at(&report, "post.md", 1),
        json!({"kind": "manual", "reason": "write failed"})
    );
    assert_eq!(
        fix_at(&report, "a.md", 1),
        json!({"kind": "applied", "url": new})
    );
}

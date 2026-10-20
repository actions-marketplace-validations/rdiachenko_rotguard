//! Report-format design: `--format text|json|html` and the flag's error handling.

use serde_json::{Value, json};

use crate::support::{Capture, Cdx, Out, Routes, World, page, redirect, status};

const SUMMARY: &str = "checked 2: ok 0, permanent 1, temporary 0, dead 1, suspect 0, unverifiable 0, blocked 0, ignored 0";

/// `post.md` links `/old` (line 1, 301 to `/new`) and `/gone` (line 3, 404 with one capture).
async fn mixed_world() -> World {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/new", page("New home", "the page moved here"))
            .fixed("/gone", status(404)),
    )
    .await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20200101000000", &gone)]));
    w.site.write(
        "post.md",
        &format!("[a]({})\n\n[b]({gone})\n", w.url("/old")),
    );
    w
}

/// `post.md` links `/gone`, which answers 404, with no capture.
async fn dead_world() -> World {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    w.archive(Cdx::new(vec![]));
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/gone")));
    w
}

fn parse(out: &Out) -> Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON value: {e}\nstdout:\n{}\nstderr:\n{}",
            out.stdout, out.stderr
        )
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s1_text_is_the_default() {
    let w = dead_world().await;
    let gone = w.url("/gone");

    let default = w.run(&[]).await;
    let text = w.run(&["--format", "text"]).await;

    assert_eq!(default.stdout, text.stdout);
    let entry = default.entry(&gone);
    assert_eq!(entry.loc, "post.md:1");
    assert_eq!(entry.verdict, "DEAD 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s2_json_lists_findings_and_summary() {
    let w = mixed_world().await;
    let (old, new) = (w.url("/old"), w.url("/new"));
    let snapshot = w.snapshot_url("20200101000000", "/gone");

    let out = w.run(&["--format", "json"]).await;

    let v = parse(&out);
    let findings = v["findings"].as_array().expect("findings is an array");
    assert_eq!(findings.len(), 2, "stdout:\n{}", out.stdout);
    assert_eq!(
        findings[0],
        json!({
            "file": "post.md",
            "line": 1,
            "url": old,
            "class": "permanent",
            "chain": [{"url": old, "status": 301}, {"url": new, "status": 200}],
            "reason": null,
            "final_url": null,
            "fix": {"kind": "proposed", "url": new},
        })
    );
    assert_eq!(findings[1]["class"], "dead");
    assert_eq!(findings[1]["chain"], json!([]));
    assert_eq!(findings[1]["reason"], "404");
    assert_eq!(
        findings[1]["fix"],
        json!({"kind": "proposed", "url": snapshot})
    );
    assert_eq!(
        v["summary"],
        json!({
            "checked": 2,
            "ok": 0,
            "permanent": 1,
            "temporary": 0,
            "dead": 1,
            "suspect": 0,
            "unverifiable": 0,
            "blocked": 0,
            "ignored": 0,
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s3_json_reports_applied_and_manual_fixes() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/old2", redirect(301, &w.url("/new")))
            .fixed("/new", page("New home", "the page moved here")),
    )
    .await;
    let (old, old2, new) = (w.url("/old"), w.url("/old2"), w.url("/new"));
    w.site.write(
        "post.md",
        &format!("---\nsource: \"{old}\"\n---\n\n[x]({old2})\n"),
    );

    let out = w.run(&["--apply", "--format", "json"]).await;

    let v = parse(&out);
    let fix_of = |url: &str| {
        v["findings"]
            .as_array()
            .expect("findings is an array")
            .iter()
            .find(|f| f["url"] == url)
            .unwrap_or_else(|| panic!("no finding for {url}; stdout:\n{}", out.stdout))["fix"]
            .clone()
    };
    assert_eq!(fix_of(&old2), json!({"kind": "applied", "url": new}));
    assert_eq!(
        fix_of(&old),
        json!({"kind": "manual", "reason": "not verbatim"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s4_html_is_a_self_contained_table() {
    let w = mixed_world().await;
    let new = w.url("/new");

    let out = w.run(&["--format", "html"]).await;

    let html = &out.stdout;
    assert!(
        html.to_lowercase().starts_with("<!doctype html>"),
        "stdout:\n{html}"
    );
    assert!(html.contains("<title>rotguard report</title>"), "{html}");
    assert!(html.contains(SUMMARY), "{html}");
    assert!(html.contains(&format!("<a href=\"{new}\">")), "{html}");
    let (_, after_open) = html.split_once("<tbody>").expect("a <tbody>");
    let (tbody, _) = after_open.split_once("</tbody>").expect("a </tbody>");
    let rows: Vec<&str> = tbody.split("<tr").skip(1).collect();
    assert_eq!(rows.len(), 2, "tbody:\n{tbody}");
    assert!(rows[0].contains("post.md:1"), "first row:\n{}", rows[0]);
    assert!(rows[1].contains("post.md:3"), "second row:\n{}", rows[1]);
    let lower = html.to_lowercase();
    for tag in ["<script", "<link", "<img"] {
        assert!(!lower.contains(tag), "found {tag} in:\n{html}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s5_html_without_findings_has_no_table() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/a", page("Alpha", "an ordinary page")))
        .await;
    w.site.write("post.md", &format!("[x]({})\n", w.url("/a")));

    let out = w.run(&["--format", "html"]).await;

    assert!(out.stdout.contains("No broken links."), "{}", out.stdout);
    assert!(!out.stdout.contains("<table"), "{}", out.stdout);
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s6_html_escapes_urls() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/x", status(404))).await;
    w.archive(Cdx::new(vec![]));
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/x?a=1&b='2'")));

    let out = w.run(&["--format", "html"]).await;

    assert!(
        out.stdout.contains("a=1&amp;b=&#39;2&#39;"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("&b='2'"), "{}", out.stdout);
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s7_json_keeps_exit_code_and_stderr() {
    let w = dead_world().await;

    let out = w.run(&["--format", "json"]).await;

    out.assert_code(1);
    assert!(
        out.stderr.lines().any(|l| l == "checking 1 link on 1 host"),
        "stderr:\n{}",
        out.stderr
    );
    parse(&out);
}

#[tokio::test(flavor = "multi_thread")]
async fn format_s8_unknown_format_is_rejected_before_any_request() {
    let w = dead_world().await;

    let out = w.run(&["--format", "yaml"]).await;

    out.assert_code(2);
    for name in ["text", "json", "html"] {
        assert!(out.stderr.contains(name), "stderr:\n{}", out.stderr);
    }
    assert_eq!(out.stdout, "");
    assert!(w.log.all().is_empty(), "requests: {:?}", w.log.all());
}

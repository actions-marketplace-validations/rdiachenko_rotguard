//! Bot walls: a final 401, 403 or 999 that a random sibling path also gets is `blocked`.

use std::time::Duration;

use serde_json::{Value, json};

use crate::support::{Out, Req, Routes, World, is_probe, redirect, status};

/// A site whose `post.md` links `/wall/page`, with M answering 403 under `/wall/`.
async fn wall_world() -> World {
    let w = World::new().await;
    w.serve(Routes::new().fixed_re("^/wall/", status(403)))
        .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/wall/page")));
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

/// Requests whose path is `folder` followed directly by a probe token.
fn probes_under(w: &World, folder: &str) -> Vec<Req> {
    w.log
        .all()
        .into_iter()
        .filter(|r| {
            r.path
                .strip_prefix(folder)
                .is_some_and(|rest| !rest.contains('/') && is_probe(rest))
        })
        .collect()
}

fn summary_with(blocked: usize, unverifiable: usize) -> String {
    format!(
        "checked 1: ok 0, permanent 0, temporary 0, dead 0, suspect 0, unverifiable {unverifiable}, blocked {blocked}, ignored 0"
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s1_wall_is_counted_not_reported() {
    let w = wall_world().await;

    let out = w.run(&[]).await;

    assert!(out.entries().is_empty(), "stdout:\n{}", out.stdout);
    assert_eq!(out.summary_line(), summary_with(1, 0));
    out.assert_code(0);
    let probes = probes_under(&w, "/wall/");
    assert_eq!(probes.len(), 1, "probes: {probes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s2_lone_403_stays_unverifiable() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/forbidden", status(403)))
        .await;
    let url = w.url("/forbidden");
    w.site.write("post.md", &format!("[x]({url})\n"));

    let out = w.run(&[]).await;

    let entry = out.entry(&url);
    assert_eq!(entry.loc, "post.md:1");
    assert_eq!(entry.verdict, "UNVERIFIABLE 403");
    assert_eq!(out.summary_line(), summary_with(0, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s3_strict_reports_one_entry() {
    let w = wall_world().await;

    let out = w.run(&["--strict"]).await;

    let entries = out.entries();
    assert_eq!(entries.len(), 1, "stdout:\n{}", out.stdout);
    assert_eq!(entries[0].loc, "post.md:1");
    assert_eq!(entries[0].verdict, "BLOCKED 403");
    assert_eq!(entries[0].url, w.url("/wall/page"));
    assert_eq!(entries[0].detail, None);
    assert!(out.summary_line().contains(", blocked 1, "));
    out.assert_code(1);
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s4_json_lists_blocked_only_under_strict() {
    let w = wall_world().await;

    let plain = parse(&w.run(&["--format", "json"]).await);
    assert_eq!(plain["findings"], json!([]));
    assert_eq!(plain["summary"]["blocked"], 1);

    let strict = parse(&w.run(&["--strict", "--format", "json"]).await);
    assert_eq!(
        strict["findings"],
        json!([{
            "file": "post.md",
            "line": 1,
            "url": w.url("/wall/page"),
            "class": "blocked",
            "chain": [],
            "reason": "403",
            "final_url": null,
            "fix": null,
        }])
    );
    assert_eq!(strict["summary"]["blocked"], 1);
    assert_eq!(strict["summary"]["unverifiable"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s5_probe_goes_to_the_final_hops_folder() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a", redirect(302, &w.url("/q/1")))
            .fixed_re("^/q/", status(999)),
    )
    .await;
    let url = w.url("/a");
    w.site.write("post.md", &format!("[x]({url})\n"));

    let out = w.run(&["--strict"]).await;

    assert_eq!(out.entry(&url).verdict, "BLOCKED 999");
    assert!(
        !probes_under(&w, "/q/").is_empty(),
        "requests: {:?}",
        w.log.all()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s6_slow_probe_leaves_the_finding_unverifiable() {
    let mut w = World::new().await;
    w.configure(|s| s.timeout_ms = Some(200));
    w.serve(
        Routes::new()
            .fixed("/w/page", status(403))
            .fixed_re("^/w/.+", status(403).set_delay(Duration::from_secs(1))),
    )
    .await;
    let url = w.url("/w/page");
    w.site.write("post.md", &format!("[x]({url})\n"));

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&url).verdict, "UNVERIFIABLE 403");
    assert!(out.summary_line().contains(", blocked 0, "));
    assert!(
        !probes_under(&w, "/w/").is_empty(),
        "requests: {:?}",
        w.log.all()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s7_one_probe_per_folder_even_with_soft404_off() {
    let mut w = World::new().await;
    w.configure(|s| s.no_soft404_hosts = vec!["127.0.0.1".to_owned()]);
    w.serve(Routes::new().fixed_re("^/wall/", status(403)))
        .await;
    w.site.write(
        "post.md",
        &format!("[a]({})\n\n[b]({})\n", w.url("/wall/a"), w.url("/wall/b")),
    );

    let out = w.run(&[]).await;

    assert!(
        out.summary_line().contains(", blocked 2, "),
        "{}",
        out.stdout
    );
    let probes = probes_under(&w, "/wall/");
    assert_eq!(probes.len(), 1, "probes: {probes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_s8_apply_leaves_the_file_unchanged() {
    let w = wall_world().await;
    let before = std::fs::read(w.site.path().join("post.md")).expect("read post.md");

    let out = w.run(&["--strict", "--apply"]).await;

    assert_eq!(out.entry(&w.url("/wall/page")).verdict, "BLOCKED 403");
    let after = std::fs::read(w.site.path().join("post.md")).expect("read post.md");
    assert_eq!(before, after);
}

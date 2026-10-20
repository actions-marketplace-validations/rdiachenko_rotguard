//! Simplify design: which positions `--apply` may rewrite, and where front-matter links are reported.

use crate::support::{Cdx, Routes, World, page, redirect, status};

const NOT_VERBATIM: &str = "manual fix: not verbatim";

/// Serves `/old` and `/old2` as 301 to `/new`, which answers 200.
async fn moved_world() -> World {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/old2", redirect(301, &w.url("/new")))
            .fixed("/new", page("New", "new home")),
    )
    .await;
    w
}

/// Serves `/gone` as 404 with no archived snapshot.
async fn dead_world() -> World {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    w.archive(Cdx::new(vec![]));
    w
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S1"]
async fn simplify_s1_yaml_front_matter_is_left_alone() {
    let w = moved_world().await;
    let (old, old2, new) = (w.url("/old"), w.url("/old2"), w.url("/new"));
    w.site.write(
        "post.md",
        &format!("---\nsource: \"{old}\"\n---\n\n[x]({old2})\n"),
    );

    let out = w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("---\nsource: \"{old}\"\n---\n\n[x]({new})\n")
    );
    assert_eq!(out.entry(&old).detail.as_deref(), Some(NOT_VERBATIM));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S2"]
async fn simplify_s2_html_href_is_left_alone() {
    let w = moved_world().await;
    let old = w.url("/old");
    let text = format!("<a href=\"{old}\">x</a>\n");
    w.site.write("post.md", &text);

    let out = w.run(&["--apply"]).await;

    assert_eq!(w.site.read("post.md"), text);
    assert_eq!(out.entry(&old).detail.as_deref(), Some(NOT_VERBATIM));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S3"]
async fn simplify_s3_bare_url_is_left_alone() {
    let w = moved_world().await;
    let old = w.url("/old");
    let text = format!("see {old} here\n");
    w.site.write("post.md", &text);

    let out = w.run(&["--apply"]).await;

    assert_eq!(w.site.read("post.md"), text);
    assert_eq!(out.entry(&old).detail.as_deref(), Some(NOT_VERBATIM));
}

#[tokio::test(flavor = "multi_thread")]
async fn simplify_s4_toml_front_matter_values_are_rewritten() {
    let w = moved_world().await;
    let (old, old2, new) = (w.url("/old"), w.url("/old2"), w.url("/new"));
    w.site.write(
        "post.md",
        &format!("+++\nsource = \"{old}\"\nurl = '{old2}'\n+++\n\nbody\n"),
    );

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("+++\nsource = \"{new}\"\nurl = '{new}'\n+++\n\nbody\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn simplify_s5_nested_yaml_link_is_reported_with_its_line() {
    let w = dead_world().await;
    let gone = w.url("/gone");
    w.site
        .write("post.md", &format!("---\nmeta:\n  link: \"{gone}\"\n---\n"));

    let out = w.run(&[]).await;

    let entry = out.entry(&gone);
    assert_eq!(entry.loc, "post.md:3");
    assert_eq!(entry.verdict, "DEAD 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn simplify_s8_same_url_on_two_yaml_lines_gets_two_entries() {
    let w = dead_world().await;
    let gone = w.url("/gone");
    w.site.write(
        "post.md",
        &format!("---\na: \"{gone}\"\nb: \"{gone}\"\n---\n"),
    );

    let out = w.run(&[]).await;

    let found: Vec<(String, String)> = out
        .entries_for(&gone)
        .into_iter()
        .map(|e| (e.loc, e.verdict))
        .collect();
    assert_eq!(
        found,
        [
            ("post.md:2".to_owned(), "DEAD 404".to_owned()),
            ("post.md:3".to_owned(), "DEAD 404".to_owned()),
        ]
    );
}

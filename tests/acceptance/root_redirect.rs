//! Root redirects: a redirect chain ending at the site root is only SUSPECT when the root is
//! the final destination; an intermediate root hop does not hide a permanent fix.

use crate::support::{Routes, World, page, redirect};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S1"]
async fn root_redirect_s1_intermediate_root_hop_keeps_permanent_fix() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/archives/123", redirect(301, &w.url("/?p=123")))
            .fixed("/", redirect(301, &w.url("/2020/post/")))
            .fixed("/2020/post/", page("Post", "first post text here")),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/archives/123")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/archives/123"));
    assert_eq!(entry.verdict, "PERMANENT REDIRECT 301 301 200");
    assert_eq!(
        entry.detail.as_deref(),
        Some(format!("fix: {}", w.url("/2020/post/")).as_str())
    );
    for e in out.entries() {
        assert!(!e.verdict.starts_with("SUSPECT"), "{e:?}");
    }
    out.assert_code(1);
}

#[tokio::test(flavor = "multi_thread")]
async fn root_redirect_s2_temporary_hop_after_root_stays_suspect() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a", redirect(301, &w.url("/")))
            .fixed("/", redirect(302, &w.url("/b")))
            .fixed("/b", page("B", "page b words")),
    )
    .await;
    let original = format!("[x]({})\n", w.url("/a"));
    w.site.write("post.md", &original);

    let out = w.run(&["--apply"]).await;

    let entry = out.entry(&w.url("/a"));
    assert_eq!(entry.verdict, "SUSPECT redirect to root");
    assert!(
        !out.stdout.contains("PERMANENT REDIRECT"),
        "stdout:\n{}",
        out.stdout
    );
    assert_eq!(w.site.read("post.md"), original);
}

#[tokio::test(flavor = "multi_thread")]
async fn root_redirect_s3_chain_ending_at_root_is_suspect() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/new", redirect(301, &w.url("/")))
            .fixed("/", page("Home", "welcome")),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/old")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/old"));
    assert_eq!(entry.verdict, "SUSPECT redirect to root");
    assert!(
        !out.stdout.contains("PERMANENT REDIRECT"),
        "stdout:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn root_redirect_s4_redirect_loop_through_root_is_suspect() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/deep/loop", redirect(301, &w.url("/")))
            .fixed("/", redirect(301, &w.url("/deep/loop"))),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/deep/loop")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/deep/loop"));
    assert_eq!(entry.verdict, "SUSPECT redirect to root");
}

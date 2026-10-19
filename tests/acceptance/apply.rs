//! R20-R23: writing fixes back to the site.

use std::path::PathBuf;

use crate::support::{Capture, Cdx, Routes, Summary, World, page, redirect, status};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S32"]
async fn s32_dry_run_changes_no_file() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/new", page("New", "new home"))
            .fixed("/gone", status(404)),
    )
    .await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20200101000000", &gone)]));
    w.site
        .write("p1.md", &format!("[x]({}#sec)\n", w.url("/old")));
    w.site.write(
        "p2.md",
        &format!("---\ndate: \"2021-03-04\"\n---\n\n[y]({gone})\n"),
    );
    let before = w.site.snapshot();

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 2,
        permanent: 1,
        dead: 1,
        ..Summary::default()
    });
    assert_eq!(w.site.snapshot(), before);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S33"]
async fn s33_apply_rewrites_only_fixable_spans() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/new", page("New", "new home"))
            .fixed("/gone", status(404))
            .fixed("/deep/page", redirect(301, &w.url("/")))
            .fixed("/", page("Home", "welcome"))
            .fixed("/t1", redirect(302, &w.url("/t2")))
            .fixed("/t2", page("T2", "target")),
    )
    .await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20200101000000", &gone)]));
    let base = w.base.clone();
    let doc = |old: &str, dead: &str| {
        format!(
            "+++\ntitle = \"Post\"\n+++\n\n\
             [a]({old}) one\n\n\
             [b]({dead}) two\n\n\
             [c]({base}/deep/page) three\n\n\
             [d]({base}/t1) four\n"
        )
    };
    w.site.write("post.md", &doc(&w.url("/old"), &gone));

    let out = w.run(&["--apply"]).await;

    out.assert_summary(Summary {
        checked: 4,
        permanent: 1,
        temporary: 1,
        dead: 1,
        suspect: 1,
        ..Summary::default()
    });
    let expected = doc(&w.url("/new"), &w.snapshot_url("20200101000000", "/gone"));
    let after = w.site.read("post.md");
    assert_eq!(after, expected);
    let front = after
        .strip_prefix("+++\n")
        .and_then(|rest| rest.split("+++").next())
        .expect("front matter");
    front.parse::<toml::Table>().expect("front matter parses");
    let files: Vec<PathBuf> = w.site.snapshot().into_keys().collect();
    assert_eq!(
        files,
        [PathBuf::from("post.md"), PathBuf::from("rotguard.toml")]
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S34"]
async fn s34_file_changed_during_run_is_left_alone() {
    let w = World::new().await;
    let post = w.site.path().join("post.md");
    let (old, other, new) = (w.url("/old"), w.url("/other"), w.url("/new"));
    w.serve(
        Routes::new()
            .with("/old", move |_, _| {
                let text = std::fs::read_to_string(&post).expect("read post");
                std::fs::write(&post, text.replace(&old, &other)).expect("rewrite post");
                redirect(301, &new)
            })
            .fixed("/new", page("New", "new home")),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/old")));

    let out = w.run(&["--apply"]).await;

    out.assert_summary(Summary {
        checked: 1,
        permanent: 1,
        ..Summary::default()
    });
    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/other"))
    );
    assert!(
        out.stdout.contains("changed during run"),
        "stdout:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S35"]
async fn s35_unquotable_snapshot_is_manual_fix() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::with_original(
        "20200101000000",
        &gone,
        &format!("{gone}'s"),
    )]));
    let text = format!("+++\nsource = '{gone}'\n+++\n");
    w.site.write("post.md", &text);

    let out = w.run(&["--apply"]).await;

    out.assert_summary(Summary {
        checked: 1,
        dead: 1,
        ..Summary::default()
    });
    assert_eq!(w.site.read("post.md"), text);
    assert!(
        out.stdout.contains("manual fix: cannot quote"),
        "stdout:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S36"]
async fn s36_escaped_link_is_manual_fix_and_others_still_apply() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a_b", redirect(301, &w.url("/c")))
            .fixed("/c", page("C", "c page"))
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/new", page("New", "new home")),
    )
    .await;
    let escaped = format!("[x]({}/a\\_b)\n\n", w.base);
    w.site
        .write("post.md", &format!("{escaped}[y]({})\n", w.url("/old")));

    let out = w.run(&["--apply"]).await;

    out.assert_summary(Summary {
        checked: 2,
        permanent: 2,
        ..Summary::default()
    });
    assert_eq!(
        w.site.read("post.md"),
        format!("{escaped}[y]({})\n", w.url("/new"))
    );
    let entry = out.entry(&w.url("/a_b"));
    assert!(
        entry
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with("manual fix")),
        "{entry:?}"
    );
    assert!(
        !out.stdout.contains("changed during run"),
        "stdout:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S45"]
async fn s45_unbalanced_paren_target_is_manual_fix() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/p", redirect(301, &w.url("/a)b")))
            .fixed("/a)b", page("AB", "target")),
    )
    .await;
    let text = format!("[x]({})\n", w.url("/p"));
    w.site.write("post.md", &text);

    let out = w.run(&["--apply"]).await;

    out.assert_summary(Summary {
        checked: 1,
        permanent: 1,
        ..Summary::default()
    });
    assert_eq!(w.site.read("post.md"), text);
    assert!(
        out.stdout.contains("manual fix: cannot quote"),
        "stdout:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S42"]
async fn s42_only_the_value_span_changes() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a", redirect(301, &w.url("/new")))
            .fixed("/new", page("New", "new home"))
            .fixed("/ab", page("Ab", "other page")),
    )
    .await;
    let b = &w.base;
    let text =
        |source: &str| format!("+++\nsource = '{source}'  # also '{b}/a'\nother = '{b}/ab'\n+++\n");
    w.site.write("post.md", &text(&w.url("/a")));

    let out = w.run(&["--apply"]).await;

    out.assert_summary(Summary {
        checked: 2,
        ok: 1,
        permanent: 1,
        ..Summary::default()
    });
    assert_eq!(w.site.read("post.md"), text(&w.url("/new")));
}

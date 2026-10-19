//! R19, R24: report order, summary line and exit codes.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::support::{Routes, Summary, World, lines_at, page, redirect, status};

fn mixed_routes(w: &World) -> Routes {
    Routes::new()
        .fixed("/old", redirect(301, &w.url("/new")))
        .fixed("/new", page("New", "new home"))
        .fixed("/gone", status(404))
        .fixed("/deep/page", redirect(301, &w.url("/")))
        .fixed("/", page("Home", "welcome"))
        .fixed("/fine", page("Fine", "all good"))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S31"]
async fn s31_entries_sorted_and_summary_counts() {
    let w = World::new().await;
    w.serve(mixed_routes(&w)).await;
    w.site.write(
        "a.md",
        &lines_at(&[
            (2, format!("[p]({})", w.url("/old"))),
            (9, format!("[d]({})", w.url("/gone"))),
        ]),
    );
    w.site.write(
        "b.md",
        &lines_at(&[
            (3, format!("[o]({})", w.url("/fine"))),
            (7, format!("[s]({})", w.url("/deep/page"))),
        ]),
    );

    let out = w.run(&[]).await;

    let locs: Vec<String> = out.entries().into_iter().map(|e| e.loc).collect();
    assert_eq!(locs, ["a.md:2", "a.md:9", "b.md:7"]);
    assert_eq!(
        out.stdout.lines().last(),
        Some(
            "checked 4: ok 1, permanent 1, temporary 0, dead 1, suspect 1, unverifiable 0, \
             ignored 0"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S37"]
async fn s37_exit_codes() {
    let w = World::new().await;
    w.serve(mixed_routes(&w)).await;

    // Only ok links: 0.
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/fine")));
    let out = w.run(&[]).await;
    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    out.assert_code(0);

    // One dead link, no --apply: 1.
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/gone")));
    let out = w.run(&[]).await;
    out.assert_summary(Summary {
        checked: 1,
        dead: 1,
        ..Summary::default()
    });
    out.assert_code(1);

    // One permanent redirect with --apply: fixed, 0.
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/old")));
    let out = w.run(&["--apply"]).await;
    out.assert_summary(Summary {
        checked: 1,
        permanent: 1,
        ..Summary::default()
    });
    out.assert_code(0);
    assert_eq!(w.site.read("post.md"), format!("[x]({})\n", w.url("/new")));

    // An unreadable content file: 2.
    let path = w.site.path().join("post.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");
    let out = w.run(&[]).await;
    out.assert_code(2);
    assert!(!out.stderr.is_empty(), "stderr should give the reason");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S38"]
async fn s38_unverifiable_fails_only_with_strict() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/forbidden", status(403)))
        .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/forbidden")));

    let out = w.run(&[]).await;
    out.assert_summary(Summary {
        checked: 1,
        unverifiable: 1,
        ..Summary::default()
    });
    out.assert_code(0);

    let strict = w.run(&["--strict"]).await;
    strict.assert_summary(Summary {
        checked: 1,
        unverifiable: 1,
        ..Summary::default()
    });
    strict.assert_code(1);
}

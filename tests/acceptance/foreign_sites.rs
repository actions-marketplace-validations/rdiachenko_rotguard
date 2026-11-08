//! S1-S9: soft-404 probes on hosts that answer every URL alike.

use crate::support::{Routes, Summary, World, page, redirect};

/// `n` distinct words, `<prefix>0 <prefix>1 ...`; a different prefix gives text with no shared
/// 3-word windows.
fn words(prefix: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!("{prefix}{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn link_to(w: &World, path: &str) {
    w.site.write("post.md", &format!("[x]({})\n", w.url(path)));
}

#[tokio::test(flavor = "multi_thread")]
async fn s1_probe_redirected_to_the_link_is_no_evidence() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed(
                "/blog/15697/post/",
                page("Post", "the real article text of this post"),
            )
            .fixed_re(
                "^/blog/15697/[a-z0-9]{24}/$",
                redirect(302, &w.url("/blog/15697/post/")),
            ),
    )
    .await;
    link_to(&w, "/blog/15697/post/");

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn s2_redirected_link_ending_where_the_probe_ends_stays_suspect() {
    let w = World::new().await;
    let landing = redirect(302, &w.url("/docs/landing"));
    w.serve(
        Routes::new()
            .fixed("/docs/real", landing.clone())
            .fixed_re("^/docs/[a-z0-9]{24}$", landing.clone())
            .fixed("/", landing)
            .fixed("/docs/landing", page("Landing", "Please enable JavaScript")),
    )
    .await;
    link_to(&w, "/docs/real");

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/docs/real"));
    assert_eq!(entry.verdict, "SUSPECT probe: same final URL");
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_site_wide_title_over_different_pages_is_ok_without_a_root_request() {
    let w = World::new().await;
    let title = "Associates Central - Help";
    w.serve(
        Routes::new()
            .fixed(
                "/help/topic/g1",
                page(
                    title,
                    &format!(
                        "{} Please enable JavaScript for the full experience",
                        words("a", 400)
                    ),
                ),
            )
            .fixed_re("^/help/topic/[a-z0-9]{24}$", page(title, &words("b", 100))),
    )
    .await;
    link_to(&w, "/help/topic/g1");

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    assert!(
        w.log.hits("/").is_empty(),
        "root hits: {:?}",
        w.log.hits("/")
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S4"]
async fn s4_page_that_needs_javascript_is_unverifiable() {
    let w = World::new().await;
    let shell = page("App", "You need to enable JavaScript to run this app.");
    w.serve(
        Routes::new()
            .fixed("/signup", shell.clone())
            .fixed_re("^/[a-z0-9]{24}$", shell)
            .fixed("/", page("Welcome", &words("home", 30))),
    )
    .await;
    link_to(&w, "/signup");

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/signup"));
    assert_eq!(entry.verdict, "UNVERIFIABLE probe: page needs JavaScript");
    out.assert_code(0);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S5"]
async fn s5_root_that_matches_the_probe_is_unverifiable_without_a_lookup() {
    let w = World::new().await;
    let same = page("Reddit", "");
    w.serve(
        Routes::new()
            .fixed("/r/x/comments/1/post/", same.clone())
            .fixed_re("^/r/x/comments/1/[a-z0-9]{24}/$", same.clone())
            .fixed("/", same),
    )
    .await;
    link_to(&w, "/r/x/comments/1/post/");

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/r/x/comments/1/post/"));
    assert_eq!(
        entry.verdict,
        "UNVERIFIABLE probe: every URL gets the same page"
    );
    out.assert_code(0);
    assert!(
        w.alog.all().is_empty(),
        "archive requests: {:?}",
        w.alog.all()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S6"]
async fn s6_root_that_mostly_matches_the_probe_is_unverifiable() {
    let w = World::new().await;
    let title = "Frequently Asked Questions";
    let body = words("w", 200);
    // The first 185 words are shared, so about 85% of the 3-word windows are too.
    let root_body = format!("{} {}", words("w", 185), words("r", 15));
    w.serve(
        Routes::new()
            .fixed("/faqs", page(title, &body))
            .fixed_re("^/[a-z0-9]{24}$", page(title, &body))
            .fixed("/", page(title, &root_body)),
    )
    .await;
    link_to(&w, "/faqs");

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/faqs"));
    assert_eq!(
        entry.verdict,
        "UNVERIFIABLE probe: every URL gets the same page"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S7"]
async fn s7_root_unlike_the_probe_keeps_the_suspect() {
    let w = World::new().await;
    let body = words("w", 200);
    w.serve(
        Routes::new()
            .fixed("/blog/gone", page("Oops", &body))
            .fixed_re("^/blog/[a-z0-9]{24}$", page("Oops", &body))
            .fixed("/", page("Home", &words("h", 50))),
    )
    .await;
    link_to(&w, "/blog/gone");

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/blog/gone"));
    assert_eq!(entry.verdict, "SUSPECT probe: same title");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S8"]
async fn s8_not_found_title_wins_over_unverifiable() {
    let w = World::new().await;
    let same = page("Page not found", "");
    w.serve(
        Routes::new()
            .fixed("/z/a", same.clone())
            .fixed_re("^/z/[a-z0-9]{24}$", same.clone())
            .fixed("/", same),
    )
    .await;
    link_to(&w, "/z/a");

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/z/a"));
    assert_eq!(entry.verdict, "SUSPECT title says \"not found\"");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S9"]
async fn s9_root_is_fetched_once_per_origin() {
    let w = World::new().await;
    let same = page("Shared Site", &words("w", 100));
    w.serve(
        Routes::new()
            .fixed("/a/x", same.clone())
            .fixed("/b/y", same.clone())
            .fixed_re("^/a/[a-z0-9]{24}$", same.clone())
            .fixed_re("^/b/[a-z0-9]{24}$", same.clone())
            .fixed("/", same),
    )
    .await;
    w.site.write(
        "post.md",
        &format!("[x]({}) [y]({})\n", w.url("/a/x"), w.url("/b/y")),
    );

    let out = w.run(&[]).await;

    for path in ["/a/x", "/b/y"] {
        let entry = out.entry(&w.url(path));
        assert!(entry.verdict.starts_with("UNVERIFIABLE"), "{entry:?}");
    }
    assert_eq!(w.log.hits("/").len(), 1, "root hits: {:?}", w.log.hits("/"));
}

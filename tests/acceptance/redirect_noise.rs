//! Redirect noise: a permanent redirect that only adds query parameters or a locale segment is
//! kept as written, a real move is rewritten to the URL before the noise, and `utm_*` pairs the
//! redirect added are stripped when the stripped URL proves itself. Design scenarios:
//!
//! - S1 to S3: added or reordered parameters are kept.
//! - S4, S5: a changed or dropped parameter is a real move.
//! - S6: a trailing slash around a locale segment is kept.
//! - S7 to S10: real moves are rewritten as before.
//! - S11 to S13: an inserted locale segment is kept.
//! - S14: a segment that is not locale-shaped is a real move.
//! - S15: a trailing slash plus a parameter is kept.
//! - S16: a real move before noise is rewritten to the URL before the noise.
//! - S17: noise before a real step is kept.
//! - S18: noise after the root is `SUSPECT redirect to root`.
//! - S19 to S22: `utm_*` pairs a redirect added are stripped, with proof.
//! - S23 to S26: a failed proof keeps the link.
//! - S27: a deadline during the proof is `UNVERIFIABLE deadline`.
//! - S28: the strip also runs on a host with `soft404 = false`.
//! - S29: a kept link still goes through the soft-404 checks.
//! - S30: a kept link changes no exit code under `--strict` and is not in the JSON findings.
//! - S31, S32: request counts for a kept link and for a fix with no `utm_*`.
//! - S33: the link's fragment survives a strip.

use std::time::Duration;

use serde_json::Value;
use wiremock::{Request, ResponseTemplate};

use crate::support::{Out, Routes, Summary, World, page, redirect, status};

/// A handler for a link that redirects only when the query is exactly `own_query` (`None` for
/// no query) and answers `then` for any other query.
fn hop(
    to: String,
    own_query: Option<&'static str>,
    then: ResponseTemplate,
) -> impl Fn(&Request, usize) -> ResponseTemplate + Send + Sync + 'static {
    move |req, _| {
        if req.url.query() == own_query {
            redirect(301, &to)
        } else {
            then.clone()
        }
    }
}

/// Writes `post.md` with 1 link to `url` and returns the file's text.
fn link(w: &World, url: &str) -> String {
    let text = format!("[x]({url})\n");
    w.site.write("post.md", &text);
    text
}

/// The design's "Kept": no entry for the link, `post.md` unchanged, 1 link checked and ok, exit 0.
fn assert_kept(w: &World, out: &Out, url: &str, original: &str) {
    assert!(out.entries_for(url).is_empty(), "stdout:\n{}", out.stdout);
    assert_eq!(w.site.read("post.md"), original);
    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    out.assert_code(0);
}

/// A rewrite: the entry's verdict and fix line, and `post.md` linking the fix.
fn assert_rewritten(w: &World, out: &Out, url: &str, verdict: &str, fix: &str) {
    let entry = out.entry(url);
    assert_eq!(entry.verdict, verdict);
    assert_eq!(
        entry.detail.as_deref(),
        Some(format!("fix: {fix}").as_str())
    );
    assert_eq!(w.site.read("post.md"), format!("[x]({fix})\n"));
}

/// The site of S19: `/old` redirects to a target with 2 `utm_*` pairs, and the target answers
/// the same page with and without a query.
async fn awin_site(w: &World) {
    w.serve(
        Routes::new()
            .fixed(
                "/old",
                redirect(
                    301,
                    &w.url("/us/getting-started?utm_medium=redirect&utm_source=old"),
                ),
            )
            .fixed(
                "/us/getting-started",
                page("Now Awin", "start earning with the network"),
            ),
    )
    .await;
}

/// The site of S26 and S27: the stripped URL answers after `delay`.
async fn slow_strip_site(w: &World, delay: Duration) {
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new?utm_source=old")))
            .with("/new", move |req, _| {
                if req.url.query().is_some() {
                    page("Guide", "how the guide works")
                } else {
                    page("Guide", "how the guide works").set_delay(delay)
                }
            }),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s1_added_parameter_is_kept_with_2_requests() {
    let w = World::new().await;
    w.serve(Routes::new().with(
        "/s/article/guide",
        hop(
            w.url("/s/article/guide?language=en_US"),
            None,
            page("Guide", "how the guide works"),
        ),
    ))
    .await;
    let url = w.url("/s/article/guide");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
    assert_eq!(w.log.hits("/s/article/guide").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s2_added_parameter_next_to_own_query_is_kept() {
    let w = World::new().await;
    w.serve(Routes::new().with(
        "/a",
        hop(
            w.url("/a?id=5&language=en_US"),
            Some("id=5"),
            page("Item", "item five details"),
        ),
    ))
    .await;
    let url = w.url("/a?id=5");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s3_reordered_parameters_with_additions_are_kept() {
    let w = World::new().await;
    w.serve(Routes::new().with(
        "/a",
        hop(
            w.url("/a?a=2&lang=en&b=1"),
            Some("b=1&a=2"),
            page("Item", "item details"),
        ),
    ))
    .await;
    let url = w.url("/a?b=1&a=2");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s4_changed_parameter_is_rewritten() {
    let w = World::new().await;
    w.serve(Routes::new().with(
        "/a",
        hop(w.url("/a?id=6"), Some("id=5"), page("Item", "item details")),
    ))
    .await;
    let url = w.url("/a?id=5");
    link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_rewritten(
        &w,
        &out,
        &url,
        "PERMANENT REDIRECT 301 200",
        &w.url("/a?id=6"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s5_dropped_parameter_is_rewritten() {
    let w = World::new().await;
    w.serve(Routes::new().with(
        "/a",
        hop(
            w.url("/a?id=5"),
            Some("id=5&x=1"),
            page("Item", "item details"),
        ),
    ))
    .await;
    let url = w.url("/a?id=5&x=1");
    link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/a?id=5"))
    );
    assert_eq!(out.entry(&url).verdict, "PERMANENT REDIRECT 301 200");
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s6_locale_segment_with_slash_change_is_kept() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed(
                "/news/storefront/",
                redirect(301, &w.url("/en-gb/news/storefront")),
            )
            .fixed(
                "/en-gb/news/storefront",
                redirect(301, &w.url("/en-gb/news/storefront/")),
            )
            .fixed(
                "/en-gb/news/storefront/",
                page("Storefront", "the storefront news"),
            ),
    )
    .await;
    let url = w.url("/news/storefront/");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s7_non_locale_segment_is_rewritten_with_1_request() {
    let w = World::new().await;
    let moved = "/en-US/docs/Web/HTTP/Reference/Status/302";
    w.serve(
        Routes::new()
            .fixed(
                "/en-US/docs/Web/HTTP/Status/302",
                redirect(301, &w.url(moved)),
            )
            .fixed(moved, page("302 Found", "the redirect status")),
    )
    .await;
    let url = w.url("/en-US/docs/Web/HTTP/Status/302");
    link(&w, &url);

    w.run(&["--apply"]).await;

    assert_eq!(w.site.read("post.md"), format!("[x]({})\n", w.url(moved)));
    assert_eq!(w.log.hits(moved).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s8_308_chain_is_rewritten_to_its_end() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed(
                "/help/en/articles/8619620",
                redirect(308, &w.url("/help/articles/8619620")),
            )
            .fixed(
                "/help/articles/8619620",
                redirect(308, &w.url("/help/articles/8619620-about-links")),
            )
            .fixed(
                "/help/articles/8619620-about-links",
                page("About links", "everything about links"),
            ),
    )
    .await;
    link(&w, &w.url("/help/en/articles/8619620"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/help/articles/8619620-about-links"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s9_trailing_slash_with_a_real_move_is_rewritten() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/pricing/", redirect(301, &w.url("/pricing/plans/")))
            .fixed("/pricing/plans/", page("Plans", "choose a plan")),
    )
    .await;
    link(&w, &w.url("/pricing/"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/pricing/plans/"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s10_added_trailing_slash_alone_is_rewritten() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/pricing", redirect(301, &w.url("/pricing/")))
            .fixed("/pricing/", page("Pricing", "what it costs")),
    )
    .await;
    link(&w, &w.url("/pricing"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/pricing/"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s11_locale_segment_in_the_middle_is_kept() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed(
                "/help/articles/9",
                redirect(301, &w.url("/help/de/articles/9")),
            )
            .fixed("/help/de/articles/9", page("Hilfe", "article nine")),
    )
    .await;
    let url = w.url("/help/articles/9");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s12_root_to_locale_root_is_kept() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", redirect(301, &w.url("/en-gb/")))
            .fixed("/en-gb/", page("Home", "welcome to the site")),
    )
    .await;
    let url = w.url("/");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s13_script_subtag_locale_is_kept() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/docs/x", redirect(301, &w.url("/docs/zh-Hans/x")))
            .fixed("/docs/zh-Hans/x", page("Docs", "the x document")),
    )
    .await;
    let url = w.url("/docs/x");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s14_three_letter_segment_is_a_real_move() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/docs/x", redirect(301, &w.url("/docs/eng/x")))
            .fixed("/docs/eng/x", page("Docs", "the x document")),
    )
    .await;
    link(&w, &w.url("/docs/x"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/docs/eng/x"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s15_trailing_slash_with_added_parameter_is_kept() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a", redirect(301, &w.url("/a/?lang=en")))
            .fixed("/a/", page("A", "the a page")),
    )
    .await;
    let url = w.url("/a");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s16_real_move_then_noise_is_rewritten_before_the_noise() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old/x", redirect(301, &w.url("/new/x")))
            .with(
                "/new/x",
                hop(
                    w.url("/new/x?language=en_US"),
                    None,
                    page("New x", "the x page"),
                ),
            ),
    )
    .await;
    let url = w.url("/old/x");
    link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_rewritten(
        &w,
        &out,
        &url,
        "PERMANENT REDIRECT 301 301 200",
        &w.url("/new/x"),
    );
    assert_eq!(w.log.hits("/new/x").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s17_noise_then_real_step_is_kept() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed(
                "/hc/articles/44",
                redirect(301, &w.url("/hc/en-us/articles/44")),
            )
            .fixed(
                "/hc/en-us/articles/44",
                redirect(301, &w.url("/hc/en-us/articles/44-getting-started")),
            )
            .fixed(
                "/hc/en-us/articles/44-getting-started",
                page("Getting started", "how to start"),
            ),
    )
    .await;
    let url = w.url("/hc/articles/44");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s18_noise_after_the_root_is_suspect() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/promo", redirect(301, &w.url("/")))
            .fixed("/", redirect(301, &w.url("/en-gb/")))
            .fixed("/en-gb/", page("Home", "welcome to the site")),
    )
    .await;
    let url = w.url("/promo");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_eq!(out.entry(&url).verdict, "SUSPECT redirect to root");
    assert_eq!(w.site.read("post.md"), original);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S19"]
async fn redirect_noise_s19_added_utm_pairs_are_stripped_with_proof() {
    let w = World::new().await;
    awin_site(&w).await;
    let url = w.url("/old");
    link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_rewritten(
        &w,
        &out,
        &url,
        "PERMANENT REDIRECT 301 200",
        &w.url("/us/getting-started"),
    );
    out.assert_code(0);
    let under_us: Vec<_> = w
        .log
        .all()
        .into_iter()
        .filter(|r| r.path.starts_with("/us/"))
        .collect();
    assert_eq!(under_us.len(), 2, "{under_us:?}");
    assert!(under_us.iter().all(|r| r.path == "/us/getting-started"));
    let queries: Vec<_> = under_us.iter().map(|r| r.query.as_deref()).collect();
    assert!(queries.contains(&None), "{queries:?}");
    assert!(
        queries.contains(&Some("utm_medium=redirect&utm_source=old")),
        "{queries:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S20"]
async fn redirect_noise_s20_utm_pair_the_link_has_stays() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .with(
                "/old",
                hop(
                    w.url("/new?utm_source=mine&utm_medium=redirect"),
                    Some("utm_source=mine"),
                    page("Never", "unreachable"),
                ),
            )
            .fixed("/new", page("New", "the new page")),
    )
    .await;
    link(&w, &w.url("/old?utm_source=mine"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/new?utm_source=mine"))
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S21"]
async fn redirect_noise_s21_other_pairs_stay_byte_for_byte() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed(
                "/old",
                redirect(301, &w.url("/new?id=7&utm_source=old&ref=x")),
            )
            .fixed("/new", page("New", "the new page")),
    )
    .await;
    link(&w, &w.url("/old"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/new?id=7&ref=x"))
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S22"]
async fn redirect_noise_s22_stripped_url_with_a_noise_first_step_counts_as_ok() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new?utm_source=old")))
            .with(
                "/new",
                hop(
                    w.url("/new?utm_source=old"),
                    None,
                    page("New", "the new page"),
                ),
            ),
    )
    .await;
    link(&w, &w.url("/old"));

    w.run(&["--apply"]).await;

    assert_eq!(w.site.read("post.md"), format!("[x]({})\n", w.url("/new")));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S23"]
async fn redirect_noise_s23_stripped_url_404_keeps_the_link() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new?utm_source=old")))
            .with("/new", |req, _| {
                if req.url.query().is_some() {
                    page("New", "the new page")
                } else {
                    status(404)
                }
            }),
    )
    .await;
    let url = w.url("/old");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S24"]
async fn redirect_noise_s24_stripped_url_redirecting_elsewhere_keeps_the_link() {
    let w = World::new().await;
    let elsewhere = w.url("/elsewhere");
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new?utm_source=old")))
            .with("/new", move |req, _| {
                if req.url.query().is_some() {
                    page("New", "the new page")
                } else {
                    redirect(302, &elsewhere)
                }
            })
            .fixed("/elsewhere", page("New", "the new page")),
    )
    .await;
    let url = w.url("/old");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S25"]
async fn redirect_noise_s25_different_title_keeps_the_link() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new?utm_source=old")))
            .with("/new", |req, _| {
                if req.url.query().is_some() {
                    page("Guide", "how the guide works")
                } else {
                    page("Welcome", "welcome to the site")
                }
            }),
    )
    .await;
    let url = w.url("/old");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S26"]
async fn redirect_noise_s26_stripped_url_timeout_keeps_the_link_without_retry() {
    let mut w = World::new().await;
    w.configure(|s| s.timeout_ms = Some(300));
    slow_strip_site(&w, Duration::from_secs(1)).await;
    let url = w.url("/old");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
    let bare: Vec<_> = w
        .log
        .hits("/new")
        .into_iter()
        .filter(|r| r.query.is_none())
        .collect();
    assert_eq!(bare.len(), 1, "{bare:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S27"]
async fn redirect_noise_s27_deadline_during_the_strip_is_unverifiable() {
    let mut w = World::new().await;
    w.configure(|s| {
        s.deadline_s = Some(1);
        s.timeout_ms = Some(5_000);
    });
    slow_strip_site(&w, Duration::from_secs(5)).await;
    let url = w.url("/old");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_eq!(out.entry(&url).verdict, "UNVERIFIABLE deadline");
    assert_eq!(w.site.read("post.md"), original);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S28"]
async fn redirect_noise_s28_strip_runs_with_soft404_off() {
    let mut w = World::new().await;
    w.configure(|s| s.no_soft404_hosts = vec!["127.0.0.1".to_owned()]);
    awin_site(&w).await;
    link(&w, &w.url("/old"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/us/getting-started"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s29_soft_404_checks_still_run_on_noise() {
    let w = World::new().await;
    let base = w.base.clone();
    w.serve(Routes::new().with_re("^/s/article/", move |req, _| {
        if req.url.query().is_none() {
            redirect(301, &format!("{base}{}?language=en_US", req.url.path()))
        } else {
            page("Page not found", "this page does not exist")
        }
    }))
    .await;
    let url = w.url("/s/article/gone");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert!(
        out.entry(&url).verdict.starts_with("SUSPECT"),
        "stdout:\n{}",
        out.stdout
    );
    assert_eq!(w.site.read("post.md"), original);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s30_kept_link_is_not_a_finding_under_strict() {
    let w = World::new().await;
    w.serve(Routes::new().with(
        "/s/article/guide",
        hop(
            w.url("/s/article/guide?language=en_US"),
            None,
            page("Guide", "how the guide works"),
        ),
    ))
    .await;
    link(&w, &w.url("/s/article/guide"));

    let out = w.run(&["--strict", "--format", "json"]).await;

    let v: Value = serde_json::from_str(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON value: {e}\nstdout:\n{}\nstderr:\n{}",
            out.stdout, out.stderr
        )
    });
    assert_eq!(v["findings"].as_array().map(Vec::len), Some(0), "{v}");
    assert_eq!(v["summary"]["ok"], 1);
    assert_eq!(v["summary"]["permanent"], 0);
    out.assert_code(0);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s31_utm_noise_from_the_link_is_kept_with_2_requests() {
    let w = World::new().await;
    w.serve(Routes::new().with(
        "/a",
        hop(w.url("/a?utm_source=x"), None, page("A", "the a page")),
    ))
    .await;
    let url = w.url("/a");
    let original = link(&w, &url);

    let out = w.run(&["--apply"]).await;

    assert_kept(&w, &out, &url, &original);
    assert_eq!(w.log.hits("/a").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_noise_s32_fix_without_utm_pairs_sends_no_strip_request() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new?ref=old")))
            .fixed("/new", page("New", "the new page")),
    )
    .await;
    link(&w, &w.url("/old"));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({})\n", w.url("/new?ref=old"))
    );
    assert_eq!(w.log.hits("/new").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S33"]
async fn redirect_noise_s33_fragment_survives_a_strip() {
    let w = World::new().await;
    awin_site(&w).await;
    link(&w, &format!("{}#start", w.url("/old")));

    w.run(&["--apply"]).await;

    assert_eq!(
        w.site.read("post.md"),
        format!("[x]({}#start)\n", w.url("/us/getting-started"))
    );
}

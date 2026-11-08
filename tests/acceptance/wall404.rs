//! 404 walls: a 404 or 410 is only DEAD when the host's root is not a wall too; redirect hops
//! without a `Location` name the last good hop.

use std::time::Duration;

use wiremock::{MockServer, ResponseTemplate};

use crate::support::{Capture, Cdx, Routes, Summary, World, page, redirect, status};

const WALL: &str = "UNVERIFIABLE 404, host root answers 404 too";

/// Host M answers every path with the same 404 page.
fn wall() -> Routes {
    Routes::new().fixed_re(".*", status(404).set_body_string("nothing here"))
}

/// Writes `post.md` linking each of `paths` on M, one per line.
fn link(w: &World, paths: &[&str]) {
    let body: String = paths
        .iter()
        .map(|p| format!("[x]({})\n", w.url(p)))
        .collect();
    w.site.write("post.md", &body);
}

/// A response with the given redirect status and no `Location` header.
fn no_location(code: u16) -> ResponseTemplate {
    ResponseTemplate::new(code)
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s1_wall_host_is_unverifiable() {
    let w = World::new().await;
    w.serve(wall()).await;
    link(&w, &["/guides/page"]);
    let before = w.site.snapshot();

    let out = w.run(&["--apply"]).await;

    assert_eq!(out.entry(&w.url("/guides/page")).verdict, WALL);
    assert_eq!(w.site.snapshot(), before);
    assert!(w.alog.all().is_empty(), "archive: {:?}", w.alog.all());
    out.assert_summary(Summary {
        checked: 1,
        unverifiable: 1,
        ..Summary::default()
    });
    out.assert_code(0);
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s2_strict_fails_and_leaves_the_file() {
    let w = World::new().await;
    w.serve(wall()).await;
    let url = w.url("/guides/page");
    w.archive(Cdx::new(vec![Capture::of("20200101000000", &url)]));
    link(&w, &["/guides/page"]);
    let before = w.site.snapshot();

    let out = w.run(&["--apply", "--strict"]).await;

    assert_eq!(out.entry(&url).verdict, WALL);
    assert_eq!(w.site.snapshot(), before);
    out.assert_code(1);
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s3_markdown_report_says_could_not_check() {
    let w = World::new().await;
    w.serve(wall()).await;
    link(&w, &["/guides/page"]);

    let out = w.run(&["--format", "markdown"]).await;

    let expected = "could not check (404, host root answers 404 too)";
    assert!(
        out.stdout.contains(expected) && out.stdout.contains(&w.url("/guides/page")),
        "stdout:\n{}",
        out.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s4_root_410_names_its_status() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", status(410))
            .fixed("/gone", status(404)),
    )
    .await;
    link(&w, &["/gone"]);

    let out = w.run(&[]).await;

    assert_eq!(
        out.entry(&w.url("/gone")).verdict,
        "UNVERIFIABLE 404, host root answers 410 too"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s5_linked_root_is_unverifiable() {
    let w = World::new().await;
    w.serve(wall()).await;
    link(&w, &["/"]);
    let before = w.site.snapshot();

    let out = w.run(&["--apply"]).await;

    assert_eq!(out.entry(&w.url("/")).verdict, WALL);
    assert_eq!(w.site.snapshot(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s6_live_root_keeps_dead() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20200101000000", &gone)]));
    link(&w, &["/gone"]);

    let out = w.run(&["--apply"]).await;

    assert_eq!(out.entry(&gone).verdict, "DEAD 404");
    assert!(
        w.site
            .read("post.md")
            .contains(&w.snapshot_url("20200101000000", "/gone")),
        "post.md:\n{}",
        w.site.read("post.md")
    );
    out.assert_code(0);
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s7_root_503_keeps_dead() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", status(503))
            .fixed("/gone", status(404)),
    )
    .await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20200101000000", &gone)]));
    link(&w, &["/gone"]);

    let out = w.run(&["--apply"]).await;

    assert_eq!(out.entry(&gone).verdict, "DEAD 404");
    assert!(
        w.site
            .read("post.md")
            .contains(&w.snapshot_url("20200101000000", "/gone")),
        "post.md:\n{}",
        w.site.read("post.md")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s8_root_403_keeps_dead() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", status(403))
            .fixed("/gone", status(404)),
    )
    .await;
    link(&w, &["/gone"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/gone")).verdict, "DEAD 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s9_slow_root_keeps_dead() {
    let mut w = World::new().await;
    // Room for 2 link GETs, the recheck delay and 1 root timeout; the root fetch has no retries.
    w.configure(|s| {
        s.timeout_ms = Some(300);
        s.deadline_s = Some(30);
    });
    w.serve(
        Routes::new()
            .fixed("/", page("Home", "late").set_delay(Duration::from_secs(1)))
            .fixed("/gone", status(404)),
    )
    .await;
    link(&w, &["/gone"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/gone")).verdict, "DEAD 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s10_root_redirect_to_404_is_a_wall() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", redirect(301, &w.url("/home")))
            .fixed("/home", status(404))
            .fixed("/gone", status(404)),
    )
    .await;
    link(&w, &["/gone"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/gone")).verdict, WALL);
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s11_root_redirect_to_live_page_keeps_dead() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", redirect(302, &w.url("/home")))
            .fixed("/home", page("Home", "welcome"))
            .fixed("/gone", status(404)),
    )
    .await;
    link(&w, &["/gone"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/gone")).verdict, "DEAD 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s12_redirect_into_a_wall_host_is_unverifiable() {
    let w = World::new().await;
    // Nothing is mounted on W, so wiremock answers every request there with 404.
    let wall_host = MockServer::start().await;
    w.serve(Routes::new().fixed("/old", redirect(301, &format!("{}/gone", wall_host.uri()))))
        .await;
    link(&w, &["/old"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/old")).verdict, WALL);
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s13_one_root_fetch_per_host() {
    let w = World::new().await;
    w.serve(wall()).await;
    link(&w, &["/a", "/b", "/c"]);

    let out = w.run(&[]).await;

    for path in ["/a", "/b", "/c"] {
        assert_eq!(out.entry(&w.url(path)).verdict, WALL, "{path}");
    }
    assert_eq!(w.log.hits("/").len(), 1, "requests: {:?}", w.log.all());
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s14_healthy_host_never_fetches_its_root() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", page("Home", "welcome"))
            .fixed("/ok", page("Ok", "fine page")),
    )
    .await;
    link(&w, &["/ok"]);

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    assert!(w.log.hits("/").is_empty(), "requests: {:?}", w.log.all());
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s15_soft404_off_still_checks_the_root() {
    let mut w = World::new().await;
    w.configure(|s| s.no_soft404_hosts = vec!["127.0.0.1".to_owned()]);
    w.serve(wall()).await;
    link(&w, &["/gone"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/gone")).verdict, WALL);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S16"]
async fn wall404_s16_no_location_names_the_last_good_hop() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed(
                "/the-infinite-dial-2020/",
                redirect(301, &w.url("/insights/the-infinite-dial-2020/")),
            )
            .fixed("/insights/the-infinite-dial-2020/", no_location(307)),
    )
    .await;
    link(&w, &["/the-infinite-dial-2020/"]);
    let before = w.site.snapshot();

    let out = w.run(&["--apply"]).await;

    let entry = out.entry(&w.url("/the-infinite-dial-2020/"));
    assert_eq!(
        entry.verdict,
        format!(
            "UNVERIFIABLE no location after 301 to {}",
            w.url("/insights/the-infinite-dial-2020/")
        )
    );
    assert_eq!(entry.detail, None);
    assert_eq!(w.site.snapshot(), before);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S17"]
async fn wall404_s17_no_location_lists_every_good_status() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a", redirect(308, &w.url("/b")))
            .fixed("/b", redirect(301, &w.url("/c")))
            .fixed("/c", no_location(307)),
    )
    .await;
    link(&w, &["/a"]);

    let out = w.run(&[]).await;

    assert_eq!(
        out.entry(&w.url("/a")).verdict,
        format!("UNVERIFIABLE no location after 308 301 to {}", w.url("/c"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s18_first_hop_without_location_stays_plain() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/a", no_location(307))).await;
    link(&w, &["/a"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/a")).verdict, "UNVERIFIABLE no location");
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s19_temporary_hops_stay_plain() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a", redirect(302, &w.url("/b")))
            .fixed("/b", no_location(307)),
    )
    .await;
    link(&w, &["/a"]);

    let out = w.run(&[]).await;

    assert_eq!(out.entry(&w.url("/a")).verdict, "UNVERIFIABLE no location");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S20"]
async fn wall404_s20_no_location_after_a_permanent_hop() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/x", redirect(301, &w.url("/y")))
            .fixed("/y", no_location(301)),
    )
    .await;
    link(&w, &["/x"]);

    let out = w.run(&[]).await;

    assert_eq!(
        out.entry(&w.url("/x")).verdict,
        format!("UNVERIFIABLE no location after 301 to {}", w.url("/y"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wall404_s21_probe_and_dead_share_one_root_fetch() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/", page("Home", "welcome"))
            .fixed("/gone", status(404))
            .fixed_re("^/a/", page("Same", "same words")),
    )
    .await;
    link(&w, &["/a/page", "/gone"]);

    let out = w.run(&[]).await;

    assert_eq!(
        out.entry(&w.url("/a/page")).verdict,
        "SUSPECT probe: same title"
    );
    assert_eq!(out.entry(&w.url("/gone")).verdict, "DEAD 404");
    assert_eq!(w.log.hits("/").len(), 1, "requests: {:?}", w.log.all());
}

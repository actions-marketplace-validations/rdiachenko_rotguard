//! R15-R18: Wayback snapshot lookups.

use std::time::Duration;

use crate::support::{Capture, Cdx, Routes, Summary, World, assert_gaps, status};

fn gone_routes() -> Routes {
    Routes::new().fixed("/gone", status(404))
}

fn dated(date: &str, body: &str) -> String {
    format!("---\ndate: \"{date}\"\n---\n\n{body}\n")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S25"]
async fn s25_latest_capture_before_page_date_wins() {
    let w = World::new().await;
    w.serve(gone_routes()).await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![
        Capture::of("20200101000000", &gone),
        Capture::of("20220101000000", &gone),
    ]));
    w.site
        .write("post.md", &dated("2021-03-04", &format!("[x]({gone})")));

    let out = w.run(&[]).await;

    let entry = out.entry(&gone);
    assert!(entry.verdict.starts_with("DEAD"), "{entry:?}");
    assert_eq!(
        entry.detail,
        Some(format!(
            "fix: {}",
            w.snapshot_url("20200101000000", "/gone")
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S26"]
async fn s26_earliest_capture_after_page_date_when_none_before() {
    let w = World::new().await;
    w.serve(gone_routes()).await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20220101000000", &gone)]));
    w.site
        .write("post.md", &dated("2021-03-04", &format!("[x]({gone})")));

    let out = w.run(&[]).await;

    let entry = out.entry(&gone);
    assert_eq!(
        entry.detail,
        Some(format!(
            "fix: {}",
            w.snapshot_url("20220101000000", "/gone")
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S27"]
async fn s27_lookup_per_url_and_page_date() {
    let w = World::new().await;
    w.serve(gone_routes()).await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![
        Capture::of("20190101000000", &gone),
        Capture::of("20230101000000", &gone),
    ]));
    let link = format!("[x]({gone}#part)");
    w.site.write("p1.md", &dated("2019-05-01", &link));
    w.site.write("p2.md", &dated("2023-05-01", &link));

    let out = w.run(&[]).await;

    let entries = out.entries_for(&gone);
    assert_eq!(entries.len(), 2, "stdout:\n{}", out.stdout);
    let by_loc = |loc: &str| {
        entries
            .iter()
            .find(|e| e.loc == loc)
            .unwrap_or_else(|| panic!("no entry at {loc}; stdout:\n{}", out.stdout))
    };
    assert_eq!(
        by_loc("p1.md:5").detail,
        Some(format!(
            "fix: {}#part",
            w.snapshot_url("20190101000000", "/gone")
        ))
    );
    assert_eq!(
        by_loc("p2.md:5").detail,
        Some(format!(
            "fix: {}#part",
            w.snapshot_url("20230101000000", "/gone")
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S28"]
async fn s28_no_capture_is_manual_fix() {
    let w = World::new().await;
    w.serve(gone_routes()).await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/gone")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/gone"));
    assert_eq!(
        entry.detail.as_deref(),
        Some("manual fix: no archived copy")
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S29"]
async fn s29_archive_down_is_three_attempts_then_manual_fix() {
    let w = World::new().await;
    w.serve(gone_routes()).await;
    w.archive(Cdx::unavailable());
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/gone")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/gone"));
    assert_eq!(
        entry.detail.as_deref(),
        Some("manual fix: archive unreachable")
    );
    assert_eq!(
        w.alog.all().len(),
        3,
        "archive requests: {:?}",
        w.alog.all()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S30"]
async fn s30_archive_requests_are_spaced() {
    let mut w = World::new().await;
    w.configure(|s| s.archive_interval_ms = 300);
    w.serve(
        gone_routes()
            .fixed("/g2", status(404))
            .fixed("/g3", status(404)),
    )
    .await;
    let urls = [w.url("/gone"), w.url("/g2"), w.url("/g3")];
    w.archive(Cdx::new(
        urls.iter()
            .map(|u| Capture::of("20200101000000", u))
            .collect(),
    ));
    w.site.write(
        "post.md",
        &format!("[a]({})\n\n[b]({})\n\n[c]({})\n", urls[0], urls[1], urls[2]),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 3,
        dead: 3,
        ..Summary::default()
    });
    let requests = w.alog.all();
    assert!(requests.len() >= 3, "archive requests: {requests:?}");
    assert_gaps(&requests, Duration::from_millis(300));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S44"]
async fn s44_deadline_covers_archive_lookups() {
    let mut w = World::new().await;
    w.configure(|s| {
        s.deadline_s = Some(2);
        s.recheck_delay_ms = 200;
    });
    w.serve(gone_routes()).await;
    let gone = w.url("/gone");
    w.archive(Cdx::slow(
        vec![Capture::of("20200101000000", &gone)],
        Duration::from_secs(10),
    ));
    w.site.write("post.md", &format!("[x]({gone})\n"));

    let out = w.run(&[]).await;

    assert!(
        out.elapsed < Duration::from_secs(4),
        "took {:?}",
        out.elapsed
    );
    let entry = out.entry(&gone);
    assert!(entry.verdict.starts_with("DEAD"), "{entry:?}");
    assert_eq!(entry.detail.as_deref(), Some("manual fix: deadline"));
}

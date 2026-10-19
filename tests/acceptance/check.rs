//! R5-R10, R14, R15: fetching and classifying URLs.

use std::time::Duration;

use crate::support::{Routes, Summary, USER_AGENT, World, assert_gaps, page, redirect, status};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S7"]
async fn s7_checks_each_distinct_url_once() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/dup", page("Dup", "dup")))
        .await;
    let url = w.url("/dup");
    w.site.write("a.md", &format!("[x]({url})\n"));
    w.site.write("b.md", &format!("[y]({url})\n"));
    w.site.write("c.md", &format!("[z]({url}#x)\n"));

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    assert_eq!(w.log.hits("/dup").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S8"]
async fn s8_starts_requests_to_a_host_one_interval_apart() {
    let mut w = World::new().await;
    w.configure(|s| s.host_interval_ms = 300);
    // Instant replies: only `host_interval_ms` can keep consecutive starts apart.
    let routes = ["/p1", "/p2", "/p3"]
        .into_iter()
        .fold(Routes::new(), |r, p| r.fixed(p, page("P", "page")))
        .fixed_re(".*", status(404));
    w.serve(routes).await;
    w.site.write(
        "post.md",
        &format!(
            "[a]({})\n\n[b]({})\n\n[c]({})\n",
            w.url("/p1"),
            w.url("/p2"),
            w.url("/p3")
        ),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 3,
        ok: 3,
        ..Summary::default()
    });
    // Soft-404 probes are requests to the host too (R6: they wait at the host gate).
    let all = w.log.all();
    assert!(all.len() >= 3, "requests: {all:?}");
    assert_gaps(&all, Duration::from_millis(300));
    for req in &all {
        assert_eq!(req.method, "GET");
        assert_eq!(req.user_agent.as_deref(), Some(USER_AGENT), "{}", req.path);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S8"]
async fn s8_requests_to_a_host_do_not_overlap() {
    let mut w = World::new().await;
    w.configure(|s| s.host_interval_ms = 300);
    let delay = Duration::from_millis(700);
    let routes = ["/p1", "/p2", "/p3"]
        .into_iter()
        .fold(Routes::new(), |r, p| {
            r.fixed(p, page("P", "page").set_delay(delay))
        })
        .fixed_re(".*", status(404).set_delay(delay));
    w.serve(routes).await;
    w.site.write(
        "post.md",
        &format!(
            "[a]({})\n\n[b]({})\n\n[c]({})\n",
            w.url("/p1"),
            w.url("/p2"),
            w.url("/p3")
        ),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 3,
        ok: 3,
        ..Summary::default()
    });
    let all = w.log.all();
    assert!(all.len() >= 3, "requests: {all:?}");
    // The reply takes longer than the interval, so only one request in flight keeps the gap
    // at the reply time or more. (The interval itself is tested without delay above.)
    assert_gaps(&all, delay);
    for req in &all {
        assert_eq!(req.method, "GET");
        assert_eq!(req.user_agent.as_deref(), Some(USER_AGENT), "{}", req.path);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S9"]
async fn s9_dead_after_two_attempts_apart() {
    let mut w = World::new().await;
    w.configure(|s| s.recheck_delay_ms = 200);
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/gone")));

    let out = w.run(&[]).await;

    let hits = w.log.hits("/gone");
    assert_eq!(hits.len(), 2, "GETs for /gone");
    assert_gaps(&hits, Duration::from_millis(200));
    let entry = out.entry(&w.url("/gone"));
    assert!(entry.verdict.starts_with("DEAD"), "{entry:?}");
    out.assert_summary(Summary {
        checked: 1,
        dead: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S10"]
async fn s10_flaky_404_then_200_is_ok() {
    let w = World::new().await;
    w.serve(Routes::new().with("/flaky", |_, n| {
        if n == 0 {
            status(404)
        } else {
            page("Flaky", "back")
        }
    }))
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/flaky")));

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    out.assert_code(0);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S11"]
async fn s11_unresolvable_host_is_dead_dns() {
    let w = World::new().await;
    w.site
        .write("post.md", "[x](https://nonexistent.invalid/page)\n");

    let out = w.run(&[]).await;

    let entry = out.entry("https://nonexistent.invalid/page");
    assert!(entry.verdict.starts_with("DEAD"), "{entry:?}");
    assert!(entry.verdict.contains("dns"), "{entry:?}");
    out.assert_summary(Summary {
        checked: 1,
        dead: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S12"]
async fn s12_permanent_redirect_proposes_target_with_fragment() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/old", redirect(301, &w.url("/new")))
            .fixed("/new", page("New", "new home")),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({}#sec)\n", w.url("/old")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/old"));
    assert_eq!(entry.verdict, "PERMANENT REDIRECT 301 200");
    assert_eq!(entry.detail, Some(format!("fix: {}#sec", w.url("/new"))));
    out.assert_summary(Summary {
        checked: 1,
        permanent: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S13"]
async fn s13_fix_is_last_permanent_hop_of_leading_run() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/p1", redirect(308, &w.url("/p2")))
            .fixed("/p2", redirect(302, &w.url("/p3")))
            .fixed("/p3", page("P3", "end")),
    )
    .await;
    w.site.write("post.md", &format!("[x]({})\n", w.url("/p1")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/p1"));
    assert!(entry.verdict.starts_with("PERMANENT REDIRECT"), "{entry:?}");
    assert_eq!(entry.detail, Some(format!("fix: {}", w.url("/p2"))));
    out.assert_summary(Summary {
        checked: 1,
        permanent: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S14"]
async fn s14_temporary_redirect_reports_final_url() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/t1", redirect(302, &w.url("/t2")))
            .fixed("/t2", page("T2", "target")),
    )
    .await;
    w.site.write("post.md", &format!("[x]({})\n", w.url("/t1")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/t1"));
    assert!(entry.verdict.starts_with("TEMPORARY REDIRECT"), "{entry:?}");
    assert_eq!(entry.detail, Some(format!("final: {}", w.url("/t2"))));
    assert!(!out.stdout.contains("fix:"), "stdout:\n{}", out.stdout);
    out.assert_summary(Summary {
        checked: 1,
        temporary: 1,
        ..Summary::default()
    });
    out.assert_code(0);

    let strict = w.run(&["--strict"]).await;
    strict.assert_summary(Summary {
        checked: 1,
        temporary: 1,
        ..Summary::default()
    });
    strict.assert_code(1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S15"]
async fn s15_redirect_to_root_is_suspect_not_permanent() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/deep/page", redirect(301, &w.url("/")))
            .fixed("/", page("Home", "welcome")),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/deep/page")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/deep/page"));
    assert_eq!(entry.verdict, "SUSPECT redirect to root");
    assert_eq!(
        entry.detail.as_deref(),
        Some("manual fix: no archived copy")
    );
    assert!(
        !out.stdout.contains("PERMANENT REDIRECT"),
        "stdout:\n{}",
        out.stdout
    );
    out.assert_summary(Summary {
        checked: 1,
        suspect: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S43"]
async fn s43_temporary_redirect_to_root_is_suspect() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/deep/tmp", redirect(302, &w.url("/")))
            .fixed("/", page("Home", "welcome")),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/deep/tmp")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/deep/tmp"));
    assert!(entry.verdict.starts_with("SUSPECT"), "{entry:?}");
    assert!(
        !out.stdout.contains("TEMPORARY REDIRECT"),
        "stdout:\n{}",
        out.stdout
    );
    out.assert_summary(Summary {
        checked: 1,
        suspect: 1,
        ..Summary::default()
    });
    out.assert_code(1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S22"]
async fn s22_unverifiable_statuses_and_retries() {
    let mut w = World::new().await;
    w.configure(|s| s.timeout_ms = Some(200));
    w.serve(
        Routes::new()
            .fixed("/forbidden", status(403))
            .fixed("/err", status(503))
            .fixed(
                "/slow",
                page("Slow", "late").set_delay(Duration::from_secs(1)),
            ),
    )
    .await;
    w.site.write(
        "post.md",
        &format!(
            "[a]({})\n\n[b]({})\n\n[c]({})\n",
            w.url("/forbidden"),
            w.url("/err"),
            w.url("/slow")
        ),
    );

    let out = w.run(&[]).await;

    for (path, reason, gets) in [
        ("/forbidden", "403", 1),
        ("/err", "503", 3),
        ("/slow", "timeout", 3),
    ] {
        let entry = out.entry(&w.url(path));
        assert_eq!(entry.verdict, format!("UNVERIFIABLE {reason}"), "{path}");
        assert_eq!(entry.detail, None, "{path}");
        assert_eq!(w.log.hits(path).len(), gets, "GETs for {path}");
    }
    out.assert_summary(Summary {
        checked: 3,
        unverifiable: 3,
        ..Summary::default()
    });
    out.assert_code(0);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S23"]
async fn s23_429_retries_after_retry_after() {
    let w = World::new().await;
    w.serve(Routes::new().with("/busy", |_, n| {
        if n == 0 {
            status(429).insert_header("retry-after", "1")
        } else {
            page("Busy", "ready")
        }
    }))
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/busy")));

    let out = w.run(&[]).await;

    let hits = w.log.hits("/busy");
    assert_eq!(hits.len(), 2, "GETs for /busy");
    assert_gaps(&hits, Duration::from_secs(1));
    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S24"]
async fn s24_deadline_marks_unfinished_urls() {
    let mut w = World::new().await;
    w.configure(|s| {
        s.deadline_s = Some(1);
        s.timeout_ms = Some(10_000);
    });
    w.serve(Routes::new().fixed(
        "/hang",
        page("Hang", "slow").set_delay(Duration::from_secs(5)),
    ))
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/hang")));

    let out = w.run(&[]).await;

    assert!(
        out.elapsed < Duration::from_secs(3),
        "took {:?}",
        out.elapsed
    );
    let entry = out.entry(&w.url("/hang"));
    assert_eq!(entry.verdict, "UNVERIFIABLE deadline");
    out.assert_summary(Summary {
        checked: 1,
        unverifiable: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S41"]
async fn s41_chain_to_404_is_dead_and_odd_answers_unverifiable() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/moved", redirect(301, &w.url("/gone2")))
            .fixed("/gone2", status(404))
            .fixed("/bad", status(405))
            .fixed("/loop", redirect(301, &w.url("/loop"))),
    )
    .await;
    w.site.write(
        "post.md",
        &format!(
            "[a]({})\n\n[b]({})\n\n[c]({})\n",
            w.url("/moved"),
            w.url("/bad"),
            w.url("/loop")
        ),
    );

    let out = w.run(&[]).await;

    assert!(
        out.entry(&w.url("/moved")).verdict.starts_with("DEAD"),
        "stdout:\n{}",
        out.stdout
    );
    assert_eq!(out.entry(&w.url("/bad")).verdict, "UNVERIFIABLE 405");
    assert_eq!(
        out.entry(&w.url("/loop")).verdict,
        "UNVERIFIABLE redirect loop"
    );
    out.assert_summary(Summary {
        checked: 3,
        dead: 1,
        unverifiable: 2,
        ..Summary::default()
    });
}

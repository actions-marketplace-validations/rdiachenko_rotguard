//! R11-R13: soft-404 probes and not-found titles.

use crate::support::{Routes, Summary, World, page};

const DOCS_PROBE: &str = "^/docs/[a-z0-9]{24}$";

fn docs_routes() -> Routes {
    Routes::new()
        .fixed(
            "/docs/real",
            page("Docs home", "alpha beta gamma delta epsilon"),
        )
        .fixed_re(
            DOCS_PROBE,
            page("Docs home", "one two three four five six seven"),
        )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S16"]
async fn s16_probe_with_same_title_marks_suspect() {
    let w = World::new().await;
    w.serve(docs_routes()).await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/docs/real")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/docs/real"));
    assert!(entry.verdict.starts_with("SUSPECT"), "{entry:?}");
    let probes = w.log.probes();
    assert_eq!(probes.len(), 1, "probes: {probes:?}");
    let re = regex::Regex::new(DOCS_PROBE).expect("test regex");
    assert!(
        re.is_match(&probes[0].path),
        "probe path {}",
        probes[0].path
    );
    out.assert_summary(Summary {
        checked: 1,
        suspect: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S17"]
async fn s17_probe_404_clears_both_and_is_sent_once() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/docs/a", page("A", "first page"))
            .fixed("/docs/b", page("B", "second page")),
    )
    .await;
    w.site.write(
        "post.md",
        &format!("[a]({})\n\n[b]({})\n", w.url("/docs/a"), w.url("/docs/b")),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 2,
        ok: 2,
        ..Summary::default()
    });
    let probes: Vec<_> = w
        .log
        .probes()
        .into_iter()
        .filter(|r| r.path.starts_with("/docs/"))
        .collect();
    assert_eq!(probes.len(), 1, "probes: {probes:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S18"]
async fn s18_similar_body_marks_suspect() {
    let w = World::new().await;
    let words: Vec<String> = (0..400).map(|i| format!("word{i}")).collect();
    let body = words.join(" ");
    w.serve(
        Routes::new()
            .fixed("/x/a", page("Real title", &body))
            .fixed_re("^/x/[a-z0-9]{24}$", page("Other title", &body)),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/x/a")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/x/a"));
    assert!(entry.verdict.starts_with("SUSPECT"), "{entry:?}");
    assert!(
        entry.verdict.to_lowercase().contains("similar"),
        "reason should name body similarity: {entry:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S19"]
async fn s19_not_found_title_marks_suspect() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/z/a", page("Page Not Found \u{2013} Example", "sorry"))
            .fixed_re(
                "^/z/[a-z0-9]{24}$",
                page(
                    "Unrelated heading",
                    "completely different words appear here",
                ),
            ),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/z/a")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/z/a"));
    assert!(entry.verdict.starts_with("SUSPECT"), "{entry:?}");
    assert!(
        entry.verdict.to_lowercase().contains("not found"),
        "reason should name the title phrase: {entry:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S20"]
async fn s20_phrase_in_title_is_ignored_when_probe_is_404() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/w/a", page("404 errors explained", "an article")))
        .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/w/a")));

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S21"]
async fn s21_soft404_false_skips_probe() {
    let mut w = World::new().await;
    w.configure(|s| s.no_soft404_hosts = vec!["127.0.0.1".to_owned()]);
    w.serve(docs_routes()).await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/docs/real")));

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    assert!(w.log.probes().is_empty(), "probes: {:?}", w.log.probes());
}

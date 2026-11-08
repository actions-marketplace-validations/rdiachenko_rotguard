//! S10: the dead link is looked up in the archive before the suspect links.

use url::form_urlencoded;

use crate::support::{Capture, Cdx, Routes, World, page, redirect, status};

fn dated(date: &str, link: &str) -> String {
    format!("+++\ndate = {date}\n+++\n\n[x]({link})\n")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S10"]
async fn s10_dead_lookups_come_before_suspect_lookups() {
    let w = World::new().await;
    // The dead path sorts after both suspect paths, so only a dead-first order passes.
    let (dead, one, two) = (w.url("/z/gone"), w.url("/a/1"), w.url("/b/2"));
    w.serve(
        Routes::new()
            .fixed("/z/gone", status(404))
            .fixed("/a/1", redirect(301, &w.url("/")))
            .fixed("/b/2", redirect(301, &w.url("/")))
            .fixed("/", page("Home", "welcome home")),
    )
    .await;
    w.archive(Cdx::new(vec![
        Capture::of("20190101000000", &dead),
        Capture::of("20200101000000", &one),
        Capture::of("20210101000000", &two),
    ]));
    w.site.write("p1.md", &dated("2019-05-01", &dead));
    w.site.write("p2.md", &dated("2020-05-01", &one));
    w.site.write("p3.md", &dated("2021-05-01", &two));

    let out = w.run(&[]).await;

    assert_eq!(
        out.entry(&one).verdict,
        "SUSPECT redirect to root",
        "stdout:\n{}",
        out.stdout
    );
    let looked_up: Vec<String> = w
        .alog
        .all()
        .iter()
        .filter_map(|req| {
            form_urlencoded::parse(req.query.as_deref()?.as_bytes())
                .find(|(key, _)| key == "url")
                .map(|(_, value)| value.into_owned())
        })
        .collect();
    let last_dead = looked_up.iter().rposition(|u| *u == dead);
    let first_suspect = looked_up.iter().position(|u| *u == one || *u == two);
    let (Some(last_dead), Some(first_suspect)) = (last_dead, first_suspect) else {
        panic!("expected lookups for the dead and a suspect URL, got {looked_up:?}");
    };
    assert!(last_dead < first_suspect, "lookup order: {looked_up:?}");
}

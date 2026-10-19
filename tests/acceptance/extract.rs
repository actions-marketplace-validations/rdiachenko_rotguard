//! R2-R4: which URLs are extracted and which are checked.

use crate::support::{Routes, Summary, World, page, redirect};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S3"]
async fn s3_extracts_every_link_form_with_lines() {
    let w = World::new().await;
    let mut routes = Routes::new().fixed("/t", page("T", "target"));
    for p in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        routes = routes.fixed(&format!("/{p}"), redirect(302, &w.url("/t")));
    }
    w.serve(routes).await;
    let b = &w.base;
    w.site.write(
        "post.md",
        &format!(
            "+++\n\
             source = '{b}/a'\n\
             \n\
             [[discussedOn]]\n\
             url = '{b}/b'\n\
             +++\n\
             \n\
             [x]({b}/c)\n\
             ![i]({b}/d)\n\
             \n\
             [r]: {b}/e\n\
             \n\
             <{b}/f>\n\
             \n\
             see {b}/g today\n\
             \n\
             <a href=\"{b}/h\">h</a>\n"
        ),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 8,
        temporary: 8,
        ..Summary::default()
    });
    for (p, line) in [
        ("a", 2),
        ("b", 5),
        ("c", 8),
        ("d", 9),
        ("e", 11),
        ("f", 13),
        ("g", 15),
        ("h", 17),
    ] {
        assert_eq!(w.log.hits(&format!("/{p}")).len(), 1, "GETs for /{p}");
        let entry = out.entry(&w.url(&format!("/{p}")));
        assert_eq!(entry.loc, format!("post.md:{line}"), "line of /{p}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S4"]
async fn s4_extracts_yaml_front_matter_links() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/y", page("Y", "yes"))).await;
    w.site.write(
        "post.md",
        &format!("---\ntitle: x\nlink: \"{}\"\n---\n\nbody\n", w.url("/y")),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    assert_eq!(w.log.hits("/y").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S5"]
async fn s5_skips_urls_in_code() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/live", page("Live", "alive")))
        .await;
    let b = &w.base;
    w.site.write(
        "post.md",
        &format!(
            "[x]({b}/live)\n\
             \n\
             ```text\n\
             {b}/k1\n\
             ```\n\
             \n\
             \x20   {b}/k2\n\
             \n\
             Inline `{b}/k3` code.\n"
        ),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ..Summary::default()
    });
    for p in ["/k1", "/k2", "/k3"] {
        assert!(w.log.hits(p).is_empty(), "M received a request for {p}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S6"]
async fn s6_checks_only_external_unignored_urls() {
    let mut w = World::new().await;
    w.configure(|s| s.ignore = vec!["/skip/".to_owned()]);
    w.serve(Routes::new().fixed("/ok", page("Ok", "fine")))
        .await;
    let b = &w.base;
    w.site.write(
        "post.md",
        &format!("[a](https://site.test/about)\n\n[b]({b}/skip/1)\n\n[c]({b}/ok)\n"),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 1,
        ok: 1,
        ignored: 1,
        ..Summary::default()
    });
    let paths: Vec<String> = w.log.own().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, ["/ok"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S39"]
async fn s39_bare_url_keeps_parens_and_shortcode_src() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a_b_(c)&x=1", page("Odd", "odd path"))
            .fixed("/s", page("S", "figure")),
    )
    .await;
    let b = &w.base;
    w.site.write(
        "post.md",
        &format!("see {b}/a_b_(c)&x=1 now\n\n{{{{< figure src=\"{b}/s\" alt=\"x\" >}}}}\n"),
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary {
        checked: 2,
        ok: 2,
        ..Summary::default()
    });
    assert_eq!(w.log.hits("/a_b_(c)&x=1").len(), 1);
    assert_eq!(w.log.hits("/s").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S40"]
async fn s40_skips_non_http_and_internal_destinations() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/p", page("P", "p"))).await;
    w.site.write(
        "post.md",
        "[m](mailto:a@example.com)\n\n[r](/posts/x/)\n\n[f](#top)\n\n[p](//127.0.0.1/p)\n",
    );

    let out = w.run(&[]).await;

    out.assert_summary(Summary::default());
    assert!(w.log.all().is_empty(), "M received {:?}", w.log.all());
}

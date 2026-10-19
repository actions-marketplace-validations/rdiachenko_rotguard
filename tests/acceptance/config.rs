//! R1: loading `rotguard.toml`.

use crate::support::{Routes, World, page};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S1"]
async fn s1_missing_config_exits_2() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/a", page("A", "a"))).await;
    w.site.remove("rotguard.toml");
    w.site.write("post.md", &format!("[x]({})\n", w.url("/a")));

    let out = w.run(&[]).await;

    out.assert_code(2);
    assert!(
        out.stderr.contains("rotguard.toml"),
        "stderr: {}",
        out.stderr
    );
    assert!(w.log.all().is_empty(), "M received {:?}", w.log.all());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S2"]
async fn s2_invalid_config_exits_2() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/a", page("A", "a"))).await;
    w.site.write("rotguard.toml", "site_url = 42\n");
    w.site.write("post.md", &format!("[x]({})\n", w.url("/a")));

    let out = w.run(&[]).await;

    out.assert_code(2);
    assert!(out.stderr.contains("site_url"), "stderr: {}", out.stderr);
    assert!(w.log.all().is_empty(), "M received {:?}", w.log.all());
}

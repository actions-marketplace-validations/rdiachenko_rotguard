//! Directory probes: a link ending in `/` is probed one level up the tree.

use crate::support::{Req, Routes, World, page, status};

const TOKEN: &str = "[a-z0-9]{24}";

/// Requests whose last path segment is a 24-character token, with or without a trailing slash.
fn probe_shaped(w: &World) -> Vec<Req> {
    let re = regex::Regex::new(&format!("/{TOKEN}/?$")).expect("test regex");
    w.log
        .all()
        .into_iter()
        .filter(|r| re.is_match(&r.path))
        .collect()
}

fn assert_path(req: &Req, pattern: &str) {
    let re = regex::Regex::new(pattern).expect("test regex");
    assert!(re.is_match(&req.path), "probe path {}", req.path);
}

#[tokio::test(flavor = "multi_thread")]
async fn directory_probe_s1_root_probe_clears_directory() {
    let w = World::new().await;
    let home = page("Docs home", "alpha beta gamma delta epsilon");
    w.serve(
        Routes::new()
            .fixed("/docs/", home.clone())
            .fixed_re(&format!("^/docs/{TOKEN}$"), home),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/docs/")));

    let out = w.run(&[]).await;

    assert!(out.entries().is_empty(), "stdout:\n{}", out.stdout);
    out.assert_code(0);
    let probes = probe_shaped(&w);
    assert_eq!(probes.len(), 1, "probes: {probes:?}");
    assert_path(&probes[0], &format!("^/{TOKEN}/$"));
}

#[tokio::test(flavor = "multi_thread")]
async fn directory_probe_s2_sibling_probe_keeps_suspect() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/blog/gone/", page("Blog", "latest posts list here"))
            .fixed_re(
                &format!("^/blog/{TOKEN}/$"),
                page("Blog", "one two three four five six"),
            ),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/blog/gone/")));

    let out = w.run(&[]).await;

    let entry = out.entry(&w.url("/blog/gone/"));
    assert_eq!(entry.verdict, "SUSPECT probe: same title", "{entry:?}");
    let probes = probe_shaped(&w);
    assert_eq!(probes.len(), 1, "probes: {probes:?}");
    assert_path(&probes[0], &format!("^/blog/{TOKEN}/$"));
}

#[tokio::test(flavor = "multi_thread")]
async fn directory_probe_s3_two_probes_per_directory_group() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/learn/a/", page("Learn A", "first lesson text"))
            .fixed("/learn/b/", page("Learn B", "second lesson words"))
            .fixed("/learn/c", page("Learn C", "third lesson notes")),
    )
    .await;
    w.site.write(
        "post.md",
        &format!(
            "[a]({})\n\n[b]({})\n\n[c]({})\n",
            w.url("/learn/a/"),
            w.url("/learn/b/"),
            w.url("/learn/c")
        ),
    );

    let out = w.run(&[]).await;

    assert!(out.entries().is_empty(), "stdout:\n{}", out.stdout);
    out.assert_code(0);
    let probes = probe_shaped(&w);
    assert_eq!(probes.len(), 2, "probes: {probes:?}");
    let dir = format!("^/learn/{TOKEN}/$");
    let file = format!("^/learn/{TOKEN}$");
    let matches = |pattern: &str| {
        let re = regex::Regex::new(pattern).expect("test regex");
        probes.iter().filter(|r| re.is_match(&r.path)).count()
    };
    assert_eq!(matches(&dir), 1, "probes: {probes:?}");
    assert_eq!(matches(&file), 1, "probes: {probes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn directory_probe_s4_blocked_directory_is_blocked_under_strict() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/docs/", status(403))
            .fixed_re(&format!("^/{TOKEN}/$"), status(403)),
    )
    .await;
    w.site
        .write("post.md", &format!("[x]({})\n", w.url("/docs/")));

    let out = w.run(&["--strict"]).await;

    let entry = out.entry(&w.url("/docs/"));
    assert_eq!(entry.verdict, "BLOCKED 403", "{entry:?}");
}

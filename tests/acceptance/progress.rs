//! Progress and config design: stderr progress lines, the terminal line, and the shipped config docs.

use std::collections::BTreeSet;

use regex::Regex;
use rotguard::progress::{Phase, Progress};

use crate::support::{Capture, Cdx, Routes, World, page, status};

fn line_index(stderr: &str, pattern: &str) -> usize {
    let re = Regex::new(pattern).unwrap();
    stderr
        .lines()
        .position(|l| re.is_match(l))
        .unwrap_or_else(|| panic!("no stderr line matching {pattern}; stderr:\n{stderr}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_s1_piped_run_prints_start_and_end_lines_on_stderr() {
    let w = World::new().await;
    w.serve(
        Routes::new()
            .fixed("/a", page("Alpha", "first page"))
            .fixed("/b", page("Beta", "second page")),
    )
    .await;
    w.site.write(
        "post.md",
        &format!("[a]({}) [b]({})\n", w.url("/a"), w.url("/b")),
    );

    let out = w.run(&[]).await;

    let start = line_index(&out.stderr, "^checking 2 links on 1 host$");
    let end = line_index(&out.stderr, "^checked 2 links in [0-9]+s$");
    assert!(start < end, "stderr:\n{}", out.stderr);
    for line in out.stdout.lines() {
        assert!(
            !line.starts_with("checking ") && !line.starts_with("checked 2 links"),
            "progress on stdout: {line}"
        );
    }
    out.summary_line();
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_s2_archive_phase_prints_its_own_lines() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20220101000000", &gone)]));
    w.site.write("post.md", &format!("[x]({gone})\n"));

    let out = w.run(&[]).await;

    let checked = line_index(&out.stderr, "^checked 1 link in [0-9]+s$");
    let looking = line_index(&out.stderr, "^looking up 1 snapshot$");
    let looked = line_index(&out.stderr, "^looked up 1 snapshot in [0-9]+s$");
    assert!(
        checked < looking && looking < looked,
        "stderr:\n{}",
        out.stderr
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_s3_no_archive_phase_lines_when_nothing_needs_a_snapshot() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/a", page("Alpha", "first page")))
        .await;
    w.site.write("post.md", &format!("[a]({})\n", w.url("/a")));

    let out = w.run(&[]).await;

    for line in out.stderr.lines() {
        assert!(
            !line.starts_with("looking up") && !line.starts_with("looked up"),
            "unexpected archive progress: {line}"
        );
    }
}

#[test]
fn progress_s4_terminal_mode_rewrites_one_line_then_clears_it() {
    let mut buf = Vec::new();
    {
        let mut p = Progress::new(&mut buf, true);
        p.start(Phase::Check { hosts: 1 }, 2);
        p.tick(1);
        p.tick(2);
        p.finish();
    }

    let text = String::from_utf8(buf).unwrap();
    let prefix =
        "\rchecking links: 0/2 done\rchecking links: 1/2 done\rchecking links: 2/2 done\r\x1b[2K";
    let rest = text
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("unexpected bytes: {text:?}"));
    assert!(
        Regex::new("^checked 2 links in [0-9]+s\n$")
            .unwrap()
            .is_match(rest),
        "unexpected end line: {rest:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_s5_config_error_prints_no_progress() {
    let w = World::new().await;
    w.site.remove("rotguard.toml");

    let out = w.run(&[]).await;

    out.assert_code(2);
    for line in out.stderr.lines() {
        assert!(!line.starts_with("checking"), "unexpected progress: {line}");
    }
}

fn read_repo_file(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// The text of every ```toml block in `text`, in order.
fn toml_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match &mut current {
            None if line.trim_end() == "```toml" => current = Some(String::new()),
            None => {}
            Some(_) if line.trim_end() == "```" => blocks.extend(current.take()),
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    blocks
}

/// Runs rotguard on a site whose config is `block` and whose one page has no links.
async fn run_with_config(block: &str) {
    let w = World::new().await;
    w.site.write("rotguard.toml", block);
    std::fs::create_dir_all(w.site.path().join("content")).unwrap();
    w.site.write("content/a.md", "# A page\n\nNo links here.\n");

    let out = w.run(&[]).await;

    assert_eq!(
        out.code, 0,
        "stdout:\n{}\nstderr:\n{}\nconfig:\n{block}",
        out.stdout, out.stderr
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_s6_config_doc_example_loads_and_covers_every_key() {
    let doc = read_repo_file("docs/config.md");
    let block = toml_blocks(&doc)
        .pop()
        .expect("docs/config.md has no toml block");

    let key_re = Regex::new(r"^([a-z_]+)\s*=").unwrap();
    let mut keys = BTreeSet::new();
    for line in block.lines() {
        if line.starts_with('[') {
            break;
        }
        if let Some(caps) = key_re.captures(line) {
            keys.insert(caps[1].to_owned());
        }
    }
    let expected: BTreeSet<String> = [
        "site_url",
        "content",
        "date_key",
        "user_agent",
        "ignore",
        "not_found_phrases",
        "archive_url",
        "host_interval_ms",
        "archive_interval_ms",
        "recheck_delay_ms",
        "timeout_ms",
        "deadline_s",
    ]
    .map(String::from)
    .into();
    assert_eq!(keys, expected, "top-level keys of the last toml block");

    // Headings only count outside fenced blocks: TOML comments also start with '#'.
    let mut in_fence = false;
    let mut headings: Vec<&str> = Vec::new();
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence {
            let rest = line.trim_start_matches('#');
            let level = line.len() - rest.len();
            if (1..=6).contains(&level) && rest.starts_with(char::is_whitespace) {
                headings.push(line);
            }
        }
    }
    for name in expected.iter().map(String::as_str).chain(["hosts"]) {
        assert!(
            headings.iter().any(|h| h.contains(name)),
            "no heading mentions {name}; headings: {headings:?}"
        );
    }

    run_with_config(&block).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_s7_readme_configure_example_loads_and_links_to_the_config_doc() {
    let readme = read_repo_file("README.md");
    let section = readme
        .split_once("\n## Configure")
        .map(|(_, rest)| rest.split_once("\n## ").map_or(rest, |(s, _)| s))
        .expect("README.md has no Configure section");
    let block = toml_blocks(section)
        .into_iter()
        .next()
        .expect("Configure section has no toml block");

    assert!(
        block.contains("[hosts.\"www.goodreads.com\"]"),
        "block lacks the goodreads host table:\n{block}"
    );
    assert!(
        section.contains("](docs/config.md"),
        "Configure section has no markdown link to docs/config.md"
    );

    run_with_config(&block).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_quiet_run_prints_nothing_on_stderr() {
    let w = World::new().await;
    w.serve(Routes::new().fixed("/gone", status(404))).await;
    let gone = w.url("/gone");
    w.archive(Cdx::new(vec![Capture::of("20220101000000", &gone)]));
    w.site.write("post.md", &format!("[x]({gone})\n"));

    let out = w.run(&["--quiet"]).await;

    assert_eq!(out.stderr, "", "stderr:\n{}", out.stderr);
    assert_eq!(out.entry(&gone).verdict, "DEAD 404");
    out.assert_code(1);
}

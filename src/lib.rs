use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use chrono::NaiveDate;
use clap::{Parser, ValueEnum};
use globset::{GlobBuilder, GlobSetBuilder};
use tokio::task::JoinSet;
use tokio::time::Instant;
use url::Url;
use walkdir::WalkDir;

use crate::apply::Unwritable;
use crate::archive::{Archive, NoFix};
use crate::check::Verdict;
use crate::config::Config;
use crate::extract::{Link, Page};
use crate::progress::{Phase, Progress};
use crate::report::Summary;

pub mod apply;
pub mod archive;
pub mod check;
pub mod config;
pub mod extract;
pub mod progress;
pub mod report;

/// One non-ok occurrence of a URL in a site file.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Path relative to the site directory.
    pub file: PathBuf,
    pub link: Link,
    pub verdict: Verdict,
    pub fix: Fix,
}

/// What can be done about a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// No fix applies.
    None,
    /// A replacement URL, not yet written.
    Proposed(String),
    /// Why no replacement could be written.
    Manual(ManualReason),
    /// A replacement URL that was written to the site.
    Applied(String),
}

/// Why a fix needs a manual edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualReason {
    /// No archived copy could be proposed.
    Archive(NoFix),
    /// The proposed URL cannot be written at the link's position.
    Unwritable(Unwritable),
    /// The file no longer holds the URL the run found.
    ChangedDuringRun,
    /// The file could not be read, written or renamed.
    WriteFailed,
}

impl fmt::Display for ManualReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Archive(reason) => reason.fmt(f),
            Self::Unwritable(reason) => reason.fmt(f),
            Self::ChangedDuringRun => f.write_str("changed during run"),
            Self::WriteFailed => f.write_str("write failed"),
        }
    }
}

/// Command-line arguments for `rotguard`.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(version, about = "Check a site's links for rot")]
pub struct Args {
    /// Directory of the site to check.
    #[arg(default_value = ".")]
    pub site_dir: PathBuf,
    /// Write fixes to the site instead of only reporting them.
    #[arg(long)]
    pub apply: bool,
    /// Fail on any finding, not only on broken links, and list links on hosts that block rotguard.
    #[arg(long)]
    pub strict: bool,
    /// Don't print progress lines on stderr; errors are still printed.
    #[arg(long)]
    pub quiet: bool,
    /// Report format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

/// How the report is printed on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One line per finding, then a summary line.
    Text,
    /// One JSON object with the findings and the summary.
    Json,
    /// One self-contained HTML page with the summary and a table of findings.
    Html,
    /// Markdown sections for a pull request body, then a counts line.
    Markdown,
}

/// `err` and each of its sources, joined by ": ".
fn chain(err: &dyn std::error::Error) -> String {
    std::iter::successors(Some(err), |e| e.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

/// Runs the main flow and returns the process exit code.
#[must_use]
pub fn run(args: Args) -> ExitCode {
    let cfg = match config::load(&args.site_dir) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("rotguard: {}", chain(&err));
            return ExitCode::from(2);
        }
    };
    let pages = match read_pages(&args.site_dir, &cfg) {
        Ok(pages) => pages,
        Err(message) => {
            eprintln!("rotguard: {message}");
            return ExitCode::from(2);
        }
    };
    let clients = check::http_client(&cfg)
        .and_then(|checker| archive::http_client(&cfg).map(|archive| Clients { checker, archive }));
    let clients = match clients {
        Ok(clients) => clients,
        Err(err) => {
            eprintln!(
                "rotguard: cannot load the system's CA certificates: {}",
                chain(&err)
            );
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("rotguard: starting runtime: {err}");
            return ExitCode::from(2);
        }
    };
    let mut progress = Progress::stderr(args.quiet);
    let (mut findings, summary) =
        runtime.block_on(check_pages(&pages, &cfg, clients, &mut progress));
    // Hung requests must not hold the process past the deadline.
    runtime.shutdown_background();
    if !args.strict {
        findings.retain(|f| !matches!(f.verdict, Verdict::Blocked { .. }));
    }
    let apply_errors = if args.apply {
        apply::apply(&args.site_dir, &mut findings)
    } else {
        Vec::new()
    };
    println!("{}", report::render_as(args.format, &findings, &summary));
    for err in &apply_errors {
        eprintln!("rotguard: applying fixes: {}", chain(err));
    }
    if apply_errors.is_empty() {
        exit_code(&findings, args.strict)
    } else {
        ExitCode::from(2)
    }
}

/// Reads and extracts every content file under `site_dir`, sorted by path.
fn read_pages(site_dir: &Path, cfg: &Config) -> Result<Vec<(PathBuf, Page)>, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in &cfg.content {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|err| format!("content glob {pattern:?}: {err}"))?;
        builder.add(glob);
    }
    let globs = builder
        .build()
        .map_err(|err| format!("content globs: {err}"))?;

    let prefixes: Vec<PathBuf> = cfg.content.iter().map(|p| literal_prefix(p)).collect();
    let mut files = Vec::new();
    for entry in WalkDir::new(site_dir) {
        let entry = match entry {
            Ok(entry) => entry,
            // A directory no content glob can match below is not ours to read.
            Err(err) if outside_content(site_dir, err.path(), &prefixes) => continue,
            Err(err) => return Err(format!("walking {}: {err}", site_dir.display())),
        };
        let Ok(relative) = entry.path().strip_prefix(site_dir) else {
            continue;
        };
        if entry.file_type().is_file() && globs.is_match(relative) {
            files.push(relative.to_path_buf());
        }
    }
    files.sort();

    files
        .into_iter()
        .map(|relative| {
            let full = site_dir.join(&relative);
            let text = std::fs::read_to_string(&full)
                .map_err(|err| format!("reading {}: {err}", full.display()))?;
            let page = extract::extract(&relative, &text, cfg);
            Ok((relative, page))
        })
        .collect()
}

/// The leading path components of `pattern` that contain no glob metacharacter or escape.
fn literal_prefix(pattern: &str) -> PathBuf {
    pattern
        .split('/')
        .take_while(|part| !part.contains(['*', '?', '[', '{', '\\']))
        .collect()
}

/// Whether `path` is neither inside nor above any content glob's literal prefix.
fn outside_content(site_dir: &Path, path: Option<&Path>, prefixes: &[PathBuf]) -> bool {
    let Some(relative) = path.and_then(|p| p.strip_prefix(site_dir).ok()) else {
        return false;
    };
    prefixes
        .iter()
        .all(|prefix| !relative.starts_with(prefix) && !prefix.starts_with(relative))
}

/// A link that survived the external and `ignore` filters.
struct Occurrence<'a> {
    file: &'a Path,
    date: Option<NaiveDate>,
    link: &'a Link,
    /// The link's URL without its fragment.
    url: Url,
}

/// The 2 HTTP clients of a run: the checker's and the archive's.
struct Clients {
    checker: reqwest::Client,
    archive: reqwest::Client,
}

async fn check_pages(
    pages: &[(PathBuf, Page)],
    cfg: &Config,
    clients: Clients,
    progress: &mut Progress<io::Stderr>,
) -> (Vec<Finding>, Summary) {
    let site_host = cfg.site_url.host_str();
    let mut occurrences = Vec::new();
    let mut ignored = HashSet::new();
    for (file, page) in pages {
        for link in &page.links {
            let Ok(mut url) = Url::parse(&link.url) else {
                continue;
            };
            url.set_fragment(None);
            if url.host_str() == site_host {
                continue;
            }
            if cfg.ignore.iter().any(|re| re.is_match(&link.url)) {
                ignored.insert(url);
                continue;
            }
            occurrences.push(Occurrence {
                file,
                date: page.date,
                link,
                url,
            });
        }
    }

    let mut distinct: Vec<Url> = occurrences.iter().map(|o| o.url.clone()).collect();
    distinct.sort();
    distinct.dedup();

    let deadline = Instant::now() + cfg.deadline;
    let hosts = distinct
        .iter()
        .filter_map(Url::host_str)
        .collect::<HashSet<_>>()
        .len();
    progress.start(Phase::Check { hosts }, distinct.len());
    let verdicts = check::check_all(distinct.clone(), clients.checker, cfg, deadline, |done| {
        progress.tick(done);
    })
    .await;
    progress.finish();

    let run_date = chrono::Local::now().date_naive();
    let lookups: HashSet<(Url, NaiveDate)> = occurrences
        .iter()
        .filter(|o| {
            matches!(
                verdicts.get(&o.url),
                Some(Verdict::Dead { .. } | Verdict::Suspect { .. })
            )
        })
        .map(|o| (o.url.clone(), o.date.unwrap_or(run_date)))
        .collect();
    let n = lookups.len();
    let snapshots = if n > 0 {
        progress.start(Phase::Archive, n);
        let snapshots = look_up_snapshots(lookups, clients.archive, cfg, deadline, |done| {
            progress.tick(done)
        })
        .await;
        progress.finish();
        snapshots
    } else {
        look_up_snapshots(lookups, clients.archive, cfg, deadline, |_| {}).await
    };

    let mut findings = Vec::new();
    for occ in &occurrences {
        let Some(verdict) = verdicts.get(&occ.url) else {
            continue;
        };
        let fragment = fragment_of(&occ.link.url);
        let fix = match verdict {
            Verdict::Ok { .. } => continue,
            Verdict::Permanent { fix, .. } => {
                let mut fix = fix.clone();
                if fix.fragment().is_none() {
                    fix.set_fragment(fragment);
                }
                Fix::Proposed(fix.to_string())
            }
            Verdict::Dead { .. } | Verdict::Suspect { .. } => {
                let key = (occ.url.clone(), occ.date.unwrap_or(run_date));
                match snapshots.get(&key) {
                    Some(Ok(snapshot)) => {
                        let mut snapshot = snapshot.clone();
                        snapshot.set_fragment(fragment);
                        Fix::Proposed(snapshot.to_string())
                    }
                    Some(Err(reason)) => Fix::Manual(ManualReason::Archive(*reason)),
                    None => Fix::Manual(ManualReason::Archive(NoFix::Deadline)),
                }
            }
            Verdict::Temporary { .. } | Verdict::Unverifiable { .. } | Verdict::Blocked { .. } => {
                Fix::None
            }
        };
        findings.push(Finding {
            file: occ.file.to_path_buf(),
            link: occ.link.clone(),
            verdict: verdict.clone(),
            fix,
        });
    }

    let mut summary = Summary {
        checked: distinct.len(),
        ignored: ignored.len(),
        ..Summary::default()
    };
    for verdict in distinct.iter().filter_map(|url| verdicts.get(url)) {
        let count = match verdict {
            Verdict::Ok { .. } => &mut summary.ok,
            Verdict::Permanent { .. } => &mut summary.permanent,
            Verdict::Temporary { .. } => &mut summary.temporary,
            Verdict::Dead { .. } => &mut summary.dead,
            Verdict::Suspect { .. } => &mut summary.suspect,
            Verdict::Unverifiable { .. } => &mut summary.unverifiable,
            Verdict::Blocked { .. } => &mut summary.blocked,
        };
        *count += 1;
    }
    (findings, summary)
}

/// The fragment of a link as written, if it has one.
fn fragment_of(link: &str) -> Option<&str> {
    link.split_once('#').map(|(_, fragment)| fragment)
}

/// Looks up all snapshots concurrently; the archive's own gate spaces the requests.
async fn look_up_snapshots(
    lookups: HashSet<(Url, NaiveDate)>,
    client: reqwest::Client,
    cfg: &Config,
    deadline: Instant,
    mut on_done: impl FnMut(usize),
) -> HashMap<(Url, NaiveDate), Result<Url, NoFix>> {
    let archive = Arc::new(Archive::new(client, cfg));
    let mut tasks = JoinSet::new();
    for (url, date) in lookups {
        let archive = Arc::clone(&archive);
        tasks.spawn(async move {
            let result = archive.snapshot(&url, date, deadline).await;
            ((url, date), result)
        });
    }
    let mut snapshots = HashMap::new();
    let mut count = 0;
    while let Some(joined) = tasks.join_next().await {
        count += 1;
        on_done(count);
        // A panicked lookup leaves its key absent, which the caller reports as a deadline.
        if let Ok((key, result)) = joined {
            snapshots.insert(key, result);
        }
    }
    snapshots
}

fn exit_code(findings: &[Finding], strict: bool) -> ExitCode {
    let unfixed = findings.iter().any(|finding| match finding.verdict {
        Verdict::Dead { .. } | Verdict::Permanent { .. } | Verdict::Suspect { .. } => {
            !matches!(finding.fix, Fix::Applied(_))
        }
        Verdict::Temporary { .. } | Verdict::Unverifiable { .. } | Verdict::Blocked { .. } => {
            strict
        }
        Verdict::Ok { .. } => false,
    });
    ExitCode::from(u8::from(unfixed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Creates `relative` under `dir` with `text`. Only `src/apply.rs` may write
    /// files with the std calls, so this goes through a zero-random-suffix tempfile.
    fn put(dir: &Path, relative: &str, text: &str) {
        let path = dir.join(relative);
        let parent = path.parent().expect("test paths have a parent");
        std::fs::create_dir_all(parent).expect("create test dir");
        let name = path.file_stem().and_then(|s| s.to_str()).expect("stem");
        let ext = path.extension().and_then(|s| s.to_str()).expect("ext");
        let mut file = tempfile::Builder::new()
            .prefix(name)
            .suffix(&format!(".{ext}"))
            .rand_bytes(0)
            .tempfile_in(parent)
            .expect("create test file");
        file.write_all(text.as_bytes()).expect("write test file");
        file.keep().expect("keep test file");
    }

    fn matched(patterns: &str) -> Vec<PathBuf> {
        let site = tempfile::tempdir().expect("temp site");
        put(site.path(), "content/a.md", "a\n");
        put(site.path(), "content/sub/b.md", "b\n");
        let cfg = config::parse(&format!(
            "site_url = \"https://site.test\"\ncontent = [{patterns}]\n"
        ))
        .expect("valid config");
        let pages = read_pages(site.path(), &cfg).expect("pages read");
        pages.into_iter().map(|(path, _)| path).collect()
    }

    /// Restores a directory's mode on drop so the temp dir can be removed.
    #[cfg(unix)]
    struct RestoreMode(PathBuf);

    #[cfg(unix)]
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt as _;
            // Best effort: a failure only leaves the temp dir behind.
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_outside_content_is_skipped() {
        use std::os::unix::fs::PermissionsExt as _;
        let site = tempfile::tempdir().expect("temp site");
        put(site.path(), "content/a.md", "a\n");
        let public = site.path().join("public");
        std::fs::create_dir(&public).expect("create public");
        let _restore = RestoreMode(public.clone());
        std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o000))
            .expect("chmod public");
        if std::fs::read_dir(&public).is_ok() {
            return; // running as root: permissions do not apply
        }
        let cfg = config::parse("site_url = \"https://site.test\"\n").expect("valid config");

        let pages = read_pages(site.path(), &cfg).expect("pages read");

        let paths: Vec<_> = pages.into_iter().map(|(path, _)| path).collect();
        assert_eq!(paths, [PathBuf::from("content/a.md")]);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_inside_content_is_an_error() {
        use std::os::unix::fs::PermissionsExt as _;
        let site = tempfile::tempdir().expect("temp site");
        put(site.path(), "content/sub/b.md", "b\n");
        let sub = site.path().join("content/sub");
        let _restore = RestoreMode(sub.clone());
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).expect("chmod sub");
        if std::fs::read_dir(&sub).is_ok() {
            return; // running as root: permissions do not apply
        }
        let cfg = config::parse("site_url = \"https://site.test\"\n").expect("valid config");

        let err = read_pages(site.path(), &cfg).expect_err("walk error kept");

        assert!(err.starts_with("walking "), "{err}");
    }

    #[test]
    fn single_star_glob_stops_at_a_slash() {
        assert_eq!(matched("\"content/*.md\""), [PathBuf::from("content/a.md")]);
    }

    #[test]
    fn double_star_glob_crosses_directories() {
        assert_eq!(
            matched("\"content/**/*.md\""),
            [
                PathBuf::from("content/a.md"),
                PathBuf::from("content/sub/b.md")
            ]
        );
    }

    #[test]
    fn chain_joins_an_error_with_its_sources() {
        let err = config::ConfigError::Read {
            path: PathBuf::from("site/rotguard.toml"),
            source: std::io::Error::other("disk on fire"),
        };

        assert_eq!(chain(&err), "cannot read site/rotguard.toml: disk on fire");
    }

    #[test]
    fn manual_reasons_print_their_report_strings() {
        let table = [
            (ManualReason::Archive(NoFix::NoCapture), "no archived copy"),
            (
                ManualReason::Archive(NoFix::Unreachable),
                "archive unreachable",
            ),
            (ManualReason::Archive(NoFix::Deadline), "deadline"),
            (
                ManualReason::Unwritable(Unwritable::NotVerbatim),
                "not verbatim",
            ),
            (
                ManualReason::Unwritable(Unwritable::CannotQuote),
                "cannot quote",
            ),
            (ManualReason::ChangedDuringRun, "changed during run"),
            (ManualReason::WriteFailed, "write failed"),
        ];
        for (reason, text) in table {
            assert_eq!(reason.to_string(), text);
        }
    }
}

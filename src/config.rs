//! Loading and validating `rotguard.toml`.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use regex::Regex;
use reqwest::header::HeaderValue;
use serde::Deserialize;
use url::Url;

/// Name of the config file inside the site directory.
pub const FILE_NAME: &str = "rotguard.toml";

/// Why the config could not be loaded.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read; `path` is the file looked for.
    Read { path: PathBuf, source: io::Error },
    /// The text is not valid TOML, has an unknown key, or a field has the wrong type or value.
    Parse(toml::de::Error),
    /// An `ignore` entry is not a valid regex.
    Ignore {
        pattern: String,
        source: regex::Error,
    },
    /// `user_agent` cannot be sent as an HTTP header value.
    UserAgent { value: String },
    /// Two `[hosts]` keys are equal once lowercased; `host` is the lowercased name.
    DuplicateHost { host: String },
    /// `deadline_s` is above 86400 (one day).
    Deadline { value: u64 },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Parse(source) => write!(f, "invalid {FILE_NAME}: {source}"),
            Self::Ignore { pattern, source } => {
                write!(
                    f,
                    "invalid {FILE_NAME}: bad regex in `ignore` ({pattern:?}): {source}"
                )
            }
            Self::UserAgent { value } => write!(
                f,
                "invalid {FILE_NAME}: `user_agent` ({value:?}) is not a valid HTTP header value"
            ),
            Self::Deadline { value } => write!(
                f,
                "invalid {FILE_NAME}: `deadline_s` ({value}) must be at most {MAX_DEADLINE_S}"
            ),
            Self::DuplicateHost { host } => write!(
                f,
                "invalid {FILE_NAME}: `[hosts]` lists {host:?} more than once (keys ignore case)"
            ),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse(source) => Some(source),
            Self::Ignore { source, .. } => Some(source),
            Self::UserAgent { .. } | Self::DuplicateHost { .. } | Self::Deadline { .. } => None,
        }
    }
}

/// Per-host settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    /// Whether soft-404 detection runs for URLs on this host.
    #[serde(default = "default_true")]
    pub soft404: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self { soft404: true }
    }
}

/// Validated settings from `rotguard.toml`.
#[derive(Debug, Clone)]
pub struct Config {
    pub site_url: Url,
    pub content: Vec<String>,
    pub date_key: String,
    pub user_agent: String,
    pub ignore: Vec<Regex>,
    pub not_found_phrases: Vec<String>,
    pub archive_url: Url,
    pub host_interval: Duration,
    pub archive_interval: Duration,
    pub recheck_delay: Duration,
    pub timeout: Duration,
    pub deadline: Duration,
    pub hosts: HashMap<String, HostConfig>,
}

/// The file as written, before conversion to parsed types.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    site_url: Url,
    #[serde(default = "default_content")]
    content: Vec<String>,
    #[serde(default = "default_date_key")]
    date_key: String,
    #[serde(default = "default_user_agent")]
    user_agent: String,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default = "default_not_found_phrases")]
    not_found_phrases: Vec<String>,
    #[serde(default = "default_archive_url")]
    archive_url: Url,
    #[serde(default = "default_host_interval_ms")]
    host_interval_ms: u64,
    #[serde(default = "default_archive_interval_ms")]
    archive_interval_ms: u64,
    #[serde(default = "default_recheck_delay_ms")]
    recheck_delay_ms: u64,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_deadline_s")]
    deadline_s: u64,
    #[serde(default)]
    hosts: HashMap<String, HostConfig>,
}

fn default_true() -> bool {
    true
}

fn default_content() -> Vec<String> {
    vec!["content/**/*.md".to_owned()]
}

fn default_date_key() -> String {
    "date".to_owned()
}

fn default_user_agent() -> String {
    concat!("rotguard/", env!("CARGO_PKG_VERSION")).to_owned()
}

fn default_not_found_phrases() -> Vec<String> {
    [
        "not found",
        "404",
        "page does not exist",
        "page doesn't exist",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn default_archive_url() -> Url {
    Url::parse("https://web.archive.org").expect("default archive URL is a valid literal")
}

fn default_host_interval_ms() -> u64 {
    1000
}

fn default_archive_interval_ms() -> u64 {
    4000
}

fn default_recheck_delay_ms() -> u64 {
    5000
}

fn default_timeout_ms() -> u64 {
    10_000
}

/// Longest run `deadline_s` may ask for: one day. No run needs more, and the
/// bound keeps `Instant::now() + deadline` from overflowing.
const MAX_DEADLINE_S: u64 = 86_400;

fn default_deadline_s() -> u64 {
    300
}

impl TryFrom<Raw> for Config {
    type Error = ConfigError;

    fn try_from(raw: Raw) -> Result<Self, ConfigError> {
        if raw.deadline_s > MAX_DEADLINE_S {
            return Err(ConfigError::Deadline {
                value: raw.deadline_s,
            });
        }
        if HeaderValue::from_str(&raw.user_agent).is_err() {
            return Err(ConfigError::UserAgent {
                value: raw.user_agent,
            });
        }
        let ignore = raw
            .ignore
            .into_iter()
            .map(|pattern| {
                Regex::new(&pattern).map_err(|source| ConfigError::Ignore { pattern, source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut hosts = HashMap::with_capacity(raw.hosts.len());
        for (name, host) in raw.hosts {
            let name = name.to_ascii_lowercase();
            if hosts.contains_key(&name) {
                return Err(ConfigError::DuplicateHost { host: name });
            }
            hosts.insert(name, host);
        }
        Ok(Self {
            site_url: raw.site_url,
            content: raw.content,
            date_key: raw.date_key,
            user_agent: raw.user_agent,
            ignore,
            not_found_phrases: raw.not_found_phrases,
            archive_url: raw.archive_url,
            host_interval: Duration::from_millis(raw.host_interval_ms),
            archive_interval: Duration::from_millis(raw.archive_interval_ms),
            recheck_delay: Duration::from_millis(raw.recheck_delay_ms),
            timeout: Duration::from_millis(raw.timeout_ms),
            deadline: Duration::from_secs(raw.deadline_s),
            hosts,
        })
    }
}

/// Reads `rotguard.toml` from `site_dir` and parses it.
pub fn load(site_dir: &Path) -> Result<Config, ConfigError> {
    let path = site_dir.join(FILE_NAME);
    let text = fs::read_to_string(&path).map_err(|source| ConfigError::Read { path, source })?;
    parse(&text)
}

/// Parses the text of a `rotguard.toml`.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let raw: Raw = toml::from_str(text).map_err(ConfigError::Parse)?;
    raw.try_into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "site_url = \"https://site.test\"\n";

    #[test]
    fn only_site_url_gets_every_default() {
        let cfg = parse(MINIMAL).expect("minimal config parses");

        assert_eq!(cfg.site_url.as_str(), "https://site.test/");
        assert_eq!(cfg.content, ["content/**/*.md"]);
        assert_eq!(cfg.date_key, "date");
        assert_eq!(
            cfg.user_agent,
            concat!("rotguard/", env!("CARGO_PKG_VERSION"))
        );
        assert!(cfg.ignore.is_empty());
        assert_eq!(
            cfg.not_found_phrases,
            [
                "not found",
                "404",
                "page does not exist",
                "page doesn't exist"
            ]
        );
        assert_eq!(cfg.archive_url.as_str(), "https://web.archive.org/");
        assert_eq!(cfg.host_interval, Duration::from_millis(1000));
        assert_eq!(cfg.archive_interval, Duration::from_millis(4000));
        assert_eq!(cfg.recheck_delay, Duration::from_millis(5000));
        assert_eq!(cfg.timeout, Duration::from_millis(10_000));
        assert_eq!(cfg.deadline, Duration::from_secs(300));
        assert!(cfg.hosts.is_empty());
    }

    #[test]
    fn explicit_values_override_defaults() {
        let cfg = parse(
            r#"
site_url = "https://site.test"
ignore = ["/skip/"]
host_interval_ms = 300
[hosts."goodreads.com"]
soft404 = false
[hosts."other.test"]
"#,
        )
        .expect("config parses");

        assert!(cfg.ignore[0].is_match("https://x.test/skip/1"));
        assert_eq!(cfg.host_interval, Duration::from_millis(300));
        assert!(!cfg.hosts["goodreads.com"].soft404);
        assert!(cfg.hosts["other.test"].soft404);
    }

    #[test]
    fn host_keys_are_lowercased() {
        let cfg =
            parse("site_url = \"https://site.test\"\n[hosts.\"GoodReads.com\"]\nsoft404 = false\n")
                .expect("config parses");

        assert!(!cfg.hosts["goodreads.com"].soft404);
        assert!(!cfg.hosts.contains_key("GoodReads.com"));
    }

    #[test]
    fn host_keys_differing_only_in_case_are_rejected() {
        let err =
            parse("site_url = \"https://site.test\"\n[hosts.\"A.test\"]\n[hosts.\"a.test\"]\n")
                .unwrap_err();

        assert!(
            matches!(&err, ConfigError::DuplicateHost { host } if host == "a.test"),
            "{err}"
        );
        assert!(err.to_string().contains("a.test"), "{err}");
    }

    #[test]
    fn deadline_above_one_day_is_rejected() {
        // i64::MAX is the largest integer TOML can hold.
        for value in ["86401", "9223372036854775807"] {
            let err = parse(&format!(
                "site_url = \"https://site.test\"\ndeadline_s = {value}\n"
            ))
            .unwrap_err();

            assert!(
                matches!(err, ConfigError::Deadline { .. }),
                "{value}: {err}"
            );
            assert!(err.to_string().contains("deadline_s"), "{value}: {err}");
        }
    }

    #[test]
    fn deadline_of_one_day_is_accepted() {
        let cfg =
            parse("site_url = \"https://site.test\"\ndeadline_s = 86400\n").expect("config parses");

        assert_eq!(cfg.deadline, Duration::from_secs(86_400));
    }

    #[test]
    fn unknown_key_is_rejected() {
        let err = parse("site_url = \"https://site.test\"\nbogus = 1\n").unwrap_err();

        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn non_string_site_url_names_the_field() {
        let err = parse("site_url = 42\n").unwrap_err();

        assert!(err.to_string().contains("site_url"), "{err}");
    }

    #[test]
    fn missing_site_url_names_the_field() {
        let err = parse("date_key = \"d\"\n").unwrap_err();

        assert!(err.to_string().contains("site_url"), "{err}");
    }

    #[test]
    fn bad_ignore_regex_names_the_field() {
        let err = parse("site_url = \"https://site.test\"\nignore = [\"(\"]\n").unwrap_err();

        assert!(matches!(err, ConfigError::Ignore { .. }));
        assert!(err.to_string().contains("ignore"), "{err}");
    }

    #[test]
    fn unsendable_user_agent_names_the_field() {
        for ua in ["rotguard\\u0001", "a\\nb"] {
            let err = parse(&format!(
                "site_url = \"https://site.test\"\nuser_agent = \"{ua}\"\n"
            ))
            .unwrap_err();

            assert!(matches!(err, ConfigError::UserAgent { .. }), "{err}");
            assert!(err.to_string().contains("user_agent"), "{err}");
        }
    }

    #[test]
    fn missing_file_names_the_path() {
        let dir = tempfile::tempdir().expect("temp dir");

        let err = load(dir.path()).unwrap_err();

        assert!(matches!(err, ConfigError::Read { .. }));
        assert!(err.to_string().contains("rotguard.toml"), "{err}");
    }
}

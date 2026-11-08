//! The action's fetch step must not hang on a stalled download.

use std::fs;
use std::path::Path;

use serde_norway::Value;

/// The `run` script of the `id: fetch` step in `action.yml`.
fn fetch_script() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("action.yml");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("action.yml is unreadable: {e}"));
    let action: Value = serde_norway::from_str(&text)
        .unwrap_or_else(|e| panic!("action.yml is not valid YAML: {e}"));
    action["runs"]["steps"]
        .as_sequence()
        .expect("action.yml has runs.steps")
        .iter()
        .find(|step| step["id"].as_str() == Some("fetch"))
        .expect("action.yml has a fetch step")["run"]
        .as_str()
        .expect("the fetch step has a run script")
        .to_owned()
}

/// Every `curl` invocation in `script`, with `\` continuations joined.
fn curl_invocations(script: &str) -> Vec<String> {
    script
        .replace("\\\n", " ")
        .lines()
        .map(str::trim)
        .filter(|line| line.split_whitespace().next() == Some("curl"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn fetch_curl_calls_time_out_a_stalled_download() {
    let calls = curl_invocations(&fetch_script());
    assert!(!calls.is_empty(), "the fetch step runs curl");
    for call in &calls {
        for flag in ["--connect-timeout", "--max-time"] {
            assert!(call.contains(flag), "curl call lacks {flag}: {call}");
        }
    }
}

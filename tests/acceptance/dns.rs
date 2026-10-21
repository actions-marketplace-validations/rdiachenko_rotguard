//! DNS hiccup design: a failed lookup is dead only when the name does not exist.

use rotguard::check::{FetchError, Verdict, classify, error_kind};
use rotguard::config::HostConfig;

fn messages(list: &[&str]) -> Vec<String> {
    list.iter().map(|&message| message.to_owned()).collect()
}

#[test]
fn dns_s1_missing_name_is_dns() {
    let lists: [&[&str]; 4] = [
        &[
            "dns error",
            "failed to lookup address information: name or service not known",
        ],
        &[
            "dns error",
            "failed to lookup address information: nodename nor servname provided, or not known",
        ],
        &["no such host is known."],
        &[
            "dns error",
            "failed to lookup address information: no address associated with hostname",
        ],
    ];
    for list in lists {
        assert_eq!(error_kind(&messages(list)), FetchError::Dns, "{list:?}");
    }
}

#[test]
#[ignore = "pending: S2"]
fn dns_s2_resolver_hiccup_is_dns_failed() {
    let lists: [&[&str]; 3] = [
        &[
            "dns error",
            "failed to lookup address information: temporary failure in name resolution",
        ],
        &[
            "dns error",
            "failed to lookup address information: try again",
        ],
        &["dns error"],
    ];
    for list in lists {
        assert_eq!(
            error_kind(&messages(list)).to_string(),
            "dns lookup failed",
            "{list:?}"
        );
    }
}

#[test]
fn dns_s3_other_kinds_are_unchanged() {
    assert_eq!(
        error_kind(&messages(&[
            "error sending request",
            "invalid peer certificate: unknown issuer",
        ])),
        FetchError::Tls
    );
    assert_eq!(
        error_kind(&messages(&["error sending request", "connection refused"])),
        FetchError::Connect
    );
}

#[test]
#[ignore = "pending: S4"]
fn dns_s4_resolver_hiccup_is_unverifiable() {
    let outcome = Err(error_kind(&messages(&["dns error"])));
    assert_eq!(
        classify(&outcome, &HostConfig::default()),
        Verdict::Unverifiable {
            reason: "dns lookup failed".to_owned()
        }
    );
}

//! A machine with no CA certificates: rotguard exits 2 with a message instead of panicking.
#![cfg(target_os = "linux")]

use std::process::Command;

use crate::support::World;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "pending: S14"]
async fn ca_certs_s14_missing_ca_bundle_exits_2_without_a_panic() {
    let w = World::new().await;
    w.site.write("p.md", &format!("[x]({})\n", w.url("/x")));
    let empty = tempfile::tempdir().expect("temp dir");

    let output = Command::new(env!("CARGO_BIN_EXE_rotguard"))
        .arg(w.site.path())
        .env("SSL_CERT_FILE", empty.path().join("none.pem"))
        .env("SSL_CERT_DIR", empty.path())
        .output()
        .expect("run rotguard");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(2), "exit code; stderr: {stderr}");
    assert!(stderr.starts_with("rotguard: "), "stderr: {stderr}");
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
}

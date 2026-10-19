//! Acceptance tests: one test per design scenario, driving only the `rotguard` binary.
//!
//! Each test is `#[ignore = "pending: S<n>"]` until the task that delivers its feature removes
//! the attribute.

mod apply;
mod archive;
mod check;
mod config;
mod extract;
mod report;
mod soft404;
mod support;

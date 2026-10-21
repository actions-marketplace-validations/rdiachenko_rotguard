//! Acceptance tests: one test per design scenario, driving the `rotguard` binary.
//! `progress.rs` also drives the library, for S4.

mod action;
mod apply;
mod apply_failed;
mod archive;
mod blocked;
mod build_cache;
mod check;
mod config;
mod directory_probe;
mod dns;
mod extract;
mod format;
mod markdown;
mod progress;
mod report;
mod simplify;
mod soft404;
mod support;

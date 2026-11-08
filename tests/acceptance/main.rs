//! Acceptance tests: one test per design scenario, driving the `rotguard` binary.
//! `progress.rs` also drives the library, for S4.

mod action;
mod action_path;
mod apply;
mod apply_failed;
mod archive;
mod blocked;
mod build_cache;
mod ca_certs;
mod check;
mod config;
mod directory_probe;
mod dns;
mod extract;
mod fetch;
mod foreign_sites;
mod format;
mod lookup_order;
mod markdown;
mod progress;
mod release;
mod report;
mod root_redirect;
mod simplify;
mod soft404;
mod support;

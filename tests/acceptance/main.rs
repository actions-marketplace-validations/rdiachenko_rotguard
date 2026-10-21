//! Acceptance tests: one test per design scenario, driving the `rotguard` binary.
//! `progress.rs` also drives the library, for S4.

mod action;
mod apply;
mod archive;
mod blocked;
mod check;
mod config;
mod dns;
mod extract;
mod format;
mod markdown;
mod progress;
mod report;
mod simplify;
mod soft404;
mod support;

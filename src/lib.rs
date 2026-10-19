use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

pub mod archive;
pub mod check;
pub mod config;
pub mod extract;

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
    /// Fail on any finding, not only on broken links.
    #[arg(long)]
    pub strict: bool,
}

/// Runs the main flow and returns the process exit code.
#[must_use]
pub fn run(args: Args) -> ExitCode {
    let _config = match config::load(&args.site_dir) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("rotguard: {err}");
            return ExitCode::from(2);
        }
    };
    ExitCode::SUCCESS
}

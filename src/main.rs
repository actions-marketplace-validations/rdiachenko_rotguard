use std::process::ExitCode;

use clap::Parser;
use rotguard::Args;

fn main() -> ExitCode {
    rotguard::run(Args::parse())
}

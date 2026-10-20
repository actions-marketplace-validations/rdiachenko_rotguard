//! Progress lines on stderr: one pair of lines per phase when piped, one rewritten line on a terminal.

use std::io::{self, IsTerminal, Write};
use std::time::Instant;

/// The phase of a run that progress is reported for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Checking links; `hosts` is the number of distinct hosts among the URLs.
    Check { hosts: usize },
    /// Looking up archive snapshots.
    Archive,
}

/// Writes progress for the current phase to `out`.
///
/// Piped (`live` false), a phase prints a start line and an end line: `checking <N> <link|links>
/// on <H> <host|hosts>` and `checked <N> <link|links> in <T>s`, or `looking up <N>
/// <snapshot|snapshots>` and `looked up <N> <snapshot|snapshots> in <T>s`. On a terminal (`live`
/// true), one line is rewritten in place: `checking links: <D>/<N> done` or `looking up
/// snapshots: <D>/<N> done`, cleared and replaced by the end line when the phase ends.
/// Write errors are ignored: progress never fails a run.
#[derive(Debug)]
pub struct Progress<W: Write> {
    #[expect(dead_code, reason = "printed from T3")]
    out: W,
    #[expect(dead_code, reason = "printed from T3")]
    live: bool,
    #[expect(dead_code, reason = "printed from T3")]
    phase: Option<(Phase, usize, Instant)>,
}

impl Progress<io::Stderr> {
    /// Progress on stderr, live when stderr is a terminal.
    #[must_use]
    pub fn stderr() -> Self {
        Self::new(io::stderr(), io::stderr().is_terminal())
    }
}

impl<W: Write> Progress<W> {
    /// Progress that writes to `out`, rewriting one line in place when `live`.
    #[must_use]
    pub fn new(out: W, live: bool) -> Self {
        Self {
            out,
            live,
            phase: None,
        }
    }

    /// Starts a phase of `total` items. Piped, prints the start line (`checking <N> <link|links>
    /// on <H> <host|hosts>` or `looking up <N> <snapshot|snapshots>`); live, draws
    /// `checking links: 0/<N> done` or `looking up snapshots: 0/<N> done`.
    pub fn start(&mut self, _phase: Phase, _total: usize) {}

    /// Reports `done` items finished. Live, redraws `<D>/<N> done` in the same line; piped,
    /// prints nothing.
    pub fn tick(&mut self, _done: usize) {}

    /// Ends the phase. Live, clears the line with `\r\x1b[2K` first; then prints the end line
    /// (`checked <N> <link|links> in <T>s` or `looked up <N> <snapshot|snapshots> in <T>s`,
    /// T being whole elapsed seconds rounded down) and a newline.
    pub fn finish(&mut self) {}
}

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
    out: W,
    live: bool,
    phase: Option<Running>,
}

/// A phase that has started and not yet finished.
#[derive(Debug)]
struct Running {
    phase: Phase,
    total: usize,
    began: Instant,
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

impl Running {
    fn label(&self) -> &'static str {
        match self.phase {
            Phase::Check { .. } => "checking links",
            Phase::Archive => "looking up snapshots",
        }
    }

    fn start_line(&self) -> String {
        let n = self.total;
        match self.phase {
            Phase::Check { hosts } => format!(
                "checking {n} {} on {hosts} {}",
                plural(n, "link", "links"),
                plural(hosts, "host", "hosts"),
            ),
            Phase::Archive => format!("looking up {n} {}", plural(n, "snapshot", "snapshots")),
        }
    }

    fn end_line(&self) -> String {
        let n = self.total;
        let secs = self.began.elapsed().as_secs();
        match self.phase {
            Phase::Check { .. } => format!("checked {n} {} in {secs}s", plural(n, "link", "links")),
            Phase::Archive => format!(
                "looked up {n} {} in {secs}s",
                plural(n, "snapshot", "snapshots")
            ),
        }
    }
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
    pub fn start(&mut self, phase: Phase, total: usize) {
        let running = Running {
            phase,
            total,
            began: Instant::now(),
        };
        let text = if self.live {
            format!("\r{}: 0/{total} done", running.label())
        } else {
            format!("{}\n", running.start_line())
        };
        self.phase = Some(running);
        self.write(&text);
    }

    /// Reports `done` items finished. Live, redraws `<D>/<N> done` in the same line; piped,
    /// prints nothing.
    pub fn tick(&mut self, done: usize) {
        if !self.live {
            return;
        }
        let Some(running) = &self.phase else { return };
        let text = format!("\r{}: {done}/{} done", running.label(), running.total);
        self.write(&text);
    }

    /// Ends the phase. Live, clears the line with `\r\x1b[2K` first; then prints the end line
    /// (`checked <N> <link|links> in <T>s` or `looked up <N> <snapshot|snapshots> in <T>s`,
    /// T being whole elapsed seconds rounded down) and a newline.
    pub fn finish(&mut self) {
        let Some(running) = self.phase.take() else {
            return;
        };
        let clear = if self.live { "\r\x1b[2K" } else { "" };
        self.write(&format!("{clear}{}\n", running.end_line()));
    }

    fn write(&mut self, text: &str) {
        // Progress never fails a run, so write and flush errors are ignored.
        let _ = self.out.write_all(text.as_bytes());
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replaces the elapsed seconds before the final `s\n` with `T`.
    fn mask_seconds(text: &str) -> String {
        let body = text.strip_suffix("s\n").expect("text ends with `s\\n`");
        let digits = body.chars().rev().take_while(char::is_ascii_digit).count();
        assert!(digits > 0, "no seconds in {text:?}");
        format!("{}Ts\n", &body[..body.len() - digits])
    }

    #[test]
    fn prints_plain_and_live_lines() {
        let check = |hosts| Phase::Check { hosts };
        let cases = [
            (
                check(1),
                1,
                vec![1],
                false,
                "checking 1 link on 1 host\nchecked 1 link in Ts\n",
            ),
            (
                check(2),
                2,
                vec![1, 2],
                false,
                "checking 2 links on 2 hosts\nchecked 2 links in Ts\n",
            ),
            (
                check(0),
                0,
                vec![],
                false,
                "checking 0 links on 0 hosts\nchecked 0 links in Ts\n",
            ),
            (
                Phase::Archive,
                1,
                vec![1],
                false,
                "looking up 1 snapshot\nlooked up 1 snapshot in Ts\n",
            ),
            (
                Phase::Archive,
                2,
                vec![1, 2],
                false,
                "looking up 2 snapshots\nlooked up 2 snapshots in Ts\n",
            ),
            (
                Phase::Archive,
                0,
                vec![],
                false,
                "looking up 0 snapshots\nlooked up 0 snapshots in Ts\n",
            ),
            (
                check(1),
                1,
                vec![1],
                true,
                "\rchecking links: 0/1 done\rchecking links: 1/1 done\r\x1b[2Kchecked 1 link in Ts\n",
            ),
            (
                check(2),
                2,
                vec![1, 2],
                true,
                "\rchecking links: 0/2 done\rchecking links: 1/2 done\rchecking links: 2/2 done\r\x1b[2Kchecked 2 links in Ts\n",
            ),
            (
                check(0),
                0,
                vec![],
                true,
                "\rchecking links: 0/0 done\r\x1b[2Kchecked 0 links in Ts\n",
            ),
            (
                Phase::Archive,
                1,
                vec![1],
                true,
                "\rlooking up snapshots: 0/1 done\rlooking up snapshots: 1/1 done\r\x1b[2Klooked up 1 snapshot in Ts\n",
            ),
            (
                Phase::Archive,
                2,
                vec![1, 2],
                true,
                "\rlooking up snapshots: 0/2 done\rlooking up snapshots: 1/2 done\rlooking up snapshots: 2/2 done\r\x1b[2Klooked up 2 snapshots in Ts\n",
            ),
            (
                Phase::Archive,
                0,
                vec![],
                true,
                "\rlooking up snapshots: 0/0 done\r\x1b[2Klooked up 0 snapshots in Ts\n",
            ),
        ];
        for (phase, total, ticks, live, expected) in cases {
            let mut buf = Vec::new();
            {
                let mut progress = Progress::new(&mut buf, live);
                progress.start(phase, total);
                for done in ticks {
                    progress.tick(done);
                }
                progress.finish();
            }
            let text = String::from_utf8(buf).unwrap();
            assert_eq!(
                mask_seconds(&text),
                expected,
                "{phase:?} total={total} live={live}"
            );
        }
    }
}

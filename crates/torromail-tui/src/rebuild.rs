//! Rebuilding an account's cache. It logs into the mailbox and can run for
//! minutes, so it is a child process the event loop looks in on between
//! frames, never something the surface waits for.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};

/// What the rebuild last said about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Working { phase: String, mailbox: String, done: u64, total: u64 },
    Finished { messages: u64, size_bytes: u64 },
    Failed(String),
}

/// One `--rebuild-cache` progress line: `{"phase": …, "done": …, "total": …}`,
/// and at the end `{"phase": "done", "messages": …, "size_bytes": …}`.
#[must_use]
pub fn parse_line(line: &str) -> Option<Progress> {
    let values = torromail_control::logs::parse_flat_object(line)?;
    let number = |key: &str| values.get(key).and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
    let phase = values.get("phase")?.clone();
    Some(if phase == "done" {
        Progress::Finished { messages: number("messages"), size_bytes: number("size_bytes") }
    } else {
        Progress::Working {
            phase,
            mailbox: values.get("mailbox").cloned().unwrap_or_default(),
            done: number("done"),
            total: number("total"),
        }
    })
}

pub struct Rebuild {
    pub account_id: String,
    child: Child,
    lines: Receiver<String>,
    finished: bool,
}

impl Rebuild {
    /// Starts the prepared command and reads its stdout on a thread of its
    /// own, so a quiet child never blocks a frame.
    pub fn start(account_id: &str, mut command: Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("torromail-mcp could not be started: {error}"))?;
        let stdout = child.stdout.take().ok_or("the rebuild has no output to read")?;
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { account_id: account_id.to_owned(), child, lines, finished: false })
    }

    /// The newest thing worth showing since the last look, if anything.
    pub fn poll(&mut self) -> Option<Progress> {
        let mut latest = None;
        loop {
            match self.lines.try_recv() {
                Ok(line) => {
                    if let Some(progress) = parse_line(&line) {
                        self.finished |= matches!(progress, Progress::Finished { .. });
                        latest = Some(progress);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Output is closed: the child is done or about to be.
                    if latest.is_none() && !self.finished {
                        self.finished = true;
                        let output = self.child.wait().ok();
                        let mut reason = String::new();
                        if let Some(mut stderr) = self.child.stderr.take() {
                            let _ = std::io::Read::read_to_string(&mut stderr, &mut reason);
                        }
                        if output.is_some_and(|status| status.success()) {
                            return Some(Progress::Finished { messages: 0, size_bytes: 0 });
                        }
                        return Some(Progress::Failed(reason.trim().to_owned()));
                    }
                    break;
                }
            }
        }
        latest
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

impl Drop for Rebuild {
    /// Leaving the surface ends the rebuild with it: a login nobody watches
    /// should not keep running behind a closed terminal.
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

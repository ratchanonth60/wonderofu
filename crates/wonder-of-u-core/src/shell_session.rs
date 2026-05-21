//! Persistent shell session management for the bash tool.
//!
//! A [`ShellSession`] keeps a bash process alive between tool calls,
//! preserving `$PWD`, environment variables, and shell functions – matching
//! the behaviour of the TypeScript `ShellSnapshot`.
//!
//! [`ShellSessionStore`] owns one session per [`SessionId`] and is meant to
//! be stored in [`crate::ToolContext::bash_session_store`] behind an
//! `Arc<Mutex<…>>` so that it can be shared across tool calls within a
//! single agent turn.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use crate::{Result, SessionId, WonderError};

// ---------------------------------------------------------------------------
// Nonce generator
// ---------------------------------------------------------------------------

static NONCE: AtomicU64 = AtomicU64::new(1);

fn next_nonce() -> u64 {
    NONCE.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// ShellOutput
// ---------------------------------------------------------------------------

/// The captured result of a single command executed in a [`ShellSession`].
#[derive(Clone, Debug, PartialEq)]
pub struct ShellOutput {
    /// Lines written to the shell's stdout while the command ran.
    pub stdout: String,
    /// Always empty: stderr is left on the terminal via `Stdio::inherit`.
    pub stderr: String,
    /// The exit code returned by the command.
    pub exit_code: i32,
}

// ---------------------------------------------------------------------------
// ShellSession
// ---------------------------------------------------------------------------

/// A long-lived bash process whose stdin/stdout remain open between calls.
pub struct ShellSession {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    /// Last known working directory (updated after every `run` call).
    cwd: PathBuf,
}

impl ShellSession {
    /// Spawns a new interactive bash session rooted at `cwd`.
    ///
    /// The session runs `bash --norc --noprofile` to keep start-up time
    /// minimal and avoid user config interfering with sentinel detection.
    pub fn spawn(cwd: &Path) -> Result<Self> {
        let mut child = Command::new("bash")
            .arg("--norc")
            .arg("--noprofile")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr stays on the terminal so error output is visible.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| WonderError::internal(format!("failed to spawn bash session: {e}")))?;

        let stdin = BufWriter::new(
            child
                .stdin
                .take()
                .ok_or_else(|| WonderError::internal("bash session stdin unavailable"))?,
        );
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| WonderError::internal("bash session stdout unavailable"))?,
        );

        Ok(Self {
            child,
            stdin,
            stdout,
            cwd: cwd.to_owned(),
        })
    }

    /// Runs `command` in the persistent bash session and returns the output.
    ///
    /// # Sentinel pattern
    ///
    /// After writing the user command we immediately write:
    /// ```text
    /// __wonder_exit=$?; printf 'DONE_SENTINEL_<nonce>_%d\n' $__wonder_exit
    /// ```
    /// We then drain stdout lines until the sentinel appears, capturing
    /// everything before it as the command's output.
    ///
    /// # Timeout
    ///
    /// If `timeout` elapses before the sentinel is seen the child process is
    /// killed and a [`WonderError`] is returned.
    pub fn run(&mut self, command: &str, timeout: Duration) -> Result<ShellOutput> {
        let nonce = next_nonce();
        let sentinel = format!("DONE_SENTINEL_{nonce}_");

        // Write the command followed immediately by the sentinel printer.
        // We capture exit status via $? immediately after the command.
        // The \n before DONE_SENTINEL ensures the sentinel always starts on its own
        // line even when the command output doesn't end with a newline.
        writeln!(
            self.stdin,
            "{command}\n__wonder_exit=$?; printf '\\nDONE_SENTINEL_{nonce}_%d\\n' $__wonder_exit"
        )
        .map_err(|e| WonderError::internal(format!("failed to write to bash stdin: {e}")))?;
        self.stdin
            .flush()
            .map_err(|e| WonderError::internal(format!("failed to flush bash stdin: {e}")))?;

        // Collect stdout lines until the sentinel appears or the timeout fires.
        let deadline = Instant::now() + timeout;
        let mut output_lines: Vec<String> = Vec::new();
        let exit_code: i32;
        let mut line = String::new();

        loop {
            if Instant::now() >= deadline {
                // Kill the child so it does not linger.
                let _ = self.child.kill();
                return Err(WonderError::internal(format!(
                    "bash command timed out after {}s",
                    timeout.as_secs()
                )));
            }

            line.clear();
            match self.stdout.read_line(&mut line) {
                Ok(0) => {
                    // EOF – process exited unexpectedly.
                    return Err(WonderError::internal(
                        "bash session closed unexpectedly (EOF on stdout)",
                    ));
                }
                Ok(_) => {
                    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                    if let Some(rest) = trimmed.strip_prefix(&sentinel) {
                        // Parse the exit code that follows the sentinel prefix.
                        exit_code = rest.trim().parse::<i32>().unwrap_or(0);
                        break;
                    }
                    output_lines.push(trimmed.to_owned());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Non-blocking pipe; back off briefly.
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => {
                    return Err(WonderError::internal(format!(
                        "error reading bash stdout: {e}"
                    )));
                }
            }
        }

        // Update the cached cwd after every command (best-effort).
        if let Ok(new_cwd) = self.query_cwd_internal() {
            self.cwd = new_cwd;
        }

        // Drop trailing empty lines injected by the leading \n of the sentinel.
        while output_lines
            .last()
            .map(|s: &String| s.is_empty())
            .unwrap_or(false)
        {
            output_lines.pop();
        }

        let stdout = output_lines.join("\n");
        let stdout = if stdout.is_empty() {
            stdout
        } else {
            format!("{stdout}\n")
        };

        Ok(ShellOutput {
            stdout,
            stderr: String::new(),
            exit_code,
        })
    }

    /// Returns the shell's current working directory by running `pwd`.
    pub fn get_cwd(&mut self) -> Result<PathBuf> {
        let out = self.run("pwd", Duration::from_secs(5))?;
        let raw = out.stdout.trim().to_owned();
        if raw.is_empty() {
            Ok(self.cwd.clone())
        } else {
            let path = PathBuf::from(raw);
            self.cwd = path.clone();
            Ok(path)
        }
    }

    /// Returns `true` if the child bash process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Query the cwd using its own sentinel, called inside `run` after output
    /// has been collected.  Uses a distinct sentinel prefix so it cannot
    /// collide with user output.
    fn query_cwd_internal(&mut self) -> std::result::Result<PathBuf, ()> {
        let nonce = next_nonce();
        let sentinel = format!("CWDQ_{nonce}_");

        writeln!(self.stdin, "printf 'CWDQ_{nonce}_%s\\n' \"$(pwd)\"").map_err(|_| ())?;
        self.stdin.flush().map_err(|_| ())?;

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut line = String::new();
        loop {
            if Instant::now() >= deadline {
                return Err(());
            }
            line.clear();
            match self.stdout.read_line(&mut line) {
                Ok(0) => return Err(()),
                Ok(_) => {
                    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                    if let Some(path_str) = trimmed.strip_prefix(&sentinel) {
                        return Ok(PathBuf::from(path_str));
                    }
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
}

impl Drop for ShellSession {
    fn drop(&mut self) {
        // Attempt a clean exit before forcing a kill.
        let _ = writeln!(self.stdin, "exit 0");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
    }
}

// ---------------------------------------------------------------------------
// ShellSessionStore
// ---------------------------------------------------------------------------

/// Holds one [`ShellSession`] per [`SessionId`].
///
/// Intended to be wrapped in `Arc<Mutex<ShellSessionStore>>` and stored on
/// [`crate::ToolContext`] so that multiple tool calls within the same agent
/// session share a single bash process.
#[derive(Default)]
pub struct ShellSessionStore {
    sessions: BTreeMap<SessionId, ShellSession>,
}

impl std::fmt::Debug for ShellSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellSessionStore")
            .field("session_count", &self.sessions.len())
            .finish()
    }
}

impl ShellSessionStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a mutable reference to the session for `session_id`, creating
    /// it (rooted at `cwd`) if none exists yet.
    ///
    /// If an existing session is found to be dead it is replaced with a fresh
    /// one.
    pub fn get_or_create(
        &mut self,
        session_id: SessionId,
        cwd: &Path,
    ) -> Result<&mut ShellSession> {
        // Evict a dead session before trying to reuse it.
        if let Some(existing) = self.sessions.get_mut(&session_id) {
            if !existing.is_alive() {
                self.sessions.remove(&session_id);
            }
        }

        if let std::collections::btree_map::Entry::Vacant(e) = self.sessions.entry(session_id) {
            let session = ShellSession::spawn(cwd)?;
            e.insert(session);
        }

        Ok(self
            .sessions
            .get_mut(&session_id)
            .expect("just inserted; cannot be absent"))
    }

    /// Removes the session for `session_id`, if one exists.
    pub fn remove(&mut self, session_id: &SessionId) {
        self.sessions.remove(session_id);
    }

    /// Removes all sessions whose bash process has already exited.
    pub fn cleanup_dead(&mut self) {
        self.sessions.retain(|_, s| s.is_alive());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn spawn_and_echo() {
        let mut session = ShellSession::spawn(&tmp_dir()).expect("spawn");
        let out = session
            .run("echo hello", Duration::from_secs(10))
            .expect("run");
        assert_eq!(out.stdout, "hello\n");
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn non_zero_exit_from_false() {
        let mut session = ShellSession::spawn(&tmp_dir()).expect("spawn");
        let out = session
            .run("false", Duration::from_secs(10))
            .expect("run false");
        assert_eq!(out.exit_code, 1);
    }

    #[test]
    fn cwd_changes_persist() {
        let mut session = ShellSession::spawn(&tmp_dir()).expect("spawn");
        session.run("cd /tmp", Duration::from_secs(10)).expect("cd");
        let cwd = session.get_cwd().expect("get_cwd");
        // /tmp may be a symlink on macOS (/private/tmp); accept either.
        let cwd_str = cwd.to_string_lossy();
        assert!(
            cwd_str == "/tmp" || cwd_str.ends_with("/tmp"),
            "expected path ending in /tmp, got {cwd_str}"
        );
    }

    #[test]
    fn sequential_commands_share_env() {
        let mut session = ShellSession::spawn(&tmp_dir()).expect("spawn");
        session
            .run("export WONDER_TEST_VAR=persistent", Duration::from_secs(10))
            .expect("export");
        let out = session
            .run("printf '%s' \"$WONDER_TEST_VAR\"", Duration::from_secs(10))
            .expect("read var");
        assert!(
            out.stdout.contains("persistent"),
            "expected 'persistent', got {:?}",
            out.stdout
        );
    }

    #[test]
    fn store_get_or_create() {
        let mut store = ShellSessionStore::new();
        let id = SessionId::new();
        let cwd = tmp_dir();
        let session = store.get_or_create(id, &cwd).expect("create");
        let out = session
            .run("echo store_test", Duration::from_secs(10))
            .expect("run");
        assert!(out.stdout.contains("store_test"));
    }

    #[test]
    fn store_reuses_session_across_calls() {
        let mut store = ShellSessionStore::new();
        let id = SessionId::new();
        let cwd = tmp_dir();

        store
            .get_or_create(id, &cwd)
            .expect("first")
            .run("export STORE_VAR=alive", Duration::from_secs(10))
            .expect("set var");

        let out = store
            .get_or_create(id, &cwd)
            .expect("second")
            .run("printf '%s' \"$STORE_VAR\"", Duration::from_secs(10))
            .expect("read var");

        assert!(
            out.stdout.contains("alive"),
            "expected 'alive', got {:?}",
            out.stdout
        );
    }
}

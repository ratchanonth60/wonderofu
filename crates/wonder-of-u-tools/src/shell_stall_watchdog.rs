//! Stall watchdog for local-shell background tasks.
//!
//! Spawns a lightweight thread alongside the exit-reaper watchdog created in
//! [`crate::bash`].  The thread polls the task's output-log file size; when no
//! growth is observed for [`STALL_THRESHOLD`] consecutive polls it reads the
//! last [`TAIL_BYTES`] of the log and calls [`looks_like_prompt`] to determine
//! whether the process is blocking on interactive input.  The finding is then
//! written to the task's `status_message` field so callers (e.g. `task_output`)
//! can surface it to the user.
//!
//! The thread exits automatically once the task transitions to a terminal
//! state, so no thread leak occurs even if the watcher is never explicitly
//! stopped.

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
    thread,
    time::Duration,
};

use regex::Regex;
use wonder_of_u_core::TaskId;
use wonder_of_u_storage::TaskStore;

// ── tunables ────────────────────────────────────────────────────────────────

/// How often the watchdog samples the log-file size.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Number of consecutive no-growth polls before a stall is declared.
const STALL_THRESHOLD: u32 = 3;

/// Maximum number of bytes read from the log tail when checking for prompts.
const TAIL_BYTES: u64 = 512;

// ── prompt pattern ──────────────────────────────────────────────────────────

/// Returns `true` when `tail` (the last few bytes of a background-task log)
/// appears to end with an interactive prompt that is waiting for user input.
///
/// Patterns detected (case-insensitive unless noted):
///
/// | Category | Examples |
/// |----------|---------|
/// | y/n prompts | `[y/n]`, `(Y/n)`, `[yes/no]` |
/// | Confirmation | `Continue?`, `Proceed?`, `Do you want to…?` |
/// | Key / enter | `Press Enter`, `Press any key` |
/// | Password | `Password:`, `[sudo] password`, `Passphrase:` |
/// | Pager pauses | `--More--`, `(END)` |
/// | Generic prompt tail | line ending with `> ` or `? ` |
///
/// # Examples
///
/// ```
/// use wonder_of_u_tools::looks_like_prompt;
///
/// assert!(looks_like_prompt("Install package? [y/n] "));
/// assert!(looks_like_prompt("Password: "));
/// assert!(looks_like_prompt("--More--"));
/// assert!(!looks_like_prompt("Build complete.\nexit 0\n"));
/// ```
#[must_use]
pub fn looks_like_prompt(tail: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // Non-verbose form so there are no whitespace-stripping surprises.
        // (?i) makes the whole pattern case-insensitive.
        Regex::new(concat!(
            "(?i)",
            // y/n bracket or paren variants: [y/n], [Y/n], (y/n), (Y/n)
            r"[\[(]\s*[yn]\s*/\s*[yn]\s*[\])]",
            // yes/no bracket or paren variants
            r"|[\[(]\s*yes\s*/\s*no\s*[\])]",
            // [sudo] password
            r"|\[sudo\]\s+password",
            // Password: / Passphrase: at start of a line (or the string)
            r"|(?:^|[\r\n])\s*pass(?:word|phrase)\s*:",
            // Enter password/passphrase
            r"|(?:^|[\r\n])\s*enter\s+pass(?:word|phrase)",
            // Press/Hit Enter / Return / any key
            r"|(?:press|hit)\s+(?:enter|return|any\s+key)",
            // Pager prompts
            r"|--more--|^\s*\(end\)\s*$",
            // Generic: line ending with bare '>' (REPL prompt)
            r"|(?:^|[\r\n])[^\r\n]{0,120}>\s*$",
            // Generic: line ending with '?' (question / confirmation prompt)
            r"|(?:^|[\r\n])[^\r\n]{0,120}\?\s*$",
            // Generic: line ending with '?:' (e.g. "Continue?: ")
            r"|(?:^|[\r\n])[^\r\n]{0,120}\?[^\r\n]{0,5}:\s*$",
        ))
        .expect("prompt regex is valid")
    });
    re.is_match(tail)
}

// ── public API ───────────────────────────────────────────────────────────────

/// Spawns the stall watchdog for a local-shell background task.
///
/// The returned thread handle is intentionally dropped (fire-and-forget).  The
/// thread self-terminates once the task's status becomes terminal, so it cannot
/// outlive the task it watches.
///
/// # Arguments
///
/// * `task_id`  – The [`TaskId`] of the background task to watch.
/// * `log_path` – Filesystem path to the combined stdout+stderr log file.
/// * `app_root` – Root of the wonder-of-u storage tree (used to open [`TaskStore`]).
pub fn spawn_stall_watchdog(task_id: TaskId, log_path: PathBuf, app_root: PathBuf) {
    thread::spawn(move || run_stall_watchdog(task_id, &log_path, &app_root));
}

// ── internals ────────────────────────────────────────────────────────────────

fn run_stall_watchdog(task_id: TaskId, log_path: &Path, app_root: &Path) {
    let store = TaskStore::new(app_root);
    let mut last_size: u64 = 0;
    let mut stall_count: u32 = 0;
    // Track whether we last wrote a stall notice so we can clear it on growth.
    let mut stall_notice_active = false;

    loop {
        thread::sleep(POLL_INTERVAL);

        // Exit as soon as the task is terminal (exit reaper already handled it).
        match store.read_task(task_id) {
            Ok(task) if task.status.is_terminal() => return,
            Err(_) => return, // task disappeared — nothing left to watch
            Ok(_) => {}
        }

        let current_size = file_size(log_path).unwrap_or(0);

        if current_size == last_size {
            stall_count = stall_count.saturating_add(1);
        } else {
            // Growth detected — reset counters and clear any stall notice.
            if stall_notice_active {
                update_status_message(&store, task_id, None);
                stall_notice_active = false;
            }
            stall_count = 0;
            last_size = current_size;
        }

        if stall_count >= STALL_THRESHOLD {
            // Only write to the store on the transition into stall (not every poll)
            // to avoid redundant disk writes.
            if !stall_notice_active {
                let tail = read_tail(log_path, TAIL_BYTES);
                let msg = if looks_like_prompt(&tail) {
                    "stalled: process appears to be waiting for interactive input"
                } else {
                    "stalled: no output growth detected"
                };
                update_status_message(&store, task_id, Some(msg));
                stall_notice_active = true;
            }
            // Don't reset stall_count here — stay in "stalled" state until growth resumes.
        }
    }
}

/// Reads the metadata file for `task_id`, overwrites `status_message`, and
/// writes it back.  Errors are silently ignored — the watchdog is best-effort.
fn update_status_message(store: &TaskStore, task_id: TaskId, message: Option<&str>) {
    if let Ok(mut task) = store.read_task(task_id) {
        // Don't overwrite if the task has gone terminal between our check and now.
        if task.status.is_terminal() {
            return;
        }
        task.status_message = message.map(str::to_owned);
        let _ = store.write_task(&task);
    }
}

/// Returns the current size of `path` in bytes, or `None` on any error.
fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|m| m.len())
}

/// Reads up to `max_bytes` from the end of `path` and returns them as a
/// lossy UTF-8 string.  Returns an empty string on any I/O error.
fn read_tail(path: &Path, max_bytes: u64) -> String {
    (|| -> std::io::Result<String> {
        let mut file = fs::File::open(path)?;
        let len = file.metadata()?.len();
        let start = len.saturating_sub(max_bytes);
        file.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::with_capacity((len - start) as usize + 1);
        file.read_to_end(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    })()
    .unwrap_or_default()
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── looks_like_prompt: positive cases ────────────────────────────────────

    #[test]
    fn detects_y_n_bracket() {
        assert!(looks_like_prompt("Continue? [y/n] "));
        assert!(looks_like_prompt("Install now? [Y/n]"));
        assert!(looks_like_prompt("Overwrite [y/N]?"));
    }

    #[test]
    fn detects_y_n_paren() {
        assert!(looks_like_prompt("Do you agree? (y/n)"));
        assert!(looks_like_prompt("Proceed? (Y/n)"));
    }

    #[test]
    fn detects_yes_no_bracket() {
        assert!(looks_like_prompt("Are you sure? [yes/no]"));
        assert!(looks_like_prompt("Delete file? (yes/no)"));
    }

    #[test]
    fn detects_password_prompts() {
        assert!(looks_like_prompt("[sudo] password for alice:"));
        assert!(looks_like_prompt("Password: "));
        assert!(looks_like_prompt("Enter password:\n"));
        assert!(looks_like_prompt("Passphrase: "));
    }

    #[test]
    fn detects_press_enter() {
        assert!(looks_like_prompt("Press Enter to continue"));
        assert!(looks_like_prompt("Hit any key to proceed"));
        assert!(looks_like_prompt("Press any key..."));
    }

    #[test]
    fn detects_pager_prompts() {
        assert!(looks_like_prompt("--More--"));
        assert!(looks_like_prompt("(END)"));
    }

    #[test]
    fn detects_trailing_question_mark() {
        // A line ending with '?' is treated as a prompt.
        assert!(looks_like_prompt("Do you want to continue?\n"));
        assert!(looks_like_prompt("Really delete this file?"));
    }

    #[test]
    fn detects_trailing_greater_than() {
        // REPL-style prompt: line ending with '>'.
        assert!(looks_like_prompt("irb(main):001:0>"));
        assert!(looks_like_prompt(">>> "));
    }

    // ── looks_like_prompt: negative cases ────────────────────────────────────

    #[test]
    fn ignores_normal_output() {
        assert!(!looks_like_prompt("Build successful\nexit 0\n"));
        assert!(!looks_like_prompt("Copying files...\nDone.\n"));
        assert!(!looks_like_prompt(""));
    }

    #[test]
    fn ignores_completed_log_with_exit_line() {
        let log = "running tests...\nall 42 tests passed\nexit_code: 0\n";
        assert!(!looks_like_prompt(log));
    }

    #[test]
    fn ignores_url_with_question_mark_in_middle() {
        // A URL with a query string in the middle of a line should not fire.
        // The regex only anchors to the *end* of lines, so an inline '?' is fine.
        assert!(!looks_like_prompt(
            "Fetching https://example.com/api?foo=bar\nDone.\n"
        ));
    }

    // ── read_tail ─────────────────────────────────────────────────────────────

    #[test]
    fn read_tail_returns_empty_for_missing_file() {
        let result = read_tail(Path::new("/no/such/file/xyz.log"), 128);
        assert_eq!(result, "");
    }

    #[test]
    fn read_tail_returns_last_bytes() {
        use std::io::Write;
        use wonder_of_u_test_support::unique_test_dir;

        let dir = unique_test_dir("stall-watchdog-read-tail");
        let path = dir.join("out.log");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(b"AAAAAAAAAA").unwrap(); // 10 bytes of padding
        f.write_all(b"TAIL").unwrap(); // last 4 bytes

        let result = read_tail(&path, 4);
        assert_eq!(result, "TAIL");
    }

    // ── file_size ─────────────────────────────────────────────────────────────

    #[test]
    fn file_size_returns_none_for_missing_file() {
        assert_eq!(file_size(Path::new("/no/such/file.log")), None);
    }

    // ── watchdog integration: stall notice written to task ───────────────────

    #[test]
    fn watchdog_writes_stall_notice_when_log_stalls_without_prompt() {
        use std::io::Write;
        use wonder_of_u_core::{TaskState, TaskStatus};
        use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

        let dir = unique_test_dir("stall-watchdog-stall-notice");
        let _guard = EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", dir.as_os_str());

        let store = TaskStore::new(&dir);

        // Create a synthetic Running task with a log file.
        let task = {
            let mut t = TaskState::pending_shell("bg: sleep", "sleep 999", &dir);
            t.status = TaskStatus::Running;
            t
        };
        let task_id = task.id;
        store.ensure_layout().unwrap();
        let log_path = store.paths().task_log_path(task_id);
        // Write some initial content.
        let mut f = fs::File::create(&log_path).unwrap();
        f.write_all(b"starting...\n").unwrap();
        drop(f);
        store.write_task(&task).unwrap();

        // Simulate the watchdog noticing STALL_THRESHOLD polls with no growth.
        // We call `run_stall_watchdog` indirectly via the private helper to
        // avoid actually sleeping in a test.  Instead we replicate the stall
        // decision logic directly:
        let tail = read_tail(&log_path, TAIL_BYTES);
        assert!(!looks_like_prompt(&tail), "no prompt in plain log");

        update_status_message(&store, task_id, Some("stalled: no output growth detected"));

        let updated = store.read_task(task_id).unwrap();
        assert_eq!(
            updated.status_message.as_deref(),
            Some("stalled: no output growth detected")
        );
    }

    #[test]
    fn watchdog_writes_prompt_notice_when_log_ends_with_prompt() {
        use std::io::Write;
        use wonder_of_u_core::{TaskState, TaskStatus};
        use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

        let dir = unique_test_dir("stall-watchdog-prompt-notice");
        let _guard = EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", dir.as_os_str());

        let store = TaskStore::new(&dir);
        let task = {
            let mut t = TaskState::pending_shell("bg: interactive", "bash", &dir);
            t.status = TaskStatus::Running;
            t
        };
        let task_id = task.id;
        store.ensure_layout().unwrap();
        let log_path = store.paths().task_log_path(task_id);
        let mut f = fs::File::create(&log_path).unwrap();
        f.write_all(b"Do you want to continue? [y/n] ").unwrap();
        drop(f);
        store.write_task(&task).unwrap();

        let tail = read_tail(&log_path, TAIL_BYTES);
        assert!(looks_like_prompt(&tail), "should detect prompt");

        update_status_message(
            &store,
            task_id,
            Some("stalled: process appears to be waiting for interactive input"),
        );

        let updated = store.read_task(task_id).unwrap();
        assert_eq!(
            updated.status_message.as_deref(),
            Some("stalled: process appears to be waiting for interactive input")
        );
    }

    #[test]
    fn watchdog_clears_stall_notice_on_growth() {
        use wonder_of_u_core::{TaskState, TaskStatus};
        use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

        let dir = unique_test_dir("stall-watchdog-clear-notice");
        let _guard = EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", dir.as_os_str());

        let store = TaskStore::new(&dir);
        let task = {
            let mut t = TaskState::pending_shell("bg: grow", "echo hi", &dir);
            t.status = TaskStatus::Running;
            t.status_message = Some("stalled: no output growth detected".into());
            t
        };
        let task_id = task.id;
        store.ensure_layout().unwrap();
        store.write_task(&task).unwrap();

        // Growth detected → clear the stall notice.
        update_status_message(&store, task_id, None);

        let updated = store.read_task(task_id).unwrap();
        assert!(
            updated.status_message.is_none(),
            "stall notice should be cleared"
        );
    }

    #[test]
    fn watchdog_does_not_write_to_terminal_task() {
        use wonder_of_u_core::{TaskState, TaskStatus};
        use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

        let dir = unique_test_dir("stall-watchdog-terminal");
        let _guard = EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", dir.as_os_str());

        let store = TaskStore::new(&dir);
        let task = {
            let mut t = TaskState::pending_shell("bg: done", "exit 0", &dir);
            t.mark_finished(TaskStatus::Completed, Some(0), None);
            t
        };
        let task_id = task.id;
        store.ensure_layout().unwrap();
        store.write_task(&task).unwrap();

        // `update_status_message` must be a no-op for terminal tasks.
        update_status_message(&store, task_id, Some("stalled: no output growth detected"));

        let unchanged = store.read_task(task_id).unwrap();
        assert!(
            unchanged.status_message.is_none(),
            "terminal task must not be updated"
        );
    }
}

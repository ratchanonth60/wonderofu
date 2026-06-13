use std::{path::PathBuf, process::Command};

use crate::SandboxPolicy;

/// Output from a sandboxed command execution.
#[derive(Clone, Debug)]
pub struct SandboxOutput {
    /// Combined stdout+stderr or separated output.
    pub stdout: String,
    /// Stderr output.
    pub stderr: String,
    /// Exit status of the wrapped process.
    pub exit_code: Option<i32>,
    /// Whether the process was terminated by timeout.
    pub timed_out: bool,
    /// Whether sandboxing was actually applied.
    pub sandbox_applied: bool,
}

/// Result of attempting to sandbox a command.
#[derive(Clone, Debug)]
pub struct SandboxResult {
    /// The command output.
    pub output: SandboxOutput,
    /// Whether the sandbox was active for this run.
    pub sandbox_applied: bool,
    /// Human-readable description of the sandbox mode used.
    pub sandbox_mode: String,
}

/// Platform-specific sandbox runner.
pub struct SandboxRunner {
    /// The sandbox policy to apply.
    policy: SandboxPolicy,
    /// Whether bubblewrap was found on PATH.
    has_bwrap: bool,
}

impl SandboxRunner {
    /// Create a new sandbox runner with the given policy.
    #[must_use]
    pub fn new(policy: SandboxPolicy) -> Self {
        let has_bwrap = which::which("bwrap").is_ok();
        Self { policy, has_bwrap }
    }

    /// Check whether sandboxing is available on this platform.
    #[must_use]
    pub fn sandbox_available(&self) -> bool {
        cfg!(target_os = "linux") && self.has_bwrap
    }

    /// Wrap a shell command in the sandbox.
    /// Returns the wrapped command builder, or the original command
    /// if sandboxing is unavailable.
    pub fn wrap_command(&self, program: &str, args: &[&str], cwd: &PathBuf) -> Command {
        build_sandbox_command(program, args, cwd, &self.policy, self.has_bwrap)
    }

    /// Build a sandbox result from a command run inside or outside the sandbox.
    #[must_use]
    pub fn build_result(
        &self,
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
        timed_out: bool,
        sandbox_applied: bool,
    ) -> SandboxResult {
        let mode = if sandbox_applied {
            "bubblewrap".to_string()
        } else if cfg!(target_os = "linux") {
            "none (bubblewrap not found)".to_string()
        } else {
            format!("none (unsupported platform: {})", std::env::consts::OS)
        };

        SandboxResult {
            output: SandboxOutput {
                stdout,
                stderr,
                exit_code,
                timed_out,
                sandbox_applied,
            },
            sandbox_applied,
            sandbox_mode: mode,
        }
    }
}

/// Build a sandboxed command using bubblewrap on Linux, or fall back to
/// a plain command on unsupported platforms.
///
/// # Bubblewrap integration
///
/// On Linux with `bwrap` available, the command is wrapped inside a
/// bubblewrap container that:
/// - Creates a new mount namespace (`--unshare-pid --unshare-ipc`)
/// - Bind-mounts readable paths as read-only (`--ro-bind`)
/// - Bind-mounts writable paths as read-write (`--bind`)
/// - Creates a minimal `/proc` (`--proc /proc`)
/// - Creates a new `/tmp` (`--tmpfs /tmp` or `--bind /tmp /tmp`)
/// - Blocks network access (`--unshare-net`) unless explicitly allowed
pub fn build_sandbox_command(
    program: &str,
    args: &[&str],
    cwd: &PathBuf,
    policy: &SandboxPolicy,
    has_bwrap: bool,
) -> Command {
    if !cfg!(target_os = "linux") || !has_bwrap {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.current_dir(cwd);
        return cmd;
    }

    let mut cmd = Command::new("bwrap");

    cmd.arg("--unshare-pid")
        .arg("--unshare-ipc")
        .arg("--die-with-parent");

    if !policy.allow_network {
        cmd.arg("--unshare-net");
    }

    cmd.arg("--proc").arg("/proc");
    cmd.arg("--tmpfs").arg("/tmp");
    cmd.arg("--bind").arg("/dev").arg("/dev");

    for entry in &policy.readable_paths {
        let path_str = entry.path.to_string_lossy();
        cmd.arg("--ro-bind")
            .arg(path_str.as_ref())
            .arg(path_str.as_ref());
    }

    for entry in &policy.writable_paths {
        let path_str = entry.path.to_string_lossy();
        cmd.arg("--bind")
            .arg(path_str.as_ref())
            .arg(path_str.as_ref());
    }

    cmd.arg("--chdir").arg(cwd.to_string_lossy().as_ref());
    cmd.arg(program);
    cmd.args(args);

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_result_no_sandbox_on_non_linux() {
        let policy = SandboxPolicy::default();
        let runner = SandboxRunner::new(policy);
        let result = runner.build_result("hello".into(), String::new(), Some(0), false, false);
        assert!(!result.sandbox_applied);
        assert!(
            result.sandbox_mode.contains("unsupported") || result.sandbox_mode.contains("none")
        );
    }

    #[test]
    fn sandbox_result_sandbox_applied() {
        let policy = SandboxPolicy::default();
        let runner = SandboxRunner::new(policy);
        let result = runner.build_result("hello".into(), String::new(), Some(0), false, true);
        assert!(result.sandbox_applied);
        assert_eq!(result.sandbox_mode, "bubblewrap");
    }
}

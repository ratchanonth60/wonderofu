//! OS-level sandboxing for shell command execution.
//!
//! On Linux, prefers bubblewrap (`bwrap`) when available.
//! Falls back to Landlock-based sandboxing on Linux 5.13+.
//! On macOS, uses Seatbelt (`sandbox-exec`).
//! On other platforms, provides a no-op runner that logs a warning.

#![warn(missing_docs)]

mod policy;
mod runner;

pub use policy::{FileSystemEntry, PolicyDecision, SandboxPolicy};
pub use runner::{SandboxOutput, SandboxResult, SandboxRunner, build_sandbox_command};

#[cfg(test)]
mod tests;

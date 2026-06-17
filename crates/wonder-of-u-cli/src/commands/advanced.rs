use std::{
    env, fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde_json::json;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
};
use wonder_of_u_storage::StoragePaths;

/// Represents diagnostics command
pub struct DiagnosticsCommand;
/// Represents debug command
pub struct DebugCommand {
    storage_dir: Option<PathBuf>,
}

impl DiagnosticsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "diagnostics",
            "Inspect LSP and project diagnostics integrations",
            CommandKind::Local,
        )
    }
}
impl DebugCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "debug",
            "Show internal debug, cache, and performance diagnostics",
            CommandKind::Local,
        );
        spec.hidden = true;
        spec
    }
}

#[async_trait]
impl Command for DiagnosticsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_diagnostics(&context.cwd)))
    }
}
#[async_trait]
impl Command for DebugCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_debug_status(
            &context.cwd,
            self.storage_dir.as_deref(),
        )))
    }
}

fn render_diagnostics(cwd: &Path) -> String {
    let servers = [
        ("rust-analyzer", "Rust"),
        ("typescript-language-server", "TypeScript"),
        ("pyright-langserver", "Python"),
        ("gopls", "Go"),
        ("clangd", "C/C++"),
        ("jdtls", "Java"),
    ];
    let mut lines = vec![
        "## Diagnostics".into(),
        format!("cwd={}", cwd.display()),
        format!("cargo_project={}", cwd.join("Cargo.toml").exists()),
        format!("node_project={}", cwd.join("package.json").exists()),
        format!("python_project={}", cwd.join("pyproject.toml").exists()),
    ];
    for (binary, label) in servers {
        lines.push(format!(
            "lsp[{binary}]={} ({label})",
            executable_on_path(binary)
        ));
    }
    lines.push("status=diagnostics discovery complete".into());
    lines.join("\n")
}
fn render_debug_status(cwd: &Path, storage_dir: Option<&Path>) -> String {
    let storage = storage_dir.map(StoragePaths::new);
    let cache_dir = storage.as_ref().map(StoragePaths::config_dir);
    let target_exists = cwd.join("target").exists();
    let payload = json!({
        "cwd": cwd.display().to_string(),
        "storage_dir": storage.as_ref().map(|paths| paths.base_dir().display().to_string()),
        "cache_dir": cache_dir.as_ref().map(|path| path.display().to_string()),
        "target_dir_exists": target_exists,
        "open_file_descriptors": count_fd_entries(),
        "status": "debug diagnostics ready"
    });
    format!("## Debug\n{payload}")
}

fn count_fd_entries() -> Option<usize> {
    fs::read_dir("/proc/self/fd")
        .ok()
        .map(|entries| entries.count())
}

fn executable_on_path(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| path.join(name).is_file())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_reports_lsp_surface() {
        let rendered = render_diagnostics(Path::new("/workspace"));

        assert!(rendered.contains("## Diagnostics"));
        assert!(rendered.contains("lsp[rust-analyzer]="));
        assert!(rendered.contains("status=diagnostics discovery complete"));
    }

    #[test]
    fn debug_status_reports_storage_and_fd_info() {
        let rendered = render_debug_status(Path::new("/workspace"), Some(Path::new("/tmp/wou")));

        assert!(rendered.contains("## Debug"));
        assert!(rendered.contains("debug diagnostics ready"));
        assert!(rendered.contains("storage_dir"));
    }
}

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
/// Represents bridge command
pub struct BridgeCommand {
    storage_dir: Option<PathBuf>,
}
/// Represents voice command
pub struct VoiceCommand;
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

impl BridgeCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "bridge",
            "Show remote bridge/server transport status",
            CommandKind::Local,
        );
        spec.aliases.extend(["server".into(), "remote".into()]);
        spec
    }
}

impl VoiceCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "voice",
            "Inspect local voice input/output integration support",
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
impl Command for BridgeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_bridge_status(
            &context.cwd,
            self.storage_dir.as_deref(),
        )))
    }
}

#[async_trait]
impl Command for VoiceCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_voice_status()))
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

fn render_bridge_status(cwd: &Path, storage_dir: Option<&Path>) -> String {
    let bridge_config = storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir().join("bridge.json"));
    let payload = json!({
        "cwd": cwd.display().to_string(),
        "stdio": true,
        "sse": false,
        "websocket": false,
        "config_path": bridge_config.as_ref().map(|path| path.display().to_string()),
        "status": "local bridge status ready"
    });
    format!("## Bridge\n{}", payload)
}

fn render_voice_status() -> String {
    let payload = json!({
        "microphone_tools": {
            "ffmpeg": executable_on_path("ffmpeg"),
            "sox": executable_on_path("sox"),
            "arecord": executable_on_path("arecord"),
        },
        "speech_env": {
            "WONDER_OF_U_STT_COMMAND": env::var("WONDER_OF_U_STT_COMMAND").ok(),
            "WONDER_OF_U_TTS_COMMAND": env::var("WONDER_OF_U_TTS_COMMAND").ok(),
        },
        "status": "voice integration discovery complete"
    });
    format!("## Voice\n{payload}")
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
    fn bridge_status_reports_transports() {
        let rendered = render_bridge_status(Path::new("/workspace"), Some(Path::new("/tmp/wou")));

        assert!(rendered.contains("## Bridge"));
        assert!(rendered.contains("\"stdio\":true"));
        assert!(rendered.contains("bridge.json"));
    }

    #[test]
    fn voice_status_reports_local_commands() {
        let rendered = render_voice_status();

        assert!(rendered.contains("## Voice"));
        assert!(rendered.contains("WONDER_OF_U_STT_COMMAND"));
        assert!(rendered.contains("voice integration discovery complete"));
    }

    #[test]
    fn debug_status_reports_storage_and_fd_info() {
        let rendered = render_debug_status(Path::new("/workspace"), Some(Path::new("/tmp/wou")));

        assert!(rendered.contains("## Debug"));
        assert!(rendered.contains("debug diagnostics ready"));
        assert!(rendered.contains("storage_dir"));
    }
}

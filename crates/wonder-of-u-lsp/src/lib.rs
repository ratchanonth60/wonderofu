//! LSP client crate — spawn language servers and collect diagnostics.
//!
//! Architecture:
//! - LspManager holds a shared diagnostics map (per-file) behind `Arc<Mutex>`
//! - Each server runs in a child process with a reader thread
//! - The tick loop calls `snapshot()` (non-blocking) to get current diagnostics
//! - Follows the subprocess+thread pattern from `tools/src/bash.rs`

use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use lsp_types::{
    ClientCapabilities, InitializeParams, PublishDiagnosticsClientCapabilities,
    PublishDiagnosticsParams, TextDocumentClientCapabilities, Uri,
};
use serde_json::Value;
use wonder_of_u_core::{Result, WonderError};

/// Per-file diagnostic list shared across threads.
pub type DiagnosticsMap = Arc<Mutex<HashMap<PathBuf, Vec<lsp_types::Diagnostic>>>>;

/// Language server identifier used as the key in the manager.
pub type LanguageId = String;

/// Project marker file name → language ID mapping.
pub const PROJECT_MARKERS: &[(&str, &str)] = &[
    ("Cargo.toml", "rust-analyzer"),
    ("package.json", "typescript-language-server"),
    ("pyproject.toml", "pyright-langserver"),
    ("go.mod", "gopls"),
];

/// Language ID → default server binary name.
#[must_use]
pub fn default_server_command(lang: &str) -> &str {
    match lang {
        "rust-analyzer" => "rust-analyzer",
        "typescript-language-server" => "typescript-language-server",
        "pyright-langserver" => "pyright-langserver",
        "gopls" => "gopls",
        "clangd" => "clangd",
        _ => lang,
    }
}

/// Holds the running server process, its stdin writer, and the reader thread.
struct ServerHandle {
    child: Mutex<Child>,
    stdin: Arc<Mutex<Box<dyn Write + Send>>>,
    _reader: JoinHandle<()>,
}

/// Manages one or more language servers and aggregates diagnostics.
pub struct LspManager {
    diagnostics: DiagnosticsMap,
    servers: HashMap<LanguageId, ServerHandle>,
}

impl LspManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for LspManager {
    fn default() -> Self {
        Self {
            diagnostics: Arc::new(Mutex::new(HashMap::new())),
            servers: HashMap::new(),
        }
    }
}

impl LspManager {
    /// Returns a shared reference to the diagnostics map.
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticsMap {
        &self.diagnostics
    }

    /// Non-blocking snapshot of current diagnostics.
    #[must_use]
    pub fn snapshot(&self) -> HashMap<PathBuf, Vec<lsp_types::Diagnostic>> {
        self.diagnostics.lock().unwrap().clone()
    }

    /// Discovers the relevant language for `project_root` via project markers
    /// and auto-starts the server.  Returns the language ID if a server is running.
    ///
    /// Checks that the server binary exists on `PATH` before spawning.
    pub fn ensure_for_project(&mut self, project_root: &Path) -> Option<String> {
        let lang_id = PROJECT_MARKERS
            .iter()
            .find(|(marker, _)| project_root.join(marker).exists())
            .map(|(_, id)| id.to_string());

        if let Some(ref lang) = lang_id {
            if self.servers.contains_key(lang) {
                return lang_id;
            }
            let cmd = default_server_command(lang);
            if which::which(cmd).is_err() {
                return None;
            }
            if self.start_server(lang.clone(), project_root, cmd).is_ok() {
                return lang_id;
            }
        }
        None
    }

    /// Starts a language server for `language`, rooted at `project_root`.
    pub fn start_server(
        &mut self,
        language: String,
        project_root: &Path,
        server_cmd: &str,
    ) -> Result<()> {
        let root_uri = uri_from_path(project_root);
        let root_uri: Uri = root_uri
            .parse()
            .map_err(|e| WonderError::validation(format!("invalid project root: {e}")))?;

        let mut child = Command::new(server_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| WonderError::validation(format!("failed to spawn {language}: {e}")))?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stdin: Box<dyn Write + Send> = Box::new(child.stdin.take().expect("stdin piped"));
        let stdin = Arc::new(Mutex::new(stdin));
        let diagnostics = Arc::clone(&self.diagnostics);

        // Handshake: initialize
        let init_params = serde_json::to_value(InitializeParams {
            process_id: Some(std::process::id()),
            #[allow(deprecated)]
            root_uri: Some(root_uri),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();

        {
            let mut lock = stdin.lock().unwrap();
            send_request(&mut *lock, 1, "initialize", init_params)?;
        }
        {
            let mut lock = stdin.lock().unwrap();
            send_notification(&mut *lock, "initialized", Value::Null)?;
        }

        // Reader thread: parse LSP frames, update diagnostics map
        let lang_label = language.clone();
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match lsp_server::Message::read(&mut reader) {
                    Ok(Some(lsp_server::Message::Notification(not))) => {
                        if not.method == "textDocument/publishDiagnostics" {
                            if let Ok(params) =
                                serde_json::from_value::<PublishDiagnosticsParams>(not.params)
                            {
                                if let Some(path) = path_from_uri(params.uri.as_str()) {
                                    let diags: Vec<lsp_types::Diagnostic> =
                                        params.diagnostics.into_iter().collect();
                                    let mut map = diagnostics.lock().unwrap();
                                    if diags.is_empty() {
                                        map.remove(&path);
                                    } else {
                                        map.insert(path, diags);
                                    }
                                }
                            }
                        }
                    }
                    Ok(Some(_)) => {} // ignore other messages
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("lsp reader [{lang_label}] error: {e}");
                        break;
                    }
                }
            }
        });

        self.servers.insert(
            language,
            ServerHandle {
                child: Mutex::new(child),
                stdin,
                _reader: reader,
            },
        );

        Ok(())
    }

    #[must_use]
    pub fn server_count(&self) -> usize {
        self.servers.len()
    }

    /// Returns running server states for the sidebar: `(language_id, is_alive)`.
    #[must_use]
    pub fn server_statuses(&self) -> Vec<(String, bool)> {
        self.servers
            .iter()
            .map(|(lang, handle)| {
                let alive = match handle.child.lock() {
                    Ok(mut child) => child.try_wait().is_ok_and(|s| s.is_none()),
                    Err(_) => false,
                };
                (lang.clone(), alive)
            })
            .collect()
    }

    /// Sends shutdown + exit to all servers and clears the server map.
    pub fn shutdown(&mut self) {
        for (_, handle) in self.servers.drain() {
            let mut lock = handle.stdin.lock().unwrap();
            let _ = send_request(&mut *lock, 0, "shutdown", Value::Null);
            let _ = send_notification(&mut *lock, "exit", Value::Null);
        }
    }
}

/// Writes a JSON-RPC request in LSP framing to `writer`.
fn send_request(
    writer: &mut dyn Write,
    id: i32,
    method: &str,
    params: Value,
) -> std::result::Result<(), WonderError> {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    write_message(writer, &msg)
}

/// Writes a JSON-RPC notification in LSP framing to `writer`.
fn send_notification(
    writer: &mut dyn Write,
    method: &str,
    params: Value,
) -> std::result::Result<(), WonderError> {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    write_message(writer, &msg)
}

/// Low-level LSP frame writer: `Content-Length: N\r\n\r\n{json}`.
fn write_message(writer: &mut dyn Write, value: &Value) -> std::result::Result<(), WonderError> {
    let body = serde_json::to_string(value)
        .map_err(|e| WonderError::internal(format!("json serialize: {e}")))?;
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)
        .map_err(|e| WonderError::internal(format!("lsp write: {e}")))?;
    writer
        .flush()
        .map_err(|e| WonderError::internal(format!("lsp flush: {e}")))
}

/// Builds a `file://` URI string from an absolute path.
fn uri_from_path(path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", abs.display())
}

/// Extracts a `PathBuf` from a `file://` URI string.
fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let stripped = uri.strip_prefix("file://")?;
    Some(PathBuf::from(stripped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_starts_empty() {
        let mgr = LspManager::new();
        assert_eq!(mgr.server_count(), 0);
        assert!(mgr.snapshot().is_empty());
    }

    #[test]
    fn shutdown_on_empty_is_noop() {
        let mut mgr = LspManager::new();
        mgr.shutdown();
        assert_eq!(mgr.server_count(), 0);
    }

    #[test]
    fn ensure_for_project_no_marker_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut mgr = LspManager::new();
        assert_eq!(mgr.ensure_for_project(dir.path()), None);
    }

    #[test]
    fn snapshot_returns_empty_when_no_servers() {
        let mgr = LspManager::new();
        let snap = mgr.snapshot();
        assert!(snap.is_empty());
    }
}

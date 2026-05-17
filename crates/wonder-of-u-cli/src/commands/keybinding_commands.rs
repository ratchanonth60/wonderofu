//! Implements the `/keybindings` command together with its config-file parsing,
//! rendering, and template-writing helpers.
//!
//! The public surface is intentionally narrow:
//! * [`KeybindingsCommand`] — the command registered in the command registry.
//! * [`load_keybinding_resolver`] — called by the TUI controller at startup.
//! * [`resolve_keybindings_path`] — shared with the terminal-setup module.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
    WonderError,
};
use wonder_of_u_storage::StoragePaths;
use wonder_of_u_tui::{
    EditAction, KeyBinding, KeyBindingContext, KeyBindingResolver, KeyCode, KeyEvent, KeyModifiers,
    Motion, ResolvedKey, SystemAction, VimCommand as TuiVimCommand,
};

use super::parse_command_args;
use super::plan_command::plan_editor_command;

// ── Command struct ────────────────────────────────────────────────────────────

/// Handles `/keybindings [show|open]` — inspects or edits the keybinding config.
pub struct KeybindingsCommand {
    storage_dir: Option<PathBuf>,
}

impl KeybindingsCommand {
    /// Creates a new `KeybindingsCommand`.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Returns the [`CommandSpec`] for this command.
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "keybindings",
            "Show the active keyboard shortcuts for the TUI",
            CommandKind::Local,
        )
    }
}

// ── Clap arg structs ──────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct KeybindingsArgs {
    #[command(subcommand)]
    command: Option<KeybindingsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum KeybindingsSubcommand {
    Show,
    Open,
}

// ── Command impl ──────────────────────────────────────────────────────────────

#[async_trait]
impl Command for KeybindingsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<KeybindingsArgs>("keybindings", &invocation)?;
        match args.command.unwrap_or(KeybindingsSubcommand::Show) {
            KeybindingsSubcommand::Show => Ok(CommandOutput::Text(render_keybindings_summary(
                &load_keybinding_resolver(self.storage_dir.as_deref())?,
                self.storage_dir.as_deref(),
            ))),
            KeybindingsSubcommand::Open => {
                keybindings_open_output(&context, self.storage_dir.as_deref())
                    .map(CommandOutput::Text)
            }
        }
    }
}

// ── Public helpers ────────────────────────────────────────────────────────────

/// Loads the user's keybinding overrides from disk and returns a configured
/// [`KeyBindingResolver`].  Falls back to an empty resolver when the config
/// file does not yet exist.
///
/// Called by the TUI controller at startup so the resolved keymap is available
/// immediately on first render.
pub(crate) fn load_keybinding_resolver(storage_dir: Option<&Path>) -> Result<KeyBindingResolver> {
    let path = resolve_keybindings_path(storage_dir);
    if !path.exists() {
        return Ok(KeyBindingResolver::new());
    }
    let content = fs::read_to_string(&path)?;
    let bindings = parse_keybinding_overrides(&content).map_err(|error| {
        WonderError::validation(format!(
            "invalid keybindings config `{}`: {error}",
            path.display()
        ))
    })?;
    KeyBindingResolver::with_overrides(bindings).map_err(|error| {
        WonderError::validation(format!(
            "invalid keybindings overrides `{}`: {error}",
            path.display()
        ))
    })
}

/// Returns the absolute path to `keybindings.json` for the given storage root.
///
/// When `storage_dir` is `None` the path falls back to
/// `~/.wonder-of-u/config/keybindings.json`.  Shared with the
/// `terminal_setup` module which needs it to build the `keybindings open`
/// hint in the terminal-setup notice.
pub(crate) fn resolve_keybindings_path(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("keybindings.json")
}

// ── Private render helpers ────────────────────────────────────────────────────

fn render_keybindings_summary(resolver: &KeyBindingResolver, storage_dir: Option<&Path>) -> String {
    let mut lines = vec![
        "## Keybindings".into(),
        format!(
            "config_path={}",
            resolve_keybindings_path(storage_dir).display()
        ),
        String::new(),
    ];
    for (context, title) in [
        (KeyBindingContext::Any, "Global"),
        (KeyBindingContext::Prompt, "Prompt"),
        (KeyBindingContext::VimInsert, "Vim insert"),
        (KeyBindingContext::VimNormal, "Vim normal"),
    ] {
        lines.push(format!("### {title}"));
        for line in render_binding_lines(resolver, context) {
            lines.push(format!("- {line}"));
        }
        lines.push(String::new());
    }
    lines.extend([
        "### Dialogs and pickers".into(),
        "- Up / Down: move the current selection".into(),
        "- Enter: confirm or close the active dialog".into(),
        "- Esc: cancel or dismiss the active dialog".into(),
        "- Permission prompt: Enter or y allows, n or Esc denies".into(),
        String::new(),
        "### Vim extras".into(),
        "- 0 / $: jump to line start or line end".into(),
        "- I / A: insert at line start or append at line end".into(),
        "- d{motion} / c{motion}: delete or change by motion".into(),
        "- counts like 3w and 2dw are supported".into(),
    ]);
    lines.join("\n")
}

fn render_binding_lines(resolver: &KeyBindingResolver, context: KeyBindingContext) -> Vec<String> {
    let mut by_key = BTreeMap::new();
    for binding in resolver.bindings() {
        if binding.context == context {
            by_key.insert(
                format_key_event(binding.event),
                format_resolved_key(binding.result),
            );
        }
    }
    if by_key.is_empty() {
        return vec!["(no bindings)".into()];
    }
    by_key
        .into_iter()
        .map(|(key, action)| format!("{key}: {action}"))
        .collect()
}

fn keybindings_open_output(context: &CommandContext, storage_dir: Option<&Path>) -> Result<String> {
    let path = resolve_keybindings_path(storage_dir);
    let existed = path.exists();
    ensure_keybindings_file(&path)?;
    if context.interactive {
        let mut lines = vec![
            "keybindings=ready".into(),
            format!("keybindings_path={}", path.display()),
            format!("created={}", !existed),
        ];
        if plan_editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push(format!("external_path={}", path.display()));
            lines.push("status=opening keybindings config".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/keybindings open`".into());
        }
        return Ok(lines.join("\n"));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(format!(
            concat!(
                "keybindings=ready\n",
                "keybindings_path={}\n",
                "created={}\n",
                "note=set VISUAL or EDITOR to enable `/keybindings open`"
            ),
            path.display(),
            !existed,
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(format!(
        concat!(
            "keybindings=ready\n",
            "keybindings_path={}\n",
            "created={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        path.display(),
        !existed,
        editor,
        status.success(),
    ))
}

fn ensure_keybindings_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, render_keybindings_template())?;
    } else {
        OpenOptions::new().create(true).append(true).open(path)?;
    }
    Ok(())
}

fn render_keybindings_template() -> String {
    serde_json::to_string_pretty(&json!({
        "bindings": [
            {
                "context": "prompt",
                "key": "enter",
                "modifiers": ["shift"],
                "action": {
                    "kind": "edit",
                    "action": "insert_newline"
                }
            },
            {
                "context": "vim_normal",
                "key": "q",
                "action": {
                    "kind": "vim",
                    "action": "cancel_pending"
                }
            }
        ]
    }))
    .unwrap_or_else(|_| "{\"bindings\":[]}".into())
        + "\n"
}

// ── Keybinding parsing ────────────────────────────────────────────────────────

fn parse_keybinding_overrides(content: &str) -> std::result::Result<Vec<KeyBinding>, String> {
    let value: Value = serde_json::from_str(content).map_err(|error| error.to_string())?;
    let bindings = value
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing `bindings` array".to_string())?;
    bindings
        .iter()
        .enumerate()
        .map(|(index, entry)| parse_keybinding_entry(index, entry))
        .collect()
}

fn parse_keybinding_entry(index: usize, value: &Value) -> std::result::Result<KeyBinding, String> {
    let context = parse_keybinding_context(
        value
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("binding[{index}] missing `context`"))?,
    )?;
    let event = KeyEvent {
        code: parse_key_code(
            value
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("binding[{index}] missing `key`"))?,
        )?,
        modifiers: parse_key_modifiers(value.get("modifiers"))?,
    };
    let result = parse_keybinding_result(
        value
            .get("action")
            .ok_or_else(|| format!("binding[{index}] missing `action`"))?,
    )?;
    Ok(KeyBinding {
        context,
        event,
        result,
    })
}

fn parse_keybinding_context(value: &str) -> std::result::Result<KeyBindingContext, String> {
    match value {
        "any" | "global" => Ok(KeyBindingContext::Any),
        "prompt" => Ok(KeyBindingContext::Prompt),
        "vim_insert" | "insert" => Ok(KeyBindingContext::VimInsert),
        "vim_normal" | "normal" => Ok(KeyBindingContext::VimNormal),
        other => Err(format!("unknown binding context `{other}`")),
    }
}

fn parse_key_code(value: &str) -> std::result::Result<KeyCode, String> {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "backspace" => Ok(KeyCode::Backspace),
        "enter" => Ok(KeyCode::Enter),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" => Ok(KeyCode::PageUp),
        "pagedown" => Ok(KeyCode::PageDown),
        "tab" => Ok(KeyCode::Tab),
        "backtab" => Ok(KeyCode::BackTab),
        "delete" => Ok(KeyCode::Delete),
        "insert" => Ok(KeyCode::Insert),
        "esc" | "escape" => Ok(KeyCode::Esc),
        _ if value.len() == 1 => Ok(KeyCode::Char(value.chars().next().unwrap_or_default())),
        _ if lower.starts_with('f') => lower[1..]
            .parse::<u8>()
            .map(KeyCode::F)
            .map_err(|_| format!("unknown key `{value}`")),
        _ => Err(format!("unknown key `{value}`")),
    }
}

fn parse_key_modifiers(value: Option<&Value>) -> std::result::Result<KeyModifiers, String> {
    let mut modifiers = KeyModifiers::default();
    let Some(value) = value else {
        return Ok(modifiers);
    };
    let entries = value
        .as_array()
        .ok_or_else(|| "`modifiers` must be an array".to_string())?;
    for modifier in entries {
        match modifier
            .as_str()
            .ok_or_else(|| "modifier entries must be strings".to_string())?
        {
            "control" | "ctrl" => modifiers.control = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            other => return Err(format!("unknown modifier `{other}`")),
        }
    }
    Ok(modifiers)
}

fn parse_keybinding_result(value: &Value) -> std::result::Result<ResolvedKey, String> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "action missing `kind`".to_string())?;
    match kind {
        "system" => Ok(ResolvedKey::System(parse_system_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "system action missing `action`".to_string())?,
        )?)),
        "edit" => Ok(ResolvedKey::Edit(parse_edit_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "edit action missing `action`".to_string())?,
        )?)),
        "vim" => Ok(ResolvedKey::Vim(parse_vim_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "vim action missing `action`".to_string())?,
        )?)),
        "insert_char" => {
            let ch = value
                .get("char")
                .and_then(Value::as_str)
                .and_then(|raw| raw.chars().next())
                .ok_or_else(|| "insert_char action missing `char`".to_string())?;
            Ok(ResolvedKey::InsertChar(ch))
        }
        other => Err(format!("unknown action kind `{other}`")),
    }
}

fn parse_system_action(value: &str) -> std::result::Result<SystemAction, String> {
    match value {
        "interrupt" => Ok(SystemAction::Interrupt),
        "redraw" => Ok(SystemAction::Redraw),
        "history_search" => Ok(SystemAction::HistorySearch),
        "open_global_search" => Ok(SystemAction::OpenGlobalSearch),
        "expand_tool_output" => Ok(SystemAction::ExpandToolOutput),
        other => Err(format!("unknown system action `{other}`")),
    }
}

fn parse_edit_action(value: &str) -> std::result::Result<EditAction, String> {
    match value {
        "move_left" => Ok(EditAction::Move(Motion::Left)),
        "move_right" => Ok(EditAction::Move(Motion::Right)),
        "move_up" => Ok(EditAction::Move(Motion::Up)),
        "move_down" => Ok(EditAction::Move(Motion::Down)),
        "move_line_start" => Ok(EditAction::Move(Motion::LineStart)),
        "move_line_end" => Ok(EditAction::Move(Motion::LineEnd)),
        "move_word_forward" => Ok(EditAction::Move(Motion::WordForward)),
        "move_word_backward" => Ok(EditAction::Move(Motion::WordBackward)),
        "move_word_end" => Ok(EditAction::Move(Motion::WordEnd)),
        "backspace" => Ok(EditAction::Backspace),
        "delete" => Ok(EditAction::Delete),
        "insert_newline" => Ok(EditAction::InsertNewline),
        other => Err(format!("unknown edit action `{other}`")),
    }
}

fn parse_vim_action(value: &str) -> std::result::Result<TuiVimCommand, String> {
    match value {
        "enter_insert_mode" => Ok(TuiVimCommand::EnterInsertMode),
        "enter_normal_mode" => Ok(TuiVimCommand::EnterNormalMode),
        "append_after_cursor" => Ok(TuiVimCommand::AppendAfterCursor),
        "append_line_end" => Ok(TuiVimCommand::AppendLineEnd),
        "insert_line_start" => Ok(TuiVimCommand::InsertLineStart),
        "delete_char" => Ok(TuiVimCommand::DeleteChar),
        "start_delete" => Ok(TuiVimCommand::StartDelete),
        "start_change" => Ok(TuiVimCommand::StartChange),
        "cancel_pending" => Ok(TuiVimCommand::CancelPending),
        other => Err(format!("unknown vim action `{other}`")),
    }
}

// ── Key formatting ────────────────────────────────────────────────────────────

fn format_key_event(event: KeyEvent) -> String {
    let mut parts = Vec::new();
    if event.modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if event.modifiers.alt {
        parts.push("Alt".to_string());
    }
    if event.modifiers.shift {
        parts.push("Shift".to_string());
    }
    parts.push(match event.code {
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Char(ch) => {
            if (event.modifiers.control || event.modifiers.alt || event.modifiers.shift)
                && ch.is_ascii_alphabetic()
            {
                ch.to_ascii_uppercase().to_string()
            } else {
                ch.to_string()
            }
        }
        KeyCode::F(index) => format!("F{index}"),
        KeyCode::Null => "Null".into(),
    });
    parts.join("-")
}

fn format_resolved_key(result: ResolvedKey) -> String {
    match result {
        ResolvedKey::Edit(action) => match action {
            EditAction::Move(Motion::Left) => "move left".into(),
            EditAction::Move(Motion::Right) => "move right".into(),
            EditAction::Move(Motion::Up) => "move up".into(),
            EditAction::Move(Motion::Down) => "move down".into(),
            EditAction::Move(Motion::LineStart) => "move to line start".into(),
            EditAction::Move(Motion::LineEnd) => "move to line end".into(),
            EditAction::Move(Motion::FirstNonBlank) => "move to first non-blank".into(),
            EditAction::Move(Motion::WordForward) => "move forward by word".into(),
            EditAction::Move(Motion::WordBackward) => "move backward by word".into(),
            EditAction::Move(Motion::WordEnd) => "move to word end".into(),
            EditAction::Backspace => "delete backward".into(),
            EditAction::Delete => "delete forward".into(),
            EditAction::InsertNewline => "submit / insert newline".into(),
            EditAction::InsertLiteralNewline => "insert literal newline".into(),
        },
        ResolvedKey::InsertChar(ch) => format!("insert `{ch}`"),
        ResolvedKey::System(SystemAction::Interrupt) => "interrupt or exit".into(),
        ResolvedKey::System(SystemAction::Redraw) => "redraw terminal".into(),
        ResolvedKey::System(SystemAction::HistorySearch) => "recall previous prompt".into(),
        ResolvedKey::System(SystemAction::OpenGlobalSearch) => "search workspace".into(),
        ResolvedKey::System(SystemAction::ExpandToolOutput) => {
            "expand or collapse tool output".into()
        }
        ResolvedKey::Vim(command) => match command {
            TuiVimCommand::EnterInsertMode => "enter insert mode".into(),
            TuiVimCommand::EnterNormalMode => "enter normal mode".into(),
            TuiVimCommand::AppendAfterCursor => "append after cursor".into(),
            TuiVimCommand::AppendLineEnd => "append at line end".into(),
            TuiVimCommand::InsertLineStart => "insert at line start".into(),
            TuiVimCommand::DeleteChar => "delete char under cursor".into(),
            TuiVimCommand::StartDelete => "start delete operator".into(),
            TuiVimCommand::StartChange => "start change operator".into(),
            TuiVimCommand::CancelPending => "cancel pending operator".into(),
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{CommandContext, FeatureSet, PermissionMode, SessionId};
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::{
        ensure_keybindings_file, load_keybinding_resolver, render_keybindings_summary,
        resolve_keybindings_path,
    };

    fn test_context(cwd: &std::path::Path) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn keybindings_summary_lists_supported_sections() {
        let rendered =
            render_keybindings_summary(&load_keybinding_resolver(None).expect("resolver"), None);

        assert!(rendered.contains("## Keybindings"));
        assert!(rendered.contains("### Global"));
        assert!(rendered.contains("### Prompt"));
        assert!(rendered.contains("### Vim insert"));
        assert!(rendered.contains("### Dialogs and pickers"));
        assert!(rendered.contains("### Vim normal"));
        assert!(rendered.contains("Ctrl-R"));
        assert!(rendered.contains("d{motion} / c{motion}"));
    }

    #[test]
    fn keybindings_template_can_be_loaded_as_overrides() {
        let dir = unique_test_dir("workflow-keybindings-template");
        let path = dir.join("config/keybindings.json");
        ensure_keybindings_file(&path).expect("write template");

        let resolver = load_keybinding_resolver(Some(dir.as_path())).expect("load resolver");
        let rendered = render_keybindings_summary(&resolver, Some(dir.as_path()));

        assert!(rendered.contains("Shift-Enter"));
        assert!(rendered.contains("q: cancel pending operator"));
    }

    #[test]
    fn keybindings_open_hints_external_editor_when_interactive() {
        let dir = unique_test_dir("workflow-keybindings-open");
        let _editor = EnvVarGuard::set("EDITOR", "true");
        let context = CommandContext {
            session_id: SessionId::new(),
            cwd: dir.clone(),
            features: FeatureSet::default(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        };
        let output =
            super::keybindings_open_output(&context, Some(dir.as_path())).expect("open output");

        assert!(output.contains("open_external=true"));
        assert!(output.contains(&format!(
            "external_path={}",
            resolve_keybindings_path(Some(dir.as_path())).display()
        )));
    }

    // Suppress unused import warning for `test_context` helper — it is used by
    // other tests in this module when CommandContext fields grow.
    #[allow(dead_code)]
    fn _uses_test_context() {
        let _ = test_context(std::path::Path::new("/workspace"));
    }
}

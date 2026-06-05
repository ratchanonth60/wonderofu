use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn open_hooks_menu(&mut self) {
        let config_path = resolve_hooks_config_path(self.storage_dir.as_deref());
        let (entries, disabled) = load_hooks_entries(&config_path);
        self.pending_hooks_menu = Some(HooksMenuState {
            entries,
            selected_index: 0,
            screen: HooksMenuScreen::Browse,
            disabled,
            config_path: config_path.to_string_lossy().into_owned(),
        });
        self.needs_render = true;
    }

    pub(in crate::tui_runtime) fn handle_hooks_menu_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let Some(menu) = &self.pending_hooks_menu else {
            return Ok(());
        };
        match &menu.screen {
            HooksMenuScreen::Browse => self.handle_hooks_menu_browse_key(key, resolved),
            HooksMenuScreen::ViewDetail(_) => self.handle_hooks_menu_detail_key(key, resolved),
        }
    }

    fn handle_hooks_menu_browse_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                if let Some(m) = &mut self.pending_hooks_menu {
                    if m.selected_index > 0 {
                        m.selected_index -= 1;
                    }
                }
                self.needs_render = true;
                Ok(())
            }
            KeyCode::Down => {
                if let Some(m) = &mut self.pending_hooks_menu {
                    let max = m.entries.len().saturating_sub(1);
                    m.selected_index = (m.selected_index + 1).min(max);
                }
                self.needs_render = true;
                Ok(())
            }
            KeyCode::Tab => self.show_hooks_menu_detail(),
            KeyCode::Esc => {
                self.pending_hooks_menu = None;
                self.dialog = None;
                self.needs_render = true;
                Ok(())
            }
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => self.show_hooks_menu_detail(),
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.pending_hooks_menu = None;
                    self.dialog = None;
                    self.needs_render = true;
                    Ok(())
                }
                _ => {
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }

    fn handle_hooks_menu_detail_key(
        &mut self,
        _key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match resolved {
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                self.pending_hooks_menu = None;
                self.dialog = None;
                self.needs_render = true;
            }
            _ => {
                // Any key: go back to browse
                if let Some(m) = &mut self.pending_hooks_menu {
                    m.screen = HooksMenuScreen::Browse;
                }
                self.dialog = None;
                self.needs_render = true;
            }
        }
        Ok(())
    }

    fn show_hooks_menu_detail(&mut self) -> Result<()> {
        let Some(menu) = &self.pending_hooks_menu else {
            return Ok(());
        };
        if menu.entries.is_empty() {
            self.pending_hooks_menu = None;
            self.needs_render = true;
            return Ok(());
        }
        let idx = menu.selected_index;
        if let Some(m) = &mut self.pending_hooks_menu {
            m.screen = HooksMenuScreen::ViewDetail(idx);
        }
        // Also show a dialog with the hook details
        if let Some(menu) = &self.pending_hooks_menu {
            if let Some(entry) = menu.entries.get(idx) {
                let mut body = vec![
                    format!("Event:   {}", entry.event),
                    format!("Matcher: {}", entry.matcher),
                    format!("Type:    {}", entry.kind),
                    format!("Target:  {}", entry.target),
                ];
                if let Some(cond) = &entry.condition {
                    body.push(format!("If:      {cond}"));
                }
                self.dialog = Some(DialogView::notice("Hook Details", body));
            }
        }
        self.needs_render = true;
        Ok(())
    }
}

fn resolve_hooks_config_path(storage_dir: Option<&std::path::Path>) -> std::path::PathBuf {
    storage_dir
        .map(wonder_of_u_storage::StoragePaths::new)
        .map(|p| p.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("hooks.json")
}

fn load_hooks_entries(path: &std::path::Path) -> (Vec<HooksMenuEntry>, bool) {
    if !path.exists() {
        return (Vec::new(), false);
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return (Vec::new(), false);
    };
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (Vec::new(), false),
    };
    let disabled = value
        .get("disable_all_hooks")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut entries = Vec::new();
    if let Some(hooks_map) = value.get("hooks").and_then(|v| v.as_object()) {
        for (event, matchers_val) in hooks_map {
            if let Some(matchers) = matchers_val.as_array() {
                for matcher_val in matchers {
                    let matcher = matcher_val
                        .get("matcher")
                        .and_then(|v| v.as_str())
                        .unwrap_or("*")
                        .to_string();
                    if let Some(hooks) = matcher_val.get("hooks").and_then(|v| v.as_array()) {
                        for hook in hooks {
                            let (kind, target) = extract_hook_kind_target(hook);
                            let condition =
                                hook.get("if").and_then(|v| v.as_str()).map(str::to_string);
                            entries.push(HooksMenuEntry {
                                event: event.clone(),
                                matcher: matcher.clone(),
                                kind,
                                target,
                                condition,
                            });
                        }
                    }
                }
            }
        }
    }
    (entries, disabled)
}

pub(super) fn extract_hook_kind_target(hook: &serde_json::Value) -> (String, String) {
    match hook.get("type").and_then(|v| v.as_str()) {
        Some("command") => (
            "command".into(),
            hook.get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        Some("prompt") => (
            "prompt".into(),
            hook.get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        Some("agent") => (
            "agent".into(),
            hook.get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        Some("http") => (
            "http".into(),
            hook.get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        _ => ("unknown".into(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_hook_kind_target, load_hooks_entries};

    #[test]
    fn extract_command_hook() {
        let hook = serde_json::json!({"type": "command", "command": "echo hi"});
        let (kind, target) = extract_hook_kind_target(&hook);
        assert_eq!(kind, "command");
        assert_eq!(target, "echo hi");
    }

    #[test]
    fn extract_http_hook() {
        let hook = serde_json::json!({"type": "http", "url": "https://example.com"});
        let (kind, target) = extract_hook_kind_target(&hook);
        assert_eq!(kind, "http");
        assert_eq!(target, "https://example.com");
    }

    #[test]
    fn extract_unknown_hook() {
        let hook = serde_json::json!({"type": "unknown_future_type"});
        let (kind, _) = extract_hook_kind_target(&hook);
        assert_eq!(kind, "unknown");
    }

    #[test]
    fn load_entries_from_valid_json() {
        let dir = wonder_of_u_test_support::unique_test_dir("hooks-load-entries");
        let path = dir.join("hooks.json");
        std::fs::write(
            &path,
            r#"{"hooks": {"PreToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": "lint"}]}]}}"#,
        )
        .unwrap();
        let (entries, disabled) = load_hooks_entries(&path);
        assert!(!disabled);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event, "PreToolUse");
        assert_eq!(entries[0].kind, "command");
        assert_eq!(entries[0].target, "lint");
    }

    #[test]
    fn load_entries_respects_disable_all_hooks() {
        let dir = wonder_of_u_test_support::unique_test_dir("hooks-disabled");
        let path = dir.join("hooks.json");
        std::fs::write(&path, r#"{"disable_all_hooks": true, "hooks": {}}"#).unwrap();
        let (_, disabled) = load_hooks_entries(&path);
        assert!(disabled);
    }

    #[test]
    fn load_entries_missing_file_returns_empty() {
        let path = std::path::Path::new("/tmp/wonder-of-u-nonexistent-hooks.json");
        let (entries, disabled) = load_hooks_entries(path);
        assert!(entries.is_empty());
        assert!(!disabled);
    }
}

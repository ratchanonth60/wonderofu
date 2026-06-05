use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn open_memory_file_selector(&mut self) {
        let cwd = self.state.session.cwd.clone();
        let storage_dir = self.storage_dir.clone();
        let mut entries = Vec::new();

        // Project CLAUDE.md
        let project_path = cwd.join("CLAUDE.md");
        entries.push(MemoryFileEntry {
            kind: "project".into(),
            label: "Project (CLAUDE.md)".into(),
            exists: project_path.exists(),
            path: project_path,
            depth: 0,
        });

        // User CLAUDE.md
        let user_path = storage_dir
            .as_deref()
            .map(wonder_of_u_storage::TranscriptStore::new)
            .map(|store| store.paths().config_dir())
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".wonder-of-u")
                    .join("config")
            })
            .join("CLAUDE.md");
        entries.push(MemoryFileEntry {
            kind: "user".into(),
            label: "User (~/.wonder-of-u/config/CLAUDE.md)".into(),
            exists: user_path.exists(),
            path: user_path,
            depth: 0,
        });

        // Scan parent directories for CLAUDE.md files
        let mut dir = cwd.parent();
        let mut depth = 1usize;
        while let Some(parent) = dir {
            let parent_claude = parent.join("CLAUDE.md");
            if parent_claude.exists() {
                let label = parent
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| parent.to_string_lossy().into_owned());
                entries.push(MemoryFileEntry {
                    kind: "parent".into(),
                    label: format!("Parent: {label}/CLAUDE.md"),
                    exists: true,
                    path: parent_claude,
                    depth,
                });
            }
            dir = parent.parent();
            depth += 1;
            if depth > 5 {
                break;
            }
        }

        self.pending_memory_file_selector = Some(MemoryFileSelectorState {
            selected_index: 0,
            entries,
        });
        self.needs_render = true;
    }

    pub(in crate::tui_runtime) fn handle_memory_file_selector_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                if let Some(s) = &mut self.pending_memory_file_selector {
                    if s.selected_index > 0 {
                        s.selected_index -= 1;
                    }
                }
                self.needs_render = true;
                Ok(())
            }
            KeyCode::Down => {
                if let Some(s) = &mut self.pending_memory_file_selector {
                    let max = s.entries.len().saturating_sub(1);
                    s.selected_index = (s.selected_index + 1).min(max);
                }
                self.needs_render = true;
                Ok(())
            }
            KeyCode::Tab => self.confirm_memory_file_selector(),
            KeyCode::Esc => {
                self.pending_memory_file_selector = None;
                self.needs_render = true;
                Ok(())
            }
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => {
                    self.confirm_memory_file_selector()
                }
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.pending_memory_file_selector = None;
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

    fn confirm_memory_file_selector(&mut self) -> Result<()> {
        let Some(selector) = self.pending_memory_file_selector.take() else {
            return Ok(());
        };
        let Some(entry) = selector.entries.get(selector.selected_index) else {
            return Ok(());
        };
        let path = entry.path.clone();
        let exists = entry.exists;

        // Ensure file exists before opening
        if !exists {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path);
        }

        // Open the file directly in the external editor
        let cwd = self.state.session.cwd.clone();
        self.pending_external_editor = Some(ExternalEditorRequest { cwd, path });
        self.status_note = Some("opening file in editor".into());
        self.needs_render = true;
        Ok(())
    }
}

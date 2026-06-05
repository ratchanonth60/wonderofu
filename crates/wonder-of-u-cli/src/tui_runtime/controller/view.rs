use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn view(&self) -> ShellView {
        let history_entries = prompt_history_entries(&self.state.messages);
        let prompt = self
            .history_search
            .as_ref()
            .and_then(|search| current_history_search_match(search, &history_entries))
            .map_or_else(|| self.prompt.text(), ToString::to_string);
        let mut view = ShellView::from_app_state(&self.state, prompt, self.expand_tool_output);
        let terminal_width = self.last_terminal_size.0;
        let summary_width = if terminal_width == 0 {
            80
        } else {
            usize::from(shell_main_area_width(terminal_width, self.sidebar_visible)).max(1)
        };
        view.messages = message_lines_for_width_with_cursor(
            &self.state.messages,
            summary_width,
            self.expand_tool_output,
            self.message_cursor_index,
        );
        view.message_cursor_index = self.message_cursor_index;
        if !self.sidebar_visible {
            view.sidebar = None;
        }
        view.spinner_frame = self.loading_frame;
        view.history_search = self
            .history_search
            .as_ref()
            .map(|search| HistorySearchView {
                query: search.query.text(),
                match_text: current_history_search_match(search, &history_entries)
                    .map(ToString::to_string),
                match_index: search.cursor,
                match_total: search.matches.len(),
            });
        // Populate sidebar sections with live controller state when the panel is
        // visible.  The renderer ignores the sidebar entirely when the terminal is
        // too narrow, so we always fill the data here.
        if let Some(sb) = view.sidebar.as_mut() {
            // Section 1 – Session: title (or id prefix), turn state, status note.
            let mut session_lines = vec![format!(
                "◈ {}",
                if self.state.session.title.is_empty() {
                    self.state
                        .session
                        .id
                        .to_string()
                        .chars()
                        .take(8)
                        .collect::<String>()
                } else {
                    self.state.session.title.clone()
                }
            )];
            if let Some(note) = &self.status_note {
                session_lines.push(format!("  {note}"));
            }
            sb.session_lines = session_lines;

            sb.context_lines = context_sidebar_lines(
                self.estimated_context_tokens(),
                self.state.context_window_size,
            );

            // Section 4 – Status: turn-state indicator + last error summary.
            let state_icon = match self.turn_state {
                TurnState::Idle | TurnState::EditingInput => "●",
                TurnState::ModelRequestActive | TurnState::StreamingResponse => "⟳",
                TurnState::CommandQueued | TurnState::ToolExecuting => "⚙",
                TurnState::ToolPermissionPending => "?",
                TurnState::Interrupted => "⚠",
                TurnState::Completed => "✓",
            };
            sb.status_lines = vec![format!(
                "{state_icon} {}",
                turn_state_label(self.turn_state)
            )];
            if let Some(verb) = loading_verb_label(self.turn_state) {
                sb.status_lines.push(format!("  {verb}…"));
            }
            if let Some(note) = &self.status_note {
                sb.status_lines.push(format!("  {note}"));
            }

            // Section 6 – Workspace: runtime label, git branch, truncated cwd.
            let mut workspace_lines = vec![
                runtime_label(self.state.provider.as_deref(), self.state.model.as_deref())
                    .to_string(),
            ];
            if let Some(branch) = &self.state.session.git_branch {
                workspace_lines.push(format!("⎇  {branch}"));
            }
            // Truncate cwd to 30 chars so it fits the sidebar column.
            if let Some(cwd) = self.state.session.cwd.to_str() {
                let label: String = if cwd.len() > 36 {
                    format!("…{}", &cwd[cwd.len() - 35..])
                } else {
                    cwd.to_string()
                };
                workspace_lines.push(format!("  {label}"));
            }
            sb.workspace_lines = workspace_lines;
        }

        let chrome_status = chrome_status_text(&self.state);
        view.status = if let Some(note) = &self.status_note {
            format!("{note} | {chrome_status}")
        } else {
            chrome_status
        };
        view.loading = self.has_active_turn() || is_loading_turn_state(self.turn_state);
        view.loading_verb = loading_verb_label(self.turn_state)
            .map(str::to_string)
            .or_else(|| self.has_active_turn().then(|| "thinking".to_string()));

        // Elapsed seconds: tick interval is 50 ms, so divide frame count by 20.
        view.loading_elapsed_secs = self.loading_frame / 20;
        // Cumulative token count from costs tracking (0 when no API calls yet).
        view.loading_total_tokens = self.state.costs.usage.total_tokens();

        // Live shell output lines (only populated while a bash/shell tool is running).
        view.tool_progress = self.tool_progress_lines.clone().into();

        // Input length warning: supplement the context warning when the user
        // has typed a very long prompt that may exceed the model's context window.
        const INPUT_WARN_CHARS: usize = 8_000;
        const INPUT_CRITICAL_CHARS: usize = 20_000;
        let input_chars = self.prompt.text().chars().count();
        if view.prompt_warning.is_none() && input_chars >= INPUT_WARN_CHARS {
            let severity = if input_chars >= INPUT_CRITICAL_CHARS {
                wonder_of_u_tui::PromptWarningSeverity::Critical
            } else {
                wonder_of_u_tui::PromptWarningSeverity::Warning
            };
            view.prompt_warning = Some(wonder_of_u_tui::PromptWarningView {
                text: format!(
                    "Long input ({} chars) — context usage may be high",
                    input_chars
                ),
                severity,
            });
        }

        // Compact Claude-style footer; verbose cwd/provider/model metadata lives in the sidebar.
        let permission_label = permission_mode_output_label(self.state.permission_mode);
        let vim_hint = if self.vim_enabled {
            match self.vim.mode() {
                VimMode::Insert => " · vim:insert",
                VimMode::Normal => " · vim:normal",
                VimMode::Visual => " · vim:visual",
            }
        } else {
            ""
        };
        view.footer = if let Some(sl_text) = self.status_line_handle.current_text() {
            sl_text
        } else {
            format!(
                "▸▸ {permission_label}{vim_hint} (shift+tab to cycle) · ⌃B sidebar · ⌃Y select · ⌃C exit"
            )
        };
        let picker_list = self.current_picker_list_view();
        view.dialog = if let Some(ref diff_state) = self.diff_dialog {
            Some(self.build_diff_dialog_view(diff_state))
        } else if picker_list.is_some() {
            None
        } else {
            self.dialog.clone()
        };
        view.picker_view = None;
        view.picker_list = picker_list;
        view.notifications = self.notifications.view(3);
        view.slash_suggestions = self.active_suggestions.as_ref().map(|state| {
            let filtered = state.filtered();
            let entries = filtered
                .iter()
                .enumerate()
                .map(|(i, s)| SlashSuggestionEntry {
                    match_ranges: find_match_chars(&s.display_text, &state.filter),
                    display: s.display_text.clone(),
                    description: s.description.clone().unwrap_or_default(),
                    selected: i == state.selected_index.min(filtered.len().saturating_sub(1)),
                })
                .collect();
            SlashSuggestionsOverlay { entries }
        });
        view.global_search = self.global_search_open.then(|| GlobalSearchOverlayView {
            query: self.global_search_query.clone(),
            results: self.global_search_results.clone(),
            selected: self.global_search_selected,
        });
        // Wire scroll position so the renderer shows the correct transcript window.
        view.scroll = TranscriptScrollView {
            offset_from_bottom: self.scroll_state.offset_from_bottom,
            total_lines: self.scroll_state.last_total_lines,
            visible_lines: self.scroll_state.last_visible_lines,
        };

        // Populate sidebar provider summary with live provider/model info.
        // Active provider is prefixed with ◈; other ready providers are shown
        // compactly below it.  A trailing count line shows how many configured
        // providers are not yet authenticated so the user knows what to set up.
        if let Some(sb) = view.sidebar.as_mut() {
            let active_provider = self.state.provider.as_deref().unwrap_or("");
            let active_model = self.state.model.as_deref().unwrap_or("");
            let mut provider_lines: Vec<String> = Vec::new();

            if let Ok(report) = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())
            {
                let ready_count = report.available_providers.len();
                for pd in &report.available_providers {
                    // Use the currently active model when this is the active provider;
                    // otherwise fall back to the provider's default.
                    let model = if pd.id == active_provider {
                        active_model
                    } else {
                        pd.default_model.as_str()
                    };
                    let label = format!("{}({})", model, pd.id);
                    if pd.id == active_provider {
                        provider_lines.push(format!("◈ {label}"));
                    } else {
                        provider_lines.push(format!("  {label}"));
                    }
                }

                // Show how many of the full registry are not yet ready so the
                // user gets a nudge without the sidebar becoming overwhelming.
                let total = ProviderRegistry::builtin().providers().count();
                let missing = total.saturating_sub(ready_count);
                if missing > 0 {
                    provider_lines.push(format!("  +{missing} more (/setup to configure)"));
                }
            }

            // Fallback: if no providers loaded, show the active selection as one line.
            if provider_lines.is_empty() && !active_provider.is_empty() {
                provider_lines.push(format!("◈ {}({})", active_model, active_provider));
            }

            sb.provider_lines = provider_lines;
        }

        // Populate tool, MCP, LSP, and todo sidebar sections.
        if let Some(sb) = view.sidebar.as_mut() {
            sb.tool_lines = self.sidebar_cache.tool_lines.clone();
            sb.mcp_lines = self.sidebar_cache.mcp_lines.clone();
            sb.lsp_lines = self.sidebar_cache.lsp_lines.clone();
            sb.todo_lines = self.sidebar_cache.todo_lines.clone();
        }

        view
    }
    pub(in crate::tui_runtime) fn refresh_sidebar_panel_cache(&mut self) -> bool {
        let tool_context = self.tool_context();
        let next = SidebarPanelCache {
            tool_lines: tool_sidebar_lines(&tool_context, self.storage_dir.as_deref()),
            mcp_lines: mcp_sidebar_lines(self.storage_dir.as_deref(), &self.state.session.cwd),
            lsp_lines: lsp_sidebar_lines(&self.state.session.cwd),
            todo_lines: todo_merged_sidebar_lines(
                &self.state.session.cwd,
                self.storage_dir.as_deref(),
                self.state.session.id,
            ),
        };
        if self.sidebar_cache == next {
            false
        } else {
            self.sidebar_cache = next;
            true
        }
    }
    pub(in crate::tui_runtime) fn prompt_cursor(&self, width: u16, height: u16) -> (u16, u16) {
        // Use the live sidebar_visible flag so cursor placement matches the
        // actual rendered layout (no sidebar column deduction when toggled off).
        let sidebar_active = self.sidebar_visible;
        if self.global_search_open {
            return global_search_cursor_position(
                width,
                height,
                &self.view(),
                self.global_search_cursor,
            );
        }
        if let Some(search) = &self.history_search {
            let view = HistorySearchView {
                query: search.query.text(),
                match_text: self
                    .view()
                    .history_search
                    .and_then(|history_search| history_search.match_text),
                match_index: search.cursor,
                match_total: search.matches.len(),
            };
            return history_search_cursor_position(
                width,
                height,
                &view,
                search.query.cursor(),
                sidebar_active,
                context_warning_visible(
                    self.estimated_context_tokens(),
                    self.state.context_window_size,
                ),
            );
        }
        prompt_cursor_position(
            width,
            height,
            &self.prompt.text(),
            self.prompt.cursor(),
            sidebar_active,
            context_warning_visible(
                self.estimated_context_tokens(),
                self.state.context_window_size,
            ),
        )
    }
}

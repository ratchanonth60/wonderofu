use super::TuiController;
use super::*;
use std::time::{Duration, Instant};

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn handle_history_search_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => return self.cancel_history_search(),
            KeyCode::Up => {
                self.step_history_search(1);
                return Ok(());
            }
            KeyCode::Down => {
                self.step_history_search(-1);
                return Ok(());
            }
            _ => {}
        }

        match resolved {
            Some(ResolvedKey::Edit(EditAction::InsertNewline)) => self.accept_history_search(),
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::HistorySearch)) => {
                self.step_history_search(1);
                Ok(())
            }
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Redraw)) => {
                self.needs_render = true;
                Ok(())
            }
            Some(ResolvedKey::System(system)) => self.handle_system_action(system),
            Some(resolved) if self.edit_history_search_query(resolved) => Ok(()),
            Some(_) | None => {
                self.status_note =
                    Some(history_search_status_note(self.history_search_has_match()));
                self.needs_render = true;
                Ok(())
            }
        }
    }
    pub(in crate::tui_runtime) fn open_or_step_history_search(&mut self) -> Result<()> {
        if self.history_search.is_none() {
            self.history_search = Some(HistorySearchState {
                query: TextBuffer::new(false),
                matches: Vec::new(),
                cursor: 0,
                saved_buffer: self.prompt.clone(),
            });
            self.refresh_history_search();
            return Ok(());
        }

        self.step_history_search(1);
        Ok(())
    }
    pub(in crate::tui_runtime) fn edit_history_search_query_text(&mut self, text: &str) {
        let Some(search) = &mut self.history_search else {
            return;
        };
        search.query.insert_text(text);
        self.refresh_history_search();
    }
    pub(in crate::tui_runtime) fn edit_history_search_query(
        &mut self,
        resolved: ResolvedKey,
    ) -> bool {
        let Some(search) = &mut self.history_search else {
            return false;
        };
        if !apply_picker_query_edit(&mut search.query, resolved) {
            return false;
        }
        self.refresh_history_search();
        true
    }
    pub(in crate::tui_runtime) fn refresh_history_search(&mut self) {
        let entries = prompt_history_entries(&self.state.messages);
        let Some(search) = &mut self.history_search else {
            return;
        };
        search.matches = filtered_picker_indices(&search.query, &entries, Clone::clone);
        if search.matches.is_empty() {
            search.cursor = 0;
        } else {
            search.cursor = search.cursor.min(search.matches.len().saturating_sub(1));
        }
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some(history_search_status_note(!search.matches.is_empty()));
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn step_history_search(&mut self, delta: isize) {
        let Some(search) = &mut self.history_search else {
            return;
        };
        if !search.matches.is_empty() {
            search.cursor =
                (search.cursor as isize + delta).rem_euclid(search.matches.len() as isize) as usize;
        }
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some(history_search_status_note(!search.matches.is_empty()));
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn accept_history_search(&mut self) -> Result<()> {
        let Some(search) = self.history_search.take() else {
            return Ok(());
        };
        let entries = prompt_history_entries(&self.state.messages);
        let Some(selection) = current_history_search_match(&search, &entries) else {
            self.history_search = Some(search);
            self.status_note = Some(history_search_status_note(false));
            self.needs_render = true;
            return Ok(());
        };
        self.prompt = TextBuffer::from_text(selection, true);
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some("history search accepted".into());
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn cancel_history_search(&mut self) -> Result<()> {
        let Some(search) = self.history_search.take() else {
            return Ok(());
        };
        self.prompt = search.saved_buffer;
        self.turn_state = self.turn_state_for_prompt();
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some("history search cancelled".into());
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn history_search_has_match(&self) -> bool {
        self.history_search
            .as_ref()
            .is_some_and(|search| !search.matches.is_empty())
    }
    pub(in crate::tui_runtime) fn open_global_search(&mut self, query: &str) {
        self.global_search_open = true;
        self.global_search_query = query.to_string();
        self.global_search_cursor = self.global_search_query.chars().count();
        self.global_search_results.clear();
        self.global_search_selected = 0;
        self.global_search_dirty_since = None;
        self.active_suggestions = None;
        self.status_note = Some("workspace search".into());
        self.needs_render = true;
        if !query.trim().is_empty() {
            self.refresh_global_search_now();
        }
    }
    pub(in crate::tui_runtime) fn toggle_global_search(&mut self, query: Option<&str>) {
        if self.global_search_open {
            self.close_global_search();
        } else {
            self.open_global_search(query.unwrap_or_default());
        }
    }
    pub(in crate::tui_runtime) fn close_global_search(&mut self) {
        self.global_search_open = false;
        self.global_search_query.clear();
        self.global_search_results.clear();
        self.global_search_selected = 0;
        self.global_search_cursor = 0;
        self.global_search_dirty_since = None;
        self.status_note = Some("workspace search closed".into());
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn handle_global_search_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.close_global_search();
                return Ok(());
            }
            KeyCode::Up => {
                self.step_global_search_selection(-1);
                return Ok(());
            }
            KeyCode::Down => {
                self.step_global_search_selection(1);
                return Ok(());
            }
            _ => {}
        }

        match resolved {
            Some(ResolvedKey::Edit(EditAction::InsertNewline)) => {
                self.accept_global_search_result()
            }
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::OpenGlobalSearch)) => {
                self.close_global_search();
                Ok(())
            }
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Redraw)) => {
                self.needs_render = true;
                Ok(())
            }
            Some(ResolvedKey::System(system)) => self.handle_system_action(system),
            Some(resolved) if self.edit_global_search_query(resolved) => Ok(()),
            Some(_) | None => {
                self.needs_render = true;
                Ok(())
            }
        }
    }
    pub(in crate::tui_runtime) fn edit_global_search_query_text(&mut self, text: &str) {
        let mut query = TextBuffer::from_text(&self.global_search_query, false);
        query.set_cursor(self.global_search_cursor);
        query.insert_text(text);
        self.update_global_search_query(query);
    }
    pub(in crate::tui_runtime) fn edit_global_search_query(
        &mut self,
        resolved: ResolvedKey,
    ) -> bool {
        let mut query = TextBuffer::from_text(&self.global_search_query, false);
        query.set_cursor(self.global_search_cursor);
        if !apply_picker_query_edit(&mut query, resolved) {
            return false;
        }
        self.update_global_search_query(query);
        true
    }
    pub(in crate::tui_runtime) fn update_global_search_query(&mut self, query: TextBuffer) {
        self.global_search_query = query.text();
        self.global_search_cursor = query.cursor();
        self.global_search_selected = 0;
        if self.global_search_query.trim().is_empty() {
            self.global_search_results.clear();
            self.global_search_dirty_since = None;
            self.status_note = Some("workspace search".into());
        } else {
            self.global_search_dirty_since = Some(Instant::now());
            self.status_note = Some("searching workspace…".into());
        }
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn refresh_global_search_if_ready(&mut self) -> Result<bool> {
        let Some(dirty_since) = self.global_search_dirty_since else {
            return Ok(false);
        };
        if !self.global_search_open || dirty_since.elapsed() < Duration::from_millis(100) {
            return Ok(false);
        }
        self.refresh_global_search_now();
        Ok(true)
    }
    pub(in crate::tui_runtime) fn refresh_global_search_now(&mut self) {
        self.global_search_dirty_since = None;
        match crate::commands::search::search_workspace(
            &self.state.session.cwd,
            &self.global_search_query,
        ) {
            Ok(results) => {
                self.global_search_selected = self
                    .global_search_selected
                    .min(results.len().saturating_sub(1));
                self.global_search_results = results;
                self.status_note = Some(format!(
                    "workspace search: {} match{}",
                    self.global_search_results.len(),
                    if self.global_search_results.len() == 1 {
                        ""
                    } else {
                        "es"
                    }
                ));
            }
            Err(error) => {
                self.global_search_results.clear();
                self.global_search_selected = 0;
                self.status_note = Some(format!("workspace search failed: {error}"));
            }
        }
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn step_global_search_selection(&mut self, delta: isize) {
        if !self.global_search_results.is_empty() {
            self.global_search_selected = (self.global_search_selected as isize + delta)
                .rem_euclid(self.global_search_results.len() as isize)
                as usize;
        }
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn accept_global_search_result(&mut self) -> Result<()> {
        if self.global_search_dirty_since.is_some() {
            self.refresh_global_search_now();
        }
        let Some(result) = self
            .global_search_results
            .get(self.global_search_selected)
            .cloned()
        else {
            return Ok(());
        };
        self.prompt
            .insert_text(&format!("{}:{} ", result.file, result.line));
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.close_global_search();
        self.status_note = Some(format!("inserted {}:{}", result.file, result.line));
        self.needs_render = true;
        Ok(())
    }
}

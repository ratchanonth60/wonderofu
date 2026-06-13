use super::FleetPanelState;
use super::TuiController;

impl TuiController<'_> {
    pub(super) fn open_fleet_panel(&mut self) {
        self.clear_picker_overlays();
        self.dialog = None;
        self.fleet_panel = Some(FleetPanelState {
            selected_index: 0,
            fleet_views: vec![],
            selected_run_index: None,
            scroll_offset: 0,
        });
        self.refresh_fleet_panel();
        self.needs_render = true;
    }

    pub(super) fn close_fleet_panel(&mut self) {
        self.fleet_panel = None;
        self.needs_render = true;
    }

    pub(super) fn refresh_fleet_panel(&mut self) {
        let Some(fleet_panel) = self.fleet_panel.as_mut() else {
            return;
        };
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return;
        };

        let inspector = wonder_of_u_storage::FleetInspector::new(storage_dir.to_path_buf());

        let Ok(runs) = wonder_of_u_storage::FleetStore::new(storage_dir).list_runs() else {
            return;
        };

        let mut views = Vec::new();
        for run in &runs {
            let Ok(observation) = inspector.observe(run.id) else {
                continue;
            };
            let status = match observation.fleet.status {
                wonder_of_u_core::FleetRunStatus::Pending => {
                    wonder_of_u_tui::fleet_view::FleetStatusView::Pending
                }
                wonder_of_u_core::FleetRunStatus::Running => {
                    wonder_of_u_tui::fleet_view::FleetStatusView::Running
                }
                wonder_of_u_core::FleetRunStatus::Completed => {
                    wonder_of_u_tui::fleet_view::FleetStatusView::Completed
                }
                wonder_of_u_core::FleetRunStatus::Failed => {
                    wonder_of_u_tui::fleet_view::FleetStatusView::Failed
                }
                wonder_of_u_core::FleetRunStatus::Cancelled => {
                    wonder_of_u_tui::fleet_view::FleetStatusView::Cancelled
                }
            };
            let members: Vec<_> = observation
                .members
                .iter()
                .map(|m| {
                    let ms = match m.class {
                        wonder_of_u_storage::MemberObservationClass::Pending => {
                            wonder_of_u_tui::fleet_view::FleetMemberStatusView::Pending
                        }
                        wonder_of_u_storage::MemberObservationClass::Running => {
                            wonder_of_u_tui::fleet_view::FleetMemberStatusView::Running
                        }
                        wonder_of_u_storage::MemberObservationClass::Completed => {
                            wonder_of_u_tui::fleet_view::FleetMemberStatusView::Completed
                        }
                        wonder_of_u_storage::MemberObservationClass::Failed => {
                            wonder_of_u_tui::fleet_view::FleetMemberStatusView::Failed
                        }
                    };
                    wonder_of_u_tui::fleet_view::FleetMemberEntryView::new(
                        m.task_id.to_string(),
                        ms,
                    )
                    .name(
                        m.task
                            .as_ref()
                            .map(|t| t.description.clone())
                            .unwrap_or_default(),
                    )
                    .excerpt(
                        m.result
                            .as_ref()
                            .map(|r| r.output_excerpt.clone())
                            .unwrap_or_default(),
                    )
                })
                .collect();
            views.push(wonder_of_u_tui::fleet_view::FleetRunView {
                fleet_id: observation.fleet.id.to_string(),
                description: observation.fleet.description.clone(),
                status,
                members,
            });
        }

        fleet_panel.fleet_views = views;
        if fleet_panel.selected_index >= fleet_panel.fleet_views.len().saturating_sub(1) {
            fleet_panel.selected_index = fleet_panel.fleet_views.len().saturating_sub(1);
        }
    }

    pub(super) fn navigate_fleet_panel(&mut self, delta: i32) {
        let Some(panel) = self.fleet_panel.as_mut() else {
            return;
        };
        if panel.fleet_views.is_empty() {
            return;
        }
        let count = panel.fleet_views.len() as i32;
        let current = panel.selected_index as i32;
        panel.selected_index = ((current + delta).rem_euclid(count)) as usize;
        panel.scroll_offset = 0;
        self.needs_render = true;
    }

    pub(super) fn steer_selected_fleet(&mut self) {
        let Some(panel) = self.fleet_panel.as_ref() else {
            return;
        };
        let Some(view) = panel.fleet_views.get(panel.selected_index) else {
            return;
        };
        // Populate the prompt with /fleet steer command so the user can type
        // their steering message and press Enter to send it.
        let command = format!("/fleet steer {} ", view.fleet_id);
        self.prompt = wonder_of_u_tui::TextBuffer::from_text(&command, true);
        self.turn_state = wonder_of_u_tui::TurnState::EditingInput;
        self.state.input_mode = wonder_of_u_core::InputMode::Prompt;
        self.close_fleet_panel();
        self.status_note = Some("steer message: type and press Enter".into());
        self.needs_render = true;
    }

    pub(super) fn handle_fleet_panel_key(
        &mut self,
        key: wonder_of_u_tui::KeyEvent,
    ) -> super::Result<()> {
        if key.is_ctrl_char('f') {
            self.close_fleet_panel();
            return Ok(());
        }
        match key.code {
            wonder_of_u_tui::KeyCode::Esc | wonder_of_u_tui::KeyCode::Char('q') => {
                self.close_fleet_panel();
            }
            wonder_of_u_tui::KeyCode::Up => {
                self.navigate_fleet_panel(-1);
            }
            wonder_of_u_tui::KeyCode::Down => {
                self.navigate_fleet_panel(1);
            }
            wonder_of_u_tui::KeyCode::Char('s') => {
                self.steer_selected_fleet();
            }
            wonder_of_u_tui::KeyCode::Enter => {
                let Some(panel) = self.fleet_panel.as_ref() else {
                    return Ok(());
                };
                if panel.fleet_views.get(panel.selected_index).is_some() {
                    let currently_selected = panel.selected_run_index;
                    self.fleet_panel.as_mut().unwrap().selected_run_index =
                        if currently_selected == Some(panel.selected_index) {
                            None
                        } else {
                            Some(panel.selected_index)
                        };
                    self.needs_render = true;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn hydrate_initial_settings(&mut self) -> Result<()> {
        if !self.state.messages.is_empty() {
            return Ok(());
        }
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        let settings = SettingsStore::new(storage_dir).read()?;
        self.state.theme = settings.theme;
        self.state.effort_level = settings.effort_level;
        self.state.fast_mode = settings.fast_mode;
        self.vim_enabled = settings.vim_mode.unwrap_or(true);
        if !self.vim_enabled {
            self.vim = VimState::new(VimMode::Insert);
        }
        if let Some(sl) = settings.status_line {
            self.status_line_command = Some(sl.command);
        }
        if settings.has_acknowledged_cost_threshold {
            self.cost_threshold_dialog_shown_session = true;
        }
        Ok(())
    }
    pub(in crate::tui_runtime) fn restore_current_session(&mut self) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        let restored = TranscriptStore::new(storage_dir).restore_session(self.state.session.id)?;
        self.persistence.transcript_message_count = restored.transcript.messages.len();
        self.persistence.transcript_warning_count = restored.transcript.warnings.len();
        self.persistence.persisted = true;
        self.state = restored.state;
        std::env::set_current_dir(&self.state.session.cwd)?;
        self.state.session.entrypoint = Some("tui".into());
        self.state.session.app_version = Some(env!("CARGO_PKG_VERSION").into());
        self.rebuild_ephemeral_state();
        // Messages have been fully replaced; bring scroll state in sync.
        self.notify_transcript_changed();
        Ok(())
    }
    pub(in crate::tui_runtime) fn refresh_runtime_state(&mut self) -> Result<bool> {
        let resolver = ProviderResolver::builtin();
        let selection =
            ProviderSelection::new(self.state.provider.clone(), self.state.model.clone());
        let provider_report = if selection.provider.is_some() || selection.model.is_some() {
            resolver
                .load_report_for_selection(self.storage_dir.as_deref(), &selection)
                .or_else(|_| resolver.load_report(self.storage_dir.as_deref()))?
        } else {
            resolver.load_report(self.storage_dir.as_deref())?
        };
        let mut changed = false;
        if self.state.provider != provider_report.provider
            || self.state.model != provider_report.model
            || self.state.auth != provider_report.auth
        {
            self.state.set_provider_context(
                provider_report.provider,
                provider_report.model,
                provider_report.auth,
            );
            changed = true;
        }

        let previous_tasks = self.state.background_tasks.clone();
        let next_tasks = match self.storage_dir.as_deref() {
            Some(storage_dir) => TaskStore::new(storage_dir)
                .list_tasks()?
                .into_iter()
                .map(|task| (task.id, task))
                .collect::<BTreeMap<_, _>>(),
            None => BTreeMap::new(),
        };
        let effective_tasks = if next_tasks.is_empty() && !self.state.background_tasks.is_empty() {
            self.state.background_tasks.clone()
        } else {
            next_tasks
        };
        if self.state.background_tasks != effective_tasks {
            if let Some(task) = next_task_notification(&previous_tasks, &effective_tasks) {
                self.dismiss_dialog();
                self.dialog = Some(DialogView::notice(
                    "Task update",
                    task_notification_lines(task),
                ));
                self.state.input_mode = InputMode::TaskNotification;
                self.status_note = Some(format!(
                    "task {}: {}",
                    task_status_label(task.status),
                    task.description
                ));
                self.task_notice_ttl = Some(TASK_NOTICE_TTL);
                self.push_notification(
                    format!("task:{}", task.id),
                    task_notification_severity(task.status),
                    "Task update",
                    task_notification_lines(task),
                    Some(SHELL_NOTIFICATION_TTL),
                    true,
                );
                // Inject once per task id into the model-facing transcript so
                // the model can observe task completion without polling.
                // The idempotence set survives session saves so a crash-recover
                // reload never double-injects the same task.
                if !self.state.injected_task_notifications.contains(&task.id) {
                    let xml = payload_from_task_state(task).render_xml();
                    let task_id = task.id;
                    if let Ok(msg) = append_contextual_message(
                        &mut self.state,
                        MessagePayload::TaskNotification {
                            task_id,
                            xml_payload: xml,
                        },
                    ) {
                        self.state.injected_task_notifications.insert(task_id);
                        let _ = self.persist_messages(&[msg]);
                    }
                }
            }
            self.state.background_tasks = effective_tasks;
            changed = true;
        }
        // Live fleet sync: detect status transitions and fire completion toasts.
        // Also drain pending requests for non-terminal fleet runs.
        if let Some(storage_dir) = self.storage_dir.clone() {
            let fleet_store = wonder_of_u_storage::FleetStore::new(&storage_dir);
            if let Ok(runs) = fleet_store.list_runs() {
                for run in &runs {
                    let inspector = wonder_of_u_storage::FleetInspector::new(storage_dir.clone());
                    if let Ok(obs) = inspector.observe(run.id) {
                        let new_status = match obs.fleet.status {
                            wonder_of_u_core::FleetRunStatus::Completed => Some("completed"),
                            wonder_of_u_core::FleetRunStatus::Failed => Some("failed"),
                            _ => None,
                        };
                        if let Some(label) = new_status {
                            let fleet_id = obs.fleet.id.to_string();
                            let key = format!("fleet:{fleet_id}:{label}");
                            if !self.fleet_completion_keys.contains(&key) {
                                let desc = obs.fleet.description.clone();
                                self.push_notification(
                                    key.clone(),
                                    if label == "failed" {
                                        wonder_of_u_tui::NotificationSeverity::Error
                                    } else {
                                        wonder_of_u_tui::NotificationSeverity::Info
                                    },
                                    format!("Fleet {label}"),
                                    std::iter::once(desc.clone()),
                                    Some(300u32),
                                    false,
                                );
                                // Persist fleet completion as a system message with aggregated results.
                                let counts =
                                    obs.members.iter().fold((0usize, 0usize), |(ok, fail), m| {
                                        match m.class {
                                        wonder_of_u_storage::MemberObservationClass::Completed => {
                                            (ok + 1, fail)
                                        }
                                        wonder_of_u_storage::MemberObservationClass::Failed => {
                                            (ok, fail + 1)
                                        }
                                        _ => (ok, fail),
                                    }
                                    });
                                let mut content = format!(
                                    "Fleet {fleet_id} {label}: \"{desc}\" — {ok_total} members ({ok} completed, {fail} failed)",
                                    ok_total = obs.members.len(),
                                    ok = counts.0,
                                    fail = counts.1,
                                );
                                // Include member result excerpts for the model to see.
                                for m in &obs.members {
                                    if let Some(ref result) = m.result {
                                        if !result.output_excerpt.is_empty() {
                                            content.push_str(&format!(
                                                "\n  [{}] {}: {}",
                                                m.class.label(),
                                                m.task
                                                    .as_ref()
                                                    .map(|t| t.description.as_str())
                                                    .unwrap_or(""),
                                                result.output_excerpt
                                            ));
                                        }
                                    }
                                }
                                if let Ok(msg) = append_contextual_message(
                                    &mut self.state,
                                    wonder_of_u_core::MessagePayload::System { content },
                                ) {
                                    self.fleet_completion_keys.insert(key);
                                    let _ = self.persist_messages(&[msg]);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }

            // Refresh the fleet panel if it's currently visible.
            if self.fleet_panel.is_some() {
                self.refresh_fleet_panel();
                self.needs_render = true;
                changed = true;
            }

            // Drain pending members from non-terminal fleet runs.
            if let Ok(runs) = fleet_store.list_runs() {
                if let Some(tm) = &self.task_manager {
                    let context = self.command_context();
                    for run in &runs {
                        if !run.status.is_terminal() {
                            if let Ok(report) =
                                ProviderResolver::builtin().load_report(self.storage_dir.as_deref())
                            {
                                let _ = crate::commands::dispatch_ready_members(
                                    tm,
                                    &fleet_store,
                                    &context,
                                    run.id,
                                    &report,
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(changed)
    }
    pub(in crate::tui_runtime) fn persist_state_snapshot(&self) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        let store = TranscriptStore::new(storage_dir);
        if self.persistence.transcript_message_count == 0 {
            store.ensure_layout()?;
            let _ = OpenOptions::new()
                .create(true)
                .append(true)
                .open(store.paths().transcript_path(self.state.session.id))?;
        }
        persist_prompt_state(&store, &self.state, &self.persistence)
    }
}

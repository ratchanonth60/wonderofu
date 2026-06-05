use super::TuiController;
use super::*;

const OUTPUT_STYLE_OPTIONS: &[OutputStyleOption] = &[
    OutputStyleOption {
        name: "markdown",
        description: "Render markdown with headings, bold, code blocks",
    },
    OutputStyleOption {
        name: "plain",
        description: "Plain text without markdown formatting",
    },
    OutputStyleOption {
        name: "raw",
        description: "Raw output, no post-processing",
    },
];

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn open_output_style_picker(&mut self) {
        let current = self
            .storage_dir
            .as_deref()
            .and_then(|d| {
                wonder_of_u_agent::SettingsStore::new(d)
                    .read()
                    .ok()
                    .and_then(|s| s.output_style)
            })
            .unwrap_or_else(|| "markdown".into());
        let selected_index = OUTPUT_STYLE_OPTIONS
            .iter()
            .position(|o| o.name == current.as_str())
            .unwrap_or(0);
        self.pending_output_style_picker = Some(OutputStylePickerState {
            selected_index,
            current,
            options: OUTPUT_STYLE_OPTIONS.to_vec(),
        });
        self.needs_render = true;
    }

    pub(in crate::tui_runtime) fn handle_output_style_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                self.step_output_style_picker(-1);
                Ok(())
            }
            KeyCode::Down => {
                self.step_output_style_picker(1);
                Ok(())
            }
            KeyCode::Tab => self.complete_output_style_picker(),
            KeyCode::Esc => self.cancel_output_style_picker(),
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => {
                    self.complete_output_style_picker()
                }
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.cancel_output_style_picker()
                }
                _ => {
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }

    fn step_output_style_picker(&mut self, delta: i32) {
        let Some(picker) = &mut self.pending_output_style_picker else {
            return;
        };
        let len = picker.options.len();
        if len == 0 {
            return;
        }
        picker.selected_index =
            (picker.selected_index as i32 + delta).rem_euclid(len as i32) as usize;
        self.needs_render = true;
    }

    fn complete_output_style_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_output_style_picker.take() else {
            return Ok(());
        };
        let Some(opt) = picker.options.get(picker.selected_index) else {
            return Ok(());
        };
        let name = opt.name;
        if let Some(storage_dir) = self.storage_dir.as_deref() {
            let store = wonder_of_u_agent::SettingsStore::new(storage_dir);
            if let Ok(mut settings) = store.read() {
                settings.output_style = (name != "markdown").then(|| name.to_string());
                let _ = store.write(&settings);
            }
        }
        self.push_notification(
            "output-style",
            NotificationSeverity::Info,
            "Output Style",
            [format!("Output style set to: {name}")],
            Some(SHELL_NOTIFICATION_TTL),
            false,
        );
        Ok(())
    }

    fn cancel_output_style_picker(&mut self) -> Result<()> {
        self.pending_output_style_picker = None;
        self.needs_render = true;
        Ok(())
    }
}

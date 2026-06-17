#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use wonder_of_u_core::{
        AppState, MessageEnvelope, MessagePayload, TaskState, TaskStatus, TokenUsage,
    };

    use super::*;
    use crate::{dialog::DialogView, message::MessageLineView};

    /// Guard against accidentally changing `MIN_SIDEBAR_WIDTH` away from 120.
    ///
    /// 120 cols is the opencode-parity minimum that gives a usable main area
    /// alongside the wider 42-col sidebar.  Lowering this further would make
    /// the transcript area too narrow on standard terminals.
    #[test]
    fn sidebar_threshold_is_120_columns() {
        assert_eq!(
            MIN_SIDEBAR_WIDTH, 120,
            "MIN_SIDEBAR_WIDTH must be 120; raising or lowering this changes sidebar activation"
        );
    }

    /// At exactly `MIN_SIDEBAR_WIDTH` the sidebar activates and the main area
    /// shrinks by `SIDEBAR_WIDTH + 1`.  One column below the threshold the
    /// sidebar must NOT activate even when `has_sidebar = true`.
    #[test]
    fn shell_main_area_width_activates_at_threshold_boundary() {
        // Exactly at threshold → sidebar deduction applies.
        assert_eq!(
            shell_main_area_width(MIN_SIDEBAR_WIDTH, true),
            MIN_SIDEBAR_WIDTH - SIDEBAR_WIDTH - 1,
            "sidebar must activate at exactly MIN_SIDEBAR_WIDTH={MIN_SIDEBAR_WIDTH}"
        );
        // One below threshold → full width even with sidebar flag set.
        assert_eq!(
            shell_main_area_width(MIN_SIDEBAR_WIDTH - 1, true),
            MIN_SIDEBAR_WIDTH - 1,
            "sidebar must NOT activate one column below the threshold"
        );
        // Threshold without sidebar flag → full width.
        assert_eq!(
            shell_main_area_width(MIN_SIDEBAR_WIDTH, false),
            MIN_SIDEBAR_WIDTH,
            "sidebar flag=false must never deduct columns"
        );
    }

    #[test]
    fn global_search_result_format_includes_location_and_truncates_preview() {
        let formatted = format_global_search_result(
            &SearchMatch {
                file: "src/main.rs".into(),
                line: 42,
                text: "let very_long_identifier_name = search_target();".into(),
            },
            24,
        );

        assert!(formatted.contains("src/main.rs:42"));
        assert!(formatted.ends_with('…'));
    }

    #[test]
    fn empty_shell_snapshot_renders_placeholder_message() {
        let view = ShellView {
            title: "Session: Empty".into(),
            messages: Vec::new(),
            prompt: String::new(),
            history_search: None,
            status: "prompt | 0 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(30, 9, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " ▸ wonder-of-u  Empty",
                "",
                "    ██╗    ██╗  ██████╗  ██╗",
                "    ██║    ██║ ██╔═══██╗ ██║",
                "╭─ prompt ───────────────────╮",
                "│›                           │",
                "│prompt · 0 messages · Enter │",
                "╰────────────────────────────╯",
                "             ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn prompt_shell_snapshot_renders_messages_and_tasks() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![
                MessageLineView::new("system> ready", MessageRole::System),
                MessageLineView::new("hello", MessageRole::Assistant),
            ],
            prompt: "/status".into(),
            history_search: None,
            status: "prompt | 2 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: Some(TaskPanelView {
                title: "Tasks".into(),
                lines: vec![MessageLineView::new(
                    "[running] shell: index workspace",
                    MessageRole::Progress,
                )],
            }),
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(48, 10, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " system> ready",
                " hello",
                "",
                " Tasks",
                " [running] shell: index workspace",
                "╭─ prompt ─────────────────────────────────────╮",
                "│› /status                                     │",
                "│prompt · 2 messages · Enter send · Shift+Enter│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_view_from_app_state_adapts_messages_and_chrome() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.session.title = "Demo".into();
        app.session.git_branch = None;
        app.provider = Some("copilot".into());
        app.model = Some("gpt-5".into());
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 12,
                output_tokens: 3,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );
        app.push_message(MessageEnvelope::system(app.session.id, "ready"))
            .expect("push system");
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::AssistantText {
                content: "working".into(),
            },
        ))
        .expect("push assistant");
        app.background_tasks.insert(
            Default::default(),
            TaskState {
                description: "index workspace".into(),
                status: TaskStatus::Running,
                ..TaskState::pending("index workspace")
            },
        );
        app.queue_command("/status", wonder_of_u_core::QueuePlacement::Later);

        let view = ShellView::from_app_state(&app, "/help", false);

        assert_eq!(view.title, "Session: Demo");
        assert_eq!(view.messages[0].text, "system> ready");
        assert!(view.status.contains("copilot:gpt-5"));
        assert!(view.footer.contains("cwd=/workspace"));
        assert!(view.queued_panel.is_some());
        assert!(view.task_panel.is_some());
    }

    #[test]
    fn shell_snapshot_renders_queued_commands_overlay() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "/plan".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: Some(TaskPanelView {
                title: "Queued".into(),
                lines: vec![
                    MessageLineView::new("1. /status", MessageRole::Progress),
                    MessageLineView::new("2. draft migration plan", MessageRole::Progress),
                    MessageLineView::new("+2 more queued", MessageRole::System),
                ],
            }),
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " ready",
                "",
                "",
                " Queued",
                " 1. /status",
                " 2. draft migration plan",
                " +2 more queued",
                "╭─ prompt ───────────────────────────────╮",
                "│› /plan                                 │",
                "│prompt · 1 messages · Enter send · Shift│",
                "╰────────────────────────────────────────╯",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn dialog_snapshot_renders_confirm_overlay() {
        let view = ShellView {
            title: "Session: Dialog".into(),
            messages: vec![MessageLineView::new("system> ready", MessageRole::System)],
            prompt: "continue?".into(),
            history_search: None,
            status: "permission | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: Some(DialogView::confirm(
                "Confirm action",
                ["Approve command execution", "This cannot be undone"],
            )),
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "╭─Confirm action─────────────────────────╮",
                "│Approve command execution               │",
                "│This cannot be undone                   │",
                "│                                        │",
                "│▶ Confirm                               │",
                "│  Cancel                                │",
                "╰────────────────────────────────────────╯",
                "╭─ prompt ───────────────────────────────╮",
                "│› continue?                             │",
                "│permission · 1 messages · Enter send · S│",
                "╰────────────────────────────────────────╯",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn history_search_snapshot_renders_overlay_and_preview() {
        let view = ShellView {
            title: "Session: Search".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "draft".into(),
            history_search: Some(HistorySearchView {
                query: "pla".into(),
                match_text: Some("draft plan".into()),
                match_index: 1,
                match_total: 3,
            }),
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(42, 16, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " ready",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "╭─ prompt ───────────────────────────────╮",
                "│draft plan                              │",
                "│history: pla · match 2/3 · ↑↓/Ctrl+R cyc│",
                "╰────────────────────────────────────────╯",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn slash_suggestions_snapshot_renders_above_prompt_footer() {
        let view = ShellView {
            title: "Session: Slash".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "/".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: Some(SlashSuggestionsOverlay {
                entries: vec![
                    SlashSuggestionEntry {
                        display: "/status".into(),
                        description: "show session stats".into(),
                        selected: true,
                        match_ranges: Vec::new(),
                    },
                    SlashSuggestionEntry {
                        display: "/theme".into(),
                        description: "pick theme".into(),
                        selected: false,
                        match_ranges: Vec::new(),
                    },
                ],
            }),
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(48, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " ready",
                "",
                "",
                "╭─commands─────────────────────╮",
                "│/status ─ show session stats  │",
                "│/theme ─ pick theme           │",
                "╰──────────────────────────────╯",
                "╭─ prompt ─────────────────────────────────────╮",
                "│› /                                           │",
                "│slash commands · ↑↓ select · Tab/Enter apply ·│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn slash_suggestions_with_argument_hints_render_in_display_column() {
        // Entries whose `display` already contains a hint (e.g. `/fast [on|off]`)
        // must be rendered verbatim in the left column of the overlay.
        let view = ShellView {
            title: "Session: Hints".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "/f".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: Some(SlashSuggestionsOverlay {
                entries: vec![
                    SlashSuggestionEntry {
                        display: "/fast [on|off]".into(),
                        description: "fast-mode model remapping".into(),
                        selected: true,
                        match_ranges: Vec::new(),
                    },
                    SlashSuggestionEntry {
                        display: "/effort [low|medium|high|max|auto]".into(),
                        description: "active effort level".into(),
                        selected: false,
                        match_ranges: Vec::new(),
                    },
                ],
            }),
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(64, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Argument hints must appear in the rendered overlay.
        assert!(
            text.contains("/fast [on|off]"),
            "hint for /fast not found in:\n{text}"
        );
        assert!(
            text.contains("/effort [low|medium|high|max|auto]"),
            "hint for /effort not found in:\n{text}"
        );
    }

    #[test]
    fn transcript_snapshot_keeps_latest_visible_lines() {
        // height=9, CHROME_HEIGHT=1 → available=8, prompt_height=3 (boxed),
        // messages_height=5.  With 8 messages only the tail 5 are visible, so
        // "line 1" is pushed off the top.
        let view = ShellView {
            title: "Session: Tail".into(),
            messages: (1..=8)
                .map(|index| MessageLineView::new(format!("line {index}"), MessageRole::Assistant))
                .collect(),
            prompt: "tail".into(),
            history_search: None,
            status: "prompt | 8 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(32, 9, &view, &Theme::default());

        assert!(!frame.to_plain_text().contains("line 1"));
        assert!(frame.to_plain_text().contains("line 6"));
    }

    #[test]
    fn shell_snapshot_renders_grouped_tool_summary_lines() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![
                MessageLineView::new("● Run(Tests)", MessageRole::Tool),
                MessageLineView::new("  └ cargo test -p wonder-of-u-tui", MessageRole::System),
                MessageLineView::new("  └ tests passed", MessageRole::System),
            ],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 3 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(48, 10, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " ● Run(Tests)",
                "   └ cargo test -p wonder-of-u-tui",
                "   └ tests passed",
                "",
                "",
                "╭─ prompt ─────────────────────────────────────╮",
                "│›                                             │",
                "│prompt · 3 messages · Enter send · Shift+Enter│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_snapshot_renders_notification_stack_overlay() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: vec![
                NotificationView {
                    key: "source-status".into(),
                    title: "Source status".into(),
                    lines: vec!["workspace index refreshed".into()],
                    severity: NotificationSeverity::Info,
                    focused: false,
                },
                NotificationView {
                    key: "task:1".into(),
                    title: "Task update".into(),
                    lines: vec!["tests passed".into()],
                    severity: NotificationSeverity::Success,
                    focused: true,
                },
            ],
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(48, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " ready",
                "                   ╭─info Source status────────╮",
                "                   │workspace index refreshed  │",
                "                   ╰───────────────────────────╯",
                "                      ╭─ok Task update • focus─╮",
                "                      │tests passed            │",
                "                      ╰────────────────────────╯",
                "╭─ prompt ─────────────────────────────────────╮",
                "│›                                             │",
                "│prompt · 1 messages · Enter send · Shift+Enter│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_snapshot_renders_picker_list_overlay() {
        let view = ShellView {
            title: "Session: Picker".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: Some(PickerListView {
                title: "Select Theme".into(),
                query: "mid".into(),
                entries: vec![
                    crate::message::PickerListEntry {
                        label: "Midnight".into(),
                        description: "Dark theme".into(),
                        tag: Some("current".into()),
                        selected: true,
                        group_header: None,
                    },
                    crate::message::PickerListEntry {
                        label: "Light".into(),
                        description: "Bright theme".into(),
                        tag: None,
                        selected: false,
                        group_header: None,
                    },
                ],
                hint: "↑↓ navigate  Tab/Enter select  Esc cancel".into(),
            }),
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(60, 16, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(text.contains("Select Theme"));
        assert!(text.contains("Search: mid"));
        // Tag is now right-aligned at the row's far-right edge; label and tag
        // appear on the same row but are no longer adjacent — verify each part
        // is present in the rendered frame.
        assert!(
            text.contains("▸ Midnight"),
            "selected cursor + label must be present"
        );
        assert!(text.contains("Dark theme"), "description must be visible");
        assert!(
            text.contains("[current]"),
            "right-aligned tag badge must be present"
        );
        // The tag must not appear immediately after the label (it was moved to
        // the right edge, so there are padding spaces between them).
        assert!(
            !text.contains("Midnight [current]"),
            "tag must not be inline immediately after label"
        );
        assert!(text.contains("↑↓ navigate  Tab/Enter select  Esc cancel"));
    }

    /// Verifies that the picker tag badge is right-aligned at the row's far-right
    /// edge and that the description is rendered with a visual gap before the tag.
    ///
    /// Layout (inner width = 46, tag "[current]" = 9 cols):
    /// ```text
    /// ▸ Midnight — Dark theme              [current]
    ///   Light — Bright theme
    /// ```
    /// The tag must be separated from the description by at least one space.
    #[test]
    fn picker_list_tag_is_right_aligned() {
        let view = ShellView {
            title: "Test".into(),
            messages: Vec::new(),
            prompt: String::new(),
            history_search: None,
            status: String::new(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: String::new(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: Some(PickerListView {
                title: "Pick".into(),
                query: String::new(),
                entries: vec![
                    crate::message::PickerListEntry {
                        label: "Alpha".into(),
                        description: "first option".into(),
                        tag: Some("active".into()),
                        selected: true,
                        group_header: None,
                    },
                    crate::message::PickerListEntry {
                        label: "Beta".into(),
                        description: "second option".into(),
                        tag: None,
                        selected: false,
                        group_header: None,
                    },
                ],
                hint: "Esc cancel".into(),
            }),
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(60, 16, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Label + description appear normally.
        assert!(text.contains("▸ Alpha"), "cursor + label must render");
        assert!(text.contains("first option"), "description must render");
        // Tag is present but not adjacent to the label.
        assert!(text.contains("[active]"), "tag badge must render");
        assert!(
            !text.contains("Alpha [active]"),
            "tag must not be immediately after label"
        );
        // Find the line that contains the tag and verify the label appears to
        // its left (tag is right-aligned, so it comes after the label in the line).
        let tag_line = text
            .lines()
            .find(|l| l.contains("[active]"))
            .expect("a line containing the tag must exist");
        let label_pos = tag_line
            .find("Alpha")
            .expect("label must be on the same line");
        let tag_pos = tag_line
            .find("[active]")
            .expect("tag must be on the same line");
        assert!(
            label_pos < tag_pos,
            "label must appear before right-aligned tag (label@{label_pos} tag@{tag_pos})"
        );
        // There must be at least one space between the description and the tag.
        assert!(
            tag_pos > label_pos + "Alpha".len() + 2,
            "tag must be separated from label by at least 2 positions"
        );
    }

    #[test]
    fn shell_snapshot_prefixes_loading_status() {
        let view = ShellView {
            title: "Session: Loading".into(),
            messages: Vec::new(),
            prompt: String::new(),
            history_search: None,
            status: "turn=active".into(),
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: String::new(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(32, 8, &view, &Theme::default());

        let text = frame.to_plain_text();
        // The spinner prefix still appears in the transcript (status row removed with CHROME_HEIGHT=1).
        assert!(text.contains("· thinking…"));
    }

    #[test]
    fn loading_progress_row_appears_above_prompt_when_loading() {
        // The loading row should be rendered in the message area, directly above
        // the prompt box — separate from the transcript content.
        let view = ShellView {
            title: "Session: Loading".into(),
            messages: vec![MessageLineView::new("hello", MessageRole::Assistant)],
            prompt: String::new(),
            history_search: None,
            status: "turn=active".into(),
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 4,
            loading_elapsed_secs: 3,
            loading_total_tokens: 150,
            footer: "▸▸ default (shift+tab to cycle) · ⌃C exit".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            fleet_panel: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            sidebar_mode: SidebarMode::default(),
            prompt_warning: None,
            tool_progress: Vec::new(),
            message_cursor_index: None,
        };

        let frame = render_snapshot(60, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Loading progress row must contain the spinner verb and "esc to interrupt".
        assert!(
            text.contains("thinking"),
            "loading row must contain verb: {text}"
        );
        assert!(
            text.contains("esc to interrupt"),
            "loading row must contain suffix: {text}"
        );
        // Loading row must NOT appear inside the transcript (spinner moved out of messages).
        // The transcript message "hello" must still appear.
        assert!(
            text.contains("hello"),
            "transcript message must still appear"
        );
        // Compact footer hint must appear.
        assert!(
            text.contains("shift+tab to cycle"),
            "compact footer hint must appear: {text}"
        );
        assert!(
            !text.contains("storage="),
            "verbose footer fields must not appear: {text}"
        );
    }

    #[test]
    fn loading_progress_row_includes_elapsed_and_tokens_when_nonzero() {
        let view = ShellView {
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 2,
            loading_elapsed_secs: 7,
            loading_total_tokens: 512,
            ..ShellView::default()
        };

        let text = build_loading_progress_text(&view);
        // Elapsed must appear (7 seconds → "00:07" mm:ss format).
        assert!(
            text.contains("00:07"),
            "elapsed seconds must appear in loading row text: {text}"
        );
        // Token count must appear.
        assert!(
            text.contains("512"),
            "token count must appear in loading row text: {text}"
        );
    }

    #[test]
    fn loading_progress_row_omits_elapsed_and_tokens_when_zero() {
        let view = ShellView {
            loading: true,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            ..ShellView::default()
        };

        let text = build_loading_progress_text(&view);
        // Neither elapsed nor token data should appear.
        assert!(
            !text.contains("tok"),
            "token count must not appear when zero: {text}"
        );
        // The "esc to interrupt" suffix must always appear.
        assert!(
            text.contains("esc to interrupt"),
            "suffix must always appear: {text}"
        );
    }

    // ── scroll rendering ─────────────────────────────────────────────────────

    fn make_long_transcript(count: usize) -> Vec<MessageLineView> {
        (1..=count)
            .map(|i| MessageLineView::new(format!("line {i:02}"), MessageRole::Assistant))
            .collect()
    }

    #[test]
    fn transcript_tail_mode_shows_newest_lines() {
        // 10 lines of content → tail mode (offset 0) must show the newest line.
        let view = ShellView {
            title: "Session: Tail".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "tail".into(),
            scroll: TranscriptScrollView::default(), // offset_from_bottom = 0
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 7, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(!text.contains("line 01"), "oldest line must not appear");
        assert!(text.contains("line 10"), "newest line must appear");
        // Status line must NOT show the scroll indicator in tail mode.
        assert!(
            !text.contains("Ctrl+End"),
            "scroll indicator must be absent in tail mode"
        );
    }

    #[test]
    fn transcript_scrolled_up_shows_earlier_window() {
        // height=10, CHROME_HEIGHT=1 → available=9, prompt_height=4 (boxed with
        // integrated footer), messages_height=5. offset_from_bottom=1:
        //   start = (10-5).saturating_sub(1) = 4 → shows lines 05–09.
        let view = ShellView {
            title: "Session: Scrolled".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "scrolled".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 1,
                total_lines: 10,
                visible_lines: 8,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 10, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(text.contains("line 05"), "window start must be visible");
        assert!(text.contains("line 09"), "window end must be visible");
        assert!(
            !text.contains("line 04"),
            "line before window must be hidden"
        );
        assert!(!text.contains("line 10"), "newest line must not appear");
    }

    #[test]
    fn scroll_indicator_appears_in_status_when_scrolled_up() {
        // Scrolling up by 5 lines must add "↑ 5 lines · Ctrl+End bottom" to the
        // status row.
        let view = ShellView {
            title: "Session: Ind".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "scrolled".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 5,
                total_lines: 10,
                visible_lines: 4,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            text.contains("5 lines"),
            "line count must appear in indicator; got: {text:?}"
        );
        assert!(
            text.contains("Ctrl+End bottom"),
            "hint text must appear in indicator; got: {text:?}"
        );
    }

    #[test]
    fn scroll_indicator_absent_when_at_tail() {
        let view = ShellView {
            title: "Session: Tail".into(),
            messages: make_long_transcript(5),
            prompt: String::new(),
            status: "tail".into(),
            scroll: TranscriptScrollView::default(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            !text.contains("Ctrl+End"),
            "scroll indicator must not appear in tail mode"
        );
    }

    #[test]
    fn welcome_screen_unchanged_when_messages_empty() {
        // The welcome screen is always top-anchored regardless of any scroll offset,
        // because `draw_lines` (not the windowed path) is used for empty messages.
        let view = ShellView {
            title: "Session: Empty".into(),
            messages: Vec::new(),
            prompt: String::new(),
            status: "welcome".into(),
            // Even if a non-zero offset somehow leaks in, the welcome path is unchanged.
            scroll: TranscriptScrollView {
                offset_from_bottom: 99,
                total_lines: 0,
                visible_lines: 10,
            },
            ..ShellView::default()
        };

        // Use a tall frame so the welcome tagline (line 8 of the welcome screen) is
        // actually visible in the transcript area.
        let frame = render_snapshot(42, 20, &view, &Theme::default());
        let text = frame.to_plain_text();
        // The ASCII-art banner should appear at the top.
        assert!(text.contains("██╗"), "welcome banner must be present");
        assert!(
            text.contains("wonder-of-u  ·  AI coding assistant"),
            "welcome tagline must be present"
        );
    }

    #[test]
    fn scrolled_up_past_top_is_clamped_to_first_line() {
        // 3 lines of content, offset = 999 (far beyond max).
        // render_snapshot(20, 12) → 6 visible transcript rows.
        // start = (3 - 6).saturating_sub(999) = 0 → all 3 lines are shown.
        let view = ShellView {
            title: "Session: Clamp".into(),
            messages: make_long_transcript(3),
            prompt: String::new(),
            status: "clamped".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 999,
                total_lines: 3,
                visible_lines: 6,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 12, &view, &Theme::default());
        let text = frame.to_plain_text();
        // All three lines must appear since start is clamped to 0.
        assert!(
            text.contains("line 01"),
            "first line must be visible when clamped"
        );
        assert!(
            text.contains("line 03"),
            "last line must be visible when clamped"
        );
    }

    // ── prompt height and cap ────────────────────────────────────────────────

    #[test]
    fn prompt_height_single_line_is_line_count() {
        // Rounded prompt: a single-line prompt occupies one content row, an
        // integrated footer row, and the borders.
        let view = ShellView {
            prompt: "hello".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 4);
    }

    #[test]
    fn prompt_height_multiline_counts_all_lines() {
        // 3-line prompt → 3 content rows + footer row + 2 border rows.
        let view = ShellView {
            prompt: "line one\nline two\nline three".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 6);
    }

    #[test]
    fn prompt_height_empty_prompt_returns_one() {
        // An empty prompt still needs one input row, the integrated footer row,
        // and the rounded borders.
        let view = ShellView {
            prompt: String::new(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 4);
    }

    // ── Shift+Enter trailing blank line (Fix 1) ──────────────────────────────

    #[test]
    fn split_lines_preserves_trailing_blank_row() {
        // "first\n" must yield two segments: the text row and a blank
        // continuation row.  str::lines() would silently drop the trailing
        // empty segment, causing the blank row to be invisible after Shift+Enter.
        assert_eq!(
            split_lines("first\n"),
            vec!["first".to_string(), String::new()],
            "trailing newline must produce a blank second segment"
        );
    }

    #[test]
    fn text_line_count_trailing_newline_counts_as_extra_row() {
        // A prompt ending in '\n' must count as 2 logical lines so that
        // prompt_height() allocates the visible blank continuation row.
        assert_eq!(
            text_line_count("first\n"),
            2,
            "trailing newline must count as an extra blank row"
        );
        // No regression: non-trailing newline still counts normally.
        assert_eq!(text_line_count("a\nb"), 2);
        // Single line without newline stays 1.
        assert_eq!(text_line_count("hello"), 1);
    }

    #[test]
    fn prompt_height_trailing_newline_grows_box() {
        // "first\n" has 2 logical rows (text + blank), so the rounded prompt
        // box needs 2 content rows + top border + integrated footer + bottom
        // border = 5 total rows.  Before the fix this returned 4.
        let view = ShellView {
            prompt: "first\n".into(),
            ..ShellView::default()
        };
        assert_eq!(
            view.prompt_height(),
            5,
            "prompt ending in '\\n' must grow the box by one row for the blank continuation line"
        );
    }

    #[test]
    fn render_trailing_newline_shows_blank_prompt_row() {
        // The prompt "first\n" must render as two visible rows inside the
        // prompt box: one carrying "› first" and one that is blank (the cursor
        // sits there after Shift+Enter at end of line).
        let view = ShellView {
            title: "Session: Blank".into(),
            prompt: "first\n".into(),
            status: String::new(),
            ..ShellView::default()
        };
        // Use a tall-enough terminal so the full 5-row prompt box fits.
        let frame = render_snapshot(40, 20, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            text.contains("› first"),
            "first prompt row must have the '› ' marker; rendered:\n{text}"
        );
        // The box must be 5 rows tall: we should see the top and bottom
        // rounded corners (╭ / ╰) plus two content rows and the footer row.
        // Rather than asserting exact y-positions, assert that "first" appears
        // AND that the frame has enough prompt box rows to show the blank row.
        assert_eq!(
            view.prompt_height(),
            5,
            "prompt_height must equal 5 with trailing newline so blank row is visible"
        );
    }

    #[test]
    fn render_shell_caps_prompt_to_one_third_of_terminal_height() {
        // Terminal height = 9 rows; one-third cap = max(9/3, 3) = 3 prompt rows.
        // A 10-line prompt would request prompt_height() = 12, but the renderer
        // must cap it at 3 so the history/transcript area always gets space.
        let ten_line_prompt = (0..10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let view = ShellView {
            title: "Session: Cap".into(),
            messages: vec![MessageLineView::new("hello", MessageRole::Assistant)],
            prompt: ten_line_prompt,
            status: "prompt".into(),
            ..ShellView::default()
        };

        // The uncapped prompt would be 11 rows tall; with a 9-row terminal the
        // render should not panic and the transcript line must still be visible.
        let frame = render_snapshot(40, 9, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            text.contains("hello"),
            "transcript must survive tall prompt; rendered:\n{text}"
        );
    }

    // ── multiline prompt marker and cursor offset ────────────────────────────

    #[test]
    fn multiline_prompt_first_line_has_marker_continuation_lines_do_not() {
        // Rounded prompt: prompt_height = 6 (3 content lines + prompt footer +
        // borders). A slightly taller frame keeps all three prompt lines visible.
        let view = ShellView {
            title: "Session: ML".into(),
            messages: Vec::new(),
            prompt: "first line\nsecond line\nthird line".into(),
            status: "prompt".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(30, 18, &view, &Theme::default());
        let text = frame.to_plain_text();

        // First line must carry the prompt marker.
        assert!(
            text.contains("› first line"),
            "first prompt line must have the '› ' marker; rendered:\n{text}"
        );
        // Continuation lines must not carry the marker — they should start
        // flush with the content column without '› '.
        assert!(
            text.contains("second line") && !text.contains("› second line"),
            "second prompt line must NOT have the '› ' marker; rendered:\n{text}"
        );
        assert!(
            text.contains("third line") && !text.contains("› third line"),
            "third prompt line must NOT have the '› ' marker; rendered:\n{text}"
        );
    }

    // ── provider error rendering via ShellView::from_app_state ──────────────

    #[test]
    fn provider_error_payload_appears_as_error_role_in_shell_view() {
        // Verify that from_app_state maps a ProviderError payload through
        // message_lines() into a MessageLineView with MessageRole::Error.
        // This complements the lower-level render_message test by confirming
        // the full pipeline from AppState → ShellView → transcript line.
        let mut app = AppState::new(PathBuf::from("/workspace"));
        let envelope = MessageEnvelope::new(
            app.session.id,
            MessagePayload::ProviderError {
                kind: "provider".into(),
                message: "connection refused".into(),
            },
        );
        app.push_message(envelope).expect("push provider error");

        let view = ShellView::from_app_state(&app, "", false);
        assert_eq!(view.messages.len(), 1, "one transcript line expected");
        assert_eq!(
            view.messages[0].role,
            MessageRole::Error,
            "ProviderError must map to MessageRole::Error"
        );
        assert!(
            view.messages[0].text.contains("connection refused"),
            "message body must appear in the transcript line; got: {:?}",
            view.messages[0].text
        );
    }

    // ── sidebar panel ─────────────────────────────────────────────────────────

    #[test]
    fn sidebar_absent_on_narrow_terminal_below_min_width() {
        // Width 119 is one below the MIN_SIDEBAR_WIDTH threshold — the sidebar
        // must not appear even when `view.sidebar` is Some.
        let view = ShellView {
            prompt: "hello".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["openai · gpt-4o".into()],
                status_lines: vec!["✓ ready".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(119, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Sidebar section headers must not appear on a narrow terminal.
        assert!(
            !text.contains("─ Providers ─"),
            "sidebar hints must be absent on narrow terminal; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_renders_on_wide_terminal_at_min_width() {
        // Width == MIN_SIDEBAR_WIDTH (120) must activate the sidebar.
        // The sidebar spans the full terminal height so a single-line prompt is
        // sufficient — no need for a tall prompt to expose sidebar rows.
        let view = ShellView {
            prompt: "say hello".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                control_lines: vec!["↵ send  ⇧↵ newline".into(), "⎋ cancel  ? help".into()],
                status_lines: vec!["✓ ready".into()],
                footer_brand: "wonder-of-u v0.1.3".into(),
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        // Use a taller terminal so the borderless panel has enough inner rows
        // to show all three sidebar sections (Providers, Status, Controls)
        // plus the pinned branded footer row.
        let frame = render_snapshot(120, 14, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Controls ─"),
            "Controls header must appear in sidebar; rendered:\n{text}"
        );
        assert!(
            text.contains("─ Providers ─"),
            "Providers header must appear in sidebar; rendered:\n{text}"
        );
        assert!(
            text.contains("copilot · gpt-4"),
            "model hint must appear in sidebar; rendered:\n{text}"
        );
        assert!(
            text.contains("ready"),
            "auth-ok label must appear; rendered:\n{text}"
        );
        // The solid panel fill replaces the rounded border entirely — no
        // border glyphs should appear at the sidebar's fixed column span
        // (the rightmost SIDEBAR_WIDTH columns).  Use the untrimmed per-cell
        // rows (`frame.lines()`) rather than `to_plain_text()` so short rows
        // don't get padded-away, which would otherwise alias the *prompt*
        // box's border into this column range.
        let sidebar_start = usize::from(120u16.saturating_sub(SIDEBAR_WIDTH));
        let sidebar_column_has_border_glyphs = frame.lines().iter().any(|line| {
            line.chars()
                .skip(sidebar_start)
                .any(|c| matches!(c, '╭' | '╮' | '╰' | '╯' | '│'))
        });
        assert!(
            !sidebar_column_has_border_glyphs,
            "borderless panel must not draw rounded border glyphs; rendered:\n{text}"
        );
        // The branded footer (`●` + version) must be pinned to the bottom row.
        let last_row = text.lines().last().unwrap_or("");
        assert!(
            last_row.contains('●') && last_row.contains("wonder-of-u v0.1.3"),
            "branded footer must render on the bottom row; got: {last_row:?}\nrendered:\n{text}"
        );
        // Prompt content must still be visible in the main (left) column.
        assert!(
            text.contains("say hello"),
            "prompt text must survive sidebar split; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_absent_when_field_is_none() {
        // sidebar = None must suppress the panel even on a wide terminal.
        let view = ShellView {
            prompt: "test".into(),
            sidebar: None,
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Providers ─") && !text.contains("─ Controls ─"),
            "sidebar must not render when field is None; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_auth_missing_warning_when_not_authenticated() {
        let view = ShellView {
            prompt: "help".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["anthropic · claude-3".into()],
                status_lines: vec!["⚠ auth missing".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("auth missing"),
            "auth-missing warning must appear; rendered:\n{text}"
        );
        assert!(
            !text.contains("✓"),
            "ready checkmark must not appear when not authenticated; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_mode_hint_for_bash_mode() {
        let view = ShellView {
            prompt: "!ls".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● bash mode".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("bash"),
            "bash mode hint must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_git_branch_when_configured() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                workspace_lines: vec!["⎇  feat/my-feature".into(), "✓ ready".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("feat/my-feature"),
            "git branch must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_cwd_hint_when_configured() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                workspace_lines: vec!["  myproject".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("myproject"),
            "cwd hint must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_task_count_when_nonzero() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                task_lines: vec!["⚙  3 tasks".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("3 tasks"),
            "task count must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn from_app_state_populates_sidebar_with_provider_and_auth() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.provider = Some("openai".into());
        app.model = Some("gpt-4o".into());
        app.context_window_size = Some(128_000);
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 32_000,
                output_tokens: 8_000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );

        let view = ShellView::from_app_state(&app, "", false);

        let sidebar = view.sidebar.expect("sidebar must be Some from_app_state");
        assert!(
            sidebar
                .provider_lines
                .iter()
                .any(|l| l.contains("openai · gpt-4o")),
            "provider_lines must combine provider and model; got: {:?}",
            sidebar.provider_lines
        );
        assert!(
            !sidebar.control_lines.is_empty(),
            "control_lines must be populated"
        );
        // Default status is the idle stub.
        assert!(
            sidebar.status_lines.iter().any(|l| l.contains("idle")),
            "default status must be idle stub; got: {:?}",
            sidebar.status_lines
        );
        assert!(
            sidebar
                .context_lines
                .iter()
                .any(|line| line.contains("40,000 / 128,000 tokens")),
            "context_lines must show token totals; got: {:?}",
            sidebar.context_lines
        );
        assert!(
            sidebar.suggestions.is_empty(),
            "suggestions must stay hidden below threshold; got: {:?}",
            sidebar.suggestions
        );
    }

    #[test]
    fn from_app_state_populates_sidebar_git_branch_and_cwd() {
        let mut app = AppState::new(PathBuf::from("/workspace/myproject"));
        app.session.git_branch = Some("feat/my-feature".into());

        let view = ShellView::from_app_state(&app, "", false);

        let sidebar = view.sidebar.expect("sidebar must be Some from_app_state");
        assert!(
            sidebar
                .workspace_lines
                .iter()
                .any(|l| l.contains("feat/my-feature")),
            "workspace_lines must contain git branch; got: {:?}",
            sidebar.workspace_lines
        );
        assert!(
            sidebar
                .workspace_lines
                .iter()
                .any(|l| l.contains("myproject")),
            "workspace_lines must contain the last cwd component; got: {:?}",
            sidebar.workspace_lines
        );
    }

    #[test]
    fn context_suggestions_shown_above_threshold() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.set_context_window_size(Some(100_000));
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 60_000,
                output_tokens: 15_000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );

        let suggestions = context_suggestions(&state);

        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.detail.contains("/compact")),
            "expected /compact guidance, got: {suggestions:?}"
        );
    }

    #[test]
    fn context_suggestions_hidden_when_context_window_unknown() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 60_000,
                output_tokens: 15_000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );

        assert!(
            context_suggestions(&state).is_empty(),
            "suggestions must be hidden when the context window is unknown"
        );
    }

    #[test]
    fn sidebar_multiline_prompt_uses_full_main_column_width() {
        // The sidebar lives at shell level; the prompt takes the entire
        // main-column width — no internal split should compress the typing area.
        let view = ShellView {
            prompt: "first line\nsecond line\nthird line".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                control_lines: vec!["↵ send  ⇧↵ newline".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        // width=120 → main_w=77, sidebar=42, sep=1.
        // height=16 keeps the multiline prompt plus the integrated footer visible.
        let frame = render_snapshot(120, 16, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("› first line"),
            "first prompt line must carry the marker; rendered:\n{text}"
        );
        assert!(
            text.contains("second line"),
            "second prompt line must be visible; rendered:\n{text}"
        );
        assert!(
            text.contains("─ Controls ─"),
            "sidebar Controls header must be present at shell level; rendered:\n{text}"
        );
    }

    #[test]
    fn prompt_spans_full_terminal_width_on_narrow_terminal() {
        // Width 119 is one below MIN_SIDEBAR_WIDTH=120.  Even when `sidebar` is Some,
        // no column is carved out — the prompt must use the full 119 columns.
        let view = ShellView {
            prompt: "hello".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(119, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Prompt marker must appear inside the rounded box with no sidebar offset.
        let prompt_line = text
            .lines()
            .find(|l| l.contains("› hello"))
            .expect("prompt marker must be present");

        // Sidebar must be suppressed on narrow terminal.
        assert!(
            !text.contains("─ Providers ─"),
            "sidebar must be suppressed below MIN_SIDEBAR_WIDTH; rendered:\n{text}"
        );
        // The prompt box must be at the start of the line (no sidebar indentation).
        assert!(
            prompt_line.starts_with("│›"),
            "prompt content must start in the full-width prompt box on narrow terminal; got: {prompt_line:?}"
        );
    }

    #[test]
    fn prompt_view_does_not_panic_on_tiny_terminal() {
        // Tiny prompt areas fall back to direct content rendering when there is no
        // room for rounded chrome. Verify no panic and that the '›' marker appears.
        let view = ShellView {
            prompt: "hi".into(),
            ..ShellView::default()
        };

        // height=3 → chrome=1, available=2, prompt is reduced to keep one message row.
        // The prompt area is height=1, which draws directly without any box chrome.
        let frame = render_snapshot(10, 3, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Must not panic; the prompt marker '›' must still appear.
        assert!(
            text.contains('›'),
            "prompt marker must appear on tiny terminal; rendered:\n{text}"
        );
        // No rounded-box corners should appear in the tiny fallback path.
        assert!(
            !text.contains('╭'),
            "box corners must not appear in tiny fallback prompt; rendered:\n{text}"
        );
    }

    // ── new sectioned-sidebar tests ──────────────────────────────────────────

    #[test]
    fn sidebar_section_header_providers_appears_on_wide_terminal() {
        // A non-empty `provider_lines` section must produce a "─ Providers ─"
        // header row visible in the rendered output on a wide terminal.
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ anthropic · claude-3-5-sonnet".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Providers ─"),
            "Providers section header must appear on wide terminal; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_provider_line_with_diamond_prefix_rendered() {
        // A `provider_lines` entry with the `◈` prefix must appear in the output.
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ openai · gpt-4o".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("◈"),
            "◈ prefix must appear in sidebar provider line; rendered:\n{text}"
        );
        assert!(
            text.contains("openai · gpt-4o"),
            "provider body text must appear; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_absent_on_narrow_terminal_below_120_cols() {
        // Narrow terminal (width < MIN_SIDEBAR_WIDTH = 120): sidebar must be
        // completely suppressed even when `sidebar` field is `Some`.
        let view = ShellView {
            prompt: "narrow".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                control_lines: vec!["↵ send  ⇧↵ newline".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(119, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Providers ─"),
            "Providers header must NOT appear on narrow (119-col) terminal; rendered:\n{text}"
        );
        assert!(
            !text.contains("─ Controls ─"),
            "Controls header must NOT appear on narrow terminal; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_empty_sections_are_omitted() {
        // Sections with empty Vec must not produce stray headers or blank rows.
        let view = ShellView {
            prompt: "check".into(),
            sidebar: Some(SidebarView {
                // Only status_lines is populated; everything else is empty.
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Status ─"),
            "Status header must appear; rendered:\n{text}"
        );
        // All empty sections — including the four new panel sections — must
        // produce no headers.
        for absent in &[
            "─ Session ─",
            "─ Context ─",
            "─ Tools ─",
            "─ MCP ─",
            "─ LSP ─",
            "─ Todo ─",
            "─ Providers ─",
            "─ Controls ─",
            "─ Workspace ─",
            "─ Tasks ─",
        ] {
            assert!(
                !text.contains(absent),
                "{absent} header must not appear when section is empty; rendered:\n{text}"
            );
        }
    }

    #[test]
    fn shell_snapshot_renders_context_visualization_in_sidebar() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ anthropic · claude-3-7-sonnet-latest".into()],
                context_lines: vec![
                    "45,000 / 200,000 tokens".into(),
                    "[#####-------------------] 22%".into(),
                ],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Context ─"),
            "missing context header:\n{text}"
        );
        assert!(
            text.contains("45,000 / 200,000 tokens"),
            "missing token totals:\n{text}"
        );
        assert!(
            text.contains("[#####-------------------] 22%"),
            "missing progress bar:\n{text}"
        );
    }

    #[test]
    fn shell_snapshot_renders_context_suggestions_in_sidebar() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                context_lines: vec![
                    "160,000 / 200,000 tokens".into(),
                    "[###################-----] 80%".into(),
                ],
                suggestions: vec![ContextSuggestion {
                    severity: SuggestionSeverity::Warning,
                    title: "Context nearing limit".into(),
                    detail: "Run /compact to reduce context usage".into(),
                }],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Suggestions ─"),
            "missing suggestions header:\n{text}"
        );
        assert!(
            text.contains("Context nearing limit"),
            "missing suggestion title:\n{text}"
        );
        assert!(
            text.contains("/compact"),
            "missing suggestion detail:\n{text}"
        );
    }

    #[test]
    fn shell_snapshot_renders_context_warning_above_prompt() {
        let view = ShellView {
            prompt: "continue".into(),
            prompt_warning: Some(PromptWarningView {
                text: "⚠ Context window 80% full — consider starting a new session".into(),
                severity: PromptWarningSeverity::Warning,
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(80, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("⚠ Context window 80% full"),
            "missing warning banner:\n{text}"
        );
        let warning_row = text
            .lines()
            .position(|line| line.contains("⚠ Context window 80% full"))
            .expect("warning row");
        let prompt_border_row = text
            .lines()
            .position(|line| line.starts_with('╭'))
            .expect("prompt border row");
        assert_eq!(warning_row.saturating_add(1), prompt_border_row);
    }

    // ── Claude visual parity: no persistent header in normal chat mode ───────

    #[test]
    fn session_title_header_absent_when_messages_present() {
        // Once the conversation has messages the "▸ wonder-of-u …" header must
        // NOT appear — Claude Code fullscreen layout starts the transcript at row 0.
        let view = ShellView {
            title: "Session: MyProject".into(),
            messages: vec![MessageLineView::new(
                "● hello world",
                MessageRole::Assistant,
            )],
            prompt: "ask me".into(),
            footer: "▸▸ default (shift+tab to cycle)".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("▸ wonder-of-u"),
            "persistent header must not appear when messages are present; rendered:\n{text}"
        );
        // The transcript content should start at the very top row.
        let first_line = text.lines().next().unwrap_or("");
        assert!(
            first_line.contains("hello world"),
            "transcript must begin at row 0 when messages exist; rendered:\n{text}"
        );
    }

    #[test]
    fn session_title_header_present_on_welcome_screen() {
        // On the empty/welcome state the "▸ wonder-of-u …" header must appear.
        let view = ShellView {
            title: "Session: MyProject".into(),
            messages: Vec::new(),
            prompt: String::new(),
            footer: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("▸ wonder-of-u"),
            "welcome header must appear when there are no messages; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: compact footer (▸▸ … shift+tab …) ────────────

    #[test]
    fn compact_footer_uses_hint_text_not_verbose_metadata() {
        // The footer row must surface the compact hint supplied by ShellView::footer,
        // not verbose fields like `storage=` or `turn=` from the old status bar.
        let view = ShellView {
            messages: vec![MessageLineView::new("● hi", MessageRole::User)],
            prompt: String::new(),
            footer: "▸▸ default (shift+tab to cycle) · ⌃C exit".into(),
            status: "turn=idle storage=/home/user/.wonder cwd=/workspace".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("shift+tab to cycle"),
            "compact footer hint must appear; rendered:\n{text}"
        );
        // Verbose session metadata fields must stay out of the footer row.
        assert!(
            !text.contains("storage="),
            "verbose storage field must not appear in footer; rendered:\n{text}"
        );
        assert!(
            !text.contains("turn="),
            "verbose turn field must not appear in footer; rendered:\n{text}"
        );
    }

    #[test]
    fn compact_footer_scroll_badge_prepended_when_scrolled_up() {
        // When the transcript is scrolled away from the tail, the footer row
        // should include the scroll badge ("↑ N lines · Ctrl+End bottom").
        let view = ShellView {
            messages: (0..20)
                .map(|i| MessageLineView::new(format!("line {i}"), MessageRole::Assistant))
                .collect(),
            prompt: String::new(),
            footer: "▸▸ default".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 5,
                total_lines: 20,
                visible_lines: 10,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(80, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("↑ 5 lines"),
            "scroll badge must appear when scrolled up; rendered:\n{text}"
        );
        assert!(
            text.contains("Ctrl+End bottom"),
            "scroll badge must contain Ctrl+End hint; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: ● bullet rows for user/assistant ──────────────

    #[test]
    fn user_message_rendered_with_bullet_prefix() {
        // User messages must open with "● " so they match the Claude Code style.
        // This tests the render path that uses MessageRole::User colouring.
        let view = ShellView {
            messages: vec![MessageLineView::new("● fix the tests", MessageRole::User)],
            prompt: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 7, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("● fix the tests"),
            "user message bullet must appear in transcript; rendered:\n{text}"
        );
    }

    #[test]
    fn assistant_message_rendered_with_bullet_prefix() {
        // Assistant messages must also open with "● " (Claude Code style).
        let view = ShellView {
            messages: vec![MessageLineView::new(
                "● I've updated the file",
                MessageRole::Assistant,
            )],
            prompt: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 7, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("● I've updated the file"),
            "assistant message bullet must appear in transcript; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: tool rows ─────────────────────────────────────

    #[test]
    fn tool_row_headline_and_detail_rendered() {
        // Tool calls must produce a "● Tool(args)" headline row followed by an
        // indented "  └ detail" row — matching the Claude Code tool activity style.
        let view = ShellView {
            messages: vec![
                MessageLineView::new("● Bash(ls -la)", MessageRole::Tool),
                MessageLineView::new("  └ success", MessageRole::Tool),
            ],
            prompt: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("● Bash(ls -la)"),
            "tool headline must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("└ success"),
            "tool detail must appear indented; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: loading row position ───────────────────────────

    #[test]
    fn loading_row_appears_directly_above_prompt_not_in_transcript() {
        // The spinner row must be rendered between the last transcript line and
        // the prompt box top border — never mixed into the scrollable transcript.
        let view = ShellView {
            messages: vec![MessageLineView::new(
                "● earlier message",
                MessageRole::Assistant,
            )],
            prompt: String::new(),
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 1,
            loading_elapsed_secs: 5,
            loading_total_tokens: 0,
            footer: "▸▸ default (shift+tab to cycle) · ⌃C exit".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();
        let lines: Vec<&str> = text.lines().collect();

        // The loading indicator must appear above the rounded prompt.
        let spinner_row = lines
            .iter()
            .position(|l| l.contains("thinking"))
            .expect("spinner row must appear");
        let prompt_border_row = lines
            .iter()
            .position(|l| l.starts_with('╭'))
            .expect("prompt border must appear");

        assert!(
            spinner_row < prompt_border_row,
            "loading row ({spinner_row}) must be above prompt marker ({prompt_border_row}); rendered:\n{text}"
        );

        // The earlier transcript message must still be visible.
        assert!(
            text.contains("earlier message"),
            "transcript must still show earlier message during loading; rendered:\n{text}"
        );
    }

    // ── new integration-panel sections (tool / mcp / lsp / todo) ────────────

    /// `tool_lines` must produce a `─ Tools ─` header followed by its body on a
    /// wide terminal, and must be silently omitted when the slice is empty.
    #[test]
    fn sidebar_tools_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                tool_lines: vec!["✓ Bash".into(), "✓ FileRead".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Tools ─"),
            "Tools header must appear when tool_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("Bash"),
            "tool body line must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("FileRead"),
            "second tool body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `tool_lines` must not produce a `─ Tools ─` header.
    #[test]
    fn sidebar_tools_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Tools ─"),
            "Tools header must not appear when tool_lines is empty; rendered:\n{text}"
        );
    }

    /// `mcp_lines` must produce a `─ MCP ─` header followed by its body.
    #[test]
    fn sidebar_mcp_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                mcp_lines: vec!["✓ filesystem".into(), "⚠ github (reconnecting)".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ MCP ─"),
            "MCP header must appear when mcp_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("filesystem"),
            "MCP body line must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("github"),
            "second MCP body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `mcp_lines` must not produce a `─ MCP ─` header.
    #[test]
    fn sidebar_mcp_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ MCP ─"),
            "MCP header must not appear when mcp_lines is empty; rendered:\n{text}"
        );
    }

    /// `lsp_lines` must produce a `─ LSP ─` header followed by its body.
    #[test]
    fn sidebar_lsp_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                lsp_lines: vec!["✓ rust-analyzer".into(), "⚠ 3 errors".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ LSP ─"),
            "LSP header must appear when lsp_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("rust-analyzer"),
            "LSP body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `lsp_lines` must not produce a `─ LSP ─` header.
    #[test]
    fn sidebar_lsp_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ LSP ─"),
            "LSP header must not appear when lsp_lines is empty; rendered:\n{text}"
        );
    }

    /// `todo_lines` must produce a `─ Todo ─` header followed by its body.
    #[test]
    fn sidebar_todo_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                todo_lines: vec!["✓ Write tests".into(), "◈ Refactor module".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Todo ─"),
            "Todo header must appear when todo_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("Write tests"),
            "Todo body line must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("Refactor module"),
            "second Todo body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `todo_lines` must not produce a `─ Todo ─` header.
    #[test]
    fn sidebar_todo_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Todo ─"),
            "Todo header must not appear when todo_lines is empty; rendered:\n{text}"
        );
    }

    /// Verify the full render order: sections must appear in the documented
    /// sequence (Session → Status → Context → Tools → MCP → LSP → Todo →
    /// Suggestions → Providers → Workspace → Controls → Tasks → footer).
    ///
    /// Session is pinned in the top title band; Status through Tasks live in
    /// the scrollable middle band (Status first, so the live turn-state
    /// indicator is visible without scrolling); the branded footer is pinned
    /// to the bottom band.
    ///
    /// We populate every section and assert that each header appears *after* its
    /// predecessor in the rendered output, using byte-offset positions.
    #[test]
    fn sidebar_sections_render_in_opencode_panel_order() {
        let view = ShellView {
            prompt: "order-check".into(),
            sidebar: Some(SidebarView {
                session_lines: vec!["◈ abc12345".into()],
                context_lines: vec!["100 / 200,000 tokens".into()],
                tool_lines: vec!["✓ Bash".into()],
                mcp_lines: vec!["✓ filesystem".into()],
                lsp_lines: vec!["✓ rust-analyzer".into()],
                todo_lines: vec!["◈ Fix lint".into()],
                suggestions: vec![ContextSuggestion {
                    severity: SuggestionSeverity::Info,
                    title: "Consider /compact".into(),
                    detail: "Free up context".into(),
                }],
                provider_lines: vec!["◈ openai · gpt-4o".into()],
                workspace_lines: vec!["⎇  main".into()],
                status_lines: vec!["● idle".into()],
                control_lines: vec!["↵ send".into()],
                task_lines: vec!["⚙  1 task".into()],
                slots: Vec::new(),
                scroll_offset: 0,
                footer_brand: "wonder-of-u v0.1.3".into(),
            }),
            ..ShellView::default()
        };

        // Use a very tall terminal so all sections fit in the sidebar inner area.
        let frame = render_snapshot(120, 60, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Helper: position of first occurrence in rendered text.
        let pos = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("'{needle}' not found in sidebar output:\n{text}"))
        };

        // Assert the strict ordering of every section header.
        let session_pos = pos("─ Session ─");
        let status_pos = pos("─ Status ─");
        let context_pos = pos("─ Context ─");
        let tools_pos = pos("─ Tools ─");
        let mcp_pos = pos("─ MCP ─");
        let lsp_pos = pos("─ LSP ─");
        let todo_pos = pos("─ Todo ─");
        let suggestions_pos = pos("─ Suggestions ─");
        let providers_pos = pos("─ Providers ─");
        let workspace_pos = pos("─ Workspace ─");
        let controls_pos = pos("─ Controls ─");
        let tasks_pos = pos("─ Tasks ─");

        assert!(session_pos < status_pos, "Session must precede Status");
        assert!(status_pos < context_pos, "Status must precede Context");
        assert!(context_pos < tools_pos, "Context must precede Tools");
        assert!(tools_pos < mcp_pos, "Tools must precede MCP");
        assert!(mcp_pos < lsp_pos, "MCP must precede LSP");
        assert!(lsp_pos < todo_pos, "LSP must precede Todo");
        assert!(todo_pos < suggestions_pos, "Todo must precede Suggestions");
        assert!(
            suggestions_pos < providers_pos,
            "Suggestions must precede Providers"
        );
        assert!(
            providers_pos < workspace_pos,
            "Providers must precede Workspace"
        );
        assert!(
            workspace_pos < controls_pos,
            "Workspace must precede Controls"
        );
        assert!(controls_pos < tasks_pos, "Controls must precede Tasks");

        // The pinned branded footer must render after every scrollable section
        // (it lives in the fixed bottom band, drawn last).
        let footer_pos = pos("wonder-of-u v0.1.3");
        assert!(
            tasks_pos < footer_pos,
            "branded footer must render after Tasks (pinned bottom band)"
        );
    }

    /// Status is the first section after Session so the turn-state indicator is
    /// immediately visible without scrolling, even when other sections are empty.
    #[test]
    fn sidebar_status_renders_before_context() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                session_lines: vec!["◈ abc12345".into()],
                status_lines: vec!["⟳ streaming".into()],
                context_lines: vec!["500 / 128,000 tokens".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 20, &view, &Theme::default());
        let text = frame.to_plain_text();

        let session_pos = text.find("─ Session ─").expect("Session header not found");
        let status_pos = text.find("─ Status ─").expect("Status header not found");
        let context_pos = text.find("─ Context ─").expect("Context header not found");

        assert!(
            session_pos < status_pos,
            "Session must precede Status; rendered:\n{text}"
        );
        assert!(
            status_pos < context_pos,
            "Status must precede Context (Status is promoted to slot 2); rendered:\n{text}"
        );
        // The turn-state content must also be visible.
        assert!(
            text.contains("streaming"),
            "turn-state label must be visible; rendered:\n{text}"
        );
    }

    /// The three-zone layout must keep the Session title pinned to the top
    /// band and the branded footer pinned to the bottom row even when the
    /// scrollable middle band is scrolled away from its own top.
    #[test]
    fn sidebar_title_and_footer_stay_pinned_while_middle_band_scrolls() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                session_lines: vec!["◈ pinned-session".into()],
                // Enough body lines to overflow a short terminal's middle band.
                status_lines: vec!["● idle".into()],
                tool_lines: vec!["✓ Bash".into(), "✓ FileRead".into(), "✓ FileWrite".into()],
                mcp_lines: vec!["✓ filesystem".into(), "✓ github".into()],
                lsp_lines: vec!["✓ rust-analyzer".into()],
                todo_lines: vec!["◈ Fix lint".into(), "✓ Write tests".into()],
                workspace_lines: vec!["⎇  main".into()],
                control_lines: vec!["↵ send".into()],
                task_lines: vec!["⚙  2 tasks".into()],
                scroll_offset: 4,
                footer_brand: "wonder-of-u v0.1.3".into(),
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        // Short terminal so the populated body overflows the middle band,
        // forcing the scroll_offset above to take effect.
        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();
        let lines: Vec<&str> = text.lines().collect();

        // The pinned title band: the first non-blank row (row 0 is a 1-row
        // top margin reserved for the shell chrome) must still show Session,
        // regardless of the middle-band scroll offset.
        let first_content_row = lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .expect("frame must have at least one non-blank row");
        assert!(
            first_content_row.contains("─ Session ─"),
            "Session title must stay pinned to the top row when scrolled; rendered:\n{text}"
        );

        // The pinned branded footer: the very last row must carry the `●` dot
        // and version string.
        let last_row = lines.last().copied().unwrap_or("");
        assert!(
            last_row.contains('●') && last_row.contains("wonder-of-u v0.1.3"),
            "branded footer must stay pinned to the bottom row when scrolled; got: {last_row:?}\nrendered:\n{text}"
        );

        // Scrolling forward must have moved the visible window away from the
        // first middle-band section header (Status), proving the offset
        // actually windowed the middle band rather than being ignored.
        assert!(
            !text.contains("─ Status ─"),
            "Status header (first middle-band section) must scroll out of view; rendered:\n{text}"
        );
    }

    /// Dialogs must render a blank separator line between the body text and the
    /// action-hint row so the call-to-action has visual breathing room.
    #[test]
    fn dialog_has_blank_separator_before_action_row() {
        let view = ShellView {
            title: "Session: spacing".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::System)],
            prompt: "test".into(),
            dialog: Some(DialogView::notice("Info", ["This is a notice."])),
            ..ShellView::default()
        };

        let frame = render_snapshot(44, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        // The blank separator must separate the body text from the action row.
        // Actions are now rendered vertically with ▶ prefix for focused action.
        let body_pos = text.find("This is a notice").expect("body text not found");
        let action_pos = text.find("Close").expect("Close action not found");
        let between = &text[body_pos..action_pos];
        assert!(
            between.contains("│  ") || between.contains("│\n"),
            "a blank interior row must appear between body and action; rendered:\n{text}"
        );
    }

    /// The controller owns population of these fields each frame, so the model
    /// layer must not pre-fill them.
    #[test]
    fn from_app_state_initialises_new_panel_fields_as_empty() {
        let app = AppState::new(std::path::PathBuf::from("/workspace"));

        let view = ShellView::from_app_state(&app, "", false);
        let sidebar = view
            .sidebar
            .expect("sidebar must be Some from from_app_state");

        assert!(
            sidebar.tool_lines.is_empty(),
            "tool_lines must be empty from from_app_state; got: {:?}",
            sidebar.tool_lines
        );
        assert!(
            sidebar.mcp_lines.is_empty(),
            "mcp_lines must be empty from from_app_state; got: {:?}",
            sidebar.mcp_lines
        );
        assert!(
            sidebar.lsp_lines.is_empty(),
            "lsp_lines must be empty from from_app_state; got: {:?}",
            sidebar.lsp_lines
        );
        assert!(
            sidebar.todo_lines.is_empty(),
            "todo_lines must be empty from from_app_state; got: {:?}",
            sidebar.todo_lines
        );
    }

    /// All four new sections must be absent from a narrow terminal even when populated.
    #[test]
    fn new_sidebar_sections_absent_on_narrow_terminal() {
        let view = ShellView {
            prompt: "narrow".into(),
            sidebar: Some(SidebarView {
                tool_lines: vec!["✓ Bash".into()],
                mcp_lines: vec!["✓ filesystem".into()],
                lsp_lines: vec!["✓ rust-analyzer".into()],
                todo_lines: vec!["◈ Fix lint".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        // Width 119 is one below MIN_SIDEBAR_WIDTH=120 – sidebar is fully suppressed.
        let frame = render_snapshot(119, 20, &view, &Theme::default());
        let text = frame.to_plain_text();

        for absent in &["─ Tools ─", "─ MCP ─", "─ LSP ─", "─ Todo ─"] {
            assert!(
                !text.contains(absent),
                "{absent} must not appear on narrow terminal; rendered:\n{text}"
            );
        }
    }

    /// Each section header must appear exactly once even when all sections are
    /// populated.  A previous merge accidentally inserted duplicate render calls
    /// for Tools/MCP/LSP/Todo at the end of `sidebar_body_lines`; this test
    /// is the regression guard.  Also covers the Session header, which now
    /// lives in `sidebar_title_lines` (the pinned top band).
    #[test]
    fn sidebar_section_headers_appear_exactly_once() {
        let view = ShellView {
            prompt: "dup-check".into(),
            sidebar: Some(SidebarView {
                tool_lines: vec!["✓ Bash".into()],
                mcp_lines: vec!["✓ filesystem".into()],
                lsp_lines: vec!["✓ rust-analyzer".into()],
                todo_lines: vec!["◈ Fix lint".into()],
                session_lines: vec!["◈ abc12345".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 40, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Every integration-panel header must appear at most once.
        for header in &["─ Tools ─", "─ MCP ─", "─ LSP ─", "─ Todo ─", "─ Session ─"]
        {
            let count = text.matches(header).count();
            assert_eq!(
                count, 1,
                "'{header}' must appear exactly once; found {count} times in:\n{text}"
            );
        }
    }
}

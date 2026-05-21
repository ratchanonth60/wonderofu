//! Provides the top-level `keybindings` CLI command.

struct Section {
    title: &'static str,
    bindings: &'static [&'static str],
}

const SECTIONS: &[Section] = &[
    Section {
        title: "Global",
        bindings: &[
            "Ctrl+C / Ctrl+D: interrupt or quit the TUI",
            "Ctrl+L: redraw the terminal",
            "Ctrl+R: search prompt history",
            "Ctrl+F: open workspace search",
            "Ctrl+B: toggle the sidebar",
            "PageUp / PageDown: scroll the transcript",
            "Ctrl+Home / Ctrl+End: jump to the transcript top or bottom",
        ],
    },
    Section {
        title: "Prompt",
        bindings: &[
            "Enter: send message",
            "Shift+Enter: insert newline",
            "Tab: accept the selected slash-command suggestion",
            "Up / Down: move through slash-command suggestions",
            "Left / Right: move cursor",
            "Home / End or Ctrl+A / Ctrl+E: move to line start or end",
            "Backspace / Ctrl+H: delete backward",
            "Delete: delete forward",
        ],
    },
    Section {
        title: "Dialogs and pickers",
        bindings: &[
            "Up / Down: move the current selection",
            "Tab / Enter: confirm the current selection",
            "Esc: cancel or dismiss the active dialog",
            "Permission prompt: Enter or y allows, n or Esc denies",
        ],
    },
    Section {
        title: "Vim mode",
        bindings: &[
            "Esc: enter normal mode or cancel a pending operator",
            "h / j / k / l or arrows: move cursor",
            "w / b / e: move by word",
            "i / a / I / A: switch between insert and append actions",
            "x: delete the character under the cursor",
            "d{motion} / c{motion}: delete or change by motion",
            "Counts like 3w and 2dw are supported",
        ],
    },
];

#[must_use]
pub(crate) fn show() -> String {
    let mut lines = vec!["## Keybindings".to_string(), String::new()];
    for section in SECTIONS {
        lines.push(format!("### {}", section.title));
        lines.extend(
            section
                .bindings
                .iter()
                .map(|binding| format!("- {binding}")),
        );
        lines.push(String::new());
    }
    lines.pop();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::show;

    #[test]
    fn keybindings_output_contains_common_shortcuts() {
        let rendered = show();

        assert!(rendered.contains("## Keybindings"));
        assert!(rendered.contains("Ctrl+C / Ctrl+D"));
        assert!(rendered.contains("Enter: send message"));
        assert!(rendered.contains("Shift+Enter: insert newline"));
        assert!(rendered.contains("Ctrl+B: toggle the sidebar"));
        assert!(rendered.contains("d{motion} / c{motion}"));
    }
}

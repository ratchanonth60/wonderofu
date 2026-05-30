//! Contextual tips shown to the user during idle or spinner periods.
//!
//! Each [`Tip`] carries eligibility conditions so that platform-irrelevant or
//! context-irrelevant tips are filtered out before selection.

/// Maximum number of startup sessions after which the new-user warmup tips are
/// no longer shown.  Tips gated on [`TipCondition::NewUserWarmup`] are only
/// eligible when the caller-supplied `startup_count` is at or below this
/// threshold.
pub const NEW_USER_STARTUP_THRESHOLD: u64 = 5;

/// A condition that must be satisfied before a tip is shown.
///
/// Conditions are evaluated at selection time; unsatisfied conditions cause the
/// tip to be excluded from the candidate set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TipCondition {
    /// Always eligible.
    Always,
    /// Only eligible when `COLORTERM` is **not** set.
    ///
    /// The "use 256-colour terminal" tip is irrelevant when the user is already
    /// in a true-colour environment.
    ColorTermAbsent,
    /// Only eligible for new users (startup count ≤ [`NEW_USER_STARTUP_THRESHOLD`]).
    NewUserWarmup,
    /// Only eligible when at least one file has been opened in the current
    /// session (i.e. `file_history_len > 0`).
    FileHistoryPresent,
    /// Only eligible on macOS (`cfg!(target_os = "macos")`).
    #[cfg(target_os = "macos")]
    MacOs,
}

/// A single tip entry with its display text, eligibility conditions, and
/// cooldown policy.
#[derive(Clone, Debug)]
pub struct Tip {
    /// Stable identifier, unique across [`BUILTIN_TIPS`].
    pub id: &'static str,
    /// Human-readable tip text displayed in the TUI footer area.
    pub text: &'static str,
    /// Minimum number of sessions that must pass between consecutive shows.
    ///
    /// A value of `0` means the tip can be shown every session.
    pub cooldown_sessions: u64,
    /// Eligibility condition.  The tip is filtered out when the condition is
    /// not satisfied for the current session context.
    pub condition: TipCondition,
    /// When `true` this tip is mutually exclusive with all other
    /// file-history-class tips; if this tip is eligible, no other
    /// file-history tip should compete.
    pub file_history_exclusive: bool,
}

/// Built-in tip registry, listed in no particular priority order.
///
/// Selection is done by [`select_next_tip`], which picks the longest-waiting
/// eligible tip.
pub static BUILTIN_TIPS: &[Tip] = &[
    Tip {
        id: "shift-enter-multiline",
        text: "Tip: Use Shift+Enter to add a new line without sending.",
        cooldown_sessions: 3,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "ctrl-r-history",
        text: "Tip: Press Ctrl+R to search your command history.",
        cooldown_sessions: 5,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "slash-help",
        text: "Tip: Type /help to see all available commands.",
        cooldown_sessions: 3,
        condition: TipCondition::NewUserWarmup,
        file_history_exclusive: false,
    },
    Tip {
        id: "esc-cancel",
        text: "Tip: Press Escape to cancel the current operation.",
        cooldown_sessions: 3,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "ctrl-c-interrupt",
        text: "Tip: Press Ctrl+C to interrupt Claude mid-response.",
        cooldown_sessions: 5,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "at-file-attach",
        text: "Tip: Use @ to attach files or folders to your prompt.",
        cooldown_sessions: 3,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "compact-context",
        text: "Tip: Use /compact to compress long conversations and free up context.",
        cooldown_sessions: 4,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "vim-mode",
        text: "Tip: Enable /vim for vi-style prompt editing.",
        cooldown_sessions: 5,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "memory-md",
        text: "Tip: Add a MEMORY.md file to persist context across sessions.",
        cooldown_sessions: 6,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "tab-complete",
        text: "Tip: Press Tab to auto-complete file paths in the prompt.",
        cooldown_sessions: 4,
        condition: TipCondition::Always,
        file_history_exclusive: false,
    },
    Tip {
        id: "colorterm-256",
        text: "Tip: Set COLORTERM=truecolor in your shell for richer output.",
        cooldown_sessions: 7,
        condition: TipCondition::ColorTermAbsent,
        file_history_exclusive: false,
    },
    Tip {
        id: "file-history-search",
        text: "Tip: You can ask Claude to search recently opened files.",
        cooldown_sessions: 4,
        condition: TipCondition::FileHistoryPresent,
        file_history_exclusive: true,
    },
    #[cfg(target_os = "macos")]
    Tip {
        id: "paste-images-mac",
        text: "Tip: On macOS you can paste images directly into the prompt.",
        cooldown_sessions: 5,
        condition: TipCondition::MacOs,
        file_history_exclusive: false,
    },
];

/// Runtime context used to evaluate tip eligibility.
#[derive(Clone, Debug, Default)]
pub struct TipContext {
    /// Number of sessions the user has started (used for new-user warmup
    /// gating).
    pub startup_count: u64,
    /// Number of files opened in the current session (drives
    /// [`TipCondition::FileHistoryPresent`]).
    pub file_history_len: usize,
    /// Whether `COLORTERM` is present in the environment.
    pub colorterm_set: bool,
}

/// Returns `true` when `tip` is eligible given the provided `ctx` and
/// `sessions_since_shown`.
///
/// A tip is ineligible when:
/// - its condition is not satisfied by `ctx`, or
/// - it has been shown within its `cooldown_sessions` window.
#[must_use]
pub fn is_tip_eligible(tip: &Tip, ctx: &TipContext, sessions_since_shown: u64) -> bool {
    if sessions_since_shown < tip.cooldown_sessions {
        return false;
    }

    match &tip.condition {
        TipCondition::Always => true,
        TipCondition::ColorTermAbsent => !ctx.colorterm_set,
        TipCondition::NewUserWarmup => ctx.startup_count <= NEW_USER_STARTUP_THRESHOLD,
        TipCondition::FileHistoryPresent => ctx.file_history_len > 0,
        #[cfg(target_os = "macos")]
        TipCondition::MacOs => true,
    }
}

/// Selects the next tip to show from `candidates`, preferring the one with the
/// highest `sessions_since_shown` value.
///
/// Returns `None` when `candidates` is empty.
#[must_use]
pub fn select_next_tip<'a>(candidates: &[(&'a Tip, u64)]) -> Option<&'a Tip> {
    candidates
        .iter()
        .max_by_key(|(_, since)| *since)
        .map(|(tip, _)| *tip)
}

/// Renders a tip's text as a displayable string.
///
/// Currently returns the raw `text` field; this function exists as an
/// extension point for future formatting (e.g. applying ANSI colour codes or
/// wrapping for a fixed-width display).
#[must_use]
pub fn render_tip(tip: &Tip) -> String {
    tip.text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_contains_expected_tips() {
        // Verify core IDs are present.
        let ids: Vec<&str> = BUILTIN_TIPS.iter().map(|t| t.id).collect();
        assert!(ids.contains(&"shift-enter-multiline"));
        assert!(ids.contains(&"ctrl-c-interrupt"));
        assert!(ids.contains(&"vim-mode"));
        assert!(ids.contains(&"memory-md"));
        // All tips must have non-empty id and text.
        for tip in BUILTIN_TIPS {
            assert!(!tip.id.is_empty(), "tip id is empty");
            assert!(!tip.text.is_empty(), "tip text is empty for {}", tip.id);
        }
    }

    #[test]
    fn colorterm_tip_not_shown_when_colorterm_set() {
        let ctx = TipContext {
            colorterm_set: true,
            ..Default::default()
        };
        let colorterm_tip = BUILTIN_TIPS
            .iter()
            .find(|t| t.id == "colorterm-256")
            .expect("colorterm tip must exist");
        // Never eligible when COLORTERM is set, regardless of cooldown.
        assert!(!is_tip_eligible(colorterm_tip, &ctx, u64::MAX));
    }

    #[test]
    fn content_renderer_returns_non_empty_string() {
        for tip in BUILTIN_TIPS {
            let rendered = render_tip(tip);
            assert!(
                !rendered.is_empty(),
                "render_tip returned empty string for {}",
                tip.id
            );
        }
    }

    #[test]
    fn eligible_tips_respects_cooldown() {
        let tip = Tip {
            id: "test-cooldown",
            text: "Test cooldown tip.",
            cooldown_sessions: 3,
            condition: TipCondition::Always,
            file_history_exclusive: false,
        };
        let ctx = TipContext::default();
        // Not yet cooled down.
        assert!(!is_tip_eligible(&tip, &ctx, 0));
        assert!(!is_tip_eligible(&tip, &ctx, 2));
        // Exactly at threshold.
        assert!(is_tip_eligible(&tip, &ctx, 3));
        assert!(is_tip_eligible(&tip, &ctx, 10));
    }

    #[test]
    fn file_history_tips_are_exclusive() {
        let exclusive = BUILTIN_TIPS
            .iter()
            .filter(|t| t.file_history_exclusive)
            .collect::<Vec<_>>();
        // There is at least one file-history-exclusive tip.
        assert!(
            !exclusive.is_empty(),
            "expected at least one file_history_exclusive tip"
        );
        // Each exclusive tip requires FileHistoryPresent.
        for tip in &exclusive {
            let ctx_no_files = TipContext {
                file_history_len: 0,
                ..Default::default()
            };
            // Not eligible with no files, regardless of cooldown.
            assert!(
                !is_tip_eligible(tip, &ctx_no_files, u64::MAX),
                "exclusive tip {} should not show with no file history",
                tip.id
            );
            let ctx_has_files = TipContext {
                file_history_len: 1,
                ..Default::default()
            };
            assert!(
                is_tip_eligible(tip, &ctx_has_files, u64::MAX),
                "exclusive tip {} should show with file history",
                tip.id
            );
        }
    }

    #[test]
    fn new_user_warmup_only_relevant_for_few_startups() {
        let warmup_tip = BUILTIN_TIPS
            .iter()
            .find(|t| t.id == "slash-help")
            .expect("slash-help tip must exist");

        let ctx_new = TipContext {
            startup_count: 1,
            ..Default::default()
        };
        assert!(is_tip_eligible(warmup_tip, &ctx_new, u64::MAX));

        let ctx_at_threshold = TipContext {
            startup_count: NEW_USER_STARTUP_THRESHOLD,
            ..Default::default()
        };
        assert!(is_tip_eligible(warmup_tip, &ctx_at_threshold, u64::MAX));

        let ctx_veteran = TipContext {
            startup_count: NEW_USER_STARTUP_THRESHOLD + 1,
            ..Default::default()
        };
        assert!(!is_tip_eligible(warmup_tip, &ctx_veteran, u64::MAX));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn paste_images_mac_only_on_macos() {
        let mac_tip = BUILTIN_TIPS
            .iter()
            .find(|t| t.id == "paste-images-mac")
            .expect("paste-images-mac tip must exist on macOS");
        let ctx = TipContext::default();
        // On macOS this tip is always eligible (no other gate besides MacOs).
        assert!(is_tip_eligible(mac_tip, &ctx, u64::MAX));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn paste_images_mac_only_on_macos() {
        // On non-macOS platforms the tip should not be present in the registry.
        let mac_tip = BUILTIN_TIPS.iter().find(|t| t.id == "paste-images-mac");
        assert!(
            mac_tip.is_none(),
            "paste-images-mac tip must not be present on non-macOS"
        );
    }

    #[test]
    fn select_next_tip_picks_longest_waiting() {
        let tip_a = Tip {
            id: "a",
            text: "A",
            cooldown_sessions: 0,
            condition: TipCondition::Always,
            file_history_exclusive: false,
        };
        let tip_b = Tip {
            id: "b",
            text: "B",
            cooldown_sessions: 0,
            condition: TipCondition::Always,
            file_history_exclusive: false,
        };
        let tip_c = Tip {
            id: "c",
            text: "C",
            cooldown_sessions: 0,
            condition: TipCondition::Always,
            file_history_exclusive: false,
        };
        let candidates = vec![(&tip_a, 5u64), (&tip_b, 10u64), (&tip_c, 3u64)];
        let selected = select_next_tip(&candidates).expect("should select a tip");
        assert_eq!(selected.id, "b");
    }

    #[test]
    fn select_next_tip_returns_none_for_empty_slice() {
        let candidates: Vec<(&Tip, u64)> = vec![];
        assert!(select_next_tip(&candidates).is_none());
    }
}

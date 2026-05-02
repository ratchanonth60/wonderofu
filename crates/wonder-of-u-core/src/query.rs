use serde::{Deserialize, Serialize};

use crate::CoordinatorMode;

/// Query lifecycle stages shared by local prompt and tool orchestration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryPhase {
    /// Represents idle
    #[default]
    Idle,
    /// Represents running
    Running,
    /// Represents tool calling
    ToolCalling,
    /// Represents awaiting tool approval
    AwaitingToolApproval,
    /// Represents completed
    Completed,
    /// Represents failed
    Failed,
    /// Represents aborted
    Aborted,
}

impl QueryPhase {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::ToolCalling => "tool_calling",
            Self::AwaitingToolApproval => "awaiting_tool_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }
}
/// Represents query state
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryState {
    /// Stores the phase
    pub phase: QueryPhase,
    /// Stores the coordinator mode
    pub coordinator_mode: CoordinatorMode,
    /// Stores the prompt preview
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_preview: Option<String>,
    /// Stores the tool roundtrips
    #[serde(default)]
    pub tool_roundtrips: usize,
    /// Stores the tool calls
    #[serde(default)]
    pub tool_calls: usize,
    /// Stores the tools used
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_used: Vec<String>,
    /// Stores the pending tool name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_tool_name: Option<String>,
    /// Stores the failure reason
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

impl Default for QueryState {
    fn default() -> Self {
        Self {
            phase: QueryPhase::Idle,
            coordinator_mode: CoordinatorMode::Direct,
            prompt_preview: None,
            tool_roundtrips: 0,
            tool_calls: 0,
            tools_used: Vec::new(),
            pending_tool_name: None,
            failure_reason: None,
        }
    }
}

impl QueryState {
    /// Handles start
    #[must_use]
    pub fn start(prompt: &str, coordinator_mode: CoordinatorMode) -> Self {
        Self {
            phase: QueryPhase::Running,
            coordinator_mode,
            prompt_preview: prompt_preview(prompt),
            ..Self::default()
        }
    }

    /// Records tool batch
    pub fn record_tool_batch<I, S>(&mut self, tools: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let tool_names = tools.into_iter().map(Into::into).collect::<Vec<_>>();
        if tool_names.is_empty() {
            return;
        }
        self.phase = QueryPhase::ToolCalling;
        self.tool_roundtrips += 1;
        self.tool_calls += tool_names.len();
        self.pending_tool_name = tool_names.last().cloned();
        self.tools_used.extend(tool_names);
    }

    /// Handles await tool approval
    pub fn await_tool_approval(&mut self, tool_name: impl Into<String>) {
        self.phase = QueryPhase::AwaitingToolApproval;
        self.pending_tool_name = Some(tool_name.into());
    }

    /// Handles resume after tool approval
    pub fn resume_after_tool_approval(&mut self) {
        self.phase = QueryPhase::ToolCalling;
    }

    /// Handles complete
    pub fn complete(&mut self) {
        self.phase = QueryPhase::Completed;
        self.pending_tool_name = None;
        self.failure_reason = None;
    }

    /// Handles fail
    pub fn fail(&mut self, reason: impl Into<String>) {
        self.phase = QueryPhase::Failed;
        self.failure_reason = Some(reason.into());
    }

    /// Handles abort
    pub fn abort(&mut self) {
        self.phase = QueryPhase::Aborted;
        self.pending_tool_name = None;
    }
}

#[must_use]
fn prompt_preview(prompt: &str) -> Option<String> {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    const MAX_CHARS: usize = 80;
    if normalized.chars().count() <= MAX_CHARS {
        Some(normalized)
    } else {
        Some(
            normalized
                .chars()
                .take(MAX_CHARS.saturating_sub(1))
                .collect::<String>()
                + "…",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_state_tracks_tool_and_approval_transitions() {
        let mut state = QueryState::start("inspect Cargo metadata", CoordinatorMode::Local);
        assert_eq!(state.phase, QueryPhase::Running);
        assert_eq!(
            state.prompt_preview.as_deref(),
            Some("inspect Cargo metadata")
        );

        state.record_tool_batch(["glob", "file_read"]);
        assert_eq!(state.phase, QueryPhase::ToolCalling);
        assert_eq!(state.tool_roundtrips, 1);
        assert_eq!(state.tool_calls, 2);
        assert_eq!(state.pending_tool_name.as_deref(), Some("file_read"));

        state.await_tool_approval("file_write");
        assert_eq!(state.phase, QueryPhase::AwaitingToolApproval);
        assert_eq!(state.pending_tool_name.as_deref(), Some("file_write"));

        state.resume_after_tool_approval();
        assert_eq!(state.phase, QueryPhase::ToolCalling);

        state.complete();
        assert_eq!(state.phase, QueryPhase::Completed);
        assert_eq!(state.pending_tool_name, None);
        assert_eq!(state.failure_reason, None);
    }

    #[test]
    fn query_state_records_failures() {
        let mut state = QueryState::start("run the tests", CoordinatorMode::Direct);
        state.fail("tool loop exhausted");
        assert_eq!(state.phase, QueryPhase::Failed);
        assert_eq!(state.failure_reason.as_deref(), Some("tool loop exhausted"));
    }
}

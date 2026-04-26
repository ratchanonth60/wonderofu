use serde::{Deserialize, Serialize};

/// User-facing permission modes planned for command and tool execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Default,
    AcceptEdits,
    BypassPermissions,
    DontAsk,
    Plan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleBehavior {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleSource {
    Policy,
    Local,
    Project,
    User,
    CliArg,
    Command,
    SessionRuntime,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    pub behavior: PermissionRuleBehavior,
    pub source: PermissionRuleSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow { reason: String },
    Ask { reason: String },
    Deny { reason: String },
}

impl PermissionDecision {
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }
}

impl PermissionMode {
    /// Conservative defaults before rule precedence, hooks, and safety checks exist.
    #[must_use]
    pub fn default_decision(self, read_only: bool, destructive: bool) -> PermissionDecision {
        match self {
            Self::BypassPermissions => PermissionDecision::Allow {
                reason: "bypass permission mode".into(),
            },
            Self::DontAsk => PermissionDecision::Deny {
                reason: "dontAsk mode denies actions requiring confirmation".into(),
            },
            Self::Plan if !read_only => PermissionDecision::Deny {
                reason: "plan mode allows read-only actions only".into(),
            },
            Self::AcceptEdits if !destructive => PermissionDecision::Allow {
                reason: "acceptEdits mode allows non-destructive edits".into(),
            },
            _ if read_only => PermissionDecision::Allow {
                reason: "read-only action".into(),
            },
            _ => PermissionDecision::Ask {
                reason: "confirmation required by default permission mode".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_denies_writes() {
        let decision = PermissionMode::Plan.default_decision(false, false);
        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn bypass_mode_allows_destructive_actions() {
        let decision = PermissionMode::BypassPermissions.default_decision(false, true);
        assert!(decision.is_allowed());
    }
}

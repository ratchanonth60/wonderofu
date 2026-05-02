//! Source-compatible worktree tool schemas with explicit unsupported execution.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
};

use crate::{
    base_spec, parse_input, require_non_empty_path, require_non_empty_text, schema_with_aliases,
};

const WORKTREE_NAME_MAX_LEN: usize = 64;
const ENTER_WORKTREE_UNSUPPORTED: &str = "enter_worktree is not supported in wonder-of-u-tools because the Rust runtime does not yet expose session-scoped worktree creation or cwd switching; no git state was changed";
const EXIT_WORKTREE_UNSUPPORTED: &str = "exit_worktree is not supported in wonder-of-u-tools because the Rust runtime does not yet expose EnterWorktree session state or cwd restoration; no git state was changed";
/// Represents worktree session state
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeSessionState {
    /// Stores the original cwd
    pub original_cwd: PathBuf,
    /// Stores the repository root
    pub repository_root: PathBuf,
    /// Stores the worktree path
    pub worktree_path: PathBuf,
    /// Stores the worktree branch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    /// Stores the original head commit
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_head_commit: Option<String>,
    /// Stores the tmux session name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_session_name: Option<String>,
}

impl WorktreeSessionState {
    /// Validates the value
    pub fn validate(&self) -> Result<()> {
        require_non_empty_path("worktree_session_state", "original_cwd", &self.original_cwd)?;
        require_non_empty_path(
            "worktree_session_state",
            "repository_root",
            &self.repository_root,
        )?;
        require_non_empty_path(
            "worktree_session_state",
            "worktree_path",
            &self.worktree_path,
        )?;
        if self.original_cwd == self.worktree_path {
            return Err(WonderError::validation(
                "worktree_session_state requires `worktree_path` to differ from `original_cwd`",
            ));
        }
        if self.repository_root == self.worktree_path {
            return Err(WonderError::validation(
                "worktree_session_state requires `worktree_path` to differ from `repository_root`",
            ));
        }
        if let Some(branch) = &self.worktree_branch {
            require_non_empty_text("worktree_session_state", "worktree_branch", branch)?;
        }
        if let Some(commit) = &self.original_head_commit {
            require_non_empty_text("worktree_session_state", "original_head_commit", commit)?;
        }
        if let Some(tmux_session_name) = &self.tmux_session_name {
            require_non_empty_text(
                "worktree_session_state",
                "tmux_session_name",
                tmux_session_name,
            )?;
        }
        Ok(())
    }
}
/// Represents enter worktree input
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnterWorktreeInput {
    /// Stores the name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl EnterWorktreeInput {
    fn validate(&self) -> Result<()> {
        if let Some(name) = &self.name {
            validate_worktree_name(name)?;
        }
        Ok(())
    }
}
/// Enumerates exit worktree action
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitWorktreeAction {
    /// Represents keep
    Keep,
    /// Represents remove
    Remove,
}
/// Represents exit worktree input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitWorktreeInput {
    /// Stores the action
    pub action: ExitWorktreeAction,
    #[serde(
        default,
        alias = "discardChanges",
        skip_serializing_if = "Option::is_none"
    )]
    /// Stores the discard changes
    pub discard_changes: Option<bool>,
}

impl ExitWorktreeInput {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}
/// Represents enter worktree tool
#[derive(Debug, Default)]
pub struct EnterWorktreeTool;
/// Represents exit worktree tool
#[derive(Debug, Default)]
pub struct ExitWorktreeTool;

#[async_trait]
impl Tool for EnterWorktreeTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "enter_worktree",
            "Source-compatible EnterWorktree alias; runtime worktree switching is unsupported in wonder-of-u-tools",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object().property(
                "name",
                ToolSchema::string(
                    "optional worktree name; slash-separated segments may only use letters, digits, dots, underscores, and dashes (max 64 chars)",
                ),
            ),
        );
        spec.aliases.push("EnterWorktree".into());
        spec.destructive = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<EnterWorktreeInput>("enter_worktree", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        _use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<EnterWorktreeInput>("enter_worktree", &input)?;
        input.validate()?;
        Err(WonderError::validation(ENTER_WORKTREE_UNSUPPORTED))
    }
}

#[async_trait]
impl Tool for ExitWorktreeTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "exit_worktree",
            "Source-compatible ExitWorktree alias; runtime worktree session restoration is unsupported in wonder-of-u-tools",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "action",
                    ToolSchema::enumeration(
                        "whether to keep or remove the EnterWorktree session worktree",
                        ["keep", "remove"],
                    ),
                )
                .property(
                    "discard_changes",
                    schema_with_aliases(
                        ToolSchema::boolean(
                            "required true when action is \"remove\" and the user confirms discarding uncommitted files or commits",
                        ),
                        &["discardChanges"],
                    ),
                )
                .required("action"),
        );
        spec.aliases.push("ExitWorktree".into());
        spec.destructive = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<ExitWorktreeInput>("exit_worktree", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        _use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<ExitWorktreeInput>("exit_worktree", &input)?;
        input.validate()?;
        Err(WonderError::validation(EXIT_WORKTREE_UNSUPPORTED))
    }
}

fn validate_worktree_name(name: &str) -> Result<()> {
    require_non_empty_text("enter_worktree", "name", name)?;
    if name.len() > WORKTREE_NAME_MAX_LEN {
        return Err(WonderError::validation(format!(
            "enter_worktree name must be at most {WORKTREE_NAME_MAX_LEN} characters"
        )));
    }

    for segment in name.split('/') {
        if segment.is_empty() {
            return Err(WonderError::validation(
                "enter_worktree name must not contain empty `/` segments",
            ));
        }
        if !segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(WonderError::validation(
                "enter_worktree name may only contain letters, digits, dots, underscores, dashes, and `/` separators",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, process::Command};

    use futures::executor::block_on;
    use serde_json::{Value, json};
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, SessionId, ToolContext, ToolUseId,
    };
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;
    use crate::WorktreeListTool;

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
        }
    }

    fn has_property(spec: &ToolSpec, property: &str) -> bool {
        spec.input_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key(property))
    }

    fn property_aliases(spec: &ToolSpec, property: &str) -> Vec<String> {
        spec.input_schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(property))
            .and_then(|schema| schema.get("x-aliases"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn init_git_repo(prefix: &str) -> PathBuf {
        let dir = unique_test_dir(prefix);
        run_git(&dir, ["init"]);
        run_git(&dir, ["config", "user.name", "wonder-of-u"]);
        run_git(&dir, ["config", "user.email", "wonder-of-u@example.com"]);
        fs::write(dir.join("README.md"), "# repo\n").expect("write readme");
        run_git(&dir, ["add", "README.md"]);
        run_git(&dir, ["commit", "-m", "initial"]);
        dir
    }

    fn run_git<const N: usize>(cwd: &std::path::Path, args: [&str; N]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn worktree_list_spec_stays_read_only() {
        let spec = WorktreeListTool.spec();

        assert_eq!(spec.name, "worktree_list");
        assert!(spec.read_only);
        assert!(spec.concurrency_safe);
        assert!(!spec.destructive);
    }

    #[test]
    fn enter_worktree_spec_exposes_source_alias() {
        let spec = EnterWorktreeTool.spec();

        assert_eq!(spec.aliases, vec!["EnterWorktree".to_string()]);
        assert!(spec.destructive);
        assert!(has_property(&spec, "name"));
    }

    #[test]
    fn exit_worktree_spec_exposes_source_alias_and_schema() {
        let spec = ExitWorktreeTool.spec();

        assert_eq!(spec.aliases, vec!["ExitWorktree".to_string()]);
        assert!(spec.destructive);
        assert!(has_property(&spec, "action"));
        assert!(has_property(&spec, "discard_changes"));
        assert_eq!(
            property_aliases(&spec, "discard_changes"),
            vec!["discardChanges"]
        );
    }

    #[test]
    fn enter_worktree_validation_rejects_invalid_slug() {
        let tool = EnterWorktreeTool;
        let error = tool
            .validate_input(&json!({ "name": "feature bad" }))
            .expect_err("invalid worktree name");

        assert!(error.to_string().contains("letters, digits"));
    }

    #[test]
    fn exit_worktree_validation_rejects_unknown_action() {
        let tool = ExitWorktreeTool;
        let error = tool
            .validate_input(&json!({ "action": "archive" }))
            .expect_err("invalid action");

        assert!(error.to_string().contains("unknown variant"));
    }

    #[test]
    fn worktree_session_state_validation_rejects_non_isolated_paths() {
        let error = WorktreeSessionState {
            original_cwd: PathBuf::from("/workspace"),
            repository_root: PathBuf::from("/workspace"),
            worktree_path: PathBuf::from("/workspace"),
            worktree_branch: Some("topic".into()),
            original_head_commit: Some("abc1234".into()),
            tmux_session_name: None,
        }
        .validate()
        .expect_err("same worktree path");

        assert!(error.to_string().contains("worktree_path"));
    }

    #[test]
    fn worktree_enter_and_exit_require_permission_review_by_default() {
        let context = tool_context(PathBuf::from("/workspace"));

        let enter = EnterWorktreeTool.permission_decision(&context, &json!({ "name": "topic" }));
        let exit = ExitWorktreeTool.permission_decision(&context, &json!({ "action": "keep" }));

        assert!(matches!(enter, PermissionDecision::Ask { .. }));
        assert!(matches!(exit, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn enter_worktree_execution_is_explicitly_unsupported() {
        let dir = unique_test_dir("tools-enter-worktree");
        let error =
            block_on(EnterWorktreeTool.execute(tool_context(dir), ToolUseId::new(), json!({})))
                .expect_err("unsupported enter worktree");

        assert!(error.to_string().contains("not supported"));
        assert!(error.to_string().contains("no git state was changed"));
    }

    #[test]
    fn exit_worktree_execution_is_explicitly_unsupported() {
        let dir = unique_test_dir("tools-exit-worktree");
        let error = block_on(ExitWorktreeTool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "action": "remove", "discard_changes": true }),
        ))
        .expect_err("unsupported exit worktree");

        assert!(error.to_string().contains("not supported"));
        assert!(error.to_string().contains("no git state was changed"));
    }

    #[test]
    fn enter_worktree_does_not_mutate_git_state_when_unsupported() {
        let repo = init_git_repo("tools-enter-worktree-no-git-mutation");
        let before_worktrees = run_git(&repo, ["worktree", "list", "--porcelain"]);
        let before_branches = run_git(&repo, ["branch", "--list", "--format=%(refname:short)"]);

        let error = block_on(EnterWorktreeTool.execute(
            tool_context(repo.clone()),
            ToolUseId::new(),
            json!({ "name": "topic" }),
        ))
        .expect_err("unsupported enter worktree");

        let after_worktrees = run_git(&repo, ["worktree", "list", "--porcelain"]);
        let after_branches = run_git(&repo, ["branch", "--list", "--format=%(refname:short)"]);

        assert!(error.to_string().contains("no git state was changed"));
        assert_eq!(before_worktrees, after_worktrees);
        assert_eq!(before_branches, after_branches);
    }

    #[test]
    fn exit_worktree_does_not_mutate_git_state_when_unsupported() {
        let repo = init_git_repo("tools-exit-worktree-no-git-mutation");
        let before_worktrees = run_git(&repo, ["worktree", "list", "--porcelain"]);
        let before_branches = run_git(&repo, ["branch", "--list", "--format=%(refname:short)"]);

        let error = block_on(ExitWorktreeTool.execute(
            tool_context(repo.clone()),
            ToolUseId::new(),
            json!({ "action": "remove", "discard_changes": true }),
        ))
        .expect_err("unsupported exit worktree");

        let after_worktrees = run_git(&repo, ["worktree", "list", "--porcelain"]);
        let after_branches = run_git(&repo, ["branch", "--list", "--format=%(refname:short)"]);

        assert!(error.to_string().contains("no git state was changed"));
        assert_eq!(before_worktrees, after_worktrees);
        assert_eq!(before_branches, after_branches);
    }
}

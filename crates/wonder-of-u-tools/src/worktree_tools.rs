use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    Result, RuntimeWorktreeState, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec,
    ToolUseId, WonderError, get_current_branch, get_git_root,
};

use crate::{
    base_spec, parse_input, require_non_empty_path, require_non_empty_text, schema_with_aliases,
};

const WORKTREE_NAME_MAX_LEN: usize = 64;
const WORKTREE_RUNTIME_ACTION_KEY: &str = "worktree_runtime_action";

/// Represents persisted worktree session state.
pub type WorktreeSessionState = RuntimeWorktreeState;

/// Enumerates runtime actions that a controller can apply after a tool call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeRuntimeActionKind {
    /// Switches the session into a worktree.
    Enter,
    /// Restores the session to its original cwd.
    Exit,
}

/// Describes a session mutation that the runtime must apply explicitly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRuntimeAction {
    /// Stores the runtime action kind.
    pub action: WorktreeRuntimeActionKind,
    /// Stores the cwd the session should switch to.
    pub cwd: PathBuf,
    /// Stores the next session worktree state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_state: Option<WorktreeSessionState>,
}

impl WorktreeRuntimeAction {
    fn validate(&self) -> Result<()> {
        require_non_empty_path("worktree_runtime_action", "cwd", &self.cwd)?;
        if let Some(state) = &self.session_state {
            validate_worktree_session_state(state)?;
        }
        match self.action {
            WorktreeRuntimeActionKind::Enter if self.session_state.is_none() => Err(
                WonderError::validation("worktree_runtime_action enter requires `session_state`"),
            ),
            WorktreeRuntimeActionKind::Exit if self.session_state.is_some() => {
                Err(WonderError::validation(
                    "worktree_runtime_action exit must not include `session_state`",
                ))
            }
            _ => Ok(()),
        }
    }
}

/// Parses a structured worktree runtime action from tool-result metadata.
pub fn parse_worktree_runtime_action(metadata: &Value) -> Result<Option<WorktreeRuntimeAction>> {
    let Some(action) = metadata.get(WORKTREE_RUNTIME_ACTION_KEY) else {
        return Ok(None);
    };
    let action =
        serde_json::from_value::<WorktreeRuntimeAction>(action.clone()).map_err(|error| {
            WonderError::validation(format!(
                "invalid {WORKTREE_RUNTIME_ACTION_KEY} metadata: {error}"
            ))
        })?;
    action.validate()?;
    Ok(Some(action))
}

/// Validates worktree session state.
pub fn validate_worktree_session_state(state: &WorktreeSessionState) -> Result<()> {
    require_non_empty_path(
        "worktree_session_state",
        "original_cwd",
        &state.original_cwd,
    )?;
    require_non_empty_path(
        "worktree_session_state",
        "repository_root",
        &state.repository_root,
    )?;
    require_non_empty_path(
        "worktree_session_state",
        "worktree_path",
        &state.worktree_path,
    )?;
    if state.original_cwd == state.worktree_path {
        return Err(WonderError::validation(
            "worktree_session_state requires `worktree_path` to differ from `original_cwd`",
        ));
    }
    if state.repository_root == state.worktree_path {
        return Err(WonderError::validation(
            "worktree_session_state requires `worktree_path` to differ from `repository_root`",
        ));
    }
    if let Some(branch) = &state.worktree_branch {
        require_non_empty_text("worktree_session_state", "worktree_branch", branch)?;
    }
    if let Some(branch) = &state.original_branch {
        require_non_empty_text("worktree_session_state", "original_branch", branch)?;
    }
    if let Some(commit) = &state.original_head_commit {
        require_non_empty_text("worktree_session_state", "original_head_commit", commit)?;
    }
    if let Some(tmux_session_name) = &state.tmux_session_name {
        require_non_empty_text(
            "worktree_session_state",
            "tmux_session_name",
            tmux_session_name,
        )?;
    }
    Ok(())
}

/// Represents enter worktree input.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnterWorktreeInput {
    /// Stores the requested worktree name.
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

/// Enumerates exit worktree actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitWorktreeAction {
    /// Leaves the worktree on disk.
    Keep,
    /// Removes the worktree and branch.
    Remove,
}

/// Represents exit worktree input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitWorktreeInput {
    /// Stores the requested action.
    pub action: ExitWorktreeAction,
    /// Stores whether destructive removal was explicitly confirmed.
    #[serde(
        default,
        alias = "discardChanges",
        skip_serializing_if = "Option::is_none"
    )]
    pub discard_changes: Option<bool>,
}

impl ExitWorktreeInput {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

/// Represents enter worktree tool.
#[derive(Debug, Default)]
pub struct EnterWorktreeTool;

/// Represents exit worktree tool.
#[derive(Debug, Default)]
pub struct ExitWorktreeTool;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ChangeSummary {
    changed_files: usize,
    commits: usize,
}

#[async_trait]
impl Tool for EnterWorktreeTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "enter_worktree",
            "Create an isolated git worktree and request a session switch into it",
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
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<EnterWorktreeInput>("enter_worktree", &input)?;
        input.validate()?;

        if context.session_worktree.is_some() {
            return Ok(ToolResult::failure(
                use_id,
                "Already in an active EnterWorktree session. Exit the current worktree before creating another one.",
            ));
        }

        let repository_root = match get_git_root(&context.cwd) {
            Ok(root) => root,
            Err(_) => {
                return Ok(ToolResult::failure(
                    use_id,
                    "Cannot create a worktree outside a readable git repository. No git state was changed.",
                ));
            }
        };

        let slug = input
            .name
            .unwrap_or_else(|| default_worktree_slug(&context.session_id));
        let (session_state, resumed) =
            create_or_resume_worktree(&context.cwd, &repository_root, &slug)?;
        let action = WorktreeRuntimeAction {
            action: WorktreeRuntimeActionKind::Enter,
            cwd: session_state.worktree_path.clone(),
            session_state: Some(session_state.clone()),
        };
        let branch_info = session_state
            .worktree_branch
            .as_deref()
            .map_or(String::new(), |branch| format!(" on branch {branch}"));
        let verb = if resumed { "Resumed" } else { "Created" };

        Ok(ToolResult::success(
            use_id,
            format!(
                "{verb} worktree at {}{branch_info}. Apply the returned runtime action to switch the session into it.",
                session_state.worktree_path.display()
            ),
        )
        .with_metadata(worktree_runtime_metadata(&action)))
    }
}

#[async_trait]
impl Tool for ExitWorktreeTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "exit_worktree",
            "Exit an active EnterWorktree session and optionally remove the worktree",
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
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<ExitWorktreeInput>("exit_worktree", &input)?;
        input.validate()?;

        let Some(session_state) = context.session_worktree.clone() else {
            return Ok(ToolResult::failure(
                use_id,
                "No-op: there is no active EnterWorktree session to exit. No filesystem changes were made.",
            ));
        };
        validate_worktree_session_state(&session_state)?;

        let restore_cwd = choose_restore_cwd(&session_state);
        if input.action == ExitWorktreeAction::Keep {
            let action = WorktreeRuntimeAction {
                action: WorktreeRuntimeActionKind::Exit,
                cwd: restore_cwd.clone(),
                session_state: None,
            };
            return Ok(ToolResult::success(
                use_id,
                format!(
                    "Exited worktree. Your work is preserved at {}{}. Apply the returned runtime action to restore the session to {}.",
                    session_state.worktree_path.display(),
                    session_state.worktree_branch.as_deref().map_or(String::new(), |branch| {
                        format!(" on branch {branch}")
                    }),
                    restore_cwd.display()
                ),
            )
            .with_metadata(worktree_runtime_metadata(&action)));
        }

        let summary = count_worktree_changes(
            &session_state.worktree_path,
            session_state.original_head_commit.as_deref(),
        )?;
        if !input.discard_changes.unwrap_or(false) {
            let Some(summary) = summary else {
                return Ok(ToolResult::failure(
                    use_id,
                    format!(
                        "Could not verify worktree state at {}. Refusing to remove without explicit confirmation. Re-invoke with discard_changes: true to proceed, or use action: \"keep\".",
                        session_state.worktree_path.display()
                    ),
                ));
            };
            if summary.changed_files > 0 || summary.commits > 0 {
                return Ok(ToolResult::failure(
                    use_id,
                    dirty_remove_message(&session_state, summary),
                ));
            }
        }

        let summary = summary.unwrap_or_default();
        let remove_note = remove_worktree(
            &session_state,
            summary,
            input.discard_changes.unwrap_or(false),
        )?;
        let action = WorktreeRuntimeAction {
            action: WorktreeRuntimeActionKind::Exit,
            cwd: restore_cwd.clone(),
            session_state: None,
        };
        let discard_note = discarded_note(summary);
        Ok(ToolResult::success(
            use_id,
            format!(
                "Exited and removed worktree at {}.{discard_note}{} Apply the returned runtime action to restore the session to {}.",
                session_state.worktree_path.display(),
                remove_note
                    .as_deref()
                    .map_or(String::new(), |note| format!(" {note}")),
                restore_cwd.display()
            ),
        )
        .with_metadata(worktree_runtime_metadata(&action)))
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
        if matches!(segment, "" | "." | "..") {
            return Err(WonderError::validation(
                "enter_worktree name must not contain empty, `.` or `..` path segments",
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

fn default_worktree_slug(session_id: &wonder_of_u_core::SessionId) -> String {
    let raw = session_id.to_string();
    let suffix = raw.split('-').next().unwrap_or(raw.as_str());
    format!("session-{suffix}")
}

fn flatten_slug(slug: &str) -> String {
    slug.replace('/', "+")
}

fn worktree_branch_name(slug: &str) -> String {
    format!("worktree-{}", flatten_slug(slug))
}

fn repository_namespace(repository_root: &Path) -> String {
    let repo_name = repository_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository");
    let mut hasher = DefaultHasher::new();
    repository_root.hash(&mut hasher);
    format!("{repo_name}-{:016x}", hasher.finish())
}

fn worktrees_dir(repository_root: &Path) -> PathBuf {
    let namespace = repository_namespace(repository_root);
    repository_root.parent().map_or_else(
        || {
            repository_root
                .join(".wonder-of-u-worktrees")
                .join(&namespace)
        },
        |parent| parent.join(".wonder-of-u-worktrees").join(&namespace),
    )
}

fn worktree_path_for(repository_root: &Path, slug: &str) -> PathBuf {
    worktrees_dir(repository_root).join(flatten_slug(slug))
}

fn worktree_runtime_metadata(action: &WorktreeRuntimeAction) -> Value {
    json!({
        WORKTREE_RUNTIME_ACTION_KEY: action,
    })
}

fn choose_restore_cwd(state: &WorktreeSessionState) -> PathBuf {
    if state.original_cwd.is_dir() {
        state.original_cwd.clone()
    } else {
        state.repository_root.clone()
    }
}

fn create_or_resume_worktree(
    original_cwd: &Path,
    repository_root: &Path,
    slug: &str,
) -> Result<(WorktreeSessionState, bool)> {
    let worktree_path = worktree_path_for(repository_root, slug);
    let worktree_branch = worktree_branch_name(slug);
    let original_branch = get_current_branch(original_cwd).ok();
    let original_head_commit = git_stdout(repository_root, ["rev-parse", "HEAD"])?;

    fs::create_dir_all(worktrees_dir(repository_root))?;
    if worktree_path.exists() {
        if !registered_worktree_paths(repository_root)?
            .contains(&canonicalize_or_existing(&worktree_path)?)
        {
            return Err(WonderError::validation(format!(
                "refusing to reuse existing path {} because it is not a registered git worktree",
                worktree_path.display()
            )));
        }
        return Ok((
            WorktreeSessionState {
                original_cwd: original_cwd.to_path_buf(),
                repository_root: repository_root.to_path_buf(),
                worktree_path: canonicalize_or_existing(&worktree_path)?,
                worktree_branch: Some(worktree_branch),
                original_branch,
                original_head_commit: Some(original_head_commit),
                tmux_session_name: None,
            },
            true,
        ));
    }

    if git_branch_exists(repository_root, &worktree_branch)? {
        return Err(WonderError::validation(format!(
            "refusing to create worktree because branch `{worktree_branch}` already exists without a matching registered worktree"
        )));
    }

    git_ok(
        repository_root,
        [
            "worktree",
            "add",
            "-b",
            worktree_branch.as_str(),
            path_as_str(&worktree_path)?,
            "HEAD",
        ],
    )?;
    Ok((
        WorktreeSessionState {
            original_cwd: original_cwd.to_path_buf(),
            repository_root: repository_root.to_path_buf(),
            worktree_path: canonicalize_or_existing(&worktree_path)?,
            worktree_branch: Some(worktree_branch),
            original_branch,
            original_head_commit: Some(original_head_commit),
            tmux_session_name: None,
        },
        false,
    ))
}

fn count_worktree_changes(
    worktree_path: &Path,
    original_head_commit: Option<&str>,
) -> Result<Option<ChangeSummary>> {
    let status = git_output(worktree_path, ["status", "--porcelain"])?;
    if !status.status.success() {
        return Ok(None);
    }
    let changed_files = String::from_utf8_lossy(&status.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    let Some(original_head_commit) = original_head_commit else {
        return Ok(None);
    };
    let rev_list = git_output(
        worktree_path,
        [
            "rev-list",
            "--count",
            &format!("{original_head_commit}..HEAD"),
        ],
    )?;
    if !rev_list.status.success() {
        return Ok(None);
    }
    let commits = String::from_utf8(rev_list.stdout)
        .map_err(|error| {
            WonderError::validation(format!("git output was not valid UTF-8: {error}"))
        })?
        .trim()
        .parse::<usize>()
        .unwrap_or(0);
    Ok(Some(ChangeSummary {
        changed_files,
        commits,
    }))
}

fn dirty_remove_message(state: &WorktreeSessionState, summary: ChangeSummary) -> String {
    let mut parts = Vec::new();
    if summary.changed_files > 0 {
        parts.push(format!(
            "{} uncommitted {}",
            summary.changed_files,
            if summary.changed_files == 1 {
                "file"
            } else {
                "files"
            }
        ));
    }
    if summary.commits > 0 {
        parts.push(format!(
            "{} {} on {}",
            summary.commits,
            if summary.commits == 1 {
                "commit"
            } else {
                "commits"
            },
            state
                .worktree_branch
                .as_deref()
                .unwrap_or("the worktree branch")
        ));
    }
    format!(
        "Worktree has {}. Removing it will discard this work permanently. Confirm with the user, then re-invoke with discard_changes: true, or use action: \"keep\".",
        parts.join(" and ")
    )
}

fn discarded_note(summary: ChangeSummary) -> String {
    let mut parts = Vec::new();
    if summary.commits > 0 {
        parts.push(format!(
            "{} {}",
            summary.commits,
            if summary.commits == 1 {
                "commit"
            } else {
                "commits"
            }
        ));
    }
    if summary.changed_files > 0 {
        parts.push(format!(
            "{} uncommitted {}",
            summary.changed_files,
            if summary.changed_files == 1 {
                "file"
            } else {
                "files"
            }
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" Discarded {}.", parts.join(" and "))
    }
}

fn remove_worktree(
    state: &WorktreeSessionState,
    summary: ChangeSummary,
    discard_changes: bool,
) -> Result<Option<String>> {
    let remove_args = if discard_changes || summary.changed_files > 0 {
        vec![
            "worktree".to_string(),
            "remove".to_string(),
            "--force".to_string(),
            state.worktree_path.display().to_string(),
        ]
    } else {
        vec![
            "worktree".to_string(),
            "remove".to_string(),
            state.worktree_path.display().to_string(),
        ]
    };
    git_ok_owned(&state.repository_root, remove_args)?;

    if let Some(branch) = &state.worktree_branch {
        let delete_flag = if discard_changes || summary.commits > 0 {
            "-D"
        } else {
            "-d"
        };
        let delete_output = git_output(&state.repository_root, ["branch", delete_flag, branch])?;
        if !delete_output.status.success() {
            return Ok(Some(format!(
                "The worktree branch `{branch}` could not be deleted automatically: {}",
                String::from_utf8_lossy(&delete_output.stderr).trim()
            )));
        }
    }

    Ok(None)
}

fn registered_worktree_paths(repository_root: &Path) -> Result<Vec<PathBuf>> {
    let output = git_stdout(repository_root, ["worktree", "list", "--porcelain"])?;
    let mut paths = Vec::new();
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            paths.push(canonicalize_or_existing(Path::new(path))?);
        }
    }
    Ok(paths)
}

fn git_branch_exists(repository_root: &Path, branch: &str) -> Result<bool> {
    let output = git_output(repository_root, ["branch", "--list", branch])?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

fn git_stdout<S, I>(cwd: &Path, args: I) -> Result<String>
where
    S: AsRef<str>,
    I: IntoIterator<Item = S>,
{
    let output = git_output(cwd, args)?;
    if !output.status.success() {
        return Err(WonderError::validation(format!(
            "git command failed in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|stdout| stdout.trim().to_string())
        .map_err(|error| {
            WonderError::validation(format!("git output was not valid UTF-8: {error}"))
        })
}

fn git_ok<S, I>(cwd: &Path, args: I) -> Result<()>
where
    S: AsRef<str>,
    I: IntoIterator<Item = S>,
{
    let output = git_output(cwd, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(WonderError::validation(format!(
            "git command failed in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn git_ok_owned(cwd: &Path, args: Vec<String>) -> Result<()> {
    git_ok(cwd, args.iter().map(String::as_str))
}

fn git_output<S, I>(cwd: &Path, args: I) -> Result<std::process::Output>
where
    S: AsRef<str>,
    I: IntoIterator<Item = S>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect::<Vec<_>>();
    Command::new("git")
        .args(&args)
        .current_dir(cwd)
        .output()
        .map_err(WonderError::from)
}

fn path_as_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| WonderError::validation("worktree path must be valid UTF-8"))
}

fn canonicalize_or_existing(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).or_else(|_| {
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .map_err(WonderError::from)
        }
    })
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
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
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

    fn parse_action(result: &ToolResult) -> WorktreeRuntimeAction {
        parse_worktree_runtime_action(&result.metadata)
            .expect("parse metadata")
            .expect("runtime action metadata")
    }

    fn worktree_context(state: &WorktreeSessionState) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd: state.worktree_path.clone(),
            session_worktree: Some(state.clone()),
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
        }
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
        let error = validate_worktree_session_state(&WorktreeSessionState {
            original_cwd: PathBuf::from("/workspace"),
            repository_root: PathBuf::from("/workspace"),
            worktree_path: PathBuf::from("/workspace"),
            worktree_branch: Some("topic".into()),
            original_branch: Some("main".into()),
            original_head_commit: Some("abc1234".into()),
            tmux_session_name: None,
        })
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
    fn enter_worktree_creates_git_worktree_and_runtime_action() {
        let repo = init_git_repo("tools-enter-worktree-runtime");
        let result = block_on(EnterWorktreeTool.execute(
            tool_context(repo.clone()),
            ToolUseId::new(),
            json!({ "name": "topic/demo" }),
        ))
        .expect("enter worktree");

        assert!(result.success, "{}", result.content);
        let action = parse_action(&result);
        let state = action.session_state.expect("session state");
        assert_eq!(action.action, WorktreeRuntimeActionKind::Enter);
        assert_eq!(action.cwd, state.worktree_path);
        assert_eq!(state.original_cwd, repo);
        assert!(state.worktree_path.exists());
        assert_ne!(state.worktree_path, state.repository_root);
        assert!(
            run_git(&state.repository_root, ["worktree", "list", "--porcelain"])
                .contains(state.worktree_path.to_string_lossy().as_ref())
        );
        assert!(
            run_git(
                &state.repository_root,
                ["branch", "--list", "--format=%(refname:short)"]
            )
            .contains(state.worktree_branch.as_deref().expect("worktree branch"))
        );
    }

    #[test]
    fn exit_worktree_keep_returns_restore_action_without_git_mutation() {
        let repo = init_git_repo("tools-exit-worktree-keep");
        let enter = block_on(EnterWorktreeTool.execute(
            tool_context(repo.clone()),
            ToolUseId::new(),
            json!({ "name": "topic" }),
        ))
        .expect("enter worktree");
        let action = parse_action(&enter);
        let state = action.session_state.expect("session state");
        let before = run_git(&repo, ["worktree", "list", "--porcelain"]);

        let exit = block_on(ExitWorktreeTool.execute(
            worktree_context(&state),
            ToolUseId::new(),
            json!({ "action": "keep" }),
        ))
        .expect("exit keep");

        assert!(exit.success, "{}", exit.content);
        let exit_action = parse_action(&exit);
        assert_eq!(exit_action.action, WorktreeRuntimeActionKind::Exit);
        assert_eq!(exit_action.cwd, repo);
        assert!(exit_action.session_state.is_none());
        assert_eq!(before, run_git(&repo, ["worktree", "list", "--porcelain"]));
    }

    #[test]
    fn exit_worktree_remove_refuses_dirty_worktree_without_discard() {
        let repo = init_git_repo("tools-exit-worktree-dirty-guard");
        let enter = block_on(EnterWorktreeTool.execute(
            tool_context(repo.clone()),
            ToolUseId::new(),
            json!({ "name": "topic" }),
        ))
        .expect("enter worktree");
        let action = parse_action(&enter);
        let state = action.session_state.expect("session state");
        fs::write(state.worktree_path.join("dirty.txt"), "dirty\n").expect("write dirty file");
        let before = run_git(&repo, ["worktree", "list", "--porcelain"]);

        let exit = block_on(ExitWorktreeTool.execute(
            worktree_context(&state),
            ToolUseId::new(),
            json!({ "action": "remove" }),
        ))
        .expect("exit remove");

        assert!(!exit.success);
        assert!(exit.content.contains("discard"));
        assert_eq!(before, run_git(&repo, ["worktree", "list", "--porcelain"]));
        assert!(state.worktree_path.exists());
    }

    #[test]
    fn exit_worktree_remove_discards_confirmed_changes() {
        let repo = init_git_repo("tools-exit-worktree-remove");
        let enter = block_on(EnterWorktreeTool.execute(
            tool_context(repo.clone()),
            ToolUseId::new(),
            json!({ "name": "topic" }),
        ))
        .expect("enter worktree");
        let action = parse_action(&enter);
        let state = action.session_state.expect("session state");
        let branch = state.worktree_branch.clone().expect("worktree branch");
        fs::write(state.worktree_path.join("dirty.txt"), "dirty\n").expect("write dirty file");

        let exit = block_on(ExitWorktreeTool.execute(
            worktree_context(&state),
            ToolUseId::new(),
            json!({ "action": "remove", "discard_changes": true }),
        ))
        .expect("exit remove");

        assert!(exit.success, "{}", exit.content);
        let exit_action = parse_action(&exit);
        assert_eq!(exit_action.action, WorktreeRuntimeActionKind::Exit);
        assert!(!state.worktree_path.exists());
        assert!(
            !run_git(&repo, ["worktree", "list", "--porcelain"])
                .contains(state.worktree_path.to_string_lossy().as_ref())
        );
        assert!(
            !run_git(&repo, ["branch", "--list", "--format=%(refname:short)"]).contains(&branch)
        );
    }

    #[test]
    fn exit_worktree_without_active_session_does_not_mutate_git_state() {
        let repo = init_git_repo("tools-exit-worktree-no-session");
        let before_worktrees = run_git(&repo, ["worktree", "list", "--porcelain"]);
        let before_branches = run_git(&repo, ["branch", "--list", "--format=%(refname:short)"]);

        let result = block_on(ExitWorktreeTool.execute(
            tool_context(repo.clone()),
            ToolUseId::new(),
            json!({ "action": "remove", "discard_changes": true }),
        ))
        .expect("exit no session");

        assert!(!result.success);
        assert!(result.content.contains("No-op"));
        assert_eq!(
            before_worktrees,
            run_git(&repo, ["worktree", "list", "--porcelain"])
        );
        assert_eq!(
            before_branches,
            run_git(&repo, ["branch", "--list", "--format=%(refname:short)"])
        );
    }
}

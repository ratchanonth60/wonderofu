//! Implements git-workflow commands: `/review`, `/commit`, `/commit-push-pr`,
//! `/security-review`, and `/statusline`.

use std::path::Path;
use std::process::Command as ProcessCommand;

use async_trait::async_trait;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    PermissionMode, Result,
};

use super::workflow::{permission_mode_label, sanitize_single_line};
use super::{detect_git_branch, git_command_output};

// ── Command structs ───────────────────────────────────────────────────────────

/// Represents review command
pub struct ReviewCommand;

impl ReviewCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "review",
            "Queue a local pull-request review prompt",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

/// Represents commit command
pub struct CommitCommand;

impl CommitCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "commit",
            "Queue a local git commit prompt for pending workspace changes",
            CommandKind::Local,
        )
    }
}

/// Represents commit push pr command
pub struct CommitPushPrCommand;

impl CommitPushPrCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "commit-push-pr",
            "Queue a local commit, push, and pull-request prompt for the current branch",
            CommandKind::Local,
        )
    }
}

/// Represents security review command
pub struct SecurityReviewCommand;

impl SecurityReviewCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "security-review",
            "Queue a local security review prompt for pending branch changes",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

/// Represents statusline command
pub struct StatuslineCommand;

impl StatuslineCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "statusline",
            "Queue a status line setup prompt for the current session",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

// ── Command impls ─────────────────────────────────────────────────────────────

#[async_trait]
impl Command for ReviewCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_review_enqueue(
            &context.cwd,
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for CommitCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_commit_enqueue(
            &context.cwd,
            context.permission_mode,
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for CommitPushPrCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_commit_push_pr_enqueue(
            &context.cwd,
            context.permission_mode,
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for SecurityReviewCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_security_review_enqueue(
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for StatuslineCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_statusline_enqueue(
            invocation.args.trim(),
        )))
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Captured output from a subprocess invocation.
#[derive(Debug, Clone)]
struct CommandCapture {
    success: bool,
    status_code: Option<i32>,
    stdout: String,
}

/// Snapshot of git repo state needed for commit/push/pr prompts.
#[derive(Debug, Clone)]
struct CommitRepoState {
    git_root: String,
    current_branch: String,
    default_branch: Option<String>,
    status_short: String,
    recent_commits: String,
    repo_dirty: bool,
    branch_has_changes: bool,
    origin_remote: Option<String>,
    gh_available: bool,
    existing_pr: Option<String>,
}

// ── Render helpers ────────────────────────────────────────────────────────────

pub(super) fn render_review_enqueue(cwd: &Path, args: &str) -> String {
    let review_target = if args.trim().is_empty() {
        current_pr_number(cwd).unwrap_or_else(|| "list-open-prs".to_string())
    } else {
        sanitize_single_line(args)
    };
    let gh_available = command_available("gh");
    let current_branch = git_command_output(cwd, &["branch", "--show-current"]);
    let pr_url = if review_target == "list-open-prs" {
        None
    } else {
        gh_pr_url(cwd, &review_target)
    };
    let prompt = format!(
        concat!(
            "You are an expert code reviewer. ",
            "Use the detected GitHub PR metadata below. ",
            "If a PR number is available, run `gh pr view <number>` and `gh pr diff <number>`. ",
            "If no PR number is available, run `gh pr list` to show open PRs. ",
            "Then provide a concise but thorough review covering correctness, project conventions, performance, test coverage, and security considerations. ",
            "PR target: {}."
        ),
        review_target
    );
    let mut lines = vec![
        "review_prompt_ready=true".into(),
        format!("review_target={review_target}"),
        format!("gh_available={gh_available}"),
        format!(
            "current_branch={}",
            current_branch.unwrap_or_else(|| "unknown".into())
        ),
        "status=review prompt queued".into(),
        format!("enqueue_prompt={}", sanitize_single_line(&prompt)),
    ];
    if let Some(pr_url) = pr_url {
        lines.push(format!("pr_url={pr_url}"));
    }
    lines.join("\n")
}

pub(super) fn render_commit_enqueue(cwd: &Path, mode: PermissionMode, args: &str) -> String {
    let Some(state) = inspect_commit_repo_state(cwd) else {
        return [
            "commit_prompt_ready=false".into(),
            format!("permission_mode={}", permission_mode_label(mode)),
            "git_repository=false".into(),
            "status=commit prompt unavailable".into(),
            "note=current working directory is not a readable git worktree".into(),
        ]
        .join("\n");
    };

    let mut lines = commit_state_lines("commit", &state, mode);
    if let Some(note) = git_mutation_block_reason(mode) {
        lines.push("commit_prompt_ready=false".into());
        lines.push("git_mutations_allowed=false".into());
        lines.push("status=commit blocked by permission mode".into());
        lines.push(format!("note={note}"));
        return lines.join("\n");
    }
    if !state.repo_dirty {
        lines.push("commit_prompt_ready=false".into());
        lines.push("git_mutations_allowed=true".into());
        lines.push("status=no changes to commit".into());
        lines.push(
            "note=working tree is clean; the Rust port will not queue an empty commit".into(),
        );
        return lines.join("\n");
    }

    let prompt = build_commit_prompt(&state, args);
    lines.push("commit_prompt_ready=true".into());
    lines.push(format!(
        "git_mutations_require_confirmation={}",
        !matches!(mode, PermissionMode::BypassPermissions)
    ));
    lines.push("status=commit prompt queued".into());
    lines.push(
        "note=the queued prompt must use normal tool permissions for git add and git commit".into(),
    );
    lines.push(format!("enqueue_prompt={}", sanitize_single_line(&prompt)));
    lines.join("\n")
}

pub(super) fn render_commit_push_pr_enqueue(
    cwd: &Path,
    mode: PermissionMode,
    args: &str,
) -> String {
    let Some(state) = inspect_commit_repo_state(cwd) else {
        return [
            "commit_push_pr_prompt_ready=false".into(),
            format!("permission_mode={}", permission_mode_label(mode)),
            "git_repository=false".into(),
            "status=commit/push/pr prompt unavailable".into(),
            "note=current working directory is not a readable git worktree".into(),
        ]
        .join("\n");
    };

    let backend_supported = state.origin_remote.is_some() && state.gh_available;
    let mut lines = commit_state_lines("commit-push-pr", &state, mode);
    lines.push(format!("push_supported={}", state.origin_remote.is_some()));
    lines.push(format!("pr_backend={}", pr_backend_label(&state)));
    lines.push(format!("pr_creation_supported={backend_supported}"));

    if let Some(note) = git_mutation_block_reason(mode) {
        lines.push("commit_push_pr_prompt_ready=false".into());
        lines.push("git_mutations_allowed=false".into());
        lines.push("status=commit/push/pr blocked by permission mode".into());
        lines.push(format!("note={note}"));
        return lines.join("\n");
    }

    if !state.repo_dirty && !state.branch_has_changes {
        lines.push("commit_push_pr_prompt_ready=false".into());
        lines.push("git_mutations_allowed=true".into());
        lines.push("status=no local changes or branch diff to publish".into());
        lines.push("note=the working tree is clean and there is no diff against the detected default branch".into());
        return lines.join("\n");
    }

    let prompt = if backend_supported {
        build_commit_push_pr_prompt(&state, args)
    } else {
        build_commit_push_pr_deferred_prompt(&state, args)
    };
    lines.push("commit_push_pr_prompt_ready=true".into());
    lines.push(format!(
        "git_mutations_require_confirmation={}",
        !matches!(mode, PermissionMode::BypassPermissions)
    ));
    if backend_supported {
        lines.push("status=commit/push/pr prompt queued".into());
        lines.push(
            "note=the queued prompt must respect normal tool permissions for git and gh mutations"
                .into(),
        );
    } else {
        lines.push("status=validation-only prompt queued".into());
        lines.push("note=push and PR creation are deferred because the local environment lacks a supported origin remote or gh backend".into());
    }
    lines.push(format!("enqueue_prompt={}", sanitize_single_line(&prompt)));
    lines.join("\n")
}

pub(super) fn render_security_review_enqueue(args: &str) -> String {
    let focus = sanitize_single_line(args.trim());
    let prompt = format!(
        concat!(
            "Perform a security-focused review of the current local branch changes. ",
            "Inspect local git state with `git status --short`, `git diff --stat`, `git diff --cached`, and `git diff`. ",
            "Compare commits against the upstream branch with `git log --oneline @{{upstream}}..HEAD` when available, and fall back to recent local commits if no upstream is configured. ",
            "Report concrete findings first, then note residual risks and missing tests. ",
            "Focus on secrets, auth/authz, input validation, command execution, filesystem access, network exposure, dependency or configuration risk, and unsafe data handling. ",
            "If nothing looks wrong, explicitly say that no security issues were found in the inspected diff. ",
            "Additional focus: {}."
        ),
        if focus.is_empty() {
            "none provided"
        } else {
            focus.as_str()
        }
    );
    [
        "security_review_prompt_ready=true".into(),
        "security_review_scope=local_branch_changes".into(),
        format!(
            "security_review_focus={}",
            if focus.is_empty() { "default" } else { &focus }
        ),
        "status=security review prompt queued".into(),
        "note=the Rust port queues a local security review prompt; it does not integrate with marketplace or plugin security services".into(),
        format!("enqueue_prompt={}", sanitize_single_line(&prompt)),
    ]
    .join("\n")
}

pub(super) fn render_statusline_enqueue(args: &str) -> String {
    let prompt = if args.trim().is_empty() {
        "Configure my status line from my shell PS1 configuration".to_string()
    } else {
        args.trim().to_string()
    };
    [
        "statusline_prompt_ready=true".into(),
        "status=statusline setup prompt queued".into(),
        "note=the Rust port queues a local setup prompt instead of the leak's remote statusline subagent".into(),
        format!("enqueue_prompt={}", sanitize_single_line(&prompt)),
    ]
    .join("\n")
}

// ── Git helpers ───────────────────────────────────────────────────────────────

fn current_pr_number(cwd: &Path) -> Option<String> {
    if !command_available("gh") {
        return None;
    }
    let output = ProcessCommand::new("gh")
        .args(["pr", "view", "--json", "number", "--jq", ".number"])
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn gh_pr_url(cwd: &Path, number: &str) -> Option<String> {
    if !command_available("gh") {
        return None;
    }
    let output = ProcessCommand::new("gh")
        .args(["pr", "view", number, "--json", "url", "--jq", ".url"])
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn command_available(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|path| path.join(name).is_file())
}

fn capture_command(cwd: &Path, program: &str, args: &[&str]) -> Option<CommandCapture> {
    let output = ProcessCommand::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    Some(CommandCapture {
        success: output.status.success(),
        status_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
    })
}

fn capture_git(cwd: &Path, args: &[&str]) -> Option<CommandCapture> {
    capture_command(cwd, "git", args)
}

fn inspect_commit_repo_state(cwd: &Path) -> Option<CommitRepoState> {
    let git_root = capture_git(cwd, &["rev-parse", "--show-toplevel"])?;
    if !git_root.success {
        return None;
    }
    let git_root = (!git_root.stdout.is_empty()).then_some(git_root.stdout)?;
    let current_branch = detect_git_branch(cwd).unwrap_or_else(|| "detached-head".into());
    let default_branch = detect_default_branch(cwd);
    let status_short = capture_git(cwd, &["status", "--short", "--branch"])
        .filter(|capture| capture.success)
        .map(|capture| capture.stdout)
        .unwrap_or_else(|| "unavailable".into());
    let repo_dirty = capture_git(cwd, &["status", "--porcelain"])
        .filter(|capture| capture.success)
        .is_some_and(|capture| !capture.stdout.is_empty());
    let recent_commits = capture_git(cwd, &["log", "--oneline", "-10"])
        .filter(|capture| capture.success)
        .map(|capture| {
            if capture.stdout.is_empty() {
                "unavailable".into()
            } else {
                capture.stdout
            }
        })
        .unwrap_or_else(|| "unavailable".into());
    let branch_has_changes = default_branch
        .as_deref()
        .and_then(|branch| git_range_has_changes(cwd, &format!("{branch}...HEAD")))
        .unwrap_or(repo_dirty);
    let gh_available = command_available("gh");
    Some(CommitRepoState {
        git_root,
        current_branch,
        default_branch,
        status_short,
        recent_commits,
        repo_dirty,
        branch_has_changes,
        origin_remote: git_command_output(cwd, &["remote", "get-url", "origin"]),
        existing_pr: gh_available.then(|| current_pr_number(cwd)).flatten(),
        gh_available,
    })
}

fn git_range_has_changes(cwd: &Path, range: &str) -> Option<bool> {
    let capture = capture_git(cwd, &["diff", "--quiet", range])?;
    Some(match (capture.success, capture.status_code) {
        (true, _) => false,
        (false, Some(1)) => true,
        _ => return None,
    })
}

fn detect_default_branch(cwd: &Path) -> Option<String> {
    let origin_head = git_command_output(cwd, &["symbolic-ref", "refs/remotes/origin/HEAD"])
        .and_then(|value| {
            value
                .strip_prefix("refs/remotes/origin/")
                .map(str::to_string)
        });
    origin_head.or_else(|| {
        ["main", "master"]
            .into_iter()
            .find(|branch| {
                capture_git(
                    cwd,
                    &["show-ref", "--verify", &format!("refs/heads/{branch}")],
                )
                .is_some_and(|capture| capture.success)
            })
            .map(str::to_string)
    })
}

fn commit_state_lines(command: &str, state: &CommitRepoState, mode: PermissionMode) -> Vec<String> {
    vec![
        format!("command={command}"),
        format!("permission_mode={}", permission_mode_label(mode)),
        "git_repository=true".into(),
        format!("git_root={}", state.git_root),
        format!("current_branch={}", state.current_branch),
        format!(
            "default_branch={}",
            state.default_branch.as_deref().unwrap_or("unavailable")
        ),
        format!("repo_dirty={}", state.repo_dirty),
        format!("branch_has_changes={}", state.branch_has_changes),
        format!("gh_available={}", state.gh_available),
        format!(
            "origin_remote={}",
            state.origin_remote.as_deref().unwrap_or("unavailable")
        ),
        format!(
            "existing_pr={}",
            state.existing_pr.as_deref().unwrap_or("none")
        ),
        format!("status_short={}", sanitize_single_line(&state.status_short)),
        format!(
            "recent_commits={}",
            sanitize_single_line(&state.recent_commits)
        ),
    ]
}

fn build_commit_prompt(state: &CommitRepoState, args: &str) -> String {
    let mut prompt = format!(
        concat!(
            "Create a single git commit for the current repository. ",
            "First inspect `git status --short`, `git diff --cached`, `git diff`, and `git log --oneline -10` so the commit matches the pending changes and the repository's recent commit style. ",
            "Current branch: {}. Default branch hint: {}. Current status summary: {}. Recent commits: {}. ",
            "Git safety protocol: never change git config, never skip hooks, never use `git commit --amend`, never create an empty commit, never commit likely secrets, and never use interactive git flags. ",
            "Stage only the relevant files and create exactly one new commit with heredoc syntax: `git commit -m \"$(cat <<'EOF'\nCommit message here.\nEOF\n)\"`. ",
            "Use normal tool permissions for `git add` and `git commit`; do not bypass approval."
        ),
        state.current_branch,
        state.default_branch.as_deref().unwrap_or("unknown"),
        sanitize_single_line(&state.status_short),
        sanitize_single_line(&state.recent_commits),
    );
    append_additional_instructions(&mut prompt, args);
    prompt
}

fn build_commit_push_pr_prompt(state: &CommitRepoState, args: &str) -> String {
    let default_branch = state.default_branch.as_deref().unwrap_or("main");
    let branch_prefix = preferred_branch_prefix();
    let mut prompt = format!(
        concat!(
            "Prepare the current branch for review. ",
            "Inspect `git status --short`, `git diff --cached`, `git diff`, `git log --oneline -10`, and `git diff {}...HEAD` before making changes. ",
            "Current branch: {}. Origin remote: {}. Existing PR: {}. ",
            "If the current branch is `{}`, create a new branch with the prefix `{}/<short-topic>`. ",
            "If the working tree is dirty, stage only the relevant files and create exactly one new commit with heredoc syntax. ",
            "Then push the branch to origin. ",
            "If `gh pr view` reports an existing PR, update it with `gh pr edit`; otherwise create one with `gh pr create`. ",
            "Keep the PR title under 70 characters and put details in the body. ",
            "Git safety protocol: never change git config, never force push, never skip hooks, never use interactive git flags, and do not commit likely secrets. ",
            "Use normal tool permissions for git and gh mutations; do not bypass approval. ",
            "Return the PR URL or explain any remaining blockers."
        ),
        default_branch,
        state.current_branch,
        state.origin_remote.as_deref().unwrap_or("unknown"),
        state.existing_pr.as_deref().unwrap_or("none"),
        default_branch,
        branch_prefix,
    );
    append_additional_instructions(&mut prompt, args);
    prompt
}

fn build_commit_push_pr_deferred_prompt(state: &CommitRepoState, args: &str) -> String {
    let mut prompt = format!(
        concat!(
            "Validate the local branch for a future push/PR handoff. ",
            "Inspect `git status --short`, `git diff --cached`, `git diff`, `git log --oneline -10`, and `git diff {}...HEAD` when available. ",
            "Current branch: {}. Origin remote: {}. gh available: {}. ",
            "If the working tree is dirty, stage only the relevant files and create exactly one new commit with heredoc syntax. ",
            "Do not push and do not attempt PR creation in this run because the local environment does not provide the required remote or gh PR backend. ",
            "Instead, summarize what was validated locally and explicitly call out that push/PR steps remain deferred. ",
            "Git safety protocol: never change git config, never skip hooks, never use `git commit --amend`, never create an empty commit, and do not commit likely secrets. ",
            "Use normal tool permissions for any git mutations."
        ),
        state.default_branch.as_deref().unwrap_or("HEAD"),
        state.current_branch,
        state.origin_remote.as_deref().unwrap_or("unavailable"),
        state.gh_available,
    );
    append_additional_instructions(&mut prompt, args);
    prompt
}

fn append_additional_instructions(prompt: &mut String, args: &str) {
    let trimmed = args.trim();
    if !trimmed.is_empty() {
        prompt.push_str(" Additional user instructions: ");
        prompt.push_str(trimmed);
    }
}

fn preferred_branch_prefix() -> String {
    std::env::var("SAFEUSER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("USER")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "user".into())
}

fn git_mutation_block_reason(mode: PermissionMode) -> Option<&'static str> {
    match mode {
        PermissionMode::Plan => Some(
            "plan mode is read-only for destructive shell mutations, so commit/push actions stay deferred",
        ),
        PermissionMode::DontAsk => Some(
            "dont-ask mode would deny git mutations, so the Rust port does not queue commit or push prompts",
        ),
        PermissionMode::Default
        | PermissionMode::AcceptEdits
        | PermissionMode::BypassPermissions => None,
    }
}

fn pr_backend_label(state: &CommitRepoState) -> &'static str {
    match (state.origin_remote.is_some(), state.gh_available) {
        (true, true) => "github_cli",
        _ => "unsupported",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{fs, path::Path, process::Command as ProcessCommand};

    use futures::executor::block_on;
    use wonder_of_u_core::{
        Command, CommandContext, CommandInvocation, CommandOutput, FeatureSet, PermissionMode,
        SessionId,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::{
        CommitCommand, CommitPushPrCommand, SecurityReviewCommand, render_commit_enqueue,
        render_commit_push_pr_enqueue, render_review_enqueue, render_security_review_enqueue,
        render_statusline_enqueue,
    };

    fn test_context(cwd: &Path) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn commit_prompt_reports_clean_repo_without_enqueueing() {
        let dir = unique_test_dir("workflow-commit-clean");
        init_git_repo(&dir);

        let rendered = render_commit_enqueue(&dir, PermissionMode::Default, "");

        assert!(rendered.contains("commit_prompt_ready=false"));
        assert!(rendered.contains("repo_dirty=false"));
        assert!(rendered.contains("status=no changes to commit"));
        assert!(!rendered.contains("enqueue_prompt="));
    }

    #[test]
    fn commit_command_queues_prompt_for_dirty_repo() {
        let dir = unique_test_dir("workflow-commit-dirty");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");

        let output = block_on(CommitCommand::new().execute(
            test_context(&dir),
            CommandInvocation {
                name: "commit".into(),
                args: "mention the README update".into(),
                raw: "/commit mention the README update".into(),
            },
        ))
        .expect("commit output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("commit_prompt_ready=true"));
        assert!(text.contains("repo_dirty=true"));
        assert!(text.contains("git_mutations_require_confirmation=true"));
        assert!(text.contains("enqueue_prompt=Create a single git commit"));
        assert!(text.contains("Additional user instructions: mention the README update"));
    }

    #[test]
    fn commit_prompt_blocks_git_mutations_in_plan_mode() {
        let dir = unique_test_dir("workflow-commit-plan");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");

        let rendered = render_commit_enqueue(&dir, PermissionMode::Plan, "");

        assert!(rendered.contains("commit_prompt_ready=false"));
        assert!(rendered.contains("git_mutations_allowed=false"));
        assert!(rendered.contains("status=commit blocked by permission mode"));
        assert!(!rendered.contains("enqueue_prompt="));
    }

    #[test]
    fn commit_push_pr_reports_unsupported_backend_when_gh_is_missing() {
        let dir = unique_test_dir("workflow-commit-push-pr-unsupported");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");
        ProcessCommand::new("git")
            .args(["remote", "add", "origin", "https://example.com/demo.git"])
            .current_dir(&dir)
            .status()
            .expect("git remote add");
        let git_path = find_git_binary();
        let bin_dir = dir.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        write_git_proxy(&bin_dir, &git_path);
        let _path = EnvVarGuard::set("PATH", bin_dir.to_string_lossy().into_owned());

        let rendered = render_commit_push_pr_enqueue(&dir, PermissionMode::Default, "");

        assert!(rendered.contains("commit_push_pr_prompt_ready=true"));
        assert!(rendered.contains("pr_backend=unsupported"));
        assert!(rendered.contains("pr_creation_supported=false"));
        assert!(rendered.contains("status=validation-only prompt queued"));
        assert!(rendered.contains("Do not push and do not attempt PR creation"));
    }

    #[test]
    fn commit_push_pr_command_enqueues_prompt_with_user_args() {
        let dir = unique_test_dir("workflow-commit-push-pr-command");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");

        let output = block_on(CommitPushPrCommand::new().execute(
            test_context(&dir),
            CommandInvocation {
                name: "commit-push-pr".into(),
                args: "call out the README change".into(),
                raw: "/commit-push-pr call out the README change".into(),
            },
        ))
        .expect("commit push pr output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("commit_push_pr_prompt_ready=true"));
        assert!(text.contains("enqueue_prompt="));
        assert!(text.contains("Additional user instructions: call out the README change"));
    }

    #[test]
    fn review_enqueue_defaults_to_listing_open_prs() {
        let rendered = render_review_enqueue(Path::new("/tmp"), "");

        assert!(rendered.contains("review_prompt_ready=true"));
        assert!(rendered.contains("review_target="));
        assert!(rendered.contains("gh_available="));
        assert!(rendered.contains("status=review prompt queued"));
        assert!(rendered.contains("enqueue_prompt=You are an expert code reviewer."));
    }

    #[test]
    fn review_enqueue_carries_requested_pr_number() {
        let rendered = render_review_enqueue(Path::new("/tmp"), "123");

        assert!(rendered.contains("review_target=123"));
        assert!(rendered.contains("PR target: 123."));
    }

    #[test]
    fn security_review_enqueue_defaults_to_local_branch_scope() {
        let rendered = render_security_review_enqueue("");

        assert!(rendered.contains("security_review_prompt_ready=true"));
        assert!(rendered.contains("security_review_scope=local_branch_changes"));
        assert!(rendered.contains("security_review_focus=default"));
        assert!(rendered.contains("status=security review prompt queued"));
        assert!(rendered.contains("git status --short"));
    }

    #[test]
    fn security_review_command_enqueues_custom_focus() {
        let dir = unique_test_dir("workflow-security-review-command");
        let output = block_on(SecurityReviewCommand::new().execute(
            test_context(&dir),
            CommandInvocation {
                name: "security-review".into(),
                args: "focus on secret handling".into(),
                raw: "/security-review focus on secret handling".into(),
            },
        ))
        .expect("security review output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("security_review_focus=focus on secret handling"));
        assert!(text.contains("Additional focus: focus on secret handling."));
    }

    #[test]
    fn statusline_enqueue_uses_default_setup_prompt() {
        let rendered = render_statusline_enqueue("");

        assert!(rendered.contains("statusline_prompt_ready=true"));
        assert!(rendered.contains("status=statusline setup prompt queued"));
        assert!(
            rendered.contains(
                "enqueue_prompt=Configure my status line from my shell PS1 configuration"
            )
        );
    }

    #[test]
    fn statusline_enqueue_preserves_custom_prompt() {
        let rendered = render_statusline_enqueue("Match my tmux and starship layout");

        assert!(rendered.contains("enqueue_prompt=Match my tmux and starship layout"));
    }

    fn init_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("create repo dir");
        ProcessCommand::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .current_dir(path)
            .status()
            .expect("git init");
        ProcessCommand::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .status()
            .expect("git config user.name");
        ProcessCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .status()
            .expect("git config user.email");
        fs::write(path.join("README.md"), "seed\n").expect("write seed");
        ProcessCommand::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .status()
            .expect("git add seed");
        ProcessCommand::new("git")
            .args(["commit", "-m", "seed"])
            .current_dir(path)
            .status()
            .expect("git commit seed");
    }

    fn find_git_binary() -> String {
        let output = ProcessCommand::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .expect("locate git");
        String::from_utf8(output.stdout)
            .expect("git path utf8")
            .trim()
            .to_string()
    }

    #[cfg(unix)]
    fn write_git_proxy(dir: &Path, git_path: &str) {
        let script_path = dir.join("git");
        fs::write(
            &script_path,
            format!(
                "#!/bin/sh\nexec '{}' \"$@\"\n",
                git_path.replace('\'', "'\"'\"'")
            ),
        )
        .expect("write git proxy");
        let mut permissions = fs::metadata(&script_path)
            .expect("git proxy metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("chmod git proxy");
    }
}

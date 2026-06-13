use std::{
    env,
    fs::{self},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wonder_of_u_agent::SettingsStore;
use wonder_of_u_core::{
    AppState, AuthMaterialKind, AuthState, Command, CommandContext, CommandInvocation, CommandKind,
    CommandOutput, CommandSpec, Result, WonderError, app::ThinkingEffort, permission_mode_label,
};
use wonder_of_u_storage::StoragePaths;
/// Represents btw command
pub struct BtwCommand;

impl BtwCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "btw",
            "Ask a quick side question without interrupting the main conversation",
            CommandKind::Local,
        )
    }
}

/// Represents advisor command
pub struct AdvisorCommand {
    storage_dir: Option<PathBuf>,
}

impl AdvisorCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "advisor",
            "Configure the advisor model for multi-model reasoning",
            CommandKind::Local,
        )
    }
}
/// Represents sandbox toggle command
pub struct SandboxToggleCommand {
    storage_dir: Option<PathBuf>,
}

impl SandboxToggleCommand {
    /// Creates a command backed by the configured storage directory.
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "sandbox",
            "Toggle sandbox isolation for bash command execution",
            CommandKind::Local,
        );
        spec.aliases.push("sandbox-toggle".into());
        spec
    }
}
/// Represents autofix pr command
pub struct AutofixPrCommand;

impl AutofixPrCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "autofix-pr",
            "Automatically fix issues in a GitHub pull request",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents pr comments command
pub struct PrCommentsCommand;

impl PrCommentsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "pr-comments",
            "Fetch and display comments from a GitHub pull request",
            CommandKind::NonInteractive,
        );
        spec.aliases.push("pr_comments".into());
        spec
    }
}

/// Represents env command
pub struct EnvCommand;

impl EnvCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "env",
            "Show a redacted environment summary",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}
/// Represents create moved to plugin command
pub struct CreateMovedToPluginCommand;

impl CreateMovedToPluginCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "plugin-migrate",
            "Show plugin migration guidance for moved commands",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct ExtrasConfig {
    #[serde(default)]
    ant_trace_enabled: bool,
    #[serde(default)]
    mock_limits_enabled: bool,
    #[serde(default)]
    mock_limit_hits: u64,
    #[serde(default)]
    bughunter_enabled: bool,
    #[serde(default)]
    sandbox_enabled: bool,
    #[serde(default)]
    ultraplan_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra_usage_quota_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteEnvConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct RemoteEnvConfig {
    host: String,
    port: u16,
    auth: String,
}
#[async_trait]
impl Command for BtwCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = invocation.args.trim();
        if args.is_empty() {
            return Ok(CommandOutput::Text(random_btw_tip().into()));
        }
        Ok(CommandOutput::Text(format!("By the way: {args}")))
    }
}

#[async_trait]
impl Command for AdvisorCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = invocation.args.trim();
        if args.is_empty() {
            return Ok(CommandOutput::Text(current_advisor_setting(
                self.storage_dir.as_deref(),
            )));
        }
        if matches!(args, "unset" | "off") {
            return Ok(CommandOutput::Text(
                "Advisor recommendation disabled for this session.".into(),
            ));
        }
        Ok(CommandOutput::Text(recommend_advisor(args)))
    }
}
#[async_trait]
impl Command for SandboxToggleCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let storage_dir = self.storage_dir.clone().or_else(|| storage_root(None));
        Ok(CommandOutput::Text(toggle_sandbox(
            storage_dir.as_deref(),
            invocation.args.trim(),
        )?))
    }
}
#[async_trait]
impl Command for AutofixPrCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(autofix_pr(invocation.args.trim())))
    }
}

#[async_trait]
impl Command for PrCommentsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = invocation.args.trim();
        if args.is_empty() || !executable_on_path("gh") {
            return Ok(CommandOutput::Text(pr_comments_usage(!executable_on_path(
                "gh",
            ))));
        }
        Ok(CommandOutput::Text(fetch_pr_comments(args)))
    }
}

#[async_trait]
impl Command for EnvCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_env_summary(&context)))
    }
}
#[async_trait]
impl Command for CreateMovedToPluginCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "This command has moved to a plugin. Run /plugin list to see available plugins.".into(),
        ))
    }
}
fn current_advisor_setting(storage_dir: Option<&Path>) -> String {
    if let Ok(model) = env::var("WONDER_OF_U_ADVISOR_MODEL").or_else(|_| env::var("ADVISOR_MODEL"))
    {
        return format!("Advisor: {model}");
    }
    if let Some(storage_dir) = storage_dir {
        if let Ok(settings) = SettingsStore::new(storage_dir).read() {
            if let Some(model) = settings.selected_model {
                return format!("Advisor: unset (current primary model: {model})");
            }
        }
    }
    "Advisor: unset".into()
}

fn recommend_advisor(task: &str) -> String {
    let lower = task.to_ascii_lowercase();
    let recommendation = if lower.contains("debug")
        || lower.contains("fix")
        || lower.contains("bug")
        || lower.contains("investigate")
    {
        "claude-3.7-sonnet"
    } else if lower.contains("plan")
        || lower.contains("architecture")
        || lower.contains("refactor")
        || lower.contains("design")
    {
        "claude-3.7-opus"
    } else if lower.contains("test")
        || lower.contains("lint")
        || lower.contains("review")
        || lower.contains("comment")
    {
        "claude-3.5-haiku"
    } else {
        "claude-3.7-sonnet"
    };
    format!("Advisor recommendation: {recommendation}\nreason={task}")
}
fn random_btw_tip() -> &'static str {
    const TIPS: &[&str] = &[
        "BTW: use /compact before long refactors to keep context tight.",
        "BTW: `/pr-comments <number>` is handy before addressing review feedback.",
        "BTW: `/summary` gives a quick health check of the current session.",
        "BTW: capture checkpoints early if you expect to use /rewind later.",
    ];
    let index = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as usize % TIPS.len())
        .unwrap_or(0);
    TIPS[index]
}
fn toggle_sandbox(storage_dir: Option<&Path>, args: &str) -> Result<String> {
    let mut config = read_extras_config(storage_dir)?;
    if let Some(enabled) = parse_toggle_request(args) {
        config.sandbox_enabled = enabled;
        write_extras_config(storage_dir, &config)?;
    } else if args.is_empty() {
        config.sandbox_enabled = !config.sandbox_enabled;
        write_extras_config(storage_dir, &config)?;
    }
    Ok(format!(
        "Sandbox {}\nplatform={}",
        if config.sandbox_enabled {
            "enabled"
        } else {
            "disabled"
        },
        env::consts::OS
    ))
}
fn storage_root(explicit: Option<&Path>) -> Option<PathBuf> {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| env::var_os("WONDER_OF_U_STORAGE_DIR").map(PathBuf::from))
        .or_else(|| {
            env::var_os("XDG_CONFIG_HOME").map(|path| PathBuf::from(path).join("wonder-of-u"))
        })
        .or_else(|| env::var_os("HOME").map(|path| PathBuf::from(path).join(".config/wonder-of-u")))
}

fn extras_config_path(storage_dir: &Path) -> PathBuf {
    StoragePaths::new(storage_dir)
        .config_dir()
        .join("extras-config.json")
}

fn read_extras_config(storage_dir: Option<&Path>) -> Result<ExtrasConfig> {
    let Some(storage_dir) = storage_dir else {
        return Ok(ExtrasConfig::default());
    };
    let path = extras_config_path(storage_dir);
    if !path.exists() {
        return Ok(ExtrasConfig::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn write_extras_config(storage_dir: Option<&Path>, config: &ExtrasConfig) -> Result<()> {
    let Some(storage_dir) = storage_dir else {
        return Ok(());
    };
    let path = extras_config_path(storage_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(config)?)?;
    Ok(())
}

fn parse_toggle_request(args: &str) -> Option<bool> {
    match args.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "enable" | "enabled" | "true" => Some(true),
        "0" | "off" | "disable" | "disabled" | "false" => Some(false),
        _ => None,
    }
}
const HELP_SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show this help"),
    ("/clear", "Clear conversation history"),
    ("/compact", "Compact conversation context"),
    ("/autocompact", "Toggle automatic context compaction on/off"),
    ("/thinking", "Toggle extended thinking on/off"),
    ("/brief", "Toggle concise-response mode"),
    (
        "/optimize-tonken",
        "Toggle token-optimisation mode (alias: /optimize-token)",
    ),
    ("/stats", "Show session statistics"),
    ("/login", "Authenticate with a provider"),
    ("/logout", "Sign out"),
    ("/doctor", "Run diagnostic checks"),
    ("/model", "Switch AI model"),
    ("/search", "(ctrl+f) Search workspace files"),
    ("/status", "Show provider status"),
    ("/review", "Start code review mode"),
    ("/exit", "Exit the TUI"),
];

const HELP_KEYBOARD_SHORTCUTS: &[(&str, &str)] = &[
    ("ctrl+c", "Interrupt current operation"),
    ("ctrl+l", "Clear screen"),
    ("ctrl+r", "History search"),
    ("ctrl+f", "Global file search"),
    ("ctrl+o", "Expand/collapse tool output"),
    ("esc", "Cancel / close overlay"),
    ("enter", "Submit prompt"),
    ("shift+enter", "Insert newline"),
    ("↑/↓", "Scroll transcript"),
];

/// Renders the `/help` slash-command response for the TUI transcript.
pub fn execute_help_command() -> Result<String> {
    let mut lines = vec!["Slash Commands".into()];
    append_help_rows(&mut lines, HELP_SLASH_COMMANDS);
    lines.push(String::new());
    lines.push("Keyboard Shortcuts".into());
    append_help_rows(&mut lines, HELP_KEYBOARD_SHORTCUTS);
    Ok(lines.join("\n"))
}

fn append_help_rows(lines: &mut Vec<String>, rows: &[(&str, &str)]) {
    let label_width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or_default()
        + 2;
    lines.extend(
        rows.iter()
            .map(|(label, description)| format!("{label:<label_width$}{description}")),
    );
}

/// Renders the `/thinking` slash-command response for the current session state.
pub fn execute_thinking_command(app: &AppState, arg: Option<&str>) -> Result<String> {
    let current = app.thinking_enabled;
    let effort = match app.thinking_effort {
        ThinkingEffort::Low => "low",
        ThinkingEffort::Medium => "medium",
        ThinkingEffort::High => "high",
    };
    match arg {
        Some("on") => Ok("Thinking enabled — Claude will reason before responding".into()),
        Some("off") => Ok("Thinking disabled".into()),
        Some("low") => Ok("Thinking effort set to low".into()),
        Some("medium") => Ok("Thinking effort set to medium".into()),
        Some("high") => Ok("Thinking effort set to high".into()),
        None | Some("") => Ok(format!(
            "Thinking is currently {} (effort: {effort}). Options: on, off, low, medium, high",
            if current { "enabled" } else { "disabled" }
        )),
        Some(other) => Err(WonderError::Validation(format!(
            "Unknown argument: {other}. Use 'on', 'off', 'low', 'medium', or 'high'"
        ))),
    }
}

/// Renders session-local token usage and estimated cost statistics.
pub fn execute_stats_command(app: &AppState) -> Result<String> {
    let usage = app.costs.usage;
    let cost = app.costs.estimated_cost_usd.unwrap_or_default();
    let model = app.model.as_deref().unwrap_or("unknown");
    let provider = app.provider.as_deref().unwrap_or("unknown");

    let mut lines = vec![
        "Session Statistics".into(),
        "─────────────────────────────".into(),
        format!("Provider:  {provider}"),
        format!("Model:     {model}"),
        "─────────────────────────────".into(),
        format!("Input tokens:        {:>10}", usage.input_tokens),
        format!("Output tokens:       {:>10}", usage.output_tokens),
        format!("Cache create tokens: {:>10}", usage.cache_creation_tokens),
        format!("Cache read tokens:   {:>10}", usage.cache_read_tokens),
        format!("Total tokens:        {:>10}", usage.total_tokens()),
        "─────────────────────────────".into(),
    ];
    if cost > 0.0 {
        lines.push(format!("Estimated cost:      ${cost:.4}"));
    }
    Ok(lines.join("\n"))
}

/// Renders the `/settings` slash-command response for the current session state.
pub fn execute_settings_command(app: &AppState) -> Result<String> {
    let usage = app.costs.usage;
    let provider = app.provider.as_deref().unwrap_or("unknown");
    let model = app.model.as_deref().unwrap_or("unknown");
    let storage = storage_root(None)
        .map(|root| StoragePaths::new(root).sessions_dir())
        .map(|path| home_relative_path(&path))
        .unwrap_or_else(|| "unavailable".into());
    let total_cost = app.costs.estimated_cost_usd.unwrap_or_default();

    Ok([
        "── Configuration ──────────────────────────────".into(),
        format_settings_row("Provider:", provider),
        format_settings_row("Model:", model),
        format_settings_row("Storage:", &storage),
        format_settings_row("Permission:", permission_mode_label(app.permission_mode)),
        format_settings_row("Thinking:", if app.thinking_enabled { "on" } else { "off" }),
        String::new(),
        "── Session Usage ───────────────────────────────".into(),
        format_settings_row("Input tokens:", &format_token_count(usage.input_tokens)),
        format_settings_row("Output tokens:", &format_token_count(usage.output_tokens)),
        format_settings_row("Cache read:", &format_token_count(usage.cache_read_tokens)),
        format_settings_row(
            "Cache write:",
            &format_token_count(usage.cache_creation_tokens),
        ),
        format_settings_row("Total cost:", &format!("${total_cost:.4}")),
        String::new(),
        "── Provider Status ─────────────────────────────".into(),
        format_settings_row("Auth:", &render_auth_status(&app.auth)),
        format_settings_row(
            "Context:",
            &render_context_status(usage.total_tokens(), app.context_window_size),
        ),
    ]
    .join("\n"))
}

fn format_settings_row(label: &str, value: &str) -> String {
    format!("  {label:<14}{value}")
}

fn render_auth_status(auth: &AuthState) -> String {
    let symbol = if auth.is_ready() { "✓" } else { "!" };
    let status = match auth.status_label() {
        "not_required" => "ready",
        other => other,
    };
    format!("{symbol} {status} ({})", auth_kind_display(auth.kind))
}

fn auth_kind_display(kind: AuthMaterialKind) -> &'static str {
    match kind {
        AuthMaterialKind::None => "none",
        AuthMaterialKind::ApiKey => "api-key",
        AuthMaterialKind::OAuth => "oauth",
        AuthMaterialKind::AwsSigV4 => "aws-sigv4",
        AuthMaterialKind::AwsBearer => "aws-bearer",
        AuthMaterialKind::AwsProfile => "aws-profile",
        AuthMaterialKind::GcpOAuth2 => "gcp-oauth2",
    }
}

fn render_context_status(used_tokens: u64, max_tokens: Option<u64>) -> String {
    let Some(max_tokens) = max_tokens.filter(|max_tokens| *max_tokens > 0) else {
        return "unknown".into();
    };
    let percentage = used_tokens.saturating_mul(100) / max_tokens;
    format!(
        "{} / {} tokens ({}%)",
        format_token_count(used_tokens),
        format_token_count(max_tokens),
        percentage.min(100)
    )
}

fn format_token_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}

fn home_relative_path(path: &Path) -> String {
    let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) else {
        return path.display().to_string();
    };
    let home = PathBuf::from(home);
    match path.strip_prefix(&home) {
        Ok(suffix) if suffix.as_os_str().is_empty() => "~".into(),
        Ok(suffix) => format!("~/{}", suffix.display()),
        Err(_) => path.display().to_string(),
    }
}
fn pr_comments_usage(include_gh_note: bool) -> String {
    let mut usage =
        "Usage: /pr-comments [pr-number]\nFetches PR-level and code review comments from GitHub."
            .to_string();
    if include_gh_note {
        usage.push_str("\nRequires gh CLI (https://cli.github.com).");
    }
    usage
}

fn fetch_pr_comments(pr_number: &str) -> String {
    match fetch_pr_comments_inner(pr_number) {
        Ok(output) => output,
        Err(error) => format!("pr-comments: {error}"),
    }
}

fn fetch_pr_comments_inner(pr_number: &str) -> std::result::Result<String, String> {
    let pr = gh_json([
        "pr",
        "view",
        pr_number,
        "--json",
        "number,url,headRefName,headRepository",
    ])?;
    let number = pr
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| "missing PR number in gh output".to_string())?;
    let url = pr.get("url").and_then(Value::as_str).unwrap_or("unknown");
    let branch = pr
        .get("headRefName")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let repository = pr
        .get("headRepository")
        .ok_or_else(|| "missing headRepository in gh output".to_string())?;
    let owner = repository
        .get("owner")
        .and_then(|value| value.get("login"))
        .and_then(Value::as_str)
        .ok_or_else(|| "missing repository owner in gh output".to_string())?;
    let repo = repository
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing repository name in gh output".to_string())?;

    let issue_comments = gh_json([
        "api",
        &format!("repos/{owner}/{repo}/issues/{number}/comments"),
    ])?;
    let review_comments = gh_json([
        "api",
        &format!("repos/{owner}/{repo}/pulls/{number}/comments"),
    ])?;
    format_pr_comments(number, url, branch, &issue_comments, &review_comments)
}

fn gh_json<const N: usize>(args: [&str; N]) -> std::result::Result<Value, String> {
    let output = ProcessCommand::new("gh")
        .args(args)
        .output()
        .map_err(|error| format!("failed to invoke gh: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            "gh command failed".into()
        } else {
            detail
        });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid gh JSON output: {error}"))
}

fn format_pr_comments(
    number: u64,
    url: &str,
    branch: &str,
    issue_comments: &Value,
    review_comments: &Value,
) -> std::result::Result<String, String> {
    let issue_comments = issue_comments
        .as_array()
        .ok_or_else(|| "issue comments payload was not an array".to_string())?;
    let review_comments = review_comments
        .as_array()
        .ok_or_else(|| "review comments payload was not an array".to_string())?;

    let mut lines = vec![
        format!("PR Comments for #{number}"),
        format!("url={url}"),
        format!("branch={branch}"),
        format!("issue_comments={}", issue_comments.len()),
        format!("review_comments={}", review_comments.len()),
    ];
    if !issue_comments.is_empty() {
        lines.push("Issue comments:".into());
        for comment in issue_comments {
            lines.push(format!(
                "• {} @ {}: {}",
                json_path_str(comment, &["user", "login"]).unwrap_or("unknown"),
                comment
                    .get("created_at")
                    .or_else(|| comment.get("createdAt"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                preview_comment_body(comment.get("body").and_then(Value::as_str).unwrap_or(""))
            ));
        }
    }
    if !review_comments.is_empty() {
        lines.push("Review comments:".into());
        for comment in review_comments {
            let path = comment
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let line = comment
                .get("line")
                .and_then(Value::as_i64)
                .map(|line| line.to_string())
                .unwrap_or_else(|| "unknown".into());
            lines.push(format!(
                "• {} {}:{}: {}",
                json_path_str(comment, &["user", "login"]).unwrap_or("unknown"),
                path,
                line,
                preview_comment_body(comment.get("body").and_then(Value::as_str).unwrap_or(""))
            ));
        }
    }
    Ok(lines.join("\n"))
}

fn json_path_str<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str()
}

fn preview_comment_body(body: &str) -> String {
    let preview = body.lines().next().unwrap_or_default().trim();
    if preview.chars().count() <= 100 {
        preview.to_string()
    } else {
        format!("{}…", preview.chars().take(100).collect::<String>())
    }
}

fn render_env_summary(context: &CommandContext) -> String {
    let shell = env::var("SHELL").unwrap_or_else(|_| "unset".into());
    let path_entries = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).count())
        .unwrap_or_default();
    [
        "## Environment".into(),
        format!("cwd={}", context.cwd.display()),
        format!("shell={shell}"),
        format!("os={}", env::consts::OS),
        format!(
            "rust_env={}",
            env::var("RUST_ENV").unwrap_or_else(|_| "unset".into())
        ),
        format!("path_entries={path_entries}"),
        format!(
            "path_summary={}",
            truncate_middle(&env::var("PATH").unwrap_or_else(|_| "unset".into()), 120)
        ),
    ]
    .join("\n")
}
fn autofix_pr(args: &str) -> String {
    if !executable_on_path("gh") {
        return "autofix-pr: requires gh CLI (https://cli.github.com).".into();
    }
    let mut command = ProcessCommand::new("gh");
    command.arg("pr").arg("diff");
    if !args.is_empty() {
        command.arg(args);
    }
    match command.output() {
        Ok(output) if output.status.success() => {
            let diff = String::from_utf8_lossy(&output.stdout);
            let files = diff
                .lines()
                .filter(|line| line.starts_with("diff --git"))
                .count();
            let excerpt_source = diff.lines().take(20).collect::<Vec<_>>().join("\n");
            let excerpt = truncate_middle(&excerpt_source, 400);
            format!(
                "Autofix PR suggestion\nchanged_files={files}\nSuggested prompt: \"Review this PR diff, identify the highest-impact fix, and produce a minimal patch.\"\nexcerpt=\n{excerpt}"
            )
        }
        Ok(output) => format!(
            "autofix-pr: gh pr diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("autofix-pr: failed to invoke gh: {error}"),
    }
}
fn truncate_middle(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.into();
    }
    let keep = max_len.saturating_sub(3) / 2;
    format!("{}...{}", &value[..keep], &value[value.len() - keep..])
}

fn executable_on_path(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| path.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use wonder_of_u_core::{PermissionMode, TokenUsage};

    use super::*;

    #[test]
    fn pr_comment_formatter_includes_issue_and_review_counts() {
        let issue_comments = json!([
            {
                "user": { "login": "reviewer-a" },
                "body": "Looks good overall",
                "createdAt": "2025-01-01T00:00:00Z"
            }
        ]);
        let review_comments = json!([
            {
                "user": { "login": "reviewer-b" },
                "body": "Please rename this variable",
                "path": "src/lib.rs",
                "line": 42
            }
        ]);

        let rendered = format_pr_comments(
            17,
            "https://github.com/example/repo/pull/17",
            "feat/extras",
            &issue_comments,
            &review_comments,
        )
        .expect("formatted comments");

        assert!(rendered.contains("PR Comments for #17"));
        assert!(rendered.contains("issue_comments=1"));
        assert!(rendered.contains("review_comments=1"));
        assert!(rendered.contains("reviewer-a"));
        assert!(rendered.contains("src/lib.rs:42"));
    }

    #[test]
    fn thinking_command_reports_and_validates_state() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        assert_eq!(
            execute_thinking_command(&app, None).expect("report thinking state"),
            "Thinking is currently disabled (effort: medium). Options: on, off, low, medium, high"
        );

        app.set_thinking_enabled(true);
        assert_eq!(
            execute_thinking_command(&app, Some("")).expect("report thinking state"),
            "Thinking is currently enabled (effort: medium). Options: on, off, low, medium, high"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("on")).expect("enable thinking"),
            "Thinking enabled — Claude will reason before responding"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("off")).expect("disable thinking"),
            "Thinking disabled"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("low")).expect("set low effort"),
            "Thinking effort set to low"
        );
        app.set_thinking_effort(ThinkingEffort::High);
        assert_eq!(
            execute_thinking_command(&app, Some("")).expect("report thinking effort"),
            "Thinking is currently enabled (effort: high). Options: on, off, low, medium, high"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("medium")).expect("set medium effort"),
            "Thinking effort set to medium"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("high")).expect("set high effort"),
            "Thinking effort set to high"
        );
        assert!(matches!(
            execute_thinking_command(&app, Some("maybe")),
            Err(WonderError::Validation(message))
                if message == "Unknown argument: maybe. Use 'on', 'off', 'low', 'medium', or 'high'"
        ));
    }

    #[test]
    fn help_command_lists_expected_slash_commands_and_shortcuts() {
        let rendered = execute_help_command().expect("render help");

        assert!(rendered.contains("Slash Commands"));
        assert!(rendered.contains("/help"));
        assert!(rendered.contains("/search"));
        assert!(rendered.contains("/exit"));
        assert!(rendered.contains("Keyboard Shortcuts"));
        assert!(rendered.contains("ctrl+f"));
        assert!(rendered.contains("shift+enter"));
    }

    #[test]
    fn stats_command_renders_session_usage_table() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.provider = Some("openai".into());
        app.model = Some("gpt-4.1".into());
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 128,
                output_tokens: 32,
                cache_creation_tokens: 16,
                cache_read_tokens: 8,
            },
            Some(0.42),
        );

        let rendered = execute_stats_command(&app).expect("render stats");

        assert!(rendered.contains("Session Statistics"));
        assert!(rendered.contains("Provider:  openai"));
        assert!(rendered.contains("Model:     gpt-4.1"));
        assert!(rendered.contains("Input tokens:               128"));
        assert!(rendered.contains("Total tokens:               184"));
        assert!(rendered.contains("Estimated cost:      $0.4200"));
    }

    #[test]
    fn settings_command_renders_configuration_and_usage_sections() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.provider = Some("anthropic".into());
        app.model = Some("claude-3-5-sonnet-20241022".into());
        app.permission_mode = PermissionMode::Default;
        app.auth = AuthState::ready(
            AuthMaterialKind::ApiKey,
            wonder_of_u_core::AuthSource::Environment,
        );
        app.set_context_window_size(Some(200_000));
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 12_450,
                output_tokens: 3_821,
                cache_creation_tokens: 1_200,
                cache_read_tokens: 8_100,
            },
            Some(0.0412),
        );

        let rendered = execute_settings_command(&app).expect("render settings");

        assert!(rendered.contains("Configuration"));
        assert!(rendered.contains("Session Usage"));
        assert!(rendered.contains("Provider Status"));
        assert!(rendered.contains("Auth:"));
        assert!(rendered.contains("12,450"));
        assert!(rendered.contains("$0.0412"));
    }
}

//! Implements session-preference toggle commands:
//! `/vim`, `/theme`, `/color`, `/brief`, `/optimize-tonken`, `/fast`, and
//! `/effort`.
//!
//! Commands that own heavier config-file I/O live in dedicated modules:
//! * keybindings → [`super::keybinding_commands`]
//! * terminal setup → [`super::terminal_setup`]
//! * hooks → [`super::hook_commands`]
//! * privacy settings → [`super::privacy_settings`]

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use wonder_of_u_agent::{ProviderResolver, SettingsStore};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
    WonderError,
};

use super::parse_command_args;

// ── Command structs ───────────────────────────────────────────────────────────

/// Represents vim command
pub struct VimCommand;

impl VimCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "vim",
            "Toggle between Vim normal and insert modes",
            CommandKind::Local,
        )
    }
}

/// Represents theme command
pub struct ThemeCommand;

impl ThemeCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "theme",
            "Show or change the active TUI theme",
            CommandKind::Local,
        )
    }
}

/// Represents color command
pub struct ColorCommand;

impl ColorCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        // immediate=true: /color must not drain queued prompts after executing.
        CommandSpec::new(
            "color",
            "Set the prompt bar color for this session",
            CommandKind::Local,
        )
        .with_immediate(true)
    }
}

/// Represents brief command
pub struct BriefCommand;

impl BriefCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "brief",
            "Toggle brief response mode for the current session",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

/// Toggles token-optimisation mode for the current session.
///
/// When enabled a system-prompt instruction is injected that tells the model
/// to minimise output tokens — omitting preambles, filler, and unnecessary
/// repetition.  The canonical name preserves the original spelling
/// (`optimize-tonken`); the alias `optimize-token` is also accepted.
pub struct OptimizeTonkenCommand;

impl OptimizeTonkenCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "optimize-tonken",
            "Toggle token-optimisation mode (minimise output tokens) for the current session",
            CommandKind::Local,
        );
        // Accept the correctly-spelled alias too so neither spelling is wrong.
        spec.aliases.push("optimize-token".into());
        spec.interactive_only = true;
        spec
    }
}

/// Represents fast command
pub struct FastCommand {
    storage_dir: Option<PathBuf>,
}

impl FastCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        // immediate=true: /fast must not drain queued prompts after executing.
        CommandSpec::new(
            "fast",
            "Show or change fast-mode model remapping",
            CommandKind::Local,
        )
        .with_immediate(true)
    }
}

/// Represents effort command
pub struct EffortCommand {
    storage_dir: Option<PathBuf>,
}

impl EffortCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        // immediate=true: /effort must not drain queued prompts after executing.
        CommandSpec::new(
            "effort",
            "Show or change the active effort level",
            CommandKind::Local,
        )
        .with_immediate(true)
    }
}

// ── Clap arg structs ─────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct ThemeArgs {
    #[command(subcommand)]
    command: Option<ThemeSubcommand>,
}

#[derive(Debug, Subcommand)]
enum ThemeSubcommand {
    Show,
    Set(ThemeSetArgs),
}

#[derive(Debug, Args)]
struct ThemeSetArgs {
    #[arg()]
    theme: String,
}

#[derive(Debug, Parser)]
struct ColorArgs {
    #[command(subcommand)]
    command: Option<ColorSubcommand>,
}

#[derive(Debug, Subcommand)]
enum ColorSubcommand {
    Show,
    Set(ColorSetArgs),
}

#[derive(Debug, Args)]
struct ColorSetArgs {
    #[arg()]
    color: String,
}

#[derive(Debug, Parser)]
struct EffortArgs {
    #[command(subcommand)]
    command: Option<EffortSubcommand>,
}

#[derive(Debug, Subcommand)]
enum EffortSubcommand {
    Show,
    Set(EffortSetArgs),
}

#[derive(Debug, Args)]
struct EffortSetArgs {
    #[arg()]
    level: String,
}

#[derive(Debug, Parser)]
struct VimArgs {
    #[command(subcommand)]
    command: Option<VimSubcommand>,
}

#[derive(Debug, Subcommand)]
enum VimSubcommand {
    Show,
    Set(VimSetArgs),
}

#[derive(Debug, Args)]
struct VimSetArgs {
    #[arg()]
    mode: String,
}

// ── Internal action enums ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BriefAction {
    Show,
    Set(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FastAction {
    Show,
    Set(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OptimizeTonkenAction {
    Show,
    Set(bool),
}

// ── Command impls ─────────────────────────────────────────────────────────────

#[async_trait]
impl Command for VimCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_vim_invocation(&invocation);
        let args = parse_command_args::<VimArgs>("vim", &normalized)?;
        match args.command.unwrap_or(VimSubcommand::Show) {
            VimSubcommand::Show => {
                if context.interactive && invocation.args.trim().is_empty() {
                    Ok(CommandOutput::Text("vim_toggle=true".into()))
                } else {
                    Ok(CommandOutput::Text(render_vim_status(None)))
                }
            }
            VimSubcommand::Set(args) => Ok(CommandOutput::Text(render_vim_transition(
                parse_vim_mode(&args.mode)?,
            ))),
        }
    }
}

#[async_trait]
impl Command for ThemeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_theme_invocation(&invocation);
        let args = parse_command_args::<ThemeArgs>("theme", &normalized)?;
        let current_theme = theme_name(context.theme.as_deref());
        match args.command.unwrap_or(ThemeSubcommand::Show) {
            ThemeSubcommand::Show => {
                if context.interactive && invocation.args.trim().is_empty() {
                    Ok(CommandOutput::Text(render_theme_picker(current_theme)))
                } else {
                    Ok(CommandOutput::Text(render_theme_status(current_theme)))
                }
            }
            ThemeSubcommand::Set(args) => Ok(CommandOutput::Text(render_theme_transition(
                parse_theme_name(&args.theme)?,
            ))),
        }
    }
}

#[async_trait]
impl Command for ColorCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_color_invocation(&invocation);
        let args = parse_command_args::<ColorArgs>("color", &normalized)?;
        let current_color = session_color_name(context.session_color.as_deref());
        match args.command.unwrap_or(ColorSubcommand::Show) {
            ColorSubcommand::Show => Ok(CommandOutput::Text(render_color_status(current_color))),
            ColorSubcommand::Set(args) => Ok(CommandOutput::Text(render_color_transition(
                parse_session_color_name(&args.color)?,
            ))),
        }
    }
}

#[async_trait]
impl Command for BriefCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        match parse_brief_action(invocation.args.trim(), context.brief_mode)? {
            BriefAction::Show => Ok(CommandOutput::Text(render_brief_status(context.brief_mode))),
            BriefAction::Set(enabled) => Ok(CommandOutput::Text(render_brief_transition(enabled))),
        }
    }
}

#[async_trait]
impl Command for OptimizeTonkenCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        match parse_optimize_tonken_action(invocation.args.trim(), context.optimize_token_mode)? {
            OptimizeTonkenAction::Show => Ok(CommandOutput::Text(render_optimize_tonken_status(
                context.optimize_token_mode,
            ))),
            OptimizeTonkenAction::Set(enabled) => Ok(CommandOutput::Text(
                render_optimize_tonken_transition(enabled),
            )),
        }
    }
}

#[async_trait]
impl Command for FastCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
        match parse_fast_action(invocation.args.trim(), context.fast_mode)? {
            FastAction::Show => Ok(CommandOutput::Text(render_fast_status(
                context.fast_mode,
                persisted_fast(self.storage_dir.as_deref())?,
                self.storage_dir.as_deref(),
                &report,
            ))),
            FastAction::Set(enabled) => {
                let persisted = write_persisted_fast(self.storage_dir.as_deref(), enabled)?;
                Ok(CommandOutput::Text(render_fast_transition(
                    enabled,
                    persisted,
                    self.storage_dir.as_deref(),
                    &report,
                )))
            }
        }
    }
}

#[async_trait]
impl Command for EffortCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_effort_invocation(&invocation);
        let args = parse_command_args::<EffortArgs>("effort", &normalized)?;
        let current_effort = effort_level_name(context.effort_level.as_deref());
        match args.command.unwrap_or(EffortSubcommand::Show) {
            EffortSubcommand::Show => Ok(CommandOutput::Text(render_effort_status(
                current_effort,
                persisted_effort(self.storage_dir.as_deref())?.as_deref(),
                self.storage_dir.as_deref(),
            ))),
            EffortSubcommand::Set(args) => {
                let level = parse_effort_level_name(&args.level)?;
                let persisted = write_persisted_effort(self.storage_dir.as_deref(), level)?;
                Ok(CommandOutput::Text(render_effort_transition(
                    level,
                    persisted,
                    self.storage_dir.as_deref(),
                )))
            }
        }
    }
}

// ── Normalisation helpers ────────────────────────────────────────────────────

fn normalize_vim_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

pub(super) fn normalize_theme_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

pub(super) fn normalize_color_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

pub(super) fn normalize_effort_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "current"
        || trimmed == "status"
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return if trimmed == "current" || trimmed == "status" {
            CommandInvocation {
                name: invocation.name.clone(),
                args: "show".into(),
                raw: invocation.raw.clone(),
            }
        } else {
            invocation.clone()
        };
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

// ── Action parsers ────────────────────────────────────────────────────────────

pub(super) fn parse_brief_action(args: &str, current: bool) -> Result<BriefAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed == "toggle" {
        return Ok(BriefAction::Set(!current));
    }
    match trimmed {
        "show" | "status" | "current" => Ok(BriefAction::Show),
        "on" | "enable" | "enabled" => Ok(BriefAction::Set(true)),
        "off" | "disable" | "disabled" => Ok(BriefAction::Set(false)),
        other => Err(WonderError::validation(format!(
            "unknown brief action: {other}"
        ))),
    }
}

pub(super) fn parse_fast_action(args: &str, current: bool) -> Result<FastAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed == "toggle" {
        return Ok(FastAction::Set(!current));
    }
    match trimmed {
        "show" | "status" | "current" => Ok(FastAction::Show),
        "on" | "enable" | "enabled" => Ok(FastAction::Set(true)),
        "off" | "disable" | "disabled" => Ok(FastAction::Set(false)),
        other => Err(WonderError::validation(format!(
            "unknown fast action: {other}"
        ))),
    }
}

pub(super) fn parse_optimize_tonken_action(
    args: &str,
    current: bool,
) -> Result<OptimizeTonkenAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed == "toggle" {
        return Ok(OptimizeTonkenAction::Set(!current));
    }
    match trimmed {
        "show" | "status" | "current" => Ok(OptimizeTonkenAction::Show),
        "on" | "enable" | "enabled" => Ok(OptimizeTonkenAction::Set(true)),
        "off" | "disable" | "disabled" => Ok(OptimizeTonkenAction::Set(false)),
        other => Err(WonderError::validation(format!(
            "unknown optimize-tonken action: {other}"
        ))),
    }
}

// ── Name / label parsers ──────────────────────────────────────────────────────

fn parse_vim_mode(value: &str) -> Result<&'static str> {
    match value {
        "insert" => Ok("insert"),
        "normal" => Ok("normal"),
        other => Err(WonderError::validation(format!(
            "unknown vim mode: {other}"
        ))),
    }
}

pub(super) fn parse_theme_name(value: &str) -> Result<&'static str> {
    match value {
        "default" => Ok("default"),
        "midnight" => Ok("midnight"),
        "light" => Ok("light"),
        other => Err(WonderError::validation(format!("unknown theme: {other}"))),
    }
}

pub(super) fn parse_session_color_name(value: &str) -> Result<&'static str> {
    match value {
        "default" | "reset" | "none" | "gray" | "grey" => Ok("default"),
        "red" => Ok("red"),
        "blue" => Ok("blue"),
        "green" => Ok("green"),
        "yellow" => Ok("yellow"),
        "purple" => Ok("purple"),
        "orange" => Ok("orange"),
        "pink" => Ok("pink"),
        "cyan" => Ok("cyan"),
        other => Err(WonderError::validation(format!("unknown color: {other}"))),
    }
}

pub(super) fn parse_effort_level_name(value: &str) -> Result<Option<&'static str>> {
    match value {
        "auto" | "unset" => Ok(None),
        "low" => Ok(Some("low")),
        "medium" => Ok(Some("medium")),
        "high" => Ok(Some("high")),
        "max" => Ok(Some("max")),
        other => Err(WonderError::validation(format!(
            "unknown effort level: {other}"
        ))),
    }
}

fn theme_name(value: Option<&str>) -> &'static str {
    match value {
        Some("midnight") => "midnight",
        Some("light") => "light",
        _ => "default",
    }
}

fn session_color_name(value: Option<&str>) -> &'static str {
    match value {
        Some("red") => "red",
        Some("blue") => "blue",
        Some("green") => "green",
        Some("yellow") => "yellow",
        Some("purple") => "purple",
        Some("orange") => "orange",
        Some("pink") => "pink",
        Some("cyan") => "cyan",
        _ => "default",
    }
}

fn effort_level_name(value: Option<&str>) -> &'static str {
    match value {
        Some("low") => "low",
        Some("medium") => "medium",
        Some("high") => "high",
        Some("max") => "max",
        _ => "auto",
    }
}

fn brief_mode_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn fast_mode_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn optimize_token_mode_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

// ── Render helpers ────────────────────────────────────────────────────────────

fn render_vim_transition(mode: &str) -> String {
    format!("vim_mode={mode}\nstatus=vim mode updated")
}

fn render_vim_status(mode: Option<&str>) -> String {
    match mode {
        Some(mode) => format!("vim_mode={mode}"),
        None => {
            "vim_mode=unavailable\nnote=vim mode is only available in the interactive TUI".into()
        }
    }
}

pub(super) fn render_theme_status(current_theme: &str) -> String {
    let mut lines = vec![
        "## Theme".into(),
        format!("current_theme={current_theme}"),
        String::new(),
    ];
    for (name, description) in theme_catalog() {
        lines.push(format!(
            "- {name}: {description}{}",
            if name == current_theme {
                " (active)"
            } else {
                ""
            }
        ));
    }
    lines.join("\n")
}

fn render_theme_transition(theme: &str) -> String {
    format!("theme={theme}\nstatus=theme updated")
}

fn render_theme_picker(current_theme: &str) -> String {
    let mut lines = vec![
        "theme_picker=true".into(),
        format!("current_theme={current_theme}"),
    ];
    for (name, description) in theme_catalog() {
        lines.push(format!(
            "theme_option={}",
            json!({
                "theme": name,
                "label": name,
                "description": description,
                "selected": name == current_theme,
            })
        ));
    }
    lines.join("\n")
}

fn theme_catalog() -> [(&'static str, &'static str); 3] {
    [
        (
            "default",
            "Dark shell with blue borders and cyan prompt accents.",
        ),
        (
            "midnight",
            "Deeper dark background with brighter cyan and magenta emphasis.",
        ),
        (
            "light",
            "Light background with dark text and blue status accents.",
        ),
    ]
}

pub(super) fn render_color_status(current_color: &str) -> String {
    let mut lines = vec![
        "## Color".into(),
        format!("current_color={current_color}"),
        String::new(),
        "Available colors: red, blue, green, yellow, purple, orange, pink, cyan, default".into(),
    ];
    for color in [
        "red", "blue", "green", "yellow", "purple", "orange", "pink", "cyan", "default",
    ] {
        lines.push(format!(
            "- {color}{}",
            if color == current_color {
                " (active)"
            } else {
                ""
            }
        ));
    }
    lines.join("\n")
}

pub(super) fn render_color_transition(color: &str) -> String {
    format!("color={color}\nstatus=color updated")
}

pub(super) fn render_effort_status(
    current_effort: &str,
    persisted_effort: Option<&str>,
    storage_dir: Option<&Path>,
) -> String {
    let mut lines = vec![
        "## Effort".into(),
        format!("current_effort={current_effort}"),
        format!("persisted_effort={}", effort_level_name(persisted_effort)),
        String::new(),
        "Available levels: low, medium, high, max, auto".into(),
        "- low: quicker and lighter-weight work".into(),
        "- medium: balanced default-style effort".into(),
        "- high: deeper reasoning before answering".into(),
        "- max: ask for the deepest available reasoning mode".into(),
        "- auto: clear the explicit override".into(),
        String::new(),
        "Provider runtime mapping is active: OpenAI/Copilot receive reasoning_effort for high/max, and Claude-family Anthropic requests receive thinking budget configuration.".into(),
    ];
    if storage_dir.is_none() {
        lines.push("Storage is disabled, so changes only apply to the current TUI session.".into());
    }
    lines.join("\n")
}

pub(super) fn render_effort_transition(
    level: Option<&str>,
    persisted: bool,
    storage_dir: Option<&Path>,
) -> String {
    let effort = effort_level_name(level);
    let mut lines = vec![
        format!("effort_level={effort}"),
        format!("persisted={persisted}"),
        "status=effort updated".into(),
    ];
    if storage_dir.is_none() {
        lines.push("note=storage disabled; effort is session-only".into());
    } else {
        lines.push(
            "note=provider runtime maps high/max effort into provider-specific inference options"
                .into(),
        );
    }
    lines.join("\n")
}

pub(super) fn render_brief_status(enabled: bool) -> String {
    [
        "## Brief".into(),
        format!("brief_mode={enabled}"),
        format!("current_brief={}", brief_mode_label(enabled)),
        String::new(),
        "The Rust port keeps replies concise by injecting a briefness system prompt into this session.".into(),
        "It does not yet replicate the leak's SendUserMessage-only tooling and hidden plain-text filtering semantics.".into(),
    ]
    .join("\n")
}

pub(super) fn render_brief_transition(enabled: bool) -> String {
    format!(
        "brief_mode={enabled}\nstatus=brief mode {}\nnote=session prompts now request concise output",
        if enabled { "enabled" } else { "disabled" }
    )
}

pub(super) fn render_optimize_tonken_status(enabled: bool) -> String {
    [
        "## Optimize Token".into(),
        format!("optimize_token_mode={enabled}"),
        format!(
            "current_optimize_token={}",
            optimize_token_mode_label(enabled)
        ),
        String::new(),
        "When enabled, a system-prompt instruction is injected that tells the model to minimise"
            .into(),
        "output tokens — omitting preambles, filler words, and unnecessary repetition.".into(),
        "Use '/optimize-tonken on' to enable, '/optimize-tonken off' to disable, or".into(),
        "'/optimize-tonken' (no args) to toggle.  '/optimize-token' is also accepted.".into(),
    ]
    .join("\n")
}

pub(super) fn render_optimize_tonken_transition(enabled: bool) -> String {
    format!(
        "optimize_token_mode={enabled}\nstatus=optimize token mode {}\nnote={}",
        if enabled { "enabled" } else { "disabled" },
        if enabled {
            "session prompts now instruct the model to minimise output tokens"
        } else {
            "token-optimisation mode disabled; model will respond normally"
        },
    )
}

pub(super) fn render_fast_status(
    current_fast: bool,
    persisted_fast: Option<bool>,
    storage_dir: Option<&Path>,
    report: &wonder_of_u_agent::ProviderStatusReport,
) -> String {
    let resolver = ProviderResolver::builtin();
    let provider = report.provider.as_deref();
    let preferred_fast = provider
        .and_then(|provider_id| resolver.registry().get(provider_id))
        .and_then(|descriptor| descriptor.preferred_fast_model())
        .map(|model| model.id.as_str());

    let mut lines = vec![
        "## Fast".into(),
        format!("fast_mode={current_fast}"),
        format!("current_fast={}", fast_mode_label(current_fast)),
        format!(
            "persisted_fast={}",
            persisted_fast.map(fast_mode_label).unwrap_or("session-only")
        ),
        format!(
            "provider={}",
            provider.unwrap_or("unconfigured")
        ),
        format!(
            "current_model={}",
            report.model.as_deref().unwrap_or("unconfigured")
        ),
        format!(
            "fast_target={}",
            preferred_fast.unwrap_or("unavailable")
        ),
        String::new(),
        "Fast mode remaps the default provider model to the fastest built-in option this Rust port can identify.".into(),
        "Right now that means mini/haiku-style models when the selected provider exposes one.".into(),
        "It does not implement the leak's entitlement, quota, cooldown, or billing-aware fast-mode semantics.".into(),
    ];
    if storage_dir.is_none() {
        lines.push("Storage is disabled, so changes only apply to the current TUI session.".into());
    } else if preferred_fast.is_none() && provider.is_some() {
        lines.push(
            "The selected provider has no known fast-model mapping, so enabling fast mode will not change runtime model selection."
                .into(),
        );
    }
    lines.join("\n")
}

pub(super) fn render_fast_transition(
    enabled: bool,
    persisted: bool,
    storage_dir: Option<&Path>,
    report: &wonder_of_u_agent::ProviderStatusReport,
) -> String {
    let resolver = ProviderResolver::builtin();
    let provider = report.provider.as_deref();
    let preferred_fast = provider
        .and_then(|provider_id| resolver.registry().get(provider_id))
        .and_then(|descriptor| descriptor.preferred_fast_model())
        .map(|model| model.id.as_str());

    let mut lines = vec![
        format!("fast_mode={enabled}"),
        format!("persisted={persisted}"),
        format!("fast_target={}", preferred_fast.unwrap_or("unavailable")),
        format!(
            "status=fast mode {}",
            if enabled { "enabled" } else { "disabled" }
        ),
    ];
    if storage_dir.is_none() {
        lines.push("note=storage disabled; fast mode is session-only".into());
    } else if enabled {
        lines.push(
            "note=runtime will remap default model selection to the provider's built-in fast model when available"
                .into(),
        );
    } else {
        lines.push("note=runtime will use the configured default model selection again".into());
    }
    if enabled && preferred_fast.is_none() {
        lines.push(
            "warning=the current provider has no known fast-model mapping; runtime behavior will not change until you switch to a supported provider"
                .into(),
        );
    }
    lines.join("\n")
}

// ── Persisted settings helpers ────────────────────────────────────────────────

fn persisted_effort(storage_dir: Option<&Path>) -> Result<Option<String>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(None);
    };
    Ok(SettingsStore::new(storage_dir).read()?.effort_level)
}

fn persisted_fast(storage_dir: Option<&Path>) -> Result<Option<bool>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(None);
    };
    Ok(Some(SettingsStore::new(storage_dir).read()?.fast_mode))
}

pub(super) fn write_persisted_effort(
    storage_dir: Option<&Path>,
    level: Option<&str>,
) -> Result<bool> {
    let Some(storage_dir) = storage_dir else {
        return Ok(false);
    };
    let store = SettingsStore::new(storage_dir);
    let mut settings = store.read()?;
    settings.effort_level = level.map(ToString::to_string);
    store.write(&settings)?;
    Ok(true)
}

pub(super) fn write_persisted_fast(storage_dir: Option<&Path>, enabled: bool) -> Result<bool> {
    let Some(storage_dir) = storage_dir else {
        return Ok(false);
    };
    let store = SettingsStore::new(storage_dir);
    let mut settings = store.read()?;
    settings.fast_mode = enabled;
    store.write(&settings)?;
    Ok(true)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wonder_of_u_agent::{AgentSettings, ProviderResolver, SettingsStore};
    use wonder_of_u_core::CommandInvocation;
    use wonder_of_u_test_support::unique_test_dir;

    use super::{
        OptimizeTonkenAction, normalize_color_invocation, normalize_effort_invocation,
        normalize_theme_invocation, normalize_vim_invocation, parse_brief_action,
        parse_effort_level_name, parse_fast_action, parse_optimize_tonken_action,
        parse_session_color_name, render_brief_status, render_brief_transition,
        render_color_status, render_color_transition, render_effort_status,
        render_effort_transition, render_fast_status, render_fast_transition,
        render_optimize_tonken_status, render_optimize_tonken_transition, render_theme_status,
        write_persisted_effort, write_persisted_fast,
    };

    #[test]
    fn vim_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "vim".into(),
            args: "normal".into(),
            raw: "/vim normal".into(),
        };

        let normalized = normalize_vim_invocation(&invocation);

        assert_eq!(normalized.args, "set normal");
    }

    #[test]
    fn theme_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "theme".into(),
            args: "midnight".into(),
            raw: "/theme midnight".into(),
        };

        let normalized = normalize_theme_invocation(&invocation);

        assert_eq!(normalized.args, "set midnight");
    }

    #[test]
    fn theme_status_marks_active_theme() {
        let rendered = render_theme_status("midnight");

        assert!(rendered.contains("## Theme"));
        assert!(rendered.contains("current_theme=midnight"));
        assert!(rendered.contains("midnight: Deeper dark background"));
        assert!(rendered.contains("(active)"));
    }

    #[test]
    fn color_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "color".into(),
            args: "purple".into(),
            raw: "/color purple".into(),
        };

        let normalized = normalize_color_invocation(&invocation);

        assert_eq!(normalized.args, "set purple");
    }

    #[test]
    fn color_status_marks_active_color() {
        let rendered = render_color_status("cyan");

        assert!(rendered.contains("## Color"));
        assert!(rendered.contains("current_color=cyan"));
        assert!(rendered.contains("- cyan (active)"));
    }

    #[test]
    fn color_transition_accepts_reset_aliases() {
        let rendered = render_color_transition(parse_session_color_name("grey").expect("color"));

        assert_eq!(rendered, "color=default\nstatus=color updated");
    }

    #[test]
    fn brief_defaults_to_toggle_and_accepts_show() {
        assert_eq!(
            parse_brief_action("", false).expect("toggle on"),
            super::BriefAction::Set(true)
        );
        assert_eq!(
            parse_brief_action("toggle", true).expect("toggle off"),
            super::BriefAction::Set(false)
        );
        assert_eq!(
            parse_brief_action("show", true).expect("show"),
            super::BriefAction::Show
        );
    }

    #[test]
    fn brief_status_reports_runtime_limitations() {
        let rendered = render_brief_status(true);

        assert!(rendered.contains("## Brief"));
        assert!(rendered.contains("brief_mode=true"));
        assert!(rendered.contains("current_brief=on"));
        assert!(rendered.contains("hidden plain-text filtering semantics"));
    }

    #[test]
    fn brief_transition_reports_new_state() {
        let rendered = render_brief_transition(false);

        assert!(rendered.contains("brief_mode=false"));
        assert!(rendered.contains("status=brief mode disabled"));
    }

    #[test]
    fn optimize_tonken_defaults_to_toggle_and_accepts_show() {
        assert_eq!(
            parse_optimize_tonken_action("", false).expect("toggle on"),
            OptimizeTonkenAction::Set(true)
        );
        assert_eq!(
            parse_optimize_tonken_action("toggle", true).expect("toggle off"),
            OptimizeTonkenAction::Set(false)
        );
        assert_eq!(
            parse_optimize_tonken_action("show", false).expect("show"),
            OptimizeTonkenAction::Show
        );
        assert_eq!(
            parse_optimize_tonken_action("on", false).expect("on"),
            OptimizeTonkenAction::Set(true)
        );
        assert_eq!(
            parse_optimize_tonken_action("off", true).expect("off"),
            OptimizeTonkenAction::Set(false)
        );
    }

    #[test]
    fn optimize_tonken_action_rejects_unknown_arg() {
        assert!(parse_optimize_tonken_action("maybe", false).is_err());
    }

    #[test]
    fn optimize_tonken_status_renders_notice_heading_and_hint_lines() {
        let rendered = render_optimize_tonken_status(true);

        assert!(rendered.starts_with("## Optimize Token"));
        assert!(rendered.contains("optimize_token_mode=true"));
        assert!(rendered.contains("current_optimize_token=on"));
        assert!(rendered.contains("minimise"));
    }

    #[test]
    fn optimize_tonken_status_shows_off_when_disabled() {
        let rendered = render_optimize_tonken_status(false);

        assert!(rendered.contains("optimize_token_mode=false"));
        assert!(rendered.contains("current_optimize_token=off"));
    }

    #[test]
    fn optimize_tonken_transition_reports_enabled_state() {
        let rendered = render_optimize_tonken_transition(true);

        assert!(rendered.contains("optimize_token_mode=true"));
        assert!(rendered.contains("status=optimize token mode enabled"));
        assert!(rendered.contains("minimise output tokens"));
    }

    #[test]
    fn optimize_tonken_transition_reports_disabled_state() {
        let rendered = render_optimize_tonken_transition(false);

        assert!(rendered.contains("optimize_token_mode=false"));
        assert!(rendered.contains("status=optimize token mode disabled"));
        assert!(rendered.contains("respond normally"));
    }

    #[test]
    fn fast_action_defaults_to_toggle_and_accepts_show() {
        assert_eq!(
            parse_fast_action("", false).expect("toggle on"),
            super::FastAction::Set(true)
        );
        assert_eq!(
            parse_fast_action("toggle", true).expect("toggle off"),
            super::FastAction::Set(false)
        );
        assert_eq!(
            parse_fast_action("show", true).expect("show"),
            super::FastAction::Show
        );
    }

    #[test]
    fn fast_status_reports_provider_mapping_limitations() {
        let dir = unique_test_dir("workflow-fast-status");
        let store = SettingsStore::new(&dir);
        store
            .write(&AgentSettings {
                selected_provider: Some("openai".into()),
                fast_mode: true,
                ..AgentSettings::default()
            })
            .expect("write settings");
        let report = ProviderResolver::builtin()
            .load_report(Some(dir.as_path()))
            .expect("load report");

        let rendered = render_fast_status(true, Some(true), Some(dir.as_path()), &report);

        assert!(rendered.contains("## Fast"));
        assert!(rendered.contains("fast_mode=true"));
        assert!(rendered.contains("current_fast=on"));
        assert!(rendered.contains("fast_target=gpt-4o-mini"));
        assert!(rendered.contains("does not implement the leak's entitlement"));
    }

    #[test]
    fn fast_transition_reports_new_state_and_persists_flag() {
        let dir = unique_test_dir("workflow-fast-persist");
        let store = SettingsStore::new(&dir);
        store
            .write(&AgentSettings {
                selected_provider: Some("openai".into()),
                ..AgentSettings::default()
            })
            .expect("write settings");
        let report = ProviderResolver::builtin()
            .load_report(Some(dir.as_path()))
            .expect("load report");

        let persisted = write_persisted_fast(Some(dir.as_path()), true).expect("persist fast");
        let rendered = render_fast_transition(true, persisted, Some(dir.as_path()), &report);

        assert!(persisted);
        assert!(rendered.contains("fast_mode=true"));
        assert!(rendered.contains("status=fast mode enabled"));
        assert!(store.read().expect("read settings").fast_mode);
    }

    #[test]
    fn effort_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "effort".into(),
            args: "high".into(),
            raw: "/effort high".into(),
        };

        let normalized = normalize_effort_invocation(&invocation);

        assert_eq!(normalized.args, "set high");
    }

    #[test]
    fn effort_status_reports_current_and_persisted_levels() {
        let rendered = render_effort_status(
            "medium",
            Some("high"),
            Some(std::path::Path::new("/workspace")),
        );

        assert!(rendered.contains("## Effort"));
        assert!(rendered.contains("current_effort=medium"));
        assert!(rendered.contains("persisted_effort=high"));
        assert!(rendered.contains("Provider runtime mapping is active"));
    }

    #[test]
    fn effort_transition_persists_to_settings_store() {
        let dir = unique_test_dir("workflow-effort-settings");

        let persisted =
            write_persisted_effort(Some(dir.as_path()), parse_effort_level_name("max").unwrap())
                .expect("write effort");
        let settings = SettingsStore::new(&dir).read().expect("read settings");
        let rendered = render_effort_transition(Some("max"), persisted, Some(dir.as_path()));

        assert!(persisted);
        assert_eq!(settings.effort_level.as_deref(), Some("max"));
        assert!(rendered.contains("effort_level=max"));
        assert!(rendered.contains("persisted=true"));
    }
}

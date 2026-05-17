//! Implements user-preference and configuration commands:
//! `/vim`, `/keybindings`, `/terminal-setup`, `/theme`, `/color`, `/brief`,
//! `/optimize-tonken`, `/fast`, `/effort`, `/hooks`, and `/privacy-settings`.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_agent::{ProviderResolver, SettingsStore};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
    ToolSpec, WonderError,
};
use wonder_of_u_storage::StoragePaths;
use wonder_of_u_tui::{
    EditAction, KeyBinding, KeyBindingContext, KeyBindingResolver, KeyCode, KeyEvent, KeyModifiers,
    Motion, ResolvedKey, SystemAction, VimCommand as TuiVimCommand,
};

use super::plan_command::plan_editor_command;
use super::{parse_command_args, try_open_browser};

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

/// Represents keybindings command
pub struct KeybindingsCommand {
    storage_dir: Option<PathBuf>,
}

impl KeybindingsCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "keybindings",
            "Show the active keyboard shortcuts for the TUI",
            CommandKind::Local,
        )
    }
}

/// Represents terminal setup command
pub struct TerminalSetupCommand {
    storage_dir: Option<PathBuf>,
}

impl TerminalSetupCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "terminal-setup",
            "Explain multiline prompt setup and keybinding options for the local TUI",
            CommandKind::Local,
        );
        spec.aliases.push("terminalSetup".into());
        spec
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
        CommandSpec::new(
            "color",
            "Set the prompt bar color for this session",
            CommandKind::Local,
        )
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
        CommandSpec::new(
            "fast",
            "Show or change fast-mode model remapping",
            CommandKind::Local,
        )
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
        CommandSpec::new(
            "effort",
            "Show or change the active effort level",
            CommandKind::Local,
        )
    }
}

/// Represents hooks command
pub struct HooksCommand {
    storage_dir: Option<PathBuf>,
    tool_specs: Arc<[ToolSpec]>,
}

impl HooksCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>, tool_specs: Arc<[ToolSpec]>) -> Self {
        Self {
            storage_dir,
            tool_specs,
        }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "hooks",
            "View hook configurations for tool events",
            CommandKind::Local,
        )
    }
}

/// Represents privacy settings command
pub struct PrivacySettingsCommand;

impl PrivacySettingsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "privacy-settings",
            "View and update your privacy settings",
            CommandKind::Local,
        )
    }
}

// ── Clap arg structs ─────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct KeybindingsArgs {
    #[command(subcommand)]
    command: Option<KeybindingsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum KeybindingsSubcommand {
    Show,
    Open,
}

#[derive(Debug, Parser)]
struct HooksArgs {
    #[command(subcommand)]
    command: Option<HooksSubcommand>,
}

#[derive(Debug, Subcommand)]
enum HooksSubcommand {
    Show,
    Open,
}

#[derive(Debug, Parser)]
struct PrivacySettingsArgs {
    #[command(subcommand)]
    command: Option<PrivacySettingsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum PrivacySettingsSubcommand {
    Show,
}

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
impl Command for KeybindingsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<KeybindingsArgs>("keybindings", &invocation)?;
        match args.command.unwrap_or(KeybindingsSubcommand::Show) {
            KeybindingsSubcommand::Show => Ok(CommandOutput::Text(render_keybindings_summary(
                &load_keybinding_resolver(self.storage_dir.as_deref())?,
                self.storage_dir.as_deref(),
            ))),
            KeybindingsSubcommand::Open => {
                keybindings_open_output(&context, self.storage_dir.as_deref())
                    .map(CommandOutput::Text)
            }
        }
    }
}

#[async_trait]
impl Command for TerminalSetupCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_terminal_setup_notice(
            self.storage_dir.as_deref(),
        )))
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

#[async_trait]
impl Command for HooksCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<HooksArgs>("hooks", &invocation)?;
        match args.command.unwrap_or(HooksSubcommand::Show) {
            HooksSubcommand::Show => Ok(CommandOutput::Text(render_hooks_summary(
                self.storage_dir.as_deref(),
                self.tool_specs.as_ref(),
            )?)),
            HooksSubcommand::Open => {
                hooks_open_output(&context, self.storage_dir.as_deref()).map(CommandOutput::Text)
            }
        }
    }
}

#[async_trait]
impl Command for PrivacySettingsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<PrivacySettingsArgs>("privacy-settings", &invocation)?;
        match args.command.unwrap_or(PrivacySettingsSubcommand::Show) {
            PrivacySettingsSubcommand::Show => Ok(CommandOutput::Text(
                render_privacy_settings_summary(try_open_browser(PRIVACY_SETTINGS_URL)),
            )),
        }
    }
}

// ── Normalisation helpers ────────────────────────────────────────────────────

pub(super) fn normalize_vim_invocation(invocation: &CommandInvocation) -> CommandInvocation {
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

// ── Terminal setup ────────────────────────────────────────────────────────────

/// Terminals that natively decode Shift+Enter via the Kitty keyboard protocol,
/// so no extra keybinding setup is needed for multi-line input.
const NATIVE_CSIU_TERMINALS: &[&str] =
    &["ghostty", "kitty", "iTerm.app", "WezTerm", "WarpTerminal"];

/// Returns the value of `TERM_PROGRAM`, or `"unknown"` when the variable is
/// not set or is empty.  Used by `render_terminal_setup_notice` to tailor its advice.
pub(crate) fn detect_terminal_type() -> String {
    std::env::var("TERM_PROGRAM")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Returns `true` when `terminal` natively handles the Kitty keyboard protocol
/// (so Shift+Enter already works without any extra setup).
pub(super) fn is_native_csiu_terminal(terminal: &str) -> bool {
    NATIVE_CSIU_TERMINALS
        .iter()
        .any(|&t| t.eq_ignore_ascii_case(terminal))
}

/// Returns a short human-readable recommendation for the given terminal.
pub(super) fn terminal_setup_recommendation(terminal: &str) -> &'static str {
    match terminal {
        "Apple_Terminal" => {
            "Use Option+Enter (⌥↵) for newlines. Shift+Enter is not supported in Apple Terminal."
        }
        "vscode" | "cursor" | "windsurf" => {
            "Use Shift+Enter for newlines. Your editor's built-in terminal handles the keybinding."
        }
        t if is_native_csiu_terminal(t) => {
            "Your terminal natively supports Shift+Enter via the Kitty keyboard protocol — no extra setup needed."
        }
        _ => {
            "Use `/keybindings open` to add a Shift+Enter → insert_newline keybinding to the config file."
        }
    }
}

pub(super) fn render_terminal_setup_notice(storage_dir: Option<&Path>) -> String {
    let path = resolve_keybindings_path(storage_dir);
    let terminal = detect_terminal_type();
    let setup_needed = !is_native_csiu_terminal(&terminal)
        && terminal != "vscode"
        && terminal != "cursor"
        && terminal != "windsurf";
    let recommendation = terminal_setup_recommendation(&terminal);

    [
        "## Terminal Setup".into(),
        format!("terminal_type={terminal}"),
        format!("setup_needed={setup_needed}"),
        recommendation.into(),
        String::new(),
        "Use `/keybindings` to inspect the active shortcuts.".into(),
        format!(
            "Use `/keybindings open` to create or edit {} with the starter template, which includes a Shift+Enter -> insert_newline example.",
            path.display()
        ),
        "status=terminal keybinding template ready".into(),
    ]
    .join("\n")
}

// ── Keybindings helpers ───────────────────────────────────────────────────────

pub(crate) fn load_keybinding_resolver(storage_dir: Option<&Path>) -> Result<KeyBindingResolver> {
    let path = resolve_keybindings_path(storage_dir);
    if !path.exists() {
        return Ok(KeyBindingResolver::new());
    }
    let content = fs::read_to_string(&path)?;
    let bindings = parse_keybinding_overrides(&content).map_err(|error| {
        WonderError::validation(format!(
            "invalid keybindings config `{}`: {error}",
            path.display()
        ))
    })?;
    KeyBindingResolver::with_overrides(bindings).map_err(|error| {
        WonderError::validation(format!(
            "invalid keybindings overrides `{}`: {error}",
            path.display()
        ))
    })
}

pub(super) fn render_keybindings_summary(
    resolver: &KeyBindingResolver,
    storage_dir: Option<&Path>,
) -> String {
    let mut lines = vec![
        "## Keybindings".into(),
        format!(
            "config_path={}",
            resolve_keybindings_path(storage_dir).display()
        ),
        String::new(),
    ];
    for (context, title) in [
        (KeyBindingContext::Any, "Global"),
        (KeyBindingContext::Prompt, "Prompt"),
        (KeyBindingContext::VimInsert, "Vim insert"),
        (KeyBindingContext::VimNormal, "Vim normal"),
    ] {
        lines.push(format!("### {title}"));
        for line in render_binding_lines(resolver, context) {
            lines.push(format!("- {line}"));
        }
        lines.push(String::new());
    }
    lines.extend([
        "### Dialogs and pickers".into(),
        "- Up / Down: move the current selection".into(),
        "- Enter: confirm or close the active dialog".into(),
        "- Esc: cancel or dismiss the active dialog".into(),
        "- Permission prompt: Enter or y allows, n or Esc denies".into(),
        String::new(),
        "### Vim extras".into(),
        "- 0 / $: jump to line start or line end".into(),
        "- I / A: insert at line start or append at line end".into(),
        "- d{motion} / c{motion}: delete or change by motion".into(),
        "- counts like 3w and 2dw are supported".into(),
    ]);
    lines.join("\n")
}

fn render_binding_lines(resolver: &KeyBindingResolver, context: KeyBindingContext) -> Vec<String> {
    let mut by_key = BTreeMap::new();
    for binding in resolver.bindings() {
        if binding.context == context {
            by_key.insert(
                format_key_event(binding.event),
                format_resolved_key(binding.result),
            );
        }
    }
    if by_key.is_empty() {
        return vec!["(no bindings)".into()];
    }
    by_key
        .into_iter()
        .map(|(key, action)| format!("{key}: {action}"))
        .collect()
}

pub(super) fn resolve_keybindings_path(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("keybindings.json")
}

fn keybindings_open_output(context: &CommandContext, storage_dir: Option<&Path>) -> Result<String> {
    let path = resolve_keybindings_path(storage_dir);
    let existed = path.exists();
    ensure_keybindings_file(&path)?;
    if context.interactive {
        let mut lines = vec![
            "keybindings=ready".into(),
            format!("keybindings_path={}", path.display()),
            format!("created={}", !existed),
        ];
        if plan_editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push(format!("external_path={}", path.display()));
            lines.push("status=opening keybindings config".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/keybindings open`".into());
        }
        return Ok(lines.join("\n"));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(format!(
            concat!(
                "keybindings=ready\n",
                "keybindings_path={}\n",
                "created={}\n",
                "note=set VISUAL or EDITOR to enable `/keybindings open`"
            ),
            path.display(),
            !existed,
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(format!(
        concat!(
            "keybindings=ready\n",
            "keybindings_path={}\n",
            "created={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        path.display(),
        !existed,
        editor,
        status.success(),
    ))
}

pub(super) fn ensure_keybindings_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, render_keybindings_template())?;
    } else {
        OpenOptions::new().create(true).append(true).open(path)?;
    }
    Ok(())
}

fn render_keybindings_template() -> String {
    serde_json::to_string_pretty(&json!({
        "bindings": [
            {
                "context": "prompt",
                "key": "enter",
                "modifiers": ["shift"],
                "action": {
                    "kind": "edit",
                    "action": "insert_newline"
                }
            },
            {
                "context": "vim_normal",
                "key": "q",
                "action": {
                    "kind": "vim",
                    "action": "cancel_pending"
                }
            }
        ]
    }))
    .unwrap_or_else(|_| "{\"bindings\":[]}".into())
        + "\n"
}

// ── Hooks helpers ─────────────────────────────────────────────────────────────

const PRIVACY_SETTINGS_URL: &str = "https://claude.ai/settings/data-privacy-controls";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct HooksConfig {
    #[serde(default)]
    disable_all_hooks: bool,
    #[serde(default)]
    allow_managed_hooks_only: bool,
    #[serde(default)]
    hooks: BTreeMap<String, Vec<HookMatcherConfig>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct HookMatcherConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    matcher: Option<String>,
    #[serde(default)]
    hooks: Vec<HookActionConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HookActionConfig {
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Prompt {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Agent {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Http {
        url: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
}

pub(super) fn render_hooks_summary(
    storage_dir: Option<&Path>,
    tool_specs: &[ToolSpec],
) -> Result<String> {
    let path = resolve_hooks_path(storage_dir);
    let config = read_hooks_config(&path)?;
    let matcher_count = config.hooks.values().map(Vec::len).sum::<usize>();
    let hook_count = config
        .hooks
        .values()
        .flat_map(|matchers| matchers.iter())
        .map(|matcher| matcher.hooks.len())
        .sum::<usize>();
    let mut lines = vec![
        "## Hooks".into(),
        format!("config_path={}", path.display()),
        format!("available_tools={}", tool_specs.len()),
        format!("events={}", config.hooks.len()),
        format!("matchers={matcher_count}"),
        format!("hooks={hook_count}"),
        format!("disable_all_hooks={}", config.disable_all_hooks),
        format!(
            "allow_managed_hooks_only={}",
            config.allow_managed_hooks_only
        ),
    ];
    if config.hooks.is_empty() {
        lines.push("No hooks configured yet.".into());
    } else {
        for (event, matchers) in &config.hooks {
            let configured_hooks = matchers
                .iter()
                .map(|matcher| matcher.hooks.len())
                .sum::<usize>();
            lines.push(format!(
                "- {event}: {} matcher(s), {configured_hooks} hook(s)",
                matchers.len()
            ));
            if let Some(summary) = hook_event_summary(event) {
                lines.push(format!("  {summary}"));
            }
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Edit {} with `/hooks open` to review or update the config template.",
        path.display()
    ));
    lines.push(
        "Note: hook execution is not yet wired into the Rust runtime; this command currently provides config parity only."
            .into(),
    );
    Ok(lines.join("\n"))
}

fn hooks_open_output(context: &CommandContext, storage_dir: Option<&Path>) -> Result<String> {
    let path = resolve_hooks_path(storage_dir);
    let existed = path.exists();
    ensure_hooks_file(&path)?;
    if context.interactive {
        let mut lines = vec![
            "hooks=ready".into(),
            format!("hooks_path={}", path.display()),
            format!("created={}", !existed),
        ];
        if plan_editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push(format!("external_path={}", path.display()));
            lines.push("status=opening hooks config".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/hooks open`".into());
        }
        return Ok(lines.join("\n"));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(format!(
            concat!(
                "hooks=ready\n",
                "hooks_path={}\n",
                "created={}\n",
                "note=set VISUAL or EDITOR to enable `/hooks open`"
            ),
            path.display(),
            !existed,
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(format!(
        concat!(
            "hooks=ready\n",
            "hooks_path={}\n",
            "created={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        path.display(),
        !existed,
        editor,
        status.success(),
    ))
}

fn render_privacy_settings_summary(browser_launch_attempted: bool) -> String {
    [
        "## Privacy Settings".into(),
        format!("privacy_settings_url={PRIVACY_SETTINGS_URL}"),
        format!("browser_launch_attempted={browser_launch_attempted}"),
        "status=privacy settings opened".into(),
        format!("Review and manage your privacy settings at {PRIVACY_SETTINGS_URL}"),
    ]
    .join("\n")
}

pub(super) fn resolve_hooks_path(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("hooks.json")
}

pub(super) fn ensure_hooks_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, render_hooks_template())?;
    } else {
        OpenOptions::new().create(true).append(true).open(path)?;
    }
    Ok(())
}

fn read_hooks_config(path: &Path) -> Result<HooksConfig> {
    if !path.exists() {
        return Ok(HooksConfig::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        WonderError::validation(format!(
            "invalid hooks config `{}`: {error}",
            path.display()
        ))
    })
}

fn render_hooks_template() -> String {
    serde_json::to_string_pretty(&json!({
        "disable_all_hooks": false,
        "allow_managed_hooks_only": false,
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "echo auditing bash tool invocation"
                        }
                    ]
                }
            ],
            "Notification": [
                {
                    "matcher": "auth_success",
                    "hooks": [
                        {
                            "type": "http",
                            "url": "https://example.invalid/hooks/notify"
                        }
                    ]
                }
            ]
        }
    }))
    .unwrap_or_else(|_| "{\"hooks\":{}}".into())
        + "\n"
}

fn hook_event_summary(event: &str) -> Option<&'static str> {
    match event {
        "PreToolUse" => Some("Before tool execution; match on tool_name."),
        "PostToolUse" => Some("After tool execution; match on tool_name."),
        "PostToolUseFailure" => Some("After a tool fails; match on tool_name."),
        "PermissionDenied" => Some("After auto mode denies a tool call; match on tool_name."),
        "Notification" => Some("When notifications are sent; match on notification_type."),
        "UserPromptSubmit" => Some("When the user submits a prompt."),
        "SessionStart" => Some("When a session starts; match on source."),
        "Stop" => Some("Right before the assistant concludes a response."),
        "StopFailure" => Some("When a turn ends because of an API error."),
        "SubagentStart" => Some("When a subagent starts; match on agent_type."),
        "SubagentStop" => Some("Right before a subagent concludes; match on agent_type."),
        "PreCompact" => Some("Before compaction; match on trigger."),
        "PostCompact" => Some("After compaction; match on trigger."),
        "SessionEnd" => Some("When a session ends; match on reason."),
        "PermissionRequest" => Some("When a permission dialog is displayed; match on tool_name."),
        "Setup" => Some("Repo setup hooks for init and maintenance."),
        "TeammateIdle" => Some("When a teammate is about to go idle."),
        "TaskCreated" => Some("When a task is created."),
        "TaskCompleted" => Some("When a task is completed."),
        "Elicitation" => Some("When an MCP server requests user input."),
        "ElicitationResult" => Some("After a user responds to an MCP elicitation."),
        "ConfigChange" => Some("When configuration files change during a session."),
        "InstructionsLoaded" => Some("When an instruction file is loaded."),
        "WorktreeCreate" => Some("When a worktree should be created."),
        "WorktreeRemove" => Some("When a worktree should be removed."),
        "CwdChanged" => Some("After the working directory changes."),
        "FileChanged" => Some("When a watched file changes."),
        _ => None,
    }
}

// ── Keybinding parsing ────────────────────────────────────────────────────────

fn parse_keybinding_overrides(content: &str) -> std::result::Result<Vec<KeyBinding>, String> {
    let value: Value = serde_json::from_str(content).map_err(|error| error.to_string())?;
    let bindings = value
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing `bindings` array".to_string())?;
    bindings
        .iter()
        .enumerate()
        .map(|(index, entry)| parse_keybinding_entry(index, entry))
        .collect()
}

fn parse_keybinding_entry(index: usize, value: &Value) -> std::result::Result<KeyBinding, String> {
    let context = parse_keybinding_context(
        value
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("binding[{index}] missing `context`"))?,
    )?;
    let event = KeyEvent {
        code: parse_key_code(
            value
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("binding[{index}] missing `key`"))?,
        )?,
        modifiers: parse_key_modifiers(value.get("modifiers"))?,
    };
    let result = parse_keybinding_result(
        value
            .get("action")
            .ok_or_else(|| format!("binding[{index}] missing `action`"))?,
    )?;
    Ok(KeyBinding {
        context,
        event,
        result,
    })
}

fn parse_keybinding_context(value: &str) -> std::result::Result<KeyBindingContext, String> {
    match value {
        "any" | "global" => Ok(KeyBindingContext::Any),
        "prompt" => Ok(KeyBindingContext::Prompt),
        "vim_insert" | "insert" => Ok(KeyBindingContext::VimInsert),
        "vim_normal" | "normal" => Ok(KeyBindingContext::VimNormal),
        other => Err(format!("unknown binding context `{other}`")),
    }
}

fn parse_key_code(value: &str) -> std::result::Result<KeyCode, String> {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "backspace" => Ok(KeyCode::Backspace),
        "enter" => Ok(KeyCode::Enter),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" => Ok(KeyCode::PageUp),
        "pagedown" => Ok(KeyCode::PageDown),
        "tab" => Ok(KeyCode::Tab),
        "backtab" => Ok(KeyCode::BackTab),
        "delete" => Ok(KeyCode::Delete),
        "insert" => Ok(KeyCode::Insert),
        "esc" | "escape" => Ok(KeyCode::Esc),
        _ if value.len() == 1 => Ok(KeyCode::Char(value.chars().next().unwrap_or_default())),
        _ if lower.starts_with('f') => lower[1..]
            .parse::<u8>()
            .map(KeyCode::F)
            .map_err(|_| format!("unknown key `{value}`")),
        _ => Err(format!("unknown key `{value}`")),
    }
}

fn parse_key_modifiers(value: Option<&Value>) -> std::result::Result<KeyModifiers, String> {
    let mut modifiers = KeyModifiers::default();
    let Some(value) = value else {
        return Ok(modifiers);
    };
    let entries = value
        .as_array()
        .ok_or_else(|| "`modifiers` must be an array".to_string())?;
    for modifier in entries {
        match modifier
            .as_str()
            .ok_or_else(|| "modifier entries must be strings".to_string())?
        {
            "control" | "ctrl" => modifiers.control = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            other => return Err(format!("unknown modifier `{other}`")),
        }
    }
    Ok(modifiers)
}

fn parse_keybinding_result(value: &Value) -> std::result::Result<ResolvedKey, String> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "action missing `kind`".to_string())?;
    match kind {
        "system" => Ok(ResolvedKey::System(parse_system_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "system action missing `action`".to_string())?,
        )?)),
        "edit" => Ok(ResolvedKey::Edit(parse_edit_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "edit action missing `action`".to_string())?,
        )?)),
        "vim" => Ok(ResolvedKey::Vim(parse_vim_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "vim action missing `action`".to_string())?,
        )?)),
        "insert_char" => {
            let ch = value
                .get("char")
                .and_then(Value::as_str)
                .and_then(|raw| raw.chars().next())
                .ok_or_else(|| "insert_char action missing `char`".to_string())?;
            Ok(ResolvedKey::InsertChar(ch))
        }
        other => Err(format!("unknown action kind `{other}`")),
    }
}

fn parse_system_action(value: &str) -> std::result::Result<SystemAction, String> {
    match value {
        "interrupt" => Ok(SystemAction::Interrupt),
        "redraw" => Ok(SystemAction::Redraw),
        "history_search" => Ok(SystemAction::HistorySearch),
        "open_global_search" => Ok(SystemAction::OpenGlobalSearch),
        "expand_tool_output" => Ok(SystemAction::ExpandToolOutput),
        other => Err(format!("unknown system action `{other}`")),
    }
}

fn parse_edit_action(value: &str) -> std::result::Result<EditAction, String> {
    match value {
        "move_left" => Ok(EditAction::Move(Motion::Left)),
        "move_right" => Ok(EditAction::Move(Motion::Right)),
        "move_up" => Ok(EditAction::Move(Motion::Up)),
        "move_down" => Ok(EditAction::Move(Motion::Down)),
        "move_line_start" => Ok(EditAction::Move(Motion::LineStart)),
        "move_line_end" => Ok(EditAction::Move(Motion::LineEnd)),
        "move_word_forward" => Ok(EditAction::Move(Motion::WordForward)),
        "move_word_backward" => Ok(EditAction::Move(Motion::WordBackward)),
        "move_word_end" => Ok(EditAction::Move(Motion::WordEnd)),
        "backspace" => Ok(EditAction::Backspace),
        "delete" => Ok(EditAction::Delete),
        "insert_newline" => Ok(EditAction::InsertNewline),
        other => Err(format!("unknown edit action `{other}`")),
    }
}

fn parse_vim_action(value: &str) -> std::result::Result<TuiVimCommand, String> {
    match value {
        "enter_insert_mode" => Ok(TuiVimCommand::EnterInsertMode),
        "enter_normal_mode" => Ok(TuiVimCommand::EnterNormalMode),
        "append_after_cursor" => Ok(TuiVimCommand::AppendAfterCursor),
        "append_line_end" => Ok(TuiVimCommand::AppendLineEnd),
        "insert_line_start" => Ok(TuiVimCommand::InsertLineStart),
        "delete_char" => Ok(TuiVimCommand::DeleteChar),
        "start_delete" => Ok(TuiVimCommand::StartDelete),
        "start_change" => Ok(TuiVimCommand::StartChange),
        "cancel_pending" => Ok(TuiVimCommand::CancelPending),
        other => Err(format!("unknown vim action `{other}`")),
    }
}

fn format_key_event(event: KeyEvent) -> String {
    let mut parts = Vec::new();
    if event.modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if event.modifiers.alt {
        parts.push("Alt".to_string());
    }
    if event.modifiers.shift {
        parts.push("Shift".to_string());
    }
    parts.push(match event.code {
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Char(ch) => {
            if (event.modifiers.control || event.modifiers.alt || event.modifiers.shift)
                && ch.is_ascii_alphabetic()
            {
                ch.to_ascii_uppercase().to_string()
            } else {
                ch.to_string()
            }
        }
        KeyCode::F(index) => format!("F{index}"),
        KeyCode::Null => "Null".into(),
    });
    parts.join("-")
}

fn format_resolved_key(result: ResolvedKey) -> String {
    match result {
        ResolvedKey::Edit(action) => match action {
            EditAction::Move(Motion::Left) => "move left".into(),
            EditAction::Move(Motion::Right) => "move right".into(),
            EditAction::Move(Motion::Up) => "move up".into(),
            EditAction::Move(Motion::Down) => "move down".into(),
            EditAction::Move(Motion::LineStart) => "move to line start".into(),
            EditAction::Move(Motion::LineEnd) => "move to line end".into(),
            EditAction::Move(Motion::FirstNonBlank) => "move to first non-blank".into(),
            EditAction::Move(Motion::WordForward) => "move forward by word".into(),
            EditAction::Move(Motion::WordBackward) => "move backward by word".into(),
            EditAction::Move(Motion::WordEnd) => "move to word end".into(),
            EditAction::Backspace => "delete backward".into(),
            EditAction::Delete => "delete forward".into(),
            EditAction::InsertNewline => "submit / insert newline".into(),
            EditAction::InsertLiteralNewline => "insert literal newline".into(),
        },
        ResolvedKey::InsertChar(ch) => format!("insert `{ch}`"),
        ResolvedKey::System(SystemAction::Interrupt) => "interrupt or exit".into(),
        ResolvedKey::System(SystemAction::Redraw) => "redraw terminal".into(),
        ResolvedKey::System(SystemAction::HistorySearch) => "recall previous prompt".into(),
        ResolvedKey::System(SystemAction::OpenGlobalSearch) => "search workspace".into(),
        ResolvedKey::System(SystemAction::ExpandToolOutput) => {
            "expand or collapse tool output".into()
        }
        ResolvedKey::Vim(command) => match command {
            TuiVimCommand::EnterInsertMode => "enter insert mode".into(),
            TuiVimCommand::EnterNormalMode => "enter normal mode".into(),
            TuiVimCommand::AppendAfterCursor => "append after cursor".into(),
            TuiVimCommand::AppendLineEnd => "append at line end".into(),
            TuiVimCommand::InsertLineStart => "insert at line start".into(),
            TuiVimCommand::DeleteChar => "delete char under cursor".into(),
            TuiVimCommand::StartDelete => "start delete operator".into(),
            TuiVimCommand::StartChange => "start change operator".into(),
            TuiVimCommand::CancelPending => "cancel pending operator".into(),
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use futures::executor::block_on;
    use wonder_of_u_agent::{AgentSettings, ProviderResolver, SettingsStore};
    use wonder_of_u_core::{
        Command, CommandContext, CommandInvocation, CommandOutput, FeatureSet, PermissionMode,
        SessionId,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::{
        OptimizeTonkenAction, PrivacySettingsCommand, TerminalSetupCommand, detect_terminal_type,
        ensure_hooks_file, ensure_keybindings_file, is_native_csiu_terminal,
        load_keybinding_resolver, normalize_color_invocation, normalize_effort_invocation,
        normalize_theme_invocation, normalize_vim_invocation, parse_brief_action,
        parse_effort_level_name, parse_fast_action, parse_optimize_tonken_action,
        parse_session_color_name, render_brief_status, render_brief_transition,
        render_color_status, render_color_transition, render_effort_status,
        render_effort_transition, render_fast_status, render_fast_transition, render_hooks_summary,
        render_keybindings_summary, render_optimize_tonken_status,
        render_optimize_tonken_transition, render_privacy_settings_summary,
        render_terminal_setup_notice, render_theme_status, resolve_hooks_path,
        resolve_keybindings_path, terminal_setup_recommendation, write_persisted_effort,
        write_persisted_fast,
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
    fn keybindings_summary_lists_supported_sections() {
        let rendered =
            render_keybindings_summary(&load_keybinding_resolver(None).expect("resolver"), None);

        assert!(rendered.contains("## Keybindings"));
        assert!(rendered.contains("### Global"));
        assert!(rendered.contains("### Prompt"));
        assert!(rendered.contains("### Vim insert"));
        assert!(rendered.contains("### Dialogs and pickers"));
        assert!(rendered.contains("### Vim normal"));
        assert!(rendered.contains("Ctrl-R"));
        assert!(rendered.contains("d{motion} / c{motion}"));
    }

    #[test]
    fn keybindings_template_can_be_loaded_as_overrides() {
        let dir = unique_test_dir("workflow-keybindings-template");
        let path = dir.join("config/keybindings.json");
        ensure_keybindings_file(&path).expect("write template");

        let resolver = load_keybinding_resolver(Some(dir.as_path())).expect("load resolver");
        let rendered = render_keybindings_summary(&resolver, Some(dir.as_path()));

        assert!(rendered.contains("Shift-Enter"));
        assert!(rendered.contains("q: cancel pending operator"));
    }

    #[test]
    fn keybindings_open_hints_external_editor_when_interactive() {
        let dir = unique_test_dir("workflow-keybindings-open");
        let _editor = EnvVarGuard::set("EDITOR", "true");
        let context = CommandContext {
            session_id: SessionId::new(),
            cwd: dir.clone(),
            features: FeatureSet::default(),
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
        };
        let output =
            super::keybindings_open_output(&context, Some(dir.as_path())).expect("open output");

        assert!(output.contains("open_external=true"));
        assert!(output.contains(&format!(
            "external_path={}",
            resolve_keybindings_path(Some(dir.as_path())).display()
        )));
    }

    #[test]
    fn hooks_summary_reports_template_events() {
        let dir = unique_test_dir("workflow-hooks-template");
        let path = dir.join("config/hooks.json");
        ensure_hooks_file(&path).expect("write hooks template");

        let rendered = render_hooks_summary(Some(dir.as_path()), &[]).expect("render hooks");

        assert!(rendered.contains("## Hooks"));
        assert!(rendered.contains("events=2"));
        assert!(rendered.contains("PreToolUse"));
        assert!(rendered.contains("Notification"));
        assert!(rendered.contains("config parity only"));
    }

    #[test]
    fn hooks_open_hints_external_editor_when_interactive() {
        let dir = unique_test_dir("workflow-hooks-open");
        let _editor = EnvVarGuard::set("EDITOR", "true");
        let context = CommandContext {
            session_id: SessionId::new(),
            cwd: dir.clone(),
            features: FeatureSet::default(),
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
        };
        let output = super::hooks_open_output(&context, Some(dir.as_path())).expect("open output");

        assert!(output.contains("open_external=true"));
        assert!(output.contains(&format!(
            "external_path={}",
            resolve_hooks_path(Some(dir.as_path())).display()
        )));
    }

    #[test]
    fn privacy_settings_summary_points_to_web_controls() {
        let rendered = render_privacy_settings_summary(false);

        assert!(rendered.contains("## Privacy Settings"));
        assert!(rendered.contains("status=privacy settings opened"));
        assert!(rendered.contains("browser_launch_attempted=false"));
        assert!(rendered.contains("https://claude.ai/settings/data-privacy-controls"));
    }

    #[test]
    fn privacy_settings_command_does_not_launch_browser_under_tests() {
        let output = block_on(PrivacySettingsCommand::new().execute(
            test_context(Path::new("/workspace")),
            CommandInvocation {
                name: "privacy-settings".into(),
                args: String::new(),
                raw: "/privacy-settings".into(),
            },
        ))
        .expect("run privacy settings command");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };

        assert!(text.contains("browser_launch_attempted=false"));
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

    // ── terminal detection / per-terminal advice ─────────────────────────────

    /// With TERM_PROGRAM unset, detect_terminal_type returns "unknown".
    #[test]
    fn detect_terminal_type_returns_unknown_when_unset() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        assert_eq!(detect_terminal_type(), "unknown");
    }

    /// With TERM_PROGRAM set, detect_terminal_type echoes its value.
    #[test]
    fn detect_terminal_type_echoes_term_program() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "ghostty");
        assert_eq!(detect_terminal_type(), "ghostty");
    }

    /// Kitty-protocol-native terminals must report setup_needed=false.
    #[test]
    fn native_csiu_terminals_report_setup_not_needed() {
        for terminal in &["ghostty", "kitty", "WezTerm", "WarpTerminal"] {
            assert!(
                is_native_csiu_terminal(terminal),
                "{terminal} should be recognised as a native Kitty-protocol terminal"
            );
        }
    }

    /// iTerm.app (mixed case) must also be recognised.
    #[test]
    fn iterm_app_is_native_csiu_terminal() {
        assert!(is_native_csiu_terminal("iTerm.app"));
    }

    /// Apple Terminal gets the Option+Enter advice.
    #[test]
    fn terminal_setup_apple_terminal_shows_option_enter_advice() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "Apple_Terminal");
        let dir = unique_test_dir("workflow-terminal-setup-apple");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            rendered.contains("Option+Enter"),
            "Apple Terminal advice must mention Option+Enter; got:\n{rendered}"
        );
        assert!(
            rendered.contains("setup_needed=true"),
            "Apple Terminal must require setup; got:\n{rendered}"
        );
    }

    /// Ghostty (a native Kitty-protocol terminal) must report setup_needed=false.
    #[test]
    fn terminal_setup_ghostty_reports_no_setup_needed() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "ghostty");
        let dir = unique_test_dir("workflow-terminal-setup-ghostty");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            rendered.contains("setup_needed=false"),
            "ghostty must not need setup; got:\n{rendered}"
        );
    }

    /// VSCode reports setup_needed=false and Shift+Enter advice.
    #[test]
    fn terminal_setup_vscode_shows_shift_enter_advice() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "vscode");
        let dir = unique_test_dir("workflow-terminal-setup-vscode");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            terminal_setup_recommendation("vscode").contains("Shift+Enter"),
            "VSCode advice must mention Shift+Enter"
        );
        assert!(
            rendered.contains("terminal_type=vscode"),
            "must include terminal_type=vscode; got:\n{rendered}"
        );
    }

    #[test]
    fn terminal_setup_notice_points_to_keybindings_flow() {
        let dir = unique_test_dir("workflow-terminal-setup-notice");
        // Run with TERM_PROGRAM unset so the output is deterministic regardless
        // of the CI environment.
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));

        assert!(rendered.starts_with("## Terminal Setup"));
        assert!(
            rendered.contains("terminal_type="),
            "must include terminal_type field; got:\n{rendered}"
        );
        assert!(
            rendered.contains("setup_needed="),
            "must include setup_needed field; got:\n{rendered}"
        );
        assert!(rendered.contains("Use `/keybindings` to inspect the active shortcuts."));
        assert!(rendered.contains(&format!(
            "Use `/keybindings open` to create or edit {}",
            resolve_keybindings_path(Some(dir.as_path())).display()
        )));
    }

    #[test]
    fn terminal_setup_command_renders_notice() {
        let dir = unique_test_dir("workflow-terminal-setup-command");
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        let output = block_on(TerminalSetupCommand::new(Some(dir.clone())).execute(
            test_context(&dir),
            CommandInvocation {
                name: "terminal-setup".into(),
                args: String::new(),
                raw: "/terminal-setup".into(),
            },
        ))
        .expect("terminal setup output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("## Terminal Setup"));
        assert!(
            text.contains("terminal_type="),
            "must include terminal_type field; got:\n{text}"
        );
        assert!(
            text.contains("setup_needed="),
            "must include setup_needed field; got:\n{text}"
        );
        assert!(text.contains("Shift+Enter -> insert_newline"));
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

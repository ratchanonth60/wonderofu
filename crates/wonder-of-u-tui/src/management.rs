//! Renderer-agnostic management screen models for agents, MCP, tasks, settings,
//! help, and sandbox flows.

use std::collections::BTreeMap;

use wonder_of_u_core::TaskStatus;

use crate::{
    catalog::{
        CatalogAction, CatalogEntry, CatalogHeader, CatalogModel, CatalogSection, CatalogTone,
        DetailPreview, EmptyState, PanelDescriptor, StatusBadge,
    },
    dialog::{DialogActionView, DialogView},
    message::TaskActivitySummaryView,
    permission::PermissionSummaryView,
    security::{ManagedSettingsSecurityDialogView, TrustDialogView},
};

/// A reusable wrapper around existing security review surfaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecurityReviewView {
    Workspace(TrustDialogView),
    ManagedSettings(ManagedSettingsSecurityDialogView),
}

impl SecurityReviewView {
    #[must_use]
    pub fn title(&self) -> &str {
        match self {
            Self::Workspace(view) => &view.title,
            Self::ManagedSettings(view) => &view.title,
        }
    }

    #[must_use]
    pub fn to_dialog_view(&self) -> DialogView {
        match self {
            Self::Workspace(view) => view.to_dialog_view(),
            Self::ManagedSettings(view) => view.to_dialog_view(),
        }
    }
}

/// A titled detail block used by task dialogs and management previews.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DetailSection {
    pub panel: PanelDescriptor,
    pub badges: Vec<StatusBadge>,
    pub lines: Vec<String>,
}

impl DetailSection {
    #[must_use]
    pub fn new(panel: PanelDescriptor) -> Self {
        Self {
            panel,
            badges: Vec::new(),
            lines: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    #[must_use]
    pub fn with_badge(mut self, badge: StatusBadge) -> Self {
        self.badges.push(badge);
        self
    }
}

/// A generic list/detail entry used for settings, help, and sandbox panes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PaneCatalogEntryView {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub lines: Vec<String>,
    pub badges: Vec<StatusBadge>,
    pub keywords: Vec<String>,
}

impl PaneCatalogEntryView {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            lines: Vec::new(),
            badges: Vec::new(),
            keywords: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    #[must_use]
    pub fn with_badge(mut self, badge: StatusBadge) -> Self {
        self.badges.push(badge);
        self
    }

    #[must_use]
    pub fn with_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keywords.push(keyword.into());
        self
    }

    fn into_catalog_entry(self) -> CatalogEntry {
        let mut entry = CatalogEntry::new(self.id, self.label);
        if let Some(description) = self.description.clone() {
            entry = entry.with_description(description);
        }
        for badge in self.badges.iter().cloned() {
            entry = entry.with_badge(badge);
        }
        for keyword in self.keywords {
            entry = entry.with_keyword(keyword);
        }
        if !self.lines.is_empty() || self.description.is_some() || !self.badges.is_empty() {
            let mut preview = DetailPreview::new(
                PanelDescriptor::new(entry.label.clone())
                    .with_subtitle(self.description.unwrap_or_default()),
            );
            preview.lines = self.lines;
            preview.badges = self.badges;
            entry = entry.with_preview(preview);
        }
        entry
    }
}

/// Where an agent definition came from in the management screens.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AgentCatalogSource {
    Project,
    Local,
    User,
    Plugin,
    BuiltIn,
}

impl AgentCatalogSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Project => "Project agents",
            Self::Local => "Local agents",
            Self::User => "User agents",
            Self::Plugin => "Plugin agents",
            Self::BuiltIn => "Built-in agents",
        }
    }
}

/// A single agent entry rendered inside the agent management catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentCatalogEntryView {
    pub id: String,
    pub name: String,
    pub source: AgentCatalogSource,
    pub description: Option<String>,
    pub model: Option<String>,
    pub memory: Option<String>,
    pub tools_summary: Option<String>,
    pub prompt_summary: Option<String>,
    pub overridden_by: Option<String>,
    pub editable: bool,
}

impl AgentCatalogEntryView {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, source: AgentCatalogSource) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            source,
            description: None,
            model: None,
            memory: None,
            tools_summary: None,
            prompt_summary: None,
            overridden_by: None,
            editable: true,
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    #[must_use]
    pub fn with_memory(mut self, memory: impl Into<String>) -> Self {
        self.memory = Some(memory.into());
        self
    }

    #[must_use]
    pub fn with_tools_summary(mut self, tools_summary: impl Into<String>) -> Self {
        self.tools_summary = Some(tools_summary.into());
        self
    }

    #[must_use]
    pub fn with_prompt_summary(mut self, prompt_summary: impl Into<String>) -> Self {
        self.prompt_summary = Some(prompt_summary.into());
        self
    }

    #[must_use]
    pub fn overridden_by(mut self, source: impl Into<String>) -> Self {
        self.overridden_by = Some(source.into());
        self
    }

    #[must_use]
    pub const fn read_only(mut self) -> Self {
        self.editable = false;
        self
    }

    fn into_catalog_entry(self) -> CatalogEntry {
        let mut entry = CatalogEntry::new(self.id, self.name.clone());
        if let Some(description) = self.description.clone() {
            entry = entry.with_description(description);
        }

        entry = entry.with_keyword(self.source.label());
        if let Some(model) = &self.model {
            entry = entry.with_keyword(model.clone());
        }
        if let Some(memory) = &self.memory {
            entry = entry.with_keyword(memory.clone());
        }

        let mut preview = DetailPreview::new(
            PanelDescriptor::new(self.name).with_subtitle(
                self.description
                    .clone()
                    .unwrap_or_else(|| self.source.label().to_string()),
            ),
        );
        preview
            .lines
            .push(format!("Source: {}", self.source.label()));
        if let Some(model) = self.model {
            preview.lines.push(format!("Model: {model}"));
            entry = entry.with_badge(StatusBadge::new("model", CatalogTone::Info));
        }
        if let Some(memory) = self.memory {
            preview.lines.push(format!("Memory: {memory}"));
        }
        if let Some(tools_summary) = self.tools_summary {
            preview.lines.push(format!("Tools: {tools_summary}"));
        }
        if let Some(prompt_summary) = self.prompt_summary {
            preview.lines.push(format!("Prompt: {prompt_summary}"));
        }
        if let Some(source) = self.overridden_by {
            preview
                .badges
                .push(StatusBadge::new("shadowed", CatalogTone::Warning));
            preview
                .lines
                .push(format!("Override: shadowed by {source}"));
        }
        if !self.editable {
            entry = entry
                .with_badge(StatusBadge::disabled())
                .disabled("Built-in agents cannot be edited");
        } else {
            entry
                .actions
                .push(CatalogAction::new("Edit").with_shortcut("Enter"));
        }
        entry.with_preview(preview)
    }
}

/// The progress state of a new-agent wizard step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentWizardStepStatus {
    Complete,
    Current,
    Pending,
}

impl AgentWizardStepStatus {
    #[must_use]
    pub fn badge(self) -> StatusBadge {
        match self {
            Self::Complete => StatusBadge {
                label: "done".into(),
                tone: CatalogTone::Success,
            },
            Self::Current => StatusBadge {
                label: "current".into(),
                tone: CatalogTone::Accent,
            },
            Self::Pending => StatusBadge {
                label: "pending".into(),
                tone: CatalogTone::Muted,
            },
        }
    }
}

/// One wizard step in the new-agent flow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentWizardStepView {
    pub label: String,
    pub summary: Option<String>,
    pub status: AgentWizardStepStatus,
}

impl AgentWizardStepView {
    #[must_use]
    pub fn new(label: impl Into<String>, status: AgentWizardStepStatus) -> Self {
        Self {
            label: label.into(),
            summary: None,
            status,
        }
    }

    #[must_use]
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

/// Summary of the new-agent wizard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentWizardView {
    pub panel: PanelDescriptor,
    pub steps: Vec<AgentWizardStepView>,
    pub actions: Vec<CatalogAction>,
}

impl AgentWizardView {
    #[must_use]
    pub fn new(steps: Vec<AgentWizardStepView>) -> Self {
        Self {
            panel: PanelDescriptor::new("Create new agent")
                .with_subtitle("Wizard progress and current inputs"),
            steps,
            actions: vec![
                CatalogAction::new("Continue")
                    .with_shortcut("Enter")
                    .primary(),
                CatalogAction::new("Cancel").with_shortcut("Esc"),
            ],
        }
    }

    #[must_use]
    pub fn as_section(&self) -> DetailSection {
        let mut section = DetailSection::new(self.panel.clone());
        for step in &self.steps {
            section.badges.push(step.status.badge());
            section.lines.push(match &step.summary {
                Some(summary) => format!("{} — {}", step.label, summary),
                None => step.label.clone(),
            });
        }
        section
    }
}

/// Agent list screen plus optional wizard summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentManagementView {
    pub catalog: CatalogModel,
    pub wizard: Option<AgentWizardView>,
}

impl AgentManagementView {
    #[must_use]
    pub fn from_entries(
        source_label: impl Into<String>,
        entries: Vec<AgentCatalogEntryView>,
        change_count: usize,
        create_enabled: bool,
    ) -> Self {
        let source_label = source_label.into();
        let mut header = CatalogHeader::new("Agents").with_subtitle(source_label);
        if create_enabled {
            header.actions.push(
                CatalogAction::new("Create new agent")
                    .with_shortcut("c")
                    .primary(),
            );
        }
        if change_count > 0 {
            header.badges.push(StatusBadge::new(
                format!(
                    "{change_count} pending change{}",
                    if change_count == 1 { "" } else { "s" }
                ),
                CatalogTone::Warning,
            ));
        }

        let mut grouped = BTreeMap::<AgentCatalogSource, Vec<CatalogEntry>>::new();
        for entry in entries {
            grouped
                .entry(entry.source)
                .or_default()
                .push(entry.into_catalog_entry());
        }

        let sections = grouped
            .into_iter()
            .map(|(source, entries)| {
                CatalogSection::new(PanelDescriptor::new(source.label()), entries)
            })
            .collect::<Vec<_>>();

        Self {
            catalog: CatalogModel::new(
                header,
                PanelDescriptor::new("Agents").with_subtitle("Visible agents"),
                PanelDescriptor::new("Agent details").with_subtitle("Current selection"),
                EmptyState::new("No agents found")
                    .with_body_line(
                        "Create specialized agents so Claude can delegate work with focused prompts and tools.",
                    )
                    .with_body_line("Built-in agents stay available automatically."),
                EmptyState::new("No matching agents")
                    .with_body_line("Clear or change the filter to see more agents."),
                sections,
            ),
            wizard: None,
        }
    }

    #[must_use]
    pub fn with_wizard(mut self, wizard: AgentWizardView) -> Self {
        self.wizard = Some(wizard);
        self
    }
}

/// High-level grouping used by MCP management catalogs.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum McpCatalogGroup {
    Project,
    Local,
    User,
    Enterprise,
    ClaudeAi,
    Agent,
    BuiltIn,
}

impl McpCatalogGroup {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Project => "Project MCPs",
            Self::Local => "Local MCPs",
            Self::User => "User MCPs",
            Self::Enterprise => "Enterprise MCPs",
            Self::ClaudeAi => "Claude.ai connectors",
            Self::Agent => "Agent MCPs",
            Self::BuiltIn => "Built-in MCPs",
        }
    }
}

/// MCP connection state summarized in list/detail screens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpConnectionState {
    Connected,
    Pending,
    NeedsAuth,
    Failed,
    Disabled,
    AgentOnly,
}

impl McpConnectionState {
    #[must_use]
    pub fn badge(self) -> StatusBadge {
        match self {
            Self::Connected => StatusBadge {
                label: "connected".into(),
                tone: CatalogTone::Success,
            },
            Self::Pending => StatusBadge {
                label: "connecting".into(),
                tone: CatalogTone::Info,
            },
            Self::NeedsAuth => StatusBadge {
                label: "needs auth".into(),
                tone: CatalogTone::Warning,
            },
            Self::Failed => StatusBadge {
                label: "failed".into(),
                tone: CatalogTone::Danger,
            },
            Self::Disabled => StatusBadge {
                label: "disabled".into(),
                tone: CatalogTone::Muted,
            },
            Self::AgentOnly => StatusBadge {
                label: "agent-only".into(),
                tone: CatalogTone::Muted,
            },
        }
    }
}

/// Authentication state for remote or agent-only MCP servers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpAuthStatus {
    NotRequired,
    Available,
    Required,
    Authenticated,
    BrowserFlow,
    Clearing,
}

impl McpAuthStatus {
    #[must_use]
    pub fn badge(self) -> Option<StatusBadge> {
        match self {
            Self::NotRequired => None,
            Self::Available => Some(StatusBadge {
                label: "auth available".into(),
                tone: CatalogTone::Info,
            }),
            Self::Required => Some(StatusBadge {
                label: "auth required".into(),
                tone: CatalogTone::Warning,
            }),
            Self::Authenticated => Some(StatusBadge {
                label: "authenticated".into(),
                tone: CatalogTone::Success,
            }),
            Self::BrowserFlow => Some(StatusBadge {
                label: "browser open".into(),
                tone: CatalogTone::Accent,
            }),
            Self::Clearing => Some(StatusBadge {
                label: "clearing auth".into(),
                tone: CatalogTone::Warning,
            }),
        }
    }
}

/// A single MCP server row plus its preview data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpServerEntryView {
    pub id: String,
    pub name: String,
    pub group: McpCatalogGroup,
    pub transport: String,
    pub connection: McpConnectionState,
    pub auth: McpAuthStatus,
    pub description: Option<String>,
    pub location: Option<String>,
    pub command: Option<String>,
    pub endpoint: Option<String>,
    pub tools_count: usize,
    pub prompts_count: usize,
    pub resources_count: usize,
    pub agent_sources: Vec<String>,
}

impl McpServerEntryView {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        group: McpCatalogGroup,
        transport: impl Into<String>,
        connection: McpConnectionState,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            group,
            transport: transport.into(),
            connection,
            auth: McpAuthStatus::NotRequired,
            description: None,
            location: None,
            command: None,
            endpoint: None,
            tools_count: 0,
            prompts_count: 0,
            resources_count: 0,
            agent_sources: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_location(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    #[must_use]
    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    #[must_use]
    pub fn with_capabilities(
        mut self,
        tools_count: usize,
        prompts_count: usize,
        resources_count: usize,
    ) -> Self {
        self.tools_count = tools_count;
        self.prompts_count = prompts_count;
        self.resources_count = resources_count;
        self
    }

    #[must_use]
    pub fn with_agent_sources(mut self, agent_sources: Vec<String>) -> Self {
        self.agent_sources = agent_sources;
        self
    }

    #[must_use]
    pub const fn with_auth(mut self, auth: McpAuthStatus) -> Self {
        self.auth = auth;
        self
    }

    fn into_catalog_entry(self) -> CatalogEntry {
        let mut entry = CatalogEntry::new(self.id, self.name.clone());
        if let Some(description) = self.description.clone() {
            entry = entry.with_description(description);
        }
        entry = entry.with_badge(self.connection.badge());
        if let Some(auth_badge) = self.auth.badge() {
            entry = entry.with_badge(auth_badge);
        }
        entry = entry.with_keyword(self.group.label());
        entry = entry.with_keyword(self.transport.clone());

        let mut preview = DetailPreview::new(
            PanelDescriptor::new(self.name).with_subtitle(self.transport.clone()),
        );
        preview.badges = entry.badges.clone();
        preview.lines.push(format!("Transport: {}", self.transport));
        if let Some(location) = self.location {
            preview.lines.push(format!("Config: {location}"));
        }
        if let Some(command) = self.command {
            preview.lines.push(format!("Command: {command}"));
        }
        if let Some(endpoint) = self.endpoint {
            preview.lines.push(format!("Endpoint: {endpoint}"));
        }
        preview.lines.push(format!(
            "Capabilities: {} tools · {} prompts · {} resources",
            self.tools_count, self.prompts_count, self.resources_count
        ));
        if !self.agent_sources.is_empty() {
            preview
                .lines
                .push(format!("Used by: {}", self.agent_sources.join(", ")));
        }
        entry.with_preview(preview)
    }
}

/// Reconnect state for explicit reconnect flows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpReconnectState {
    Running,
    Reconnected,
    NeedsAuth,
    Failed,
}

/// Summary of an MCP reconnect attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpReconnectView {
    pub server_name: String,
    pub state: McpReconnectState,
    pub message: String,
    pub details: Vec<String>,
}

impl McpReconnectView {
    #[must_use]
    pub fn running(server_name: impl Into<String>) -> Self {
        let server_name = server_name.into();
        Self {
            message: format!("Reconnecting to {server_name}"),
            server_name,
            state: McpReconnectState::Running,
            details: vec!["Establishing connection to MCP server.".into()],
        }
    }

    #[must_use]
    pub fn reconnected(server_name: impl Into<String>) -> Self {
        let server_name = server_name.into();
        Self {
            message: format!("Successfully reconnected to {server_name}"),
            server_name,
            state: McpReconnectState::Reconnected,
            details: vec!["The server is connected and ready to expose tools again.".into()],
        }
    }

    #[must_use]
    pub fn needs_auth(server_name: impl Into<String>) -> Self {
        let server_name = server_name.into();
        Self {
            message: format!("{server_name} requires authentication"),
            server_name,
            state: McpReconnectState::NeedsAuth,
            details: vec!["Authenticate the server before retrying the connection.".into()],
        }
    }

    #[must_use]
    pub fn failed(server_name: impl Into<String>, detail: impl Into<String>) -> Self {
        let server_name = server_name.into();
        Self {
            message: format!("Failed to reconnect to {server_name}"),
            server_name,
            state: McpReconnectState::Failed,
            details: vec![detail.into()],
        }
    }
}

/// Authentication summary for MCP server menus and OAuth flows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpAuthSummaryView {
    pub server_name: String,
    pub status: McpAuthStatus,
    pub lines: Vec<String>,
    pub actions: Vec<CatalogAction>,
}

impl McpAuthSummaryView {
    #[must_use]
    pub fn required(server_name: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
            status: McpAuthStatus::Required,
            lines: vec![hint.into()],
            actions: vec![
                CatalogAction::new("Authenticate")
                    .with_shortcut("Enter")
                    .primary(),
                CatalogAction::new("Back").with_shortcut("Esc"),
            ],
        }
    }

    #[must_use]
    pub fn authenticated(server_name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
            status: McpAuthStatus::Authenticated,
            lines: vec![detail.into()],
            actions: vec![CatalogAction::new("Re-authenticate").with_shortcut("Enter")],
        }
    }

    #[must_use]
    pub fn browser_flow(
        server_name: impl Into<String>,
        authorization_url: impl Into<String>,
    ) -> Self {
        Self {
            server_name: server_name.into(),
            status: McpAuthStatus::BrowserFlow,
            lines: vec![
                "A browser window is open for authentication.".into(),
                authorization_url.into(),
                "Return after the browser flow completes.".into(),
            ],
            actions: vec![CatalogAction::new("Cancel").with_shortcut("Esc")],
        }
    }
}

/// The MCP settings catalog plus optional auth/reconnect/security summaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpSettingsView {
    pub catalog: CatalogModel,
    pub reconnect: Option<McpReconnectView>,
    pub auth: Option<McpAuthSummaryView>,
    pub security_review: Option<SecurityReviewView>,
}

impl McpSettingsView {
    #[must_use]
    pub fn from_servers(entries: Vec<McpServerEntryView>) -> Self {
        let mut grouped = BTreeMap::<McpCatalogGroup, Vec<CatalogEntry>>::new();
        for entry in entries {
            grouped
                .entry(entry.group)
                .or_default()
                .push(entry.into_catalog_entry());
        }

        Self {
            catalog: CatalogModel::new(
                CatalogHeader::new("MCP servers")
                    .with_subtitle("Connection, auth, and capability summaries"),
                PanelDescriptor::new("Servers").with_subtitle("Visible MCP servers"),
                PanelDescriptor::new("Server details").with_subtitle("Current selection"),
                EmptyState::new("No MCP servers configured")
                    .with_body_line("Run doctor or configure MCP settings if this is unexpected."),
                EmptyState::new("No matching MCP servers")
                    .with_body_line("Clear the filter or choose another server group."),
                grouped
                    .into_iter()
                    .map(|(group, entries)| {
                        CatalogSection::new(PanelDescriptor::new(group.label()), entries)
                    })
                    .collect(),
            ),
            reconnect: None,
            auth: None,
            security_review: None,
        }
    }

    #[must_use]
    pub fn with_reconnect(mut self, reconnect: McpReconnectView) -> Self {
        self.reconnect = Some(reconnect);
        self
    }

    #[must_use]
    pub fn with_auth(mut self, auth: McpAuthSummaryView) -> Self {
        self.auth = Some(auth);
        self
    }

    #[must_use]
    pub fn with_security_review(mut self, security_review: SecurityReviewView) -> Self {
        self.security_review = Some(security_review);
        self
    }
}

/// Supported task detail surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskDetailKind {
    Shell,
    Agent,
    RemoteSession,
    Dream,
    Workflow,
    Monitor,
}

/// A dialog-friendly summary of a single task detail screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskDetailDialogView {
    pub kind: TaskDetailKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: TaskStatus,
    pub badges: Vec<StatusBadge>,
    pub sections: Vec<DetailSection>,
    pub activities: Vec<TaskActivitySummaryView>,
    pub permission_summary: Option<PermissionSummaryView>,
    pub actions: Vec<DialogActionView>,
}

impl TaskDetailDialogView {
    #[must_use]
    pub fn new(kind: TaskDetailKind, title: impl Into<String>, status: TaskStatus) -> Self {
        Self {
            kind,
            title: title.into(),
            subtitle: None,
            status,
            badges: vec![task_status_badge(status)],
            sections: Vec::new(),
            activities: Vec::new(),
            permission_summary: None,
            actions: vec![DialogActionView::new("Close", true)],
        }
    }

    #[must_use]
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    #[must_use]
    pub fn with_badge(mut self, badge: StatusBadge) -> Self {
        self.badges.push(badge);
        self
    }

    #[must_use]
    pub fn with_section(mut self, section: DetailSection) -> Self {
        self.sections.push(section);
        self
    }

    #[must_use]
    pub fn with_activity(mut self, activity: TaskActivitySummaryView) -> Self {
        self.activities.push(activity);
        self
    }

    #[must_use]
    pub fn with_permission_summary(mut self, permission_summary: PermissionSummaryView) -> Self {
        self.permission_summary = Some(permission_summary);
        self
    }

    #[must_use]
    pub fn to_dialog_view(&self) -> DialogView {
        let mut body = vec![format!("Status: {}", task_status_label(self.status))];
        if let Some(subtitle) = &self.subtitle {
            body.push(subtitle.clone());
        }
        if !self.badges.is_empty() {
            body.push(format!(
                "Badges: {}",
                self.badges
                    .iter()
                    .map(|badge| badge.label.clone())
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        for section in &self.sections {
            body.push(format!("{}:", section.panel.title));
            body.extend(section.lines.iter().cloned());
        }
        if !self.activities.is_empty() {
            body.push("Recent activity:".into());
            for activity in &self.activities {
                body.extend(activity.display_lines(80).into_iter().map(|line| line.text));
            }
        }
        if let Some(permission_summary) = &self.permission_summary {
            body.push("Pending permission:".into());
            body.extend(permission_summary.to_dialog_view().body);
        }

        DialogView {
            title: self.title.clone(),
            body,
            actions: self.actions.clone(),
        }
    }
}

/// Settings tab browser summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsPaneView {
    pub catalog: CatalogModel,
    pub diagnostics: Vec<String>,
    pub security_review: Option<SecurityReviewView>,
}

impl SettingsPaneView {
    #[must_use]
    pub fn from_sections(entries: Vec<PaneCatalogEntryView>, diagnostics: Vec<String>) -> Self {
        let mut header = CatalogHeader::new("Settings").with_subtitle("Status, config, and usage");
        if !diagnostics.is_empty() {
            header.badges.push(StatusBadge::new(
                format!(
                    "{} diagnostic{}",
                    diagnostics.len(),
                    if diagnostics.len() == 1 { "" } else { "s" }
                ),
                CatalogTone::Warning,
            ));
        }
        Self {
            catalog: CatalogModel::new(
                header,
                PanelDescriptor::new("Sections").with_subtitle("Visible settings panes"),
                PanelDescriptor::new("Section details").with_subtitle("Current selection"),
                EmptyState::new("No settings sections")
                    .with_body_line("Add sections for status, config, or usage details."),
                EmptyState::new("No matching settings sections")
                    .with_body_line("Clear the search to browse all settings panes."),
                vec![CatalogSection::new(
                    PanelDescriptor::new("Settings"),
                    entries
                        .into_iter()
                        .map(PaneCatalogEntryView::into_catalog_entry)
                        .collect(),
                )],
            ),
            diagnostics,
            security_review: None,
        }
    }

    #[must_use]
    pub fn with_security_review(mut self, security_review: SecurityReviewView) -> Self {
        self.security_review = Some(security_review);
        self
    }
}

/// Help browser summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpPaneView {
    pub catalog: CatalogModel,
    pub docs_url: Option<String>,
    pub footer: Option<String>,
}

impl HelpPaneView {
    #[must_use]
    pub fn from_sections(entries: Vec<PaneCatalogEntryView>, docs_url: Option<String>) -> Self {
        Self {
            catalog: CatalogModel::new(
                CatalogHeader::new("Help")
                    .with_subtitle("General usage, shortcuts, and command browsing"),
                PanelDescriptor::new("Topics").with_subtitle("Visible help panes"),
                PanelDescriptor::new("Topic details").with_subtitle("Current selection"),
                EmptyState::new("No help topics")
                    .with_body_line("Add general help or command browser topics."),
                EmptyState::new("No matching help topics")
                    .with_body_line("Clear the search to see all help topics."),
                vec![CatalogSection::new(
                    PanelDescriptor::new("Help"),
                    entries
                        .into_iter()
                        .map(PaneCatalogEntryView::into_catalog_entry)
                        .collect(),
                )],
            ),
            docs_url,
            footer: Some("Esc closes help.".into()),
        }
    }
}

/// Current sandbox execution mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxModeView {
    AutoAllow,
    Regular,
    Disabled,
}

impl SandboxModeView {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AutoAllow => "Sandbox BashTool, with auto-allow",
            Self::Regular => "Sandbox BashTool, with regular permissions",
            Self::Disabled => "No sandbox",
        }
    }
}

/// Doctor output for sandbox dependency checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxDoctorView {
    pub status: StatusBadge,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub recommendation: Option<String>,
}

impl SandboxDoctorView {
    #[must_use]
    pub fn from_issues(errors: Vec<String>, warnings: Vec<String>) -> Option<Self> {
        if errors.is_empty() && warnings.is_empty() {
            return None;
        }

        let status = if errors.is_empty() {
            StatusBadge::new("available with warnings", CatalogTone::Warning)
        } else {
            StatusBadge::new("missing dependencies", CatalogTone::Danger)
        };
        let recommendation =
            (!errors.is_empty()).then(|| "Run /sandbox for install instructions.".to_string());

        Some(Self {
            status,
            errors,
            warnings,
            recommendation,
        })
    }
}

/// Sandbox settings tabs plus optional doctor/security summaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxSettingsView {
    pub catalog: CatalogModel,
    pub mode: SandboxModeView,
    pub warning_banner: Option<String>,
    pub doctor: Option<SandboxDoctorView>,
    pub security_review: Option<SecurityReviewView>,
}

impl SandboxSettingsView {
    #[must_use]
    pub fn new(
        mode: SandboxModeView,
        has_dependency_issues: bool,
        show_socket_warning: bool,
        doctor: Option<SandboxDoctorView>,
    ) -> Self {
        let mut mode_entry = PaneCatalogEntryView::new("mode", "Mode")
            .with_description(mode.label())
            .with_line(format!("Current mode: {}", mode.label()));
        if show_socket_warning {
            mode_entry =
                mode_entry.with_badge(StatusBadge::new("socket warning", CatalogTone::Warning));
            mode_entry = mode_entry
                .with_line("Cannot block unix domain sockets with the current dependency set.");
        }

        let mut entries = vec![mode_entry];
        if has_dependency_issues {
            entries.push(
                PaneCatalogEntryView::new("dependencies", "Dependencies")
                    .with_description("Dependency checks and install guidance")
                    .with_badge(StatusBadge::new("attention", CatalogTone::Warning))
                    .with_line(
                        "Review missing dependencies and warnings before relying on sandboxing.",
                    ),
            );
        }
        entries.push(
            PaneCatalogEntryView::new("overrides", "Overrides")
                .with_description("Policy and per-project overrides")
                .with_line("Inspect network and command overrides."),
        );
        entries.push(
            PaneCatalogEntryView::new("config", "Config")
                .with_description("Underlying sandbox configuration")
                .with_line("Inspect the effective sandbox settings."),
        );

        Self {
            catalog: CatalogModel::new(
                CatalogHeader::new("Sandbox")
                    .with_subtitle("Mode, dependencies, overrides, and config"),
                PanelDescriptor::new("Tabs").with_subtitle("Visible sandbox panes"),
                PanelDescriptor::new("Tab details").with_subtitle("Current selection"),
                EmptyState::new("No sandbox sections")
                    .with_body_line("Add sandbox tabs for mode, overrides, or dependency checks."),
                EmptyState::new("No matching sandbox sections")
                    .with_body_line("Clear the search to see all sandbox tabs."),
                vec![CatalogSection::new(
                    PanelDescriptor::new("Sandbox"),
                    entries
                        .into_iter()
                        .map(PaneCatalogEntryView::into_catalog_entry)
                        .collect(),
                )],
            ),
            mode,
            warning_banner: show_socket_warning
                .then(|| "Cannot block unix domain sockets (see Dependencies tab).".to_string()),
            doctor,
            security_review: None,
        }
    }

    #[must_use]
    pub fn with_security_review(mut self, security_review: SecurityReviewView) -> Self {
        self.security_review = Some(security_review);
        self
    }
}

fn task_status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn task_status_badge(status: TaskStatus) -> StatusBadge {
    match status {
        TaskStatus::Pending => StatusBadge::pending(),
        TaskStatus::Running => StatusBadge::new("running", CatalogTone::Accent),
        TaskStatus::Completed => StatusBadge::new("completed", CatalogTone::Success),
        TaskStatus::Failed => StatusBadge::failed(),
        TaskStatus::Killed => StatusBadge::new("killed", CatalogTone::Danger),
        TaskStatus::Cancelled => StatusBadge::new("cancelled", CatalogTone::Warning),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wonder_of_u_core::PermissionRequest;

    use super::*;
    use crate::security::{ManagedSettingsEnforcement, ManagedSettingsSecurityDialogView};

    #[test]
    fn empty_agent_screen_uses_create_focused_empty_state() {
        let view = AgentManagementView::from_entries("All sources", Vec::new(), 0, true);

        assert_eq!(view.catalog.visible_count(), 0);
        assert_eq!(
            view.catalog
                .active_empty_state()
                .map(|state| state.title.as_str()),
            Some("No agents found")
        );
        assert_eq!(view.catalog.header.actions.len(), 1);
        assert_eq!(view.catalog.header.actions[0].label, "Create new agent");
    }

    #[test]
    fn loaded_agent_screen_groups_sources_and_exposes_wizard_progress() {
        let wizard = AgentWizardView::new(vec![
            AgentWizardStepView::new("Location", AgentWizardStepStatus::Complete)
                .with_summary("project agent"),
            AgentWizardStepView::new("Prompt", AgentWizardStepStatus::Current)
                .with_summary("Generate from template"),
            AgentWizardStepView::new("Confirm", AgentWizardStepStatus::Pending),
        ]);
        let view = AgentManagementView::from_entries(
            "All sources",
            vec![
                AgentCatalogEntryView::new("reviewer", "Reviewer", AgentCatalogSource::Project)
                    .with_description("Checks diffs")
                    .with_model("Claude Sonnet")
                    .with_tools_summary("read, diff"),
                AgentCatalogEntryView::new("core", "Core", AgentCatalogSource::BuiltIn)
                    .with_description("Always available")
                    .read_only(),
                AgentCatalogEntryView::new("auth", "Auth Plugin", AgentCatalogSource::Plugin)
                    .with_description("OAuth helper")
                    .overridden_by("user override"),
            ],
            2,
            true,
        )
        .with_wizard(wizard.clone());

        let sections = view.catalog.visible_sections();
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].panel.title, "Project agents");
        assert_eq!(sections[1].panel.title, "Plugin agents");
        assert_eq!(sections[2].panel.title, "Built-in agents");
        assert_eq!(
            view.catalog.header.badges[0],
            StatusBadge::new("2 pending changes", CatalogTone::Warning)
        );
        assert_eq!(view.wizard, Some(wizard));
        assert_eq!(
            view.wizard.expect("wizard").as_section().lines[1],
            "Prompt — Generate from template"
        );
    }

    #[test]
    fn mcp_reconnect_and_auth_views_cover_needs_auth_and_browser_flow() {
        let reconnect = McpReconnectView::needs_auth("github");
        let auth = McpAuthSummaryView::browser_flow("github", "https://example.invalid/oauth");
        let view = McpSettingsView::from_servers(vec![
            McpServerEntryView::new(
                "github",
                "GitHub",
                McpCatalogGroup::User,
                "http",
                McpConnectionState::NeedsAuth,
            )
            .with_auth(McpAuthStatus::Required)
            .with_endpoint("https://mcp.example.invalid")
            .with_capabilities(12, 1, 4),
        ])
        .with_reconnect(reconnect.clone())
        .with_auth(auth.clone());

        assert_eq!(view.catalog.visible_count(), 1);
        assert_eq!(view.reconnect, Some(reconnect));
        assert_eq!(view.auth, Some(auth));
        let selected = view.catalog.selected_entry().expect("selected server");
        assert_eq!(selected.label, "GitHub");
        assert_eq!(selected.badges[0], McpConnectionState::NeedsAuth.badge());
        assert_eq!(
            selected.badges[1],
            McpAuthStatus::Required.badge().expect("auth badge")
        );
    }

    #[test]
    fn task_detail_dialog_includes_activity_and_permission_state() {
        let permission = PermissionSummaryView::from_request(
            &PermissionRequest::new("bash"),
            &json!({
                "command": ["cargo", "test"],
                "cwd": "/workspace/app"
            }),
        );
        let dialog =
            TaskDetailDialogView::new(TaskDetailKind::Agent, "Async agent", TaskStatus::Running)
                .with_subtitle("reviewer › reviewing pull request")
                .with_section(
                    DetailSection::new(PanelDescriptor::new("Prompt"))
                        .with_line("Review the latest diff and call out risky changes."),
                )
                .with_activity(TaskActivitySummaryView::output(
                    "task-17",
                    TaskStatus::Running,
                    "Tool call in progress",
                    "Reading modified files",
                ))
                .with_permission_summary(permission)
                .to_dialog_view();

        assert_eq!(dialog.title, "Async agent");
        assert!(dialog.body.iter().any(|line| line == "Status: running"));
        assert!(dialog.body.iter().any(|line| line == "Prompt:"));
        assert!(
            dialog
                .body
                .iter()
                .any(|line| line.contains("task[output]> #task-17 running"))
        );
        assert!(dialog.body.iter().any(|line| line == "Pending permission:"));
        assert!(
            dialog
                .body
                .iter()
                .any(|line| line.contains("Allow Claude to run this shell command?"))
        );
    }

    #[test]
    fn settings_and_help_panes_surface_expected_sections() {
        let settings = SettingsPaneView::from_sections(
            vec![
                PaneCatalogEntryView::new("status", "Status")
                    .with_description("Session, MCP, sandbox, and diagnostics")
                    .with_line("Version, session name, cwd, model, MCP, sandbox"),
                PaneCatalogEntryView::new("config", "Config")
                    .with_description("Interactive settings editor")
                    .with_line("Search and edit saved settings"),
                PaneCatalogEntryView::new("usage", "Usage")
                    .with_description("Rate limits and overage usage")
                    .with_line("Current session and weekly usage"),
            ],
            vec!["Sandbox dependencies are missing.".into()],
        );
        let help = HelpPaneView::from_sections(
            vec![
                PaneCatalogEntryView::new("general", "General")
                    .with_description("Core shortcuts and usage")
                    .with_line("Claude understands your codebase from the terminal."),
                PaneCatalogEntryView::new("commands", "Commands")
                    .with_description("Browse default commands")
                    .with_line("/help, /config, /mcp"),
            ],
            Some("https://code.claude.com/docs/en/overview".into()),
        );

        assert_eq!(settings.catalog.header.title, "Settings");
        assert_eq!(settings.catalog.header.badges.len(), 1);
        assert_eq!(settings.catalog.visible_count(), 3);
        assert_eq!(help.catalog.header.title, "Help");
        assert_eq!(help.catalog.visible_count(), 2);
        assert_eq!(
            help.catalog
                .selected_entry()
                .expect("selected help entry")
                .label,
            "General"
        );
    }

    #[test]
    fn sandbox_views_surface_warnings_and_doctor_findings() {
        let doctor = SandboxDoctorView::from_issues(
            vec!["sandbox-exec is not installed".into()],
            vec!["Cannot block unix domain sockets".into()],
        )
        .expect("doctor state");
        let security_review =
            SecurityReviewView::ManagedSettings(ManagedSettingsSecurityDialogView::from_settings(
                &json!({
                    "hooks": {
                        "PostToolUse": [{ "matcher": "*", "hooks": [] }]
                    }
                }),
                Some("remote-managed-settings"),
                ManagedSettingsEnforcement::InformationalOnly,
            ));
        let view = SandboxSettingsView::new(SandboxModeView::AutoAllow, true, true, Some(doctor))
            .with_security_review(security_review);

        assert_eq!(view.catalog.visible_count(), 4);
        assert_eq!(
            view.warning_banner.as_deref(),
            Some("Cannot block unix domain sockets (see Dependencies tab).")
        );
        let doctor = view.doctor.expect("doctor");
        assert_eq!(
            doctor.status,
            StatusBadge::new("missing dependencies", CatalogTone::Danger)
        );
        assert_eq!(
            doctor.recommendation.as_deref(),
            Some("Run /sandbox for install instructions.")
        );
        assert_eq!(
            view.security_review.expect("security review").title(),
            "Managed settings require approval"
        );
    }
}

//! Foundation primitives for the future wonder-of-u terminal UI.
//!
//! This crate intentionally stays small in the first pass: it owns terminal
//! lifecycle guards, normalized input events, frame/style/layout primitives,
//! and a snapshot-friendly shell renderer.
#![warn(missing_docs)]

/// Provides ansi support
pub mod ansi;
/// Provides app shell support
pub mod app_shell;
/// Provides catalog support
pub mod catalog;
/// Provides components support
pub mod components;
/// Provides design system support
pub mod design_system;
/// Provides dialog support
pub mod dialog;
/// Provides diff support
pub mod diff;
/// Provides event support
pub mod event;
/// Provides fleet view support
pub mod fleet_view;
/// Provides frame support
pub mod frame;
/// Provides input support
pub mod input;
/// Provides interaction support
pub mod interaction;
/// Provides keymap support
pub mod keymap;
/// Provides layout support
pub mod layout;
/// Provides management support
pub mod management;
/// Provides measure support
pub mod measure;
/// Provides message support
pub mod message;
/// Provides notification support
pub mod notification;
/// Provides permission support
pub mod permission;
/// Provides prompt support
pub mod prompt;
/// Provides render support
pub mod render;
/// Provides security support
pub mod security;
/// Provides style support
pub mod style;
/// Provides task view support
pub mod task_view;
/// Provides terminal support
pub mod terminal;
/// Provides vim support
pub mod vim;

/// Re-exports items from `ansi`
pub use ansi::{
    AnsiColor, AnsiNamedColor, AnsiStyle, RawAnsiPolicy, UnderlineStyle, apply_sgr_to_text_style,
};
/// Re-exports items from `app_shell`
pub use app_shell::{
    AppShellView, ConsoleOAuthFlowView, DevBarView, ExitFlowView, OnboardingStepView,
    OnboardingView, TagTabView, TagTabsView,
};
/// Re-exports items from `catalog`
pub use catalog::{
    CatalogAction, CatalogEntry, CatalogEntryView, CatalogHeader, CatalogModel, CatalogSection,
    CatalogSectionView, CatalogSelection, CatalogTone, DetailPreview, EmptyState, PanelDescriptor,
    ScrollWindow, StatusBadge,
};
/// Re-exports items from `components`
pub use components::{
    ActionRowView, AlignItems, AlternateScreenView, AppViewState, BoxView, ButtonInteractionState,
    ButtonView, ClawdPose, ErrorExcerptLineView, ErrorLocationView, ErrorOverviewView,
    ErrorStackFrameView, Insets, JustifyContent, LayoutDirection, LayoutOverflow, LayoutSpacing,
    LayoutWrap, LinkView, LogoFeedItem, LogoView, NewlineView, NoSelectMode, NoSelectView,
    ScrollBoxState, SpacerView, SpinnerCharStyle, SpinnerCharView, SpinnerFrameView, SpinnerMode,
    SpinnerView, TerminalFocusState, TerminalFocusView, TerminalSize, TextAttributes, TextView,
    TextWeight, TextWrap, WelcomeView,
};
/// Re-exports items from `design_system`
pub use design_system::{
    BylineView, ListItemView, PaneView, RatchetLock, RatchetView, ThemedBoxView, ThemedTextView,
};
/// Re-exports items from `dialog`
pub use dialog::{DialogActionView, DialogKind, DialogView};
/// Re-exports items from `diff`
pub use diff::{
    FileEditHunkSummary, FilePlaceholderKind, FilePlaceholderView, FileVisualView,
    HighlightedCodeView, NoDiffState, NoDiffView, PathLinkView, StructuredDiffHunk,
    StructuredDiffLine, StructuredDiffLineKind, StructuredDiffView, TruncatedText, VisualLine,
    VisualLineKind,
};
/// Re-exports items from `event`
pub use event::{
    ClickEvent, ClickTracker, CrosstermEventSource, EventLoop, EventLoopState, EventSource,
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind, TurnState, UiEvent,
    normalize_key_event, normalize_mouse_event,
};
/// Re-exports items from `fleet_view`
pub use fleet_view::{
    FleetMemberEntryView, FleetMemberStatusView, FleetRunCounts, FleetRunView, FleetStatusView,
};
/// Re-exports items from `frame`
pub use frame::{Cell, FrameBuffer, Rect};
/// Re-exports items from `input`
pub use input::{EditAction, Motion, TextBuffer};
/// Re-exports items from `interaction`
pub use interaction::{
    DEFAULT_FOCUS_HISTORY_LIMIT, DEFAULT_TAB_WIDTH, FocusChange, FocusState, HitRegion,
    ScreenPoint, SearchCase, SearchMatch, SelectionBounds, SelectionMode, SelectionState, TabStops,
    ViewportState, hit_test, search_matches_in_lines,
};
/// Re-exports items from `keymap`
pub use keymap::{
    KeyBinding, KeyBindingContext, KeyBindingError, KeyBindingResolver, ResolvedKey, SystemAction,
    VimCommand,
};
/// Re-exports items from `layout`
pub use layout::ShellLayout;
/// Re-exports items from `management`
pub use management::{
    AgentCatalogEntryView, AgentCatalogSource, AgentManagementView, AgentWizardStepStatus,
    AgentWizardStepView, AgentWizardView, DetailSection, HelpPaneView, McpAuthStatus,
    McpAuthSummaryView, McpCatalogGroup, McpConnectionState, McpReconnectState, McpReconnectView,
    McpServerEntryView, McpSettingsView, PaneCatalogEntryView, SandboxDoctorView, SandboxModeView,
    SandboxSettingsView, SecurityReviewView, SettingsPaneView, TaskDetailDialogView,
    TaskDetailKind,
};
/// Re-exports items from `measure`
pub use measure::{
    TextMeasurement, line_width, measure_text, strip_ansi, widest_line, wrap_text_hard,
};
/// Re-exports items from `message`
pub use message::{
    AttachmentKind, AttachmentSummaryView, CollapsedReadSearchGroupView, FileEditReferenceView,
    GroupedToolCallView, HistorySearchView, MarkdownBlockView, MarkdownCodeBlockView,
    MarkdownSummaryView, McpCatalogItemView, McpCatalogKind, McpCatalogSummaryView,
    MessageLineView, MessageRole, NotebookEditMode, NotebookRejectionSummaryView, PickerListEntry,
    PickerListView, PickerView, RejectedPermissionSummaryView, RejectedToolMessageKind,
    RejectedToolMessageView, RichMessageView, SystemErrorKind, SystemErrorView, TaskActivityKind,
    TaskActivitySummaryView, TaskPanelView, ThinkingBlockView, ToolCallView, ToolResultCounts,
    ToolResultStatus, TranscriptBoundaryView, UnknownToolOutputView, footer_text, message_lines,
    message_lines_for_width, message_lines_for_width_with_cursor, queued_panel_view,
    rich_message_views, status_text, task_panel_view,
};
/// Re-exports items from `notification`
pub use notification::{
    BUILT_IN_TIPS, NotificationInput, NotificationLifetime, NotificationQueue,
    NotificationSeverity, NotificationView, OsNotificationOptions, OsNotificationResult, Tip,
    select_tip, send_os_notification, send_terminal_bell,
};
/// Re-exports items from `permission`
pub use permission::{PermissionAccessKind, PermissionDetailView, PermissionSummaryView};
/// Re-exports items from `prompt`
pub use prompt::{
    PromptAttachmentIndicator, PromptFooterHint, PromptFooterModel, PromptInputModel, PromptLayout,
    PromptModeIndicator, PromptQueueView, PromptQueuedCommandView, PromptSuggestion,
    PromptSuggestionState, find_match_chars,
};
/// Re-exports items from `render`
pub use render::{
    ContextSuggestion, ContextSuggestionsView, GlobalSearchOverlayView, MIN_SIDEBAR_WIDTH,
    PromptWarningSeverity, PromptWarningView, SIDEBAR_WIDTH, ShellView, SidebarView,
    SlashSuggestionEntry, SlashSuggestionsOverlay, SuggestionSeverity, TranscriptScrollView,
    render_shell, render_snapshot, shell_main_area_width,
};
/// Re-exports items from `security`
pub use security::{
    ManagedSettingRiskView, ManagedSettingsEnforcement, ManagedSettingsSecurityDialogView,
    SecurityActionKind, SecurityActionView, TrustDialogView, WorkspaceRiskKind, WorkspaceRiskView,
    WorkspaceTrustState, dangerous_managed_settings, security_action_hint,
};
/// Re-exports items from `style`
pub use style::{Color, TextStyle, Theme};
/// Re-exports items from `task_view`
pub use task_view::{
    AgentTypeOptionView, AgentTypeStepView, FastIconView, HookModeView, McpToolEntryView,
    McpToolListView, OffscreenFreezeView, PassesView, TaskEntryView, TaskListView, TaskStatusView,
    TeleportStashEntryView, TeleportStashView, ToolUseLoaderView,
};
/// Re-exports items from `terminal`
pub use terminal::{
    ClearTerminalCommand, CrosstermControl, CursorHomeCommand, TerminalCapabilities,
    TerminalCommand, TerminalConfig, TerminalControl, TerminalEnv, TerminalLifecycle,
    TerminalPlatform, TerminalState, has_cursor_up_viewport_yank_bug, supports_hyperlinks,
    supports_synchronized_output,
};
/// Re-exports items from `vim`
pub use vim::{VimMode, VimState};

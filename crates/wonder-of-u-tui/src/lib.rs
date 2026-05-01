//! Foundation primitives for the future wonder-of-u terminal UI.
//!
//! This crate intentionally stays small in the first pass: it owns terminal
//! lifecycle guards, normalized input events, frame/style/layout primitives,
//! and a snapshot-friendly shell renderer.

pub mod ansi;
pub mod catalog;
pub mod components;
pub mod dialog;
pub mod diff;
pub mod event;
pub mod frame;
pub mod input;
pub mod interaction;
pub mod keymap;
pub mod layout;
pub mod management;
pub mod measure;
pub mod message;
pub mod notification;
pub mod permission;
pub mod prompt;
pub mod render;
pub mod security;
pub mod style;
pub mod terminal;
pub mod vim;

pub use ansi::{
    AnsiColor, AnsiNamedColor, AnsiStyle, RawAnsiPolicy, UnderlineStyle, apply_sgr_to_text_style,
};
pub use catalog::{
    CatalogAction, CatalogEntry, CatalogEntryView, CatalogHeader, CatalogModel, CatalogSection,
    CatalogSectionView, CatalogSelection, CatalogTone, DetailPreview, EmptyState, PanelDescriptor,
    ScrollWindow, StatusBadge,
};
pub use components::{
    ActionRowView, AlignItems, AlternateScreenView, AppViewState, BoxView, ButtonInteractionState,
    ButtonView, ErrorExcerptLineView, ErrorLocationView, ErrorOverviewView, ErrorStackFrameView,
    Insets, JustifyContent, LayoutDirection, LayoutOverflow, LayoutSpacing, LayoutWrap, LinkView,
    NewlineView, NoSelectMode, NoSelectView, ScrollBoxState, SpacerView, TerminalFocusState,
    TerminalFocusView, TerminalSize, TextAttributes, TextView, TextWeight, TextWrap,
};
pub use dialog::{DialogActionView, DialogKind, DialogView};
pub use diff::{
    FileEditHunkSummary, FilePlaceholderKind, FilePlaceholderView, FileVisualView,
    HighlightedCodeView, NoDiffState, NoDiffView, PathLinkView, StructuredDiffHunk,
    StructuredDiffLine, StructuredDiffLineKind, StructuredDiffView, TruncatedText, VisualLine,
    VisualLineKind,
};
pub use event::{
    ClickEvent, ClickTracker, CrosstermEventSource, EventLoop, EventLoopState, EventSource,
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind, TurnState, UiEvent,
    normalize_key_event, normalize_mouse_event,
};
pub use frame::{Cell, FrameBuffer, Rect};
pub use input::{EditAction, Motion, TextBuffer};
pub use interaction::{
    DEFAULT_FOCUS_HISTORY_LIMIT, DEFAULT_TAB_WIDTH, FocusChange, FocusState, HitRegion,
    ScreenPoint, SearchCase, SearchMatch, SelectionBounds, SelectionMode, SelectionState, TabStops,
    ViewportState, hit_test, search_matches_in_lines,
};
pub use keymap::{
    KeyBinding, KeyBindingContext, KeyBindingError, KeyBindingResolver, ResolvedKey, SystemAction,
    VimCommand,
};
pub use layout::ShellLayout;
pub use management::{
    AgentCatalogEntryView, AgentCatalogSource, AgentManagementView, AgentWizardStepStatus,
    AgentWizardStepView, AgentWizardView, DetailSection, HelpPaneView, McpAuthStatus,
    McpAuthSummaryView, McpCatalogGroup, McpConnectionState, McpReconnectState, McpReconnectView,
    McpServerEntryView, McpSettingsView, PaneCatalogEntryView, SandboxDoctorView, SandboxModeView,
    SandboxSettingsView, SecurityReviewView, SettingsPaneView, TaskDetailDialogView,
    TaskDetailKind,
};
pub use measure::{
    TextMeasurement, line_width, measure_text, strip_ansi, widest_line, wrap_text_hard,
};
pub use message::{
    AttachmentKind, AttachmentSummaryView, FileEditReferenceView, GroupedToolCallView,
    HistorySearchView, MarkdownBlockView, MarkdownCodeBlockView, MarkdownSummaryView,
    McpCatalogItemView, McpCatalogKind, McpCatalogSummaryView, MessageLineView, MessageRole,
    NotebookEditMode, NotebookRejectionSummaryView, PickerView, RejectedPermissionSummaryView,
    RejectedToolMessageKind, RejectedToolMessageView, RichMessageView, SystemErrorKind,
    SystemErrorView, TaskActivityKind, TaskActivitySummaryView, TaskPanelView, ThinkingBlockView,
    ToolCallView, ToolResultCounts, ToolResultStatus, TranscriptBoundaryView,
    UnknownToolOutputView, footer_text, message_lines, queued_panel_view, rich_message_views,
    status_text, task_panel_view,
};
pub use notification::{
    NotificationInput, NotificationLifetime, NotificationQueue, NotificationSeverity,
    NotificationView,
};
pub use permission::{PermissionAccessKind, PermissionDetailView, PermissionSummaryView};
pub use prompt::{
    PromptAttachmentIndicator, PromptFooterHint, PromptFooterModel, PromptInputModel, PromptLayout,
    PromptModeIndicator, PromptQueueView, PromptQueuedCommandView, PromptSuggestion,
    PromptSuggestionState,
};
pub use render::{ShellView, render_shell, render_snapshot};
pub use security::{
    ManagedSettingRiskView, ManagedSettingsEnforcement, ManagedSettingsSecurityDialogView,
    SecurityActionKind, SecurityActionView, TrustDialogView, WorkspaceRiskKind, WorkspaceRiskView,
    WorkspaceTrustState, dangerous_managed_settings, security_action_hint,
};
pub use style::{Color, TextStyle, Theme};
pub use terminal::{
    ClearTerminalCommand, CrosstermControl, CursorHomeCommand, TerminalCapabilities,
    TerminalCommand, TerminalConfig, TerminalControl, TerminalEnv, TerminalLifecycle,
    TerminalPlatform, TerminalState, has_cursor_up_viewport_yank_bug, supports_hyperlinks,
    supports_synchronized_output,
};
pub use vim::{VimMode, VimState};

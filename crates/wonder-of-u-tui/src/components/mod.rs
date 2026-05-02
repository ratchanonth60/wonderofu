//! Renderer-agnostic component view models inspired by Ink primitives.

mod alternate_screen;
mod app;
mod button;
mod dialog;
mod divider;
mod error;
mod layout_box;
mod link;
mod loading_state;
mod logo;
mod newline;
mod no_select;
mod progress_bar;
mod scroll_box;
mod spacer;
mod spinner;
mod status_icon;
mod tabs;
mod terminal;
mod text;

/// Re-exports items from `alternate_screen`
pub use alternate_screen::AlternateScreenView;
/// Re-exports items from `app`
pub use app::AppViewState;
/// Re-exports items from `button`
pub use button::{ActionRowView, ButtonInteractionState, ButtonView};
/// Re-exports items from `dialog`
pub use dialog::DialogView;
/// Re-exports items from `divider`
pub use divider::{DividerOrientation, DividerView};
/// Re-exports items from `error`
pub use error::{ErrorExcerptLineView, ErrorLocationView, ErrorOverviewView, ErrorStackFrameView};
/// Re-exports items from `layout_box`
pub use layout_box::{
    AlignItems, BoxView, Insets, JustifyContent, LayoutDirection, LayoutOverflow, LayoutSpacing,
    LayoutWrap,
};
/// Re-exports items from `link`
pub use link::LinkView;
/// Re-exports items from `loading_state`
pub use loading_state::{LoadingStateView, LoadingStep};
/// Re-exports items from `logo`
pub use logo::{ClawdPose, LogoFeedItem, LogoView, WelcomeView};
/// Re-exports items from `newline`
pub use newline::NewlineView;
/// Re-exports items from `no_select`
pub use no_select::{NoSelectMode, NoSelectView};
/// Re-exports items from `progress_bar`
pub use progress_bar::{ProgressBarView, ProgressDirection};
/// Re-exports items from `scroll_box`
pub use scroll_box::ScrollBoxState;
/// Re-exports items from `spacer`
pub use spacer::SpacerView;
/// Re-exports items from `spinner`
pub use spinner::{
    SPINNER_FRAMES as ANIMATED_SPINNER_FRAMES, SPINNER_GLYPHS, SpinnerCharStyle, SpinnerCharView,
    SpinnerFrameView, SpinnerMode, SpinnerView,
};
/// Re-exports items from `status_icon`
pub use status_icon::{SPINNER_FRAMES, StatusIconKind, StatusIconView};
/// Re-exports items from `tabs`
pub use tabs::{TabEntry, TabsView};
/// Re-exports items from `terminal`
pub use terminal::{TerminalFocusState, TerminalFocusView, TerminalSize};
/// Re-exports items from `text`
pub use text::{TextAttributes, TextView, TextWeight, TextWrap};

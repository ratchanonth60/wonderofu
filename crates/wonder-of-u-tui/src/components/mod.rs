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
mod newline;
mod no_select;
mod progress_bar;
mod scroll_box;
mod spacer;
mod status_icon;
mod tabs;
mod terminal;
mod text;

pub use alternate_screen::AlternateScreenView;
pub use app::AppViewState;
pub use button::{ActionRowView, ButtonInteractionState, ButtonView};
pub use dialog::DialogView;
pub use divider::{DividerOrientation, DividerView};
pub use error::{ErrorExcerptLineView, ErrorLocationView, ErrorOverviewView, ErrorStackFrameView};
pub use layout_box::{
    AlignItems, BoxView, Insets, JustifyContent, LayoutDirection, LayoutOverflow, LayoutSpacing,
    LayoutWrap,
};
pub use link::LinkView;
pub use loading_state::{LoadingStateView, LoadingStep};
pub use newline::NewlineView;
pub use no_select::{NoSelectMode, NoSelectView};
pub use progress_bar::{ProgressBarView, ProgressDirection};
pub use scroll_box::ScrollBoxState;
pub use spacer::SpacerView;
pub use status_icon::{StatusIconKind, StatusIconView, SPINNER_FRAMES};
pub use tabs::{TabEntry, TabsView};
pub use terminal::{TerminalFocusState, TerminalFocusView, TerminalSize};
pub use text::{TextAttributes, TextView, TextWeight, TextWrap};

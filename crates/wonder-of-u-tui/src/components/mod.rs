//! Renderer-agnostic component view models inspired by Ink primitives.

mod alternate_screen;
mod app;
mod button;
mod error;
mod layout_box;
mod link;
mod newline;
mod no_select;
mod scroll_box;
mod spacer;
mod terminal;
mod text;

pub use alternate_screen::AlternateScreenView;
pub use app::AppViewState;
pub use button::{ActionRowView, ButtonInteractionState, ButtonView};
pub use error::{ErrorExcerptLineView, ErrorLocationView, ErrorOverviewView, ErrorStackFrameView};
pub use layout_box::{
    AlignItems, BoxView, Insets, JustifyContent, LayoutDirection, LayoutOverflow, LayoutSpacing,
    LayoutWrap,
};
pub use link::LinkView;
pub use newline::NewlineView;
pub use no_select::{NoSelectMode, NoSelectView};
pub use scroll_box::ScrollBoxState;
pub use spacer::SpacerView;
pub use terminal::{TerminalFocusState, TerminalFocusView, TerminalSize};
pub use text::{TextAttributes, TextView, TextWeight, TextWrap};

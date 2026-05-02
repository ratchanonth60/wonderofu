//! Flex-style layout primitives.

/// Main-axis layout direction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayoutDirection {
    /// Represents row
    #[default]
    Row,
    /// Represents column
    Column,
}

/// Flex wrapping behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayoutWrap {
    /// Represents no wrap
    #[default]
    NoWrap,
    /// Represents wrap
    Wrap,
}

/// Cross-axis alignment.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlignItems {
    /// Represents start
    #[default]
    Start,
    /// Represents center
    Center,
    /// Represents end
    End,
    /// Represents stretch
    Stretch,
}

/// Main-axis distribution for siblings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JustifyContent {
    /// Represents start
    #[default]
    Start,
    /// Represents center
    Center,
    /// Represents end
    End,
    /// Represents space between
    SpaceBetween,
    /// Represents space around
    SpaceAround,
}

/// Overflow policy for one axis.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayoutOverflow {
    /// Represents visible
    #[default]
    Visible,
    /// Represents hidden
    Hidden,
    /// Represents scroll
    Scroll,
}

/// Four-sided spacing values.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Insets {
    /// Stores the top
    pub top: u16,
    /// Stores the right
    pub right: u16,
    /// Stores the bottom
    pub bottom: u16,
    /// Stores the left
    pub left: u16,
}

impl Insets {
    /// Constant fn
    #[must_use]
    pub const fn all(value: u16) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn xy(x: u16, y: u16) -> Self {
        Self {
            top: y,
            right: x,
            bottom: y,
            left: x,
        }
    }
}

/// Shared spacing values for a box container.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutSpacing {
    /// Stores the gap
    pub gap: u16,
    /// Stores the row gap
    pub row_gap: Option<u16>,
    /// Stores the column gap
    pub column_gap: Option<u16>,
    /// Stores the padding
    pub padding: Insets,
    /// Stores the margin
    pub margin: Insets,
}

impl LayoutSpacing {
    /// Handles row gap
    #[must_use]
    pub fn row_gap(self) -> u16 {
        match self.row_gap {
            Some(row_gap) => row_gap,
            None => self.gap,
        }
    }
    /// Handles column gap
    #[must_use]
    pub fn column_gap(self) -> u16 {
        match self.column_gap {
            Some(column_gap) => column_gap,
            None => self.gap,
        }
    }
}

/// Box layout props distilled into a renderer-neutral form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoxView {
    /// Stores the direction
    pub direction: LayoutDirection,
    /// Stores the wrap
    pub wrap: LayoutWrap,
    /// Stores the align items
    pub align_items: AlignItems,
    /// Stores the justify content
    pub justify_content: JustifyContent,
    /// Stores the spacing
    pub spacing: LayoutSpacing,
    /// Stores the flex grow
    pub flex_grow: u16,
    /// Stores the flex shrink
    pub flex_shrink: u16,
    /// Stores the width
    pub width: Option<u16>,
    /// Stores the height
    pub height: Option<u16>,
    /// Stores the overflow x
    pub overflow_x: LayoutOverflow,
    /// Stores the overflow y
    pub overflow_y: LayoutOverflow,
}

impl Default for BoxView {
    fn default() -> Self {
        Self {
            direction: LayoutDirection::Row,
            wrap: LayoutWrap::NoWrap,
            align_items: AlignItems::Start,
            justify_content: JustifyContent::Start,
            spacing: LayoutSpacing::default(),
            flex_grow: 0,
            flex_shrink: 1,
            width: None,
            height: None,
            overflow_x: LayoutOverflow::Visible,
            overflow_y: LayoutOverflow::Visible,
        }
    }
}

impl BoxView {
    /// Constant fn
    #[must_use]
    pub const fn column(mut self) -> Self {
        self.direction = LayoutDirection::Column;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn grow(mut self, flex_grow: u16) -> Self {
        self.flex_grow = flex_grow;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn gap(mut self, gap: u16) -> Self {
        self.spacing.gap = gap;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_uses_gap_as_default_for_row_and_column() {
        let spacing = LayoutSpacing {
            gap: 2,
            ..LayoutSpacing::default()
        };

        assert_eq!(spacing.row_gap(), 2);
        assert_eq!(spacing.column_gap(), 2);
    }

    #[test]
    fn box_defaults_match_ink_box_basics() {
        let view = BoxView::default();

        assert_eq!(view.direction, LayoutDirection::Row);
        assert_eq!(view.wrap, LayoutWrap::NoWrap);
        assert_eq!(view.flex_grow, 0);
        assert_eq!(view.flex_shrink, 1);
    }
}

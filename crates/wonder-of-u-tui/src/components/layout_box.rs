//! Flex-style layout primitives.

/// Main-axis layout direction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayoutDirection {
    #[default]
    Row,
    Column,
}

/// Flex wrapping behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayoutWrap {
    #[default]
    NoWrap,
    Wrap,
}

/// Cross-axis alignment.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlignItems {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

/// Main-axis distribution for siblings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JustifyContent {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

/// Overflow policy for one axis.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayoutOverflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
}

/// Four-sided spacing values.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Insets {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

impl Insets {
    #[must_use]
    pub const fn all(value: u16) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

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
    pub gap: u16,
    pub row_gap: Option<u16>,
    pub column_gap: Option<u16>,
    pub padding: Insets,
    pub margin: Insets,
}

impl LayoutSpacing {
    #[must_use]
    pub fn row_gap(self) -> u16 {
        match self.row_gap {
            Some(row_gap) => row_gap,
            None => self.gap,
        }
    }

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
    pub direction: LayoutDirection,
    pub wrap: LayoutWrap,
    pub align_items: AlignItems,
    pub justify_content: JustifyContent,
    pub spacing: LayoutSpacing,
    pub flex_grow: u16,
    pub flex_shrink: u16,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub overflow_x: LayoutOverflow,
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
    #[must_use]
    pub const fn column(mut self) -> Self {
        self.direction = LayoutDirection::Column;
        self
    }

    #[must_use]
    pub const fn grow(mut self, flex_grow: u16) -> Self {
        self.flex_grow = flex_grow;
        self
    }

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

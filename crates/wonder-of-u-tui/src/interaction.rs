//! Reusable interaction state for focus, selection, hit-testing, search, and tabs.

use std::{collections::VecDeque, ops::Range};

use unicode_width::UnicodeWidthChar;

use crate::frame::Rect;

/// Mirrors Ink's bounded focus history to make focus restoration predictable.
pub const DEFAULT_FOCUS_HISTORY_LIMIT: usize = 32;
/// Default terminal tab interval.
pub const DEFAULT_TAB_WIDTH: u16 = 8;

/// A zero-based screen coordinate.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScreenPoint {
    pub column: u16,
    pub row: u16,
}

impl ScreenPoint {
    #[must_use]
    pub const fn new(column: u16, row: u16) -> Self {
        Self { column, row }
    }
}

/// The normalized bounds of a selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionBounds {
    pub start: ScreenPoint,
    pub end: ScreenPoint,
}

impl SelectionBounds {
    #[must_use]
    pub fn contains(self, point: ScreenPoint) -> bool {
        point.row >= self.start.row
            && point.row <= self.end.row
            && !(point.row == self.start.row && point.column < self.start.column)
            && !(point.row == self.end.row && point.column > self.end.column)
    }
}

/// Coarse selection granularity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SelectionMode {
    #[default]
    Character,
    Word,
    Line,
}

/// Pure selection state for pointer and keyboard driven flows.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectionState {
    anchor: Option<ScreenPoint>,
    focus: Option<ScreenPoint>,
    mode: SelectionMode,
    dragging: bool,
}

impl SelectionState {
    #[must_use]
    pub const fn anchor(&self) -> Option<ScreenPoint> {
        self.anchor
    }

    #[must_use]
    pub const fn focus(&self) -> Option<ScreenPoint> {
        self.focus
    }

    #[must_use]
    pub const fn mode(&self) -> SelectionMode {
        self.mode
    }

    #[must_use]
    pub const fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Begins a drag selection without marking any cells yet.
    pub fn begin(&mut self, point: ScreenPoint, mode: SelectionMode) {
        self.anchor = Some(point);
        self.focus = None;
        self.mode = mode;
        self.dragging = true;
    }

    /// Updates the focus point during an active drag.
    pub fn extend_to(&mut self, point: ScreenPoint) {
        if !self.dragging {
            return;
        }

        if self.focus.is_none() && self.anchor == Some(point) {
            return;
        }

        self.focus = Some(point);
    }

    /// Sets an explicit range, which is useful for keyboard or programmatic selection.
    pub fn select_range(&mut self, anchor: ScreenPoint, focus: ScreenPoint, mode: SelectionMode) {
        self.anchor = Some(anchor);
        self.focus = Some(focus);
        self.mode = mode;
        self.dragging = false;
    }

    /// Moves the focus endpoint while keeping the anchor fixed.
    pub fn move_focus(&mut self, point: ScreenPoint) {
        match self.anchor {
            Some(_) => {
                self.focus = Some(point);
            }
            None => {
                self.anchor = Some(point);
                self.focus = Some(point);
            }
        }
        self.mode = SelectionMode::Character;
        self.dragging = false;
    }

    pub fn finish(&mut self) {
        self.dragging = false;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    #[must_use]
    pub const fn has_selection(&self) -> bool {
        self.anchor.is_some() && self.focus.is_some()
    }

    #[must_use]
    pub fn bounds(&self) -> Option<SelectionBounds> {
        let (anchor, focus) = (self.anchor?, self.focus?);
        Some(if anchor <= focus {
            SelectionBounds {
                start: anchor,
                end: focus,
            }
        } else {
            SelectionBounds {
                start: focus,
                end: anchor,
            }
        })
    }

    #[must_use]
    pub fn contains(&self, point: ScreenPoint) -> bool {
        self.bounds().is_some_and(|bounds| bounds.contains(point))
    }
}

/// A viewport plus scroll offsets into backing content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViewportState {
    pub area: Rect,
    pub scroll_x: u16,
    pub scroll_y: u16,
}

impl ViewportState {
    #[must_use]
    pub const fn new(area: Rect) -> Self {
        Self {
            area,
            scroll_x: 0,
            scroll_y: 0,
        }
    }

    #[must_use]
    pub fn contains_screen_point(self, point: ScreenPoint) -> bool {
        if self.area.is_empty() {
            return false;
        }

        point.column >= self.area.x
            && point.column < self.area.right()
            && point.row >= self.area.y
            && point.row < self.area.bottom()
    }

    #[must_use]
    pub fn clamp_screen_point(self, point: ScreenPoint) -> Option<ScreenPoint> {
        if self.area.is_empty() {
            return None;
        }

        let max_column = self.area.right().saturating_sub(1);
        let max_row = self.area.bottom().saturating_sub(1);
        Some(ScreenPoint::new(
            point.column.clamp(self.area.x, max_column),
            point.row.clamp(self.area.y, max_row),
        ))
    }

    #[must_use]
    pub fn screen_to_local(self, point: ScreenPoint) -> Option<ScreenPoint> {
        self.contains_screen_point(point).then(|| {
            ScreenPoint::new(
                point.column.saturating_sub(self.area.x),
                point.row.saturating_sub(self.area.y),
            )
        })
    }

    #[must_use]
    pub fn screen_to_content(self, point: ScreenPoint) -> Option<ScreenPoint> {
        self.screen_to_local(point).map(|local| {
            ScreenPoint::new(
                self.scroll_x.saturating_add(local.column),
                self.scroll_y.saturating_add(local.row),
            )
        })
    }

    #[must_use]
    pub fn content_to_screen(self, point: ScreenPoint) -> Option<ScreenPoint> {
        let local_column = point.column.checked_sub(self.scroll_x)?;
        let local_row = point.row.checked_sub(self.scroll_y)?;
        let screen = ScreenPoint::new(
            self.area.x.saturating_add(local_column),
            self.area.y.saturating_add(local_row),
        );
        self.contains_screen_point(screen).then_some(screen)
    }

    pub fn scroll_by(&mut self, dx: i16, dy: i16) {
        self.scroll_x = scroll_offset(self.scroll_x, dx);
        self.scroll_y = scroll_offset(self.scroll_y, dy);
    }
}

/// A hit-testable region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HitRegion<T> {
    pub id: T,
    pub area: Rect,
    pub z_index: i32,
}

impl<T> HitRegion<T> {
    #[must_use]
    pub const fn new(id: T, area: Rect) -> Self {
        Self {
            id,
            area,
            z_index: 0,
        }
    }

    #[must_use]
    pub const fn with_z_index(mut self, z_index: i32) -> Self {
        self.z_index = z_index;
        self
    }
}

/// Returns the topmost region containing `point`.
#[must_use]
pub fn hit_test<T>(regions: &[HitRegion<T>], point: ScreenPoint) -> Option<&HitRegion<T>> {
    regions
        .iter()
        .enumerate()
        .filter(|(_, region)| rect_contains(region.area, point))
        .max_by_key(|(index, region)| (region.z_index, *index))
        .map(|(_, region)| region)
}

/// Search case handling for match projection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchCase {
    Sensitive,
    #[default]
    Insensitive,
}

/// A visible search match identified by row and character columns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub row: usize,
    pub columns: Range<usize>,
}

/// Finds non-overlapping matches in visible lines.
#[must_use]
pub fn search_matches_in_lines<'a, I>(lines: I, query: &str, case: SearchCase) -> Vec<SearchMatch>
where
    I: IntoIterator<Item = &'a str>,
{
    lines
        .into_iter()
        .enumerate()
        .flat_map(|(row, line)| {
            search_matches_in_line(line, query, case)
                .into_iter()
                .map(move |columns| SearchMatch { row, columns })
        })
        .collect()
}

/// Tab stop helpers for plain-text layouts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TabStops {
    interval: u16,
}

impl Default for TabStops {
    fn default() -> Self {
        Self::new(DEFAULT_TAB_WIDTH)
    }
}

impl TabStops {
    #[must_use]
    pub const fn new(interval: u16) -> Self {
        Self {
            interval: if interval == 0 { 1 } else { interval },
        }
    }

    #[must_use]
    pub const fn interval(self) -> u16 {
        self.interval
    }

    #[must_use]
    pub fn next_stop(self, column: u16) -> u16 {
        let remainder = column % self.interval;
        if remainder == 0 {
            column.saturating_add(self.interval)
        } else {
            column.saturating_add(self.interval - remainder)
        }
    }

    #[must_use]
    pub fn advance_column(self, column: u16, text: &str) -> u16 {
        let mut column = column;
        for ch in text.chars() {
            match ch {
                '\n' => column = 0,
                '\t' => column = self.next_stop(column),
                _ => {
                    let width = UnicodeWidthChar::width(ch).unwrap_or_default() as u16;
                    column = column.saturating_add(width);
                }
            }
        }
        column
    }

    #[must_use]
    pub fn expand(self, text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut column = 0;

        for ch in text.chars() {
            match ch {
                '\n' => {
                    result.push('\n');
                    column = 0;
                }
                '\t' => {
                    let next = self.next_stop(column);
                    let spaces = next.saturating_sub(column);
                    result.extend(std::iter::repeat_n(' ', usize::from(spaces)));
                    column = next;
                }
                _ => {
                    result.push(ch);
                    let width = UnicodeWidthChar::width(ch).unwrap_or_default() as u16;
                    column = column.saturating_add(width);
                }
            }
        }

        result
    }
}

/// A pure focus ring with bounded restoration history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusState<T> {
    active: Option<T>,
    enabled: bool,
    history_limit: usize,
    history: VecDeque<T>,
}

impl<T: Clone + Eq> Default for FocusState<T> {
    fn default() -> Self {
        Self::new(DEFAULT_FOCUS_HISTORY_LIMIT)
    }
}

/// Describes a focus transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusChange<T> {
    pub previous: Option<T>,
    pub current: Option<T>,
}

impl<T: Clone + Eq> FocusState<T> {
    #[must_use]
    pub fn new(history_limit: usize) -> Self {
        Self {
            active: None,
            enabled: true,
            history_limit,
            history: VecDeque::new(),
        }
    }

    #[must_use]
    pub const fn active(&self) -> Option<&T> {
        self.active.as_ref()
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn focus(&mut self, target: T) -> Option<FocusChange<T>> {
        if !self.enabled || self.active.as_ref() == Some(&target) {
            return None;
        }

        let previous = self.active.replace(target.clone());
        if let Some(ref previous) = previous {
            self.push_history(previous.clone());
        }

        Some(FocusChange {
            previous,
            current: Some(target),
        })
    }

    pub fn blur(&mut self) -> Option<FocusChange<T>> {
        let previous = self.active.take()?;
        Some(FocusChange {
            previous: Some(previous),
            current: None,
        })
    }

    pub fn remove(&mut self, target: &T, focusable: &[T]) -> Option<FocusChange<T>> {
        self.history
            .retain(|candidate| candidate != target && focusable.contains(candidate));

        if self.active.as_ref() != Some(target) {
            return None;
        }

        let previous = self.active.take();
        while let Some(candidate) = self.history.pop_back() {
            if focusable.contains(&candidate) && &candidate != target {
                self.active = Some(candidate.clone());
                return Some(FocusChange {
                    previous,
                    current: Some(candidate),
                });
            }
        }

        Some(FocusChange {
            previous,
            current: None,
        })
    }

    pub fn focus_next(&mut self, focusable: &[T]) -> Option<FocusChange<T>> {
        self.cycle(focusable, 1)
    }

    pub fn focus_previous(&mut self, focusable: &[T]) -> Option<FocusChange<T>> {
        self.cycle(focusable, -1)
    }

    fn cycle(&mut self, focusable: &[T], direction: isize) -> Option<FocusChange<T>> {
        if !self.enabled || focusable.is_empty() {
            return None;
        }

        let current = self
            .active
            .as_ref()
            .and_then(|active| focusable.iter().position(|candidate| candidate == active));
        let next = match current {
            Some(index) => {
                let len = focusable.len() as isize;
                ((index as isize + direction).rem_euclid(len)) as usize
            }
            None if direction >= 0 => 0,
            None => focusable.len() - 1,
        };

        self.focus(focusable[next].clone())
    }

    fn push_history(&mut self, item: T) {
        if self.history_limit == 0 {
            return;
        }

        self.history.retain(|candidate| candidate != &item);
        self.history.push_back(item);
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
    }
}

fn rect_contains(area: Rect, point: ScreenPoint) -> bool {
    !area.is_empty()
        && point.column >= area.x
        && point.column < area.right()
        && point.row >= area.y
        && point.row < area.bottom()
}

fn scroll_offset(offset: u16, delta: i16) -> u16 {
    if delta >= 0 {
        offset.saturating_add(delta as u16)
    } else {
        offset.saturating_sub(delta.unsigned_abs())
    }
}

fn search_matches_in_line(line: &str, query: &str, case: SearchCase) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }

    let (haystack, haystack_map) = projected_chars(line, case);
    let (needle, _) = projected_chars(query, case);
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut start = 0;
    while start + needle.len() <= haystack.len() {
        if haystack[start..start + needle.len()] == needle {
            let start_column = haystack_map[start];
            let end_column = haystack_map[start + needle.len() - 1].saturating_add(1);
            matches.push(start_column..end_column);
            start += needle.len();
        } else {
            start += 1;
        }
    }

    matches
}

fn projected_chars(input: &str, case: SearchCase) -> (Vec<char>, Vec<usize>) {
    let mut chars = Vec::new();
    let mut map = Vec::new();

    for (index, ch) in input.chars().enumerate() {
        match case {
            SearchCase::Sensitive => {
                chars.push(ch);
                map.push(index);
            }
            SearchCase::Insensitive => {
                for lowered in ch.to_lowercase() {
                    chars.push(lowered);
                    map.push(index);
                }
            }
        }
    }

    (chars, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_requires_motion_before_it_highlights() {
        let mut selection = SelectionState::default();
        let anchor = ScreenPoint::new(4, 2);

        selection.begin(anchor, SelectionMode::Character);
        selection.extend_to(anchor);
        assert!(!selection.has_selection());

        selection.extend_to(ScreenPoint::new(7, 2));
        selection.finish();

        assert_eq!(
            selection.bounds(),
            Some(SelectionBounds {
                start: anchor,
                end: ScreenPoint::new(7, 2),
            })
        );
        assert!(selection.contains(ScreenPoint::new(5, 2)));
        assert!(!selection.is_dragging());
    }

    #[test]
    fn viewport_translates_between_screen_and_content_space() {
        let mut viewport = ViewportState::new(Rect::new(10, 5, 4, 3));
        viewport.scroll_by(3, 2);

        let point = ScreenPoint::new(12, 6);
        assert!(viewport.contains_screen_point(point));
        assert_eq!(
            viewport.screen_to_local(point),
            Some(ScreenPoint::new(2, 1))
        );
        assert_eq!(
            viewport.screen_to_content(point),
            Some(ScreenPoint::new(5, 3))
        );
        assert_eq!(
            viewport.content_to_screen(ScreenPoint::new(6, 4)),
            Some(ScreenPoint::new(13, 7))
        );
        assert_eq!(
            viewport.clamp_screen_point(ScreenPoint::new(100, 1)),
            Some(ScreenPoint::new(13, 5))
        );
    }

    #[test]
    fn hit_test_prefers_higher_z_index_then_later_regions() {
        let regions = vec![
            HitRegion::new("base", Rect::new(0, 0, 10, 10)),
            HitRegion::new("top", Rect::new(2, 2, 4, 4)).with_z_index(1),
            HitRegion::new("same-z-later", Rect::new(2, 2, 4, 4)).with_z_index(1),
        ];

        assert_eq!(
            hit_test(&regions, ScreenPoint::new(3, 3)).map(|region| region.id),
            Some("same-z-later")
        );
        assert!(hit_test::<&str>(&[], ScreenPoint::new(0, 0)).is_none());
    }

    #[test]
    fn search_matches_are_case_insensitive_and_non_overlapping() {
        let lines = ["İstanbul", "aaaa", "Beta"];

        assert_eq!(
            search_matches_in_lines(lines.iter().copied(), "i", SearchCase::Insensitive),
            vec![SearchMatch {
                row: 0,
                columns: 0..1,
            }]
        );
        assert_eq!(
            search_matches_in_lines(lines.iter().copied(), "aa", SearchCase::Insensitive),
            vec![
                SearchMatch {
                    row: 1,
                    columns: 0..2,
                },
                SearchMatch {
                    row: 1,
                    columns: 2..4,
                },
            ]
        );
        assert_eq!(
            search_matches_in_lines(lines.iter().copied(), "be", SearchCase::Sensitive),
            Vec::<SearchMatch>::new()
        );
    }

    #[test]
    fn tab_stops_expand_using_display_width() {
        let tabs = TabStops::new(4);

        assert_eq!(tabs.next_stop(4), 8);
        assert_eq!(tabs.advance_column(0, "好\t!"), 5);
        assert_eq!(tabs.expand("a\t好\tb\n\tc"), "a   好  b\n    c");
    }

    #[test]
    fn focus_state_cycles_and_restores_recent_focus() {
        let mut focus = FocusState::new(2);
        let focusable = ["one", "two", "three"];

        assert_eq!(
            focus.focus_next(&focusable),
            Some(FocusChange {
                previous: None,
                current: Some("one"),
            })
        );
        assert_eq!(
            focus.focus_next(&focusable),
            Some(FocusChange {
                previous: Some("one"),
                current: Some("two"),
            })
        );
        assert_eq!(
            focus.focus_previous(&focusable),
            Some(FocusChange {
                previous: Some("two"),
                current: Some("one"),
            })
        );

        focus.focus("three");
        assert_eq!(
            focus.remove(&"three", &focusable),
            Some(FocusChange {
                previous: Some("three"),
                current: Some("one"),
            })
        );
    }

    #[test]
    fn disabled_focus_state_rejects_changes() {
        let mut focus = FocusState::default();
        focus.disable();

        assert!(focus.focus("item").is_none());
        assert!(focus.focus_next(&["item"]).is_none());
    }
}

//! Loading state view model — mirrors `claude-leak/components/design-system/LoadingState.tsx`.

/// A named step in a multi-step loading sequence.
#[derive(Clone, Debug)]
pub struct LoadingStep {
    pub label: String,
    pub done: bool,
}

impl LoadingStep {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            done: false,
        }
    }

    #[must_use]
    pub fn done(mut self) -> Self {
        self.done = true;
        self
    }
}

/// View model for a loading-state overlay.
#[derive(Clone, Debug, Default)]
pub struct LoadingStateView {
    /// Message shown above the step list.
    pub title: Option<String>,
    /// Ordered list of loading steps.
    pub steps: Vec<LoadingStep>,
    /// Whether the loading has completed (all steps done).
    pub is_complete: bool,
    /// Current spinner frame for the active step.
    pub spinner_frame: usize,
}

impl LoadingStateView {
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = LoadingStep>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Returns the index of the first in-progress step (not yet done).
    #[must_use]
    pub fn active_step_index(&self) -> Option<usize> {
        self.steps.iter().position(|s| !s.done)
    }

    /// Mark the next pending step as done. Returns `true` if there was one.
    pub fn advance(&mut self) -> bool {
        if let Some(idx) = self.active_step_index() {
            self.steps[idx].done = true;
            if self.steps.iter().all(|s| s.done) {
                self.is_complete = true;
            }
            return true;
        }
        false
    }

    /// Advance the spinner frame.
    pub fn tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
    }
}

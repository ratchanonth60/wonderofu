use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{Result, WonderError};

/// Coarse feature switches used to filter commands, tools, and UI surfaces.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureFlag {
    Tui,
    ModelProvider,
    Tools,
    WebTools,
    Permissions,
    SessionPersistence,
    Mcp,
    Plugins,
    Skills,
    Agents,
    BackgroundTasks,
}

/// Deterministic set wrapper for serializable feature gates.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FeatureSet(BTreeSet<FeatureFlag>);

impl FeatureSet {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn first_release() -> Self {
        Self(BTreeSet::from([
            FeatureFlag::Tui,
            FeatureFlag::ModelProvider,
            FeatureFlag::Tools,
            FeatureFlag::WebTools,
            FeatureFlag::Permissions,
            FeatureFlag::SessionPersistence,
            FeatureFlag::Mcp,
            FeatureFlag::Plugins,
            FeatureFlag::Skills,
            FeatureFlag::Agents,
            FeatureFlag::BackgroundTasks,
        ]))
    }

    pub fn enable(&mut self, flag: FeatureFlag) -> bool {
        self.0.insert(flag)
    }

    pub fn disable(&mut self, flag: FeatureFlag) -> bool {
        self.0.remove(&flag)
    }

    #[must_use]
    pub fn contains(&self, flag: FeatureFlag) -> bool {
        self.0.contains(&flag)
    }

    #[must_use]
    pub fn contains_all<'a>(&self, flags: impl IntoIterator<Item = &'a FeatureFlag>) -> bool {
        flags.into_iter().all(|flag| self.contains(*flag))
    }

    pub fn require(&self, flag: FeatureFlag) -> Result<()> {
        if self.contains(flag) {
            Ok(())
        } else {
            Err(WonderError::validation(format!(
                "feature is disabled: {flag:?}"
            )))
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = FeatureFlag> + '_ {
        self.0.iter().copied()
    }
}

impl FromIterator<FeatureFlag> for FeatureSet {
    fn from_iter<T: IntoIterator<Item = FeatureFlag>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_release_enables_expected_foundation_flags() {
        let features = FeatureSet::first_release();
        assert!(features.contains(FeatureFlag::SessionPersistence));
        assert!(features.contains(FeatureFlag::Permissions));
        assert!(features.contains(FeatureFlag::Agents));
    }

    #[test]
    fn require_reports_disabled_feature() {
        let features = FeatureSet::empty();
        let error = features.require(FeatureFlag::Mcp).expect_err("disabled");
        assert!(error.to_string().contains("Mcp"));
    }
}

//! Bundled, local, and plugin-provided skill catalog foundations.
#![warn(missing_docs)]

mod bundled;
mod catalog;
/// Disk-based skill loader for `.md` files with YAML frontmatter.
pub mod loader;
mod manifest;

/// Re-exports items from `bundled`
pub use bundled::{BundledSkill, bundled_skills};
/// Re-exports items from `catalog`
pub use catalog::{SkillCatalog, SkillCatalogError, SkillRegistration, SkillSource, SkillTrust};
/// Re-exports items from `loader`
pub use loader::{DiskSkillLoader, LoadedSkill};
/// Re-exports items from `manifest`
pub use manifest::{SKILL_MANIFEST_SCHEMA_VERSION, SkillManifest};

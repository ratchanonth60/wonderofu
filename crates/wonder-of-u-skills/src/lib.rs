//! Bundled, local, and plugin-provided skill catalog foundations.

mod bundled;
mod catalog;
mod manifest;

pub use bundled::{BundledSkill, bundled_skills};
pub use catalog::{SkillCatalog, SkillCatalogError, SkillRegistration, SkillSource, SkillTrust};
pub use manifest::{SKILL_MANIFEST_SCHEMA_VERSION, SkillManifest};

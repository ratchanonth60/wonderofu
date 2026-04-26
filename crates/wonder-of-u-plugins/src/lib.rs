//! Plugin manifest, trust, and discovery foundations.

mod catalog;
mod config;
mod manifest;

pub use catalog::{
    PluginCatalog, PluginCatalogEntry, PluginCatalogError, PluginDiscoveryRoot, PluginReadiness,
    PluginSource, PluginTrustLevel,
};
pub use config::{
    PLUGIN_CONFIG_SCHEMA_VERSION, PluginConfig, PluginConfigStore, PluginTrustDecision,
};
pub use manifest::{
    PLUGIN_MANIFEST_SCHEMA_VERSION, PluginCommandDefinition, PluginCommandRegistration,
    PluginManifest, PluginSkillDefinition, PluginSkillSource, normalize_plugin_id,
};

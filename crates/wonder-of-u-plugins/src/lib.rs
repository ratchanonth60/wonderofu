//! Plugin manifest, trust, and discovery foundations.
#![warn(missing_docs)]

mod catalog;
mod config;
mod manifest;

/// Re-exports items from `catalog`
pub use catalog::{
    PLUGIN_MANIFEST_FILE_NAMES, PluginCatalog, PluginCatalogEntry, PluginCatalogError,
    PluginDiscoveryRoot, PluginReadiness, PluginSource, PluginTrustLevel, plugin_manifest_path,
};
/// Re-exports items from `config`
pub use config::{
    PLUGIN_CONFIG_SCHEMA_VERSION, PluginConfig, PluginConfigStore, PluginTrustDecision,
};
/// Re-exports items from `manifest`
pub use manifest::{
    PLUGIN_MANIFEST_SCHEMA_VERSION, PluginCommandDefinition, PluginCommandRegistration,
    PluginManifest, PluginSkillDefinition, PluginSkillSource, normalize_plugin_id,
};

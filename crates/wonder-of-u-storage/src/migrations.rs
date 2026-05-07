//! Storage schema migration framework.
//!
//! Applies pending migrations in ascending version order at startup,
//! persisting the current version in `config/storage_version.json`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use wonder_of_u_core::Result;

/// Trait implemented by each schema migration step.
pub trait Migration: Send + Sync {
    /// The schema version this migration upgrades from.
    fn schema_version_from(&self) -> u16;
    /// The schema version this migration upgrades to.
    fn schema_version_to(&self) -> u16;
    /// Human-readable description of what this migration does.
    fn description(&self) -> &str;
    /// Applies the migration to the storage at `base_dir`.
    fn migrate(&self, base_dir: &Path) -> Result<()>;
}

/// Runs registered migrations in order, bumping the stored version after each one.
#[derive(Default)]
pub struct MigrationRunner {
    migrations: Vec<Box<dyn Migration>>,
}

impl MigrationRunner {
    /// Creates an empty runner with no migrations registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a migration. Migrations are applied in ascending `schema_version_from` order.
    pub fn register(&mut self, m: Box<dyn Migration>) {
        self.migrations.push(m);
    }

    /// Returns the current schema version stored on disk (1 if the file is absent).
    pub fn current_version(base_dir: &Path) -> Result<u16> {
        StorageVersionFile::load(base_dir).map(|f| f.schema_version)
    }

    /// Applies all pending migrations whose `schema_version_from` matches the current stored
    /// version. Returns the final version after all applicable migrations have run.
    pub fn run_pending(&self, base_dir: &Path) -> Result<u16> {
        let mut version_file = StorageVersionFile::load(base_dir)?;
        let mut current = version_file.schema_version;

        // Sort by from-version so migrations are applied in the correct order.
        let mut sorted: Vec<&dyn Migration> = self.migrations.iter().map(|m| m.as_ref()).collect();
        sorted.sort_by_key(|m| m.schema_version_from());

        for migration in sorted {
            if migration.schema_version_from() == current {
                migration.migrate(base_dir)?;
                current = migration.schema_version_to();
                version_file.schema_version = current;
                version_file.save(base_dir)?;
            }
        }

        Ok(current)
    }
}

/// Versioned file stored at `<base_dir>/config/storage_version.json`.
#[derive(Serialize, Deserialize)]
pub struct StorageVersionFile {
    /// Schema version applied to this storage directory.
    pub schema_version: u16,
}

impl StorageVersionFile {
    /// The baseline schema version used when no version file exists yet.
    pub const BASELINE: u16 = 1;

    /// Loads the version file; returns `schema_version = 1` if absent.
    pub fn load(base_dir: &Path) -> Result<Self> {
        let path = Self::path(base_dir);
        if !path.exists() {
            return Ok(Self { schema_version: Self::BASELINE });
        }
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Atomically writes the version file.
    pub fn save(&self, base_dir: &Path) -> Result<()> {
        let path = Self::path(base_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let next = path.with_extension("json.next");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&next, json.as_bytes())?;
        fs::rename(&next, &path)?;
        Ok(())
    }

    fn path(base_dir: &Path) -> std::path::PathBuf {
        base_dir.join("config").join("storage_version.json")
    }
}

/// Returns a runner with no migrations registered yet.
/// Actual migrations are added here as storage schema changes occur.
pub fn default_migration_runner() -> MigrationRunner {
    MigrationRunner::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp_dir(label: &str) -> std::path::PathBuf {
        let dir = env::temp_dir()
            .join(format!("wonder_migrations_test_{}_{}", label, std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn run_pending_no_migrations_keeps_version_at_one() {
        let dir = tmp_dir("no_migrations");
        let runner = MigrationRunner::new();
        let version = runner.run_pending(&dir).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn run_pending_applies_one_migration() {
        struct BumpTo2;
        impl Migration for BumpTo2 {
            fn schema_version_from(&self) -> u16 { 1 }
            fn schema_version_to(&self) -> u16 { 2 }
            fn description(&self) -> &str { "bump to v2" }
            fn migrate(&self, _base_dir: &Path) -> Result<()> { Ok(()) }
        }

        let dir = tmp_dir("applies_one");
        let mut runner = MigrationRunner::new();
        runner.register(Box::new(BumpTo2));
        let version = runner.run_pending(&dir).unwrap();
        assert_eq!(version, 2);

        // Running again is idempotent (no migration from v2 exists).
        let version2 = runner.run_pending(&dir).unwrap();
        assert_eq!(version2, 2);
    }

    #[test]
    fn run_pending_skips_already_applied_migration() {
        struct BumpTo2;
        impl Migration for BumpTo2 {
            fn schema_version_from(&self) -> u16 { 1 }
            fn schema_version_to(&self) -> u16 { 2 }
            fn description(&self) -> &str { "bump to v2" }
            fn migrate(&self, _base_dir: &Path) -> Result<()> {
                panic!("should not run — already applied");
            }
        }

        let dir = tmp_dir("skips_applied");
        // Pre-set version to 2 so migration 1→2 is already applied.
        StorageVersionFile { schema_version: 2 }.save(&dir).unwrap();

        let mut runner = MigrationRunner::new();
        runner.register(Box::new(BumpTo2));
        let version = runner.run_pending(&dir).unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn version_file_roundtrip() {
        let dir = tmp_dir("roundtrip");
        let f = StorageVersionFile { schema_version: 5 };
        f.save(&dir).unwrap();
        let loaded = StorageVersionFile::load(&dir).unwrap();
        assert_eq!(loaded.schema_version, 5);
    }

    #[test]
    fn load_absent_returns_baseline() {
        let dir = tmp_dir("absent");
        let f = StorageVersionFile::load(&dir).unwrap();
        assert_eq!(f.schema_version, StorageVersionFile::BASELINE);
    }
}

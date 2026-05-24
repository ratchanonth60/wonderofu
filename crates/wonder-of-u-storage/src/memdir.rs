//! File-backed memory directory helpers.

use std::{
    collections::BTreeSet,
    env,
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

use sha2::{Digest, Sha256};
use wonder_of_u_core::{Result, WonderError};

use crate::write_text_atomically;

const MEMORY_EXTENSION: &str = "md";
const ENTRYPOINT_NAME: &str = "MEMORY.md";
const PROJECTS_DIRNAME: &str = "projects";
const CLAUDE_DIRNAME: &str = ".claude";
const MEMORY_DIRNAME: &str = "memory";
const TEAM_MEMORY_DIRNAME: &str = "team-memory";
const MAX_COMPONENT_LEN: usize = 120;

/// Memory storage scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    /// Represents user
    User,
    /// Represents project
    Project,
    /// Represents team
    Team,
}

/// A memory file loaded from disk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryEntry {
    /// Stores the id
    pub id: String,
    /// Stores the content
    pub content: String,
    /// Stores the kind
    pub kind: MemoryKind,
    /// Stores the path
    pub path: PathBuf,
    /// Stores the modified
    pub modified: SystemTime,
}

/// Returns the default user memory directory.
#[must_use]
pub fn memdir_root() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CLAUDE_DIRNAME)
        .join(MEMORY_DIRNAME)
}

/// Returns the project-scoped memory directory for a working tree.
#[must_use]
pub fn project_memdir(cwd: &Path) -> PathBuf {
    memdir_root()
        .join(PROJECTS_DIRNAME)
        .join(sanitize_component(&normalize_project_path(cwd), "project"))
}

/// Returns the team-scoped memory directory for a team identifier.
#[must_use]
pub fn team_memdir(team_id: &str) -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CLAUDE_DIRNAME)
        .join(TEAM_MEMORY_DIRNAME)
        .join(sanitize_component(team_id, "team"))
}

/// Lists all `.md` memories in a directory, newest first.
pub fn list_memories(dir: &Path) -> Result<Vec<MemoryEntry>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut memories = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        if path.extension().and_then(OsStr::to_str) != Some(MEMORY_EXTENSION) {
            continue;
        }
        if path.file_name().and_then(OsStr::to_str) == Some(ENTRYPOINT_NAME) {
            continue;
        }
        memories.push(read_memory(&path)?);
    }

    memories.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(memories)
}

/// Reads a memory file from disk.
pub fn read_memory(path: &Path) -> Result<MemoryEntry> {
    if !path.exists() {
        return Err(WonderError::not_found("memory", path.display().to_string()));
    }

    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(WonderError::validation(format!(
            "memory path is not a file: {}",
            path.display()
        )));
    }

    let id = path
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| WonderError::validation(format!("invalid memory path: {}", path.display())))?
        .to_owned();

    Ok(MemoryEntry {
        id,
        content: fs::read_to_string(path)?,
        kind: classify_memory_kind(path),
        path: path.to_path_buf(),
        modified: metadata.modified()?,
    })
}

/// Writes a memory file and returns its full path.
pub fn write_memory(dir: &Path, id: &str, content: &str) -> Result<PathBuf> {
    let path = memory_file_path(dir, id)?;
    fs::create_dir_all(dir)?;
    write_text_atomically(&path, content)?;
    Ok(path)
}

/// Deletes a memory file.
pub fn delete_memory(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Err(WonderError::not_found("memory", path.display().to_string()))
        }
        Err(error) => Err(error.into()),
    }
}

/// Finds memories whose id or content overlaps with a text query.
pub fn find_relevant_memories(dir: &Path, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return Ok(Vec::new());
    }

    let keywords = extract_keywords(&normalized_query);
    let mut matches = list_memories(dir)?
        .into_iter()
        .filter_map(|entry| {
            let score = relevance_score(&entry, &normalized_query, &keywords);
            (score > 0).then_some((score, entry))
        })
        .collect::<Vec<_>>();

    matches.sort_by(|(left_score, left_entry), (right_score, right_entry)| {
        right_score
            .cmp(left_score)
            .then_with(|| right_entry.modified.cmp(&left_entry.modified))
            .then_with(|| left_entry.id.cmp(&right_entry.id))
    });

    Ok(matches
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry)
        .collect())
}

/// Returns true when a memory is older than the requested age.
#[must_use]
pub fn is_expired(entry: &MemoryEntry, max_age_days: u64) -> bool {
    let max_age = Duration::from_secs(max_age_days.saturating_mul(86_400));
    SystemTime::now()
        .duration_since(entry.modified)
        .map(|age| age >= max_age)
        .unwrap_or(false)
}

/// Deletes expired memories and returns the number removed.
pub fn prune_expired(dir: &Path, max_age_days: u64) -> Result<usize> {
    let mut removed = 0;
    for entry in list_memories(dir)? {
        if is_expired(&entry, max_age_days) {
            delete_memory(&entry.path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Formats loaded team memories as a prompt block.
#[must_use]
pub fn team_memory_prompt(entries: &[MemoryEntry]) -> String {
    if entries.is_empty() {
        return "No team memories available.".into();
    }

    let mut lines = vec!["# Team Memory".to_string(), String::new()];
    for entry in entries {
        lines.push(format!("## {}", entry.id));
        lines.push(entry.content.trim().to_string());
        lines.push(String::new());
    }
    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn normalize_project_path(cwd: &Path) -> String {
    cwd.canonicalize()
        .unwrap_or_else(|_| cwd.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn sanitize_component(input: &str, fallback: &str) -> String {
    let sanitized = input
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    if sanitized.is_empty() {
        return fallback.into();
    }
    if sanitized.len() <= MAX_COMPONENT_LEN {
        return sanitized;
    }

    let hash: String = Sha256::digest(input.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!(
        "{}-{}",
        &sanitized[..MAX_COMPONENT_LEN],
        &hash[..16.min(hash.len())]
    )
}

fn classify_memory_kind(path: &Path) -> MemoryKind {
    let team_root = home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CLAUDE_DIRNAME)
        .join(TEAM_MEMORY_DIRNAME);
    if path.starts_with(&team_root) {
        return MemoryKind::Team;
    }

    if path.starts_with(memdir_root().join(PROJECTS_DIRNAME)) {
        return MemoryKind::Project;
    }

    MemoryKind::User
}

fn memory_file_path(dir: &Path, id: &str) -> Result<PathBuf> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(WonderError::validation("memory id cannot be empty"));
    }

    let id_path = Path::new(trimmed);
    if id_path.is_absolute() {
        return Err(WonderError::validation(format!(
            "memory id must be relative: {trimmed}"
        )));
    }
    if id_path
        .components()
        .any(|component| component != Component::Normal(OsStr::new(trimmed)))
    {
        return Err(WonderError::validation(format!(
            "memory id must not contain path separators: {trimmed}"
        )));
    }

    Ok(dir.join(format!("{trimmed}.{MEMORY_EXTENSION}")))
}

fn extract_keywords(query: &str) -> BTreeSet<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.len() >= 2)
        .map(str::to_owned)
        .collect()
}

fn relevance_score(entry: &MemoryEntry, query: &str, keywords: &BTreeSet<String>) -> usize {
    let id = entry.id.to_lowercase();
    let content = entry.content.to_lowercase();
    let mut score = 0;

    if id.contains(query) || content.contains(query) {
        score += 10;
    }

    for keyword in keywords {
        if id.contains(keyword) {
            score += 3;
        }
        if content.contains(keyword) {
            score += 1;
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn writes_reads_lists_and_deletes_memories() {
        let dir = tempdir().expect("tempdir");

        let written = write_memory(dir.path(), "release-plan", "Freeze begins Thursday.")
            .expect("write memory");
        let read = read_memory(&written).expect("read memory");
        let listed = list_memories(dir.path()).expect("list memories");

        assert_eq!(
            written.file_name().and_then(OsStr::to_str),
            Some("release-plan.md")
        );
        assert_eq!(read.id, "release-plan");
        assert_eq!(read.content, "Freeze begins Thursday.");
        assert_eq!(read.kind, MemoryKind::User);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "release-plan");

        delete_memory(&written).expect("delete memory");
        assert!(
            list_memories(dir.path())
                .expect("list after delete")
                .is_empty()
        );
    }

    #[test]
    fn search_returns_best_matches_and_respects_limit() {
        let dir = tempdir().expect("tempdir");
        write_memory(
            dir.path(),
            "deploy-checklist",
            "Run smoke tests before deploy and verify dashboards.",
        )
        .expect("write deploy memory");
        write_memory(
            dir.path(),
            "incident-notes",
            "Dashboard alerts were noisy during the auth deploy incident.",
        )
        .expect("write incident memory");
        write_memory(dir.path(), "tea-break", "Remember to hydrate.").expect("write unrelated");

        let matches =
            find_relevant_memories(dir.path(), "deploy dashboard", 1).expect("find memories");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "deploy-checklist");
    }

    #[test]
    fn prune_expired_removes_matching_files() {
        let dir = tempdir().expect("tempdir");
        write_memory(dir.path(), "old-one", "first").expect("write first");
        write_memory(dir.path(), "old-two", "second").expect("write second");

        let removed = prune_expired(dir.path(), 0).expect("prune memories");

        assert_eq!(removed, 2);
        assert!(
            list_memories(dir.path())
                .expect("list after prune")
                .is_empty()
        );
    }

    #[test]
    fn team_prompt_renders_entries() {
        let entry = MemoryEntry {
            id: "deploy".into(),
            content: "Use canary rollouts.".into(),
            kind: MemoryKind::Team,
            path: PathBuf::from("/team/deploy.md"),
            modified: SystemTime::UNIX_EPOCH,
        };

        let prompt = team_memory_prompt(&[entry]);

        assert!(prompt.contains("# Team Memory"));
        assert!(prompt.contains("## deploy"));
        assert!(prompt.contains("Use canary rollouts."));
    }

    #[test]
    fn expired_handles_past_and_future_times() {
        let stale = MemoryEntry {
            id: "stale".into(),
            content: String::new(),
            kind: MemoryKind::User,
            path: PathBuf::from("stale.md"),
            modified: SystemTime::UNIX_EPOCH,
        };
        let fresh = MemoryEntry {
            id: "fresh".into(),
            content: String::new(),
            kind: MemoryKind::User,
            path: PathBuf::from("fresh.md"),
            modified: SystemTime::now() + Duration::from_secs(60),
        };

        assert!(is_expired(&stale, 1));
        assert!(!is_expired(&fresh, 1));
    }
}

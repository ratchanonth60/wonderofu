//! Disk-based skill loader: reads `.md` files with YAML frontmatter from directories.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Loads skills from `.md` files with YAML frontmatter from disk directories.
pub struct DiskSkillLoader {
    /// `(directory_path, source_label)` pairs, searched in order.
    search_roots: Vec<(PathBuf, String)>,
}

/// A skill loaded from a `.md` file on disk.
pub struct LoadedSkill {
    /// Skill name (from frontmatter `name:` field, or derived from file stem).
    pub name: String,
    /// Human-readable description (from frontmatter `description:` field).
    pub description: String,
    /// Optional version string from frontmatter.
    pub version: Option<String>,
    /// The prompt body (everything after the closing `---` delimiter).
    pub prompt: String,
    /// Absolute path of the source file.
    pub source_path: PathBuf,
}

impl DiskSkillLoader {
    /// Creates a loader with default search roots.
    ///
    /// - `user_dir` is typically `~/.claude/skills/`
    /// - `project_dir` is typically `.claude/skills/` in the current working directory
    #[must_use]
    pub fn with_defaults(user_dir: &Path, project_dir: Option<&Path>) -> Self {
        let mut search_roots = Vec::new();
        if let Some(project) = project_dir {
            search_roots.push((project.to_path_buf(), "project".to_string()));
        }
        search_roots.push((user_dir.to_path_buf(), "user".to_string()));
        Self { search_roots }
    }

    /// Scans all `search_roots` for `*.md` files, parses frontmatter, and returns
    /// successfully loaded skills. Files with missing or malformed frontmatter are
    /// silently skipped.
    #[must_use]
    pub fn load_all(&self) -> Vec<LoadedSkill> {
        let mut skills = Vec::new();
        for (root, _label) in &self.search_roots {
            if let Ok(entries) = fs::read_dir(root) {
                let mut paths: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
                    .collect();
                paths.sort();

                for path in paths {
                    let content = match fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let Some((frontmatter, prompt)) = parse_skill_markdown(&content) else {
                        continue;
                    };

                    // Derive name: prefer frontmatter `name:`, fall back to file stem.
                    let name = frontmatter
                        .get("name")
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .or_else(|| {
                            path.file_stem()
                                .and_then(|s| s.to_str())
                                .map(|s| s.to_string())
                        });

                    let Some(name) = name else { continue };

                    // description is required
                    let Some(description) = frontmatter.get("description").cloned() else {
                        continue;
                    };
                    if description.is_empty() {
                        continue;
                    }

                    let version = frontmatter.get("version").cloned();

                    skills.push(LoadedSkill {
                        name,
                        description,
                        version,
                        prompt,
                        source_path: path,
                    });
                }
            }
        }
        skills
    }
}

/// Parse a skill `.md` file: split on `---` delimiters, parse `key: value` frontmatter
/// manually. Returns `None` if frontmatter is absent or malformed.
///
/// The expected format is:
/// ```text
/// ---
/// name: my-skill
/// description: Does something useful
/// version: 1.0.0
/// ---
///
/// The actual prompt text starts here...
/// ```
fn parse_skill_markdown(content: &str) -> Option<(BTreeMap<String, String>, String)> {
    // Must start with `---\n` (or `---\r\n` for Windows line endings).
    let after_first = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;

    // Find the closing `---` on its own line.
    let closing = after_first
        .find("\n---\n")
        .or_else(|| after_first.find("\n---\r\n"));

    let (fm_block, body_start) = if let Some(pos) = closing {
        let fm = &after_first[..pos];
        // Skip past `\n---\n` (5 bytes) or `\n---\r\n` (6 bytes).
        let skip = if after_first[pos + 1..].starts_with("---\r\n") {
            6
        } else {
            5
        };
        (fm, &after_first[pos + skip..])
    } else {
        // Try end-of-string terminator `\n---` with nothing after.
        let alt_closing = after_first
            .rfind("\n---")
            .filter(|&p| after_first[p + 4..].trim().is_empty())?;
        (&after_first[..alt_closing], "")
    };

    let mut map = BTreeMap::new();
    for line in fm_block.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split on the first `:` only.
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim().to_string();
        if !key.is_empty() {
            map.insert(key, value);
        }
    }

    Some((map, body_start.to_string()))
}

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn write_skill_md(dir: &Path, file_name: &str, content: &str) {
        fs::create_dir_all(dir).expect("create dir");
        fs::write(dir.join(file_name), content).expect("write skill md");
    }

    #[test]
    fn load_all_returns_valid_skill() {
        let dir = unique_test_dir("disk-skill-loader-valid");
        let content = "\
---
name: my-skill
description: Does something useful
version: 1.0.0
---

The actual prompt text starts here.
";
        write_skill_md(&dir, "my-skill.md", content);

        let loader = DiskSkillLoader::with_defaults(&dir, None);
        let skills = loader.load_all();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert_eq!(skills[0].description, "Does something useful");
        assert_eq!(skills[0].version.as_deref(), Some("1.0.0"));
        assert!(skills[0].prompt.contains("The actual prompt text"));
    }

    #[test]
    fn load_all_skips_file_with_missing_frontmatter() {
        let dir = unique_test_dir("disk-skill-loader-no-fm");
        let content = "No frontmatter here, just plain text.";
        write_skill_md(&dir, "plain.md", content);

        let loader = DiskSkillLoader::with_defaults(&dir, None);
        let skills = loader.load_all();

        assert!(
            skills.is_empty(),
            "expected no skills from file without frontmatter"
        );
    }

    #[test]
    fn load_all_skips_file_with_malformed_yaml() {
        let dir = unique_test_dir("disk-skill-loader-malformed");
        // Frontmatter block opens but never closes.
        let content = "---\nname: broken\n";
        write_skill_md(&dir, "broken.md", content);

        let loader = DiskSkillLoader::with_defaults(&dir, None);
        let skills = loader.load_all();

        assert!(
            skills.is_empty(),
            "expected no skills from malformed frontmatter"
        );
    }

    #[test]
    fn load_all_skips_file_missing_description() {
        let dir = unique_test_dir("disk-skill-loader-no-desc");
        let content = "---\nname: no-desc\n---\n\nPrompt body.";
        write_skill_md(&dir, "no-desc.md", content);

        let loader = DiskSkillLoader::with_defaults(&dir, None);
        let skills = loader.load_all();

        assert!(
            skills.is_empty(),
            "expected no skills from file without description"
        );
    }

    #[test]
    fn load_all_uses_file_stem_when_name_absent() {
        let dir = unique_test_dir("disk-skill-loader-stem");
        let content = "---\ndescription: No name in frontmatter\n---\n\nPrompt.";
        write_skill_md(&dir, "inferred-name.md", content);

        let loader = DiskSkillLoader::with_defaults(&dir, None);
        let skills = loader.load_all();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "inferred-name");
    }

    #[test]
    fn load_all_merges_project_and_user_dirs() {
        let user_dir = unique_test_dir("disk-skill-loader-user");
        let project_dir = unique_test_dir("disk-skill-loader-project");

        write_skill_md(
            &user_dir,
            "user-skill.md",
            "---\nname: user-skill\ndescription: User skill\n---\nUser prompt.",
        );
        write_skill_md(
            &project_dir,
            "project-skill.md",
            "---\nname: project-skill\ndescription: Project skill\n---\nProject prompt.",
        );

        let loader = DiskSkillLoader::with_defaults(&user_dir, Some(&project_dir));
        let skills = loader.load_all();
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"project-skill"), "missing project-skill");
        assert!(names.contains(&"user-skill"), "missing user-skill");
    }

    #[test]
    fn parse_skill_markdown_returns_none_for_empty_content() {
        assert!(parse_skill_markdown("").is_none());
    }

    #[test]
    fn parse_skill_markdown_handles_values_with_colons() {
        let content = "---\nname: my-skill\ndescription: Foo: bar baz\n---\nBody.";
        let result = parse_skill_markdown(content);
        assert!(result.is_some());
        let (fm, _) = result.unwrap();
        // Only first `:` is used as separator, so value is "bar baz"
        assert_eq!(
            fm.get("description").map(String::as_str),
            Some("Foo: bar baz")
        );
    }
}

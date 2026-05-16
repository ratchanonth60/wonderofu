use std::{
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use glob::Pattern;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use walkdir::WalkDir;
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
    resolve_path,
};

use crate::{base_spec, display_path, parse_input, require_non_empty_path, require_non_empty_text};

const DEFAULT_GLOB_LIMIT: usize = 200;
const DEFAULT_GREP_LIMIT: usize = 100;
/// Enumerates glob entry type
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobEntryType {
    /// Represents all
    All,
    /// Represents files
    #[default]
    Files,
    /// Represents directories
    Directories,
}

impl GlobEntryType {
    fn matches(self, path: &Path) -> bool {
        match self {
            Self::All => true,
            Self::Files => path.is_file(),
            Self::Directories => path.is_dir(),
        }
    }
}
/// Represents glob input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobInput {
    /// Stores the pattern
    pub pattern: String,
    /// Stores the path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Stores the entry type
    #[serde(default)]
    pub entry_type: GlobEntryType,
    /// Stores the limit
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl GlobInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("glob", "pattern", &self.pattern)?;
        if let Some(path) = &self.path {
            require_non_empty_path("glob", "path", path)?;
        }
        if self.limit == Some(0) {
            return Err(WonderError::validation(
                "glob limit must be greater than zero",
            ));
        }
        compile_glob(&self.pattern)?;
        Ok(())
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_GLOB_LIMIT)
    }
}
/// Represents grep input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrepInput {
    /// Stores the pattern
    pub pattern: String,
    /// Stores the path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Stores the glob
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    /// Stores the case insensitive
    #[serde(default)]
    pub case_insensitive: bool,
    /// Stores the limit
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl GrepInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("grep", "pattern", &self.pattern)?;
        if let Some(path) = &self.path {
            require_non_empty_path("grep", "path", path)?;
        }
        if let Some(glob) = &self.glob {
            compile_glob(glob)?;
        }
        if self.limit == Some(0) {
            return Err(WonderError::validation(
                "grep limit must be greater than zero",
            ));
        }
        compile_regex(&self.pattern, self.case_insensitive)?;
        Ok(())
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_GREP_LIMIT)
    }
}
/// Represents glob tool
#[derive(Debug, Default)]
pub struct GlobTool;
/// Represents grep tool
#[derive(Debug, Default)]
pub struct GrepTool;

#[async_trait]
impl Tool for GlobTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "glob",
            "Find files and directories by glob pattern",
            ToolKind::Search,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "pattern",
                    ToolSchema::string("glob pattern to match against relative paths"),
                )
                .property(
                    "path",
                    ToolSchema::string("optional directory to search from"),
                )
                .property(
                    "entry_type",
                    ToolSchema::enumeration(
                        "which entry types to include",
                        ["all", "files", "directories"],
                    ),
                )
                .property(
                    "limit",
                    ToolSchema::integer("maximum number of matches to return"),
                )
                .required("pattern"),
        );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<GlobInput>("glob", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<GlobInput>("glob", &input)?;
        input.validate()?;

        let root = resolve_search_root(&context, input.path.as_deref(), true, "glob")?;
        let matcher = compile_glob(&input.pattern)?;
        let mut matches = collect_entries(&root)?
            .into_iter()
            .filter(|path| input.entry_type.matches(path))
            .filter(|path| {
                relative_to_root(path, &root)
                    .map(|relative| matcher.matches_path(relative))
                    .unwrap_or(false)
            })
            .map(|path| display_path(&path, &context.cwd))
            .collect::<Vec<_>>();
        matches.sort();
        matches.truncate(input.limit());

        let mut result = ToolResult::success(
            use_id,
            if matches.is_empty() {
                "No matches found.".into()
            } else {
                matches.join("\n")
            },
        );
        result.metadata = json!({
            "path": root.display().to_string(),
            "pattern": input.pattern,
            "matches": matches.len(),
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "grep",
            "Search UTF-8 files with a regular expression",
            ToolKind::Search,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "pattern",
                    ToolSchema::string("regular expression to search for"),
                )
                .property(
                    "path",
                    ToolSchema::string("optional file or directory to search"),
                )
                .property(
                    "glob",
                    ToolSchema::string("optional glob used to filter candidate files"),
                )
                .property(
                    "case_insensitive",
                    ToolSchema::boolean("when true, search without case sensitivity"),
                )
                .property(
                    "limit",
                    ToolSchema::integer("maximum number of matches to return"),
                )
                .required("pattern"),
        );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<GrepInput>("grep", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<GrepInput>("grep", &input)?;
        input.validate()?;

        let root = resolve_search_root(&context, input.path.as_deref(), false, "grep")?;
        let regex = compile_regex(&input.pattern, input.case_insensitive)?;
        let filter = input.glob.as_deref().map(compile_glob).transpose()?;
        let files = collect_files(&root)?;
        let files_scanned = files.len();
        let mut matches = Vec::new();
        let mut skipped_files = 0usize;

        'files: for path in files {
            if let Some(filter) = &filter
                && !relative_to_root(&path, &root)
                    .is_some_and(|relative| filter.matches_path(relative))
            {
                continue;
            }

            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    skipped_files += 1;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };

            for (line_number, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(format!(
                        "{}:{}:{}",
                        display_path(&path, &context.cwd),
                        line_number + 1,
                        line
                    ));
                    if matches.len() >= input.limit() {
                        break 'files;
                    }
                }
            }
        }

        let mut result = ToolResult::success(
            use_id,
            if matches.is_empty() {
                "No matches found.".into()
            } else {
                matches.join("\n")
            },
        );
        result.metadata = json!({
            "path": root.display().to_string(),
            "pattern": input.pattern,
            "matches": matches.len(),
            "files_scanned": files_scanned,
            "files_skipped": skipped_files,
        });
        Ok(result)
    }
}

fn compile_glob(pattern: &str) -> Result<Pattern> {
    Pattern::new(pattern).map_err(|error| {
        WonderError::validation(format!("invalid glob pattern `{pattern}`: {error}"))
    })
}

fn compile_regex(pattern: &str, case_insensitive: bool) -> Result<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|error| {
            WonderError::validation(format!("invalid grep pattern `{pattern}`: {error}"))
        })
}

fn resolve_search_root(
    context: &ToolContext,
    path: Option<&Path>,
    require_directory: bool,
    tool_name: &str,
) -> Result<PathBuf> {
    let root = path
        .map(|path| resolve_path(path, &context.cwd))
        .unwrap_or_else(|| context.cwd.clone());
    let metadata = fs::metadata(&root).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => WonderError::not_found("path", root.display().to_string()),
        _ => error.into(),
    })?;
    if require_directory && !metadata.is_dir() {
        return Err(WonderError::validation(format!(
            "{tool_name} requires a directory path: {}",
            root.display()
        )));
    }
    Ok(root)
}

fn collect_entries(root: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry.map_err(|error| {
            WonderError::validation(format!("failed to walk {}: {error}", root.display()))
        })?;
        entries.push(entry.path().to_path_buf());
    }
    entries.sort();
    Ok(entries)
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    if root.is_file() {
        return Ok(vec![root.to_path_buf()]);
    }

    let mut files = collect_entries(root)?
        .into_iter()
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn relative_to_root<'a>(path: &'a Path, root: &'a Path) -> Option<&'a Path> {
    if root.is_file() {
        path.file_name().map(Path::new)
    } else {
        path.strip_prefix(root).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, ToolContext, ToolUseId};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
        }
    }

    #[test]
    fn glob_validation_rejects_invalid_pattern() {
        let tool = GlobTool;
        let error = tool
            .validate_input(&json!({ "pattern": "[" }))
            .expect_err("invalid glob");

        assert!(error.to_string().contains("invalid glob pattern"));
    }

    #[test]
    fn glob_finds_matching_files() {
        let dir = unique_test_dir("tools-glob");
        fs::create_dir_all(dir.join("src")).expect("mkdir src");
        fs::create_dir_all(dir.join("docs")).expect("mkdir docs");
        fs::write(dir.join("src/lib.rs"), "pub fn demo() {}\n").expect("write rs");
        fs::write(dir.join("docs/readme.md"), "hello\n").expect("write md");
        let tool = GlobTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "pattern": "src/**/*.rs" }),
        ))
        .expect("run glob");

        assert!(result.success);
        assert_eq!(result.content, "src/lib.rs");
    }

    #[test]
    fn grep_validation_rejects_invalid_regex() {
        let tool = GrepTool;
        let error = tool
            .validate_input(&json!({ "pattern": "(" }))
            .expect_err("invalid regex");

        assert!(error.to_string().contains("invalid grep pattern"));
    }

    #[test]
    fn grep_searches_matching_lines() {
        let dir = unique_test_dir("tools-grep");
        fs::create_dir_all(dir.join("src")).expect("mkdir src");
        fs::write(dir.join("src/lib.rs"), "alpha\nbeta\nalpha beta\n").expect("write file");
        let tool = GrepTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "pattern": "alpha", "glob": "src/**/*.rs" }),
        ))
        .expect("run grep");

        assert!(result.success);
        assert_eq!(
            result.content,
            "src/lib.rs:1:alpha\nsrc/lib.rs:3:alpha beta"
        );
    }
}

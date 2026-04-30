use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
    resolve_path,
};

use crate::{base_spec, display_path, parse_input, require_non_empty_path, require_non_empty_text};

const DEFAULT_FILE_READ_MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReadInput {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
}

impl FileReadInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_path("file_read", "path", &self.path)?;
        if self.start_line == Some(0) {
            return Err(WonderError::validation(
                "file_read start_line must be greater than zero",
            ));
        }
        if self.end_line == Some(0) {
            return Err(WonderError::validation(
                "file_read end_line must be greater than zero",
            ));
        }
        if self.max_bytes == Some(0) {
            return Err(WonderError::validation(
                "file_read max_bytes must be greater than zero",
            ));
        }
        if let (Some(start), Some(end)) = (self.start_line, self.end_line)
            && end < start
        {
            return Err(WonderError::validation(
                "file_read end_line must be greater than or equal to start_line",
            ));
        }
        Ok(())
    }

    fn max_bytes(&self) -> usize {
        self.max_bytes.unwrap_or(DEFAULT_FILE_READ_MAX_BYTES)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileWriteMode {
    #[default]
    Create,
    Overwrite,
    Append,
}

impl FileWriteMode {
    fn action_label(self) -> &'static str {
        match self {
            Self::Create => "wrote",
            Self::Overwrite => "overwrote",
            Self::Append => "appended",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWriteInput {
    pub path: PathBuf,
    pub content: String,
    #[serde(default)]
    pub mode: FileWriteMode,
}

impl FileWriteInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_path("file_write", "path", &self.path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEditInput {
    pub path: PathBuf,
    pub old_text: String,
    pub new_text: String,
    #[serde(default)]
    pub replace_all: bool,
}

impl FileEditInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_path("file_edit", "path", &self.path)?;
        require_non_empty_text("file_edit", "old_text", &self.old_text)
    }
}

#[derive(Debug, Default)]
pub struct FileReadTool;

#[derive(Debug, Default)]
pub struct FileWriteTool;

#[derive(Debug, Default)]
pub struct FileEditTool;

#[async_trait]
impl Tool for FileReadTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("file_read", "Read a UTF-8 file", ToolKind::FileRead)
            .with_input_schema(
                ToolSchema::object()
                    .property("path", ToolSchema::string("path to the file to read"))
                    .property(
                        "start_line",
                        ToolSchema::integer("optional 1-indexed starting line to include"),
                    )
                    .property(
                        "end_line",
                        ToolSchema::integer("optional 1-indexed ending line to include"),
                    )
                    .property(
                        "max_bytes",
                        ToolSchema::integer("optional maximum number of bytes allowed to be read"),
                    )
                    .required("path"),
            );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<FileReadInput>("file_read", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<FileReadInput>("file_read", &input)?;
        input.validate()?;

        let path = resolve_path(&input.path, &context.cwd);
        let bytes = fs::read(&path).map_err(|error| map_file_error("file", &path, error))?;
        if bytes.len() > input.max_bytes() {
            return Err(WonderError::validation(format!(
                "file_read refused to read {} because it is {} bytes (limit {})",
                path.display(),
                bytes.len(),
                input.max_bytes()
            )));
        }
        let text = String::from_utf8(bytes).map_err(|error| {
            WonderError::validation(format!("file_read requires UTF-8 content: {error}"))
        })?;
        let lines = text.lines().collect::<Vec<_>>();
        let total_lines = lines.len();

        let selected = if total_lines == 0 {
            Vec::new()
        } else {
            let start = input.start_line.unwrap_or(1);
            if start > total_lines {
                return Err(WonderError::validation(format!(
                    "file_read start_line {start} is past the end of {} ({} lines)",
                    display_path(&path, &context.cwd),
                    total_lines
                )));
            }
            let end = input.end_line.unwrap_or(total_lines);
            if end > total_lines {
                return Err(WonderError::validation(format!(
                    "file_read end_line {end} is past the end of {} ({} lines)",
                    display_path(&path, &context.cwd),
                    total_lines
                )));
            }
            lines[start - 1..end]
                .iter()
                .enumerate()
                .map(|(index, line)| format!("{}. {}", start + index, line))
                .collect::<Vec<_>>()
        };

        let mut result = ToolResult::success(use_id, selected.join("\n"));
        result.metadata = json!({
            "path": path.display().to_string(),
            "display_path": display_path(&path, &context.cwd),
            "total_lines": total_lines,
            "start_line": input.start_line.unwrap_or(1),
            "end_line": input.end_line.unwrap_or(total_lines),
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn spec(&self) -> ToolSpec {
        base_spec("file_write", "Write text to a file", ToolKind::FileWrite).with_input_schema(
            ToolSchema::object()
                .property("path", ToolSchema::string("path to the file to write"))
                .property("content", ToolSchema::string("text content to write"))
                .property(
                    "mode",
                    ToolSchema::enumeration(
                        "write strategy to use",
                        ["create", "overwrite", "append"],
                    ),
                )
                .required("path")
                .required("content"),
        )
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<FileWriteInput>("file_write", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<FileWriteInput>("file_write", &input)?;
        input.validate()?;

        let path = resolve_path(&input.path, &context.cwd);
        ensure_parent_dir("file_write", &path)?;
        if path.is_dir() {
            return Err(WonderError::validation(format!(
                "file_write target is a directory: {}",
                path.display()
            )));
        }

        let mut file = open_for_write(&path, input.mode)?;
        file.write_all(input.content.as_bytes())?;
        file.sync_all()?;

        let mut result = ToolResult::success(
            use_id,
            format!(
                "{} {} bytes to {}",
                input.mode.action_label(),
                input.content.len(),
                display_path(&path, &context.cwd)
            ),
        );
        result.metadata = json!({
            "path": path.display().to_string(),
            "mode": input.mode,
            "bytes": input.content.len(),
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for FileEditTool {
    fn spec(&self) -> ToolSpec {
        base_spec(
            "file_edit",
            "Replace text inside a file",
            ToolKind::FileWrite,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("path", ToolSchema::string("path to the file to edit"))
                .property("old_text", ToolSchema::string("text to replace"))
                .property("new_text", ToolSchema::string("replacement text"))
                .property(
                    "replace_all",
                    ToolSchema::boolean(
                        "when true, replace every occurrence instead of exactly one",
                    ),
                )
                .required("path")
                .required("old_text")
                .required("new_text"),
        )
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<FileEditInput>("file_edit", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<FileEditInput>("file_edit", &input)?;
        input.validate()?;

        let path = resolve_path(&input.path, &context.cwd);
        let text = read_utf8_file("file_edit", &path)?;
        let occurrences = text.matches(&input.old_text).count();
        if occurrences == 0 {
            return Err(WonderError::validation(format!(
                "file_edit could not find the requested text in {}",
                display_path(&path, &context.cwd)
            )));
        }
        if !input.replace_all && occurrences > 1 {
            return Err(WonderError::validation(format!(
                "file_edit found {} matches in {}; set replace_all to true to continue",
                occurrences,
                display_path(&path, &context.cwd)
            )));
        }

        let replacements = if input.replace_all { occurrences } else { 1 };
        let updated = if input.replace_all {
            text.replace(&input.old_text, &input.new_text)
        } else {
            text.replacen(&input.old_text, &input.new_text, 1)
        };
        overwrite_text_file(&path, &updated)?;

        let mut result = ToolResult::success(
            use_id,
            format!(
                "edited {} ({} replacement{})",
                display_path(&path, &context.cwd),
                replacements,
                if replacements == 1 { "" } else { "s" }
            ),
        );
        result.metadata = json!({
            "path": path.display().to_string(),
            "replacements": replacements,
        });
        Ok(result)
    }
}

fn map_file_error(kind: &str, path: &Path, error: std::io::Error) -> WonderError {
    match error.kind() {
        std::io::ErrorKind::NotFound => WonderError::not_found(kind, path.display().to_string()),
        _ => error.into(),
    }
}

fn read_utf8_file(tool_name: &str, path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|error| map_file_error("file", path, error))?;
    String::from_utf8(bytes).map_err(|error| {
        WonderError::validation(format!("{tool_name} requires UTF-8 content: {error}"))
    })
}

fn ensure_parent_dir(tool_name: &str, path: &Path) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        WonderError::validation(format!("{tool_name} requires a parent directory"))
    })?;
    let metadata =
        fs::metadata(parent).map_err(|error| map_file_error("directory", parent, error))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(WonderError::validation(format!(
            "{tool_name} parent is not a directory: {}",
            parent.display()
        )))
    }
}

fn open_for_write(path: &Path, mode: FileWriteMode) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true);
    match mode {
        FileWriteMode::Create => {
            options.create_new(true);
        }
        FileWriteMode::Overwrite => {
            options.create(true).truncate(true);
        }
        FileWriteMode::Append => {
            options.create(true).append(true);
        }
    }
    Ok(options.open(path)?)
}

fn overwrite_text_file(path: &Path, content: &str) -> Result<()> {
    ensure_parent_dir("file_edit", path)?;
    let mut file = File::create(path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, SessionId, ToolContext, ToolUseId,
    };
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
        }
    }

    #[test]
    fn file_read_validation_rejects_inverted_ranges() {
        let tool = FileReadTool;
        let error = tool
            .validate_input(&json!({ "path": "notes.txt", "start_line": 3, "end_line": 1 }))
            .expect_err("invalid range");

        assert!(error.to_string().contains("end_line"));
    }

    #[test]
    fn file_read_permission_requires_review_for_absolute_path_outside_scope() {
        let tool = FileReadTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(&context, &json!({ "path": "/secret.txt" }));

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn file_read_reads_line_ranges() {
        let dir = unique_test_dir("tools-file-read");
        let path = dir.join("notes.txt");
        fs::write(&path, "one\ntwo\nthree\n").expect("seed file");
        let tool = FileReadTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "path": "notes.txt", "start_line": 2, "end_line": 3 }),
        ))
        .expect("read file");

        assert!(result.success);
        assert_eq!(result.content, "2. two\n3. three");
    }

    #[test]
    fn file_write_creates_and_appends() {
        let dir = unique_test_dir("tools-file-write");
        let tool = FileWriteTool;
        let context = tool_context(dir.clone());

        block_on(tool.execute(
            context.clone(),
            ToolUseId::new(),
            json!({ "path": "notes.txt", "content": "hello" }),
        ))
        .expect("create file");
        let append = block_on(tool.execute(
            context,
            ToolUseId::new(),
            json!({ "path": "notes.txt", "content": " world", "mode": "append" }),
        ))
        .expect("append file");

        assert!(append.success);
        assert_eq!(
            fs::read_to_string(dir.join("notes.txt")).expect("load"),
            "hello world"
        );
    }

    #[test]
    fn file_edit_requires_replace_all_for_ambiguous_edits() {
        let dir = unique_test_dir("tools-file-edit-ambiguous");
        fs::write(dir.join("notes.txt"), "hello\nhello\n").expect("seed file");
        let tool = FileEditTool;
        let error = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "path": "notes.txt", "old_text": "hello", "new_text": "hi" }),
        ))
        .expect_err("ambiguous edit");

        assert!(error.to_string().contains("replace_all"));
    }

    #[test]
    fn file_edit_replaces_text() {
        let dir = unique_test_dir("tools-file-edit");
        fs::write(dir.join("notes.txt"), "hello\nworld\n").expect("seed file");
        let tool = FileEditTool;
        let result = block_on(tool.execute(
            tool_context(dir.clone()),
            ToolUseId::new(),
            json!({ "path": "notes.txt", "old_text": "world", "new_text": "rust" }),
        ))
        .expect("edit file");

        assert!(result.success);
        assert_eq!(
            fs::read_to_string(dir.join("notes.txt")).expect("load"),
            "hello\nrust\n"
        );
    }
}

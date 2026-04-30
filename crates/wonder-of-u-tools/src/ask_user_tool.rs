//! Interactive ask-user tool support.

use std::io::{self, BufRead, Write};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
};

use crate::{base_spec, parse_input, require_non_empty_text};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserInput {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

impl AskUserInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("ask_user", "question", &self.question)?;
        if let Some(options) = &self.options {
            if options.iter().any(|option| option.trim().is_empty()) {
                return Err(WonderError::validation(
                    "ask_user options must be non-empty when provided",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn spec(&self) -> ToolSpec {
        base_spec("ask_user", "Ask the user for input", ToolKind::Interaction).with_input_schema(
            ToolSchema::object()
                .property("question", ToolSchema::string("question to present"))
                .property(
                    "options",
                    json!({
                        "type": "array",
                        "description": "optional predefined choices",
                        "items": { "type": "string" }
                    }),
                )
                .required("question"),
        )
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<AskUserInput>("ask_user", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<AskUserInput>("ask_user", &input)?;
        input.validate()?;

        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = stdin.lock();
        let mut writer = stdout.lock();
        let response = ask_with_io(&input, &mut reader, &mut writer)?;
        Ok(ToolResult::success(use_id, response))
    }
}

fn ask_with_io<R: BufRead, W: Write>(
    input: &AskUserInput,
    reader: &mut R,
    writer: &mut W,
) -> Result<String> {
    writer.write_all(render_prompt(input).as_bytes())?;
    writer.flush()?;

    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(WonderError::validation("ask_user received no response"));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn render_prompt(input: &AskUserInput) -> String {
    let mut prompt = String::new();
    prompt.push_str(&input.question);
    prompt.push('\n');
    if let Some(options) = &input.options {
        prompt.push_str("Options: ");
        prompt.push_str(&options.join(", "));
        prompt.push('\n');
    }
    prompt.push_str("> ");
    prompt
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::*;

    #[test]
    fn ask_user_validation_rejects_empty_question() {
        let tool = AskUserTool;
        let error = tool
            .validate_input(&json!({ "question": "  " }))
            .expect_err("empty question");

        assert!(error.to_string().contains("question"));
    }

    #[test]
    fn ask_user_reads_response_from_input() {
        let mut reader = Cursor::new(b"yes\n".to_vec());
        let mut writer = Vec::new();
        let response = ask_with_io(
            &AskUserInput {
                question: "Continue?".into(),
                options: Some(vec!["yes".into(), "no".into()]),
            },
            &mut reader,
            &mut writer,
        )
        .expect("response");

        assert_eq!(response, "yes");
        assert!(
            String::from_utf8(writer)
                .expect("prompt")
                .contains("Options: yes, no")
        );
    }

    #[test]
    fn ask_user_errors_when_input_is_closed() {
        let mut reader = Cursor::new(Vec::<u8>::new());
        let mut writer = Vec::new();
        let error = ask_with_io(
            &AskUserInput {
                question: "Continue?".into(),
                options: None,
            },
            &mut reader,
            &mut writer,
        )
        .expect_err("missing response");

        assert!(error.to_string().contains("no response"));
    }
}

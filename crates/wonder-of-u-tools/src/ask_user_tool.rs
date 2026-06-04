//! Interactive ask-user tool support.

use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
};

use crate::{base_spec, parse_input, require_non_empty_text};
/// Represents ask user input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserInput {
    /// Stores the question
    pub question: String,
    /// Stores the options
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

impl AskUserInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("ask_user", "question", &self.question)?;
        if let Some(options) = &self.options
            && options.iter().any(|opt| opt.trim().is_empty())
        {
            return Err(WonderError::validation(
                "ask_user options must be non-empty when provided",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskUserQuestionOption {
    label: String,
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskUserQuestion {
    question: String,
    header: String,
    options: Vec<AskUserQuestionOption>,
    #[serde(default, rename = "multiSelect", alias = "multi_select")]
    multi_select: bool,
}

impl AskUserQuestion {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("ask_user", "questions[].question", &self.question)?;
        require_non_empty_text("ask_user", "questions[].header", &self.header)?;
        if self.options.len() < 2 || self.options.len() > 4 {
            return Err(WonderError::validation(
                "ask_user source-compatible questions require 2-4 options",
            ));
        }
        if self.multi_select {
            return Err(WonderError::validation(
                "ask_user source-compatible `multiSelect` is not supported in the Rust runtime",
            ));
        }

        let mut labels = BTreeSet::new();
        for option in &self.options {
            require_non_empty_text("ask_user", "questions[].options[].label", &option.label)?;
            require_non_empty_text(
                "ask_user",
                "questions[].options[].description",
                &option.description,
            )?;
            if option.preview.is_some() {
                return Err(WonderError::validation(
                    "ask_user source-compatible option previews are not supported in the Rust runtime",
                ));
            }
            if !labels.insert(option.label.trim().to_string()) {
                return Err(WonderError::validation(
                    "ask_user source-compatible option labels must be unique",
                ));
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskUserCompatInput {
    questions: Vec<AskUserQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    answers: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    annotations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

impl AskUserCompatInput {
    fn validate(&self) -> Result<()> {
        if self.answers.is_some() {
            return Err(WonderError::validation(
                "ask_user source-compatible `answers` is output-only and not supported as input",
            ));
        }
        if self.annotations.is_some() {
            return Err(WonderError::validation(
                "ask_user source-compatible `annotations` is not supported in the Rust runtime",
            ));
        }
        if self.metadata.is_some() {
            return Err(WonderError::validation(
                "ask_user source-compatible `metadata` is not supported in the Rust runtime",
            ));
        }
        if self.questions.len() != 1 {
            return Err(WonderError::validation(
                "ask_user source-compatible input requires exactly one question in the Rust runtime",
            ));
        }
        self.questions[0].validate()
    }

    fn into_basic_input(self) -> AskUserInput {
        let question = self
            .questions
            .into_iter()
            .next()
            .expect("validated question");
        AskUserInput {
            question: question.question,
            options: Some(
                question
                    .options
                    .into_iter()
                    .map(|option| option.label)
                    .collect(),
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ParsedAskUserInput {
    Basic(AskUserInput),
    Compat(AskUserCompatInput),
}
/// Represents ask user tool
#[derive(Debug, Default)]
pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("ask_user", "Ask the user for input", ToolKind::Interaction)
            .with_input_schema(ToolSchema::object());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "question": ToolSchema::string("question to present"),
                "options": {
                    "type": "array",
                    "description": "optional predefined choices",
                    "items": { "type": "string" }
                },
                "questions": {
                    "type": "array",
                    "description": "source-compatible multiple-choice questions",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": ToolSchema::string("question to present"),
                            "header": ToolSchema::string("short source-compatible question label"),
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": ToolSchema::string("choice label"),
                                        "description": ToolSchema::string("choice description"),
                                        "preview": ToolSchema::string(
                                            "source-compatible preview payload; unsupported in the Rust runtime",
                                        ),
                                    },
                                    "required": ["label", "description"],
                                    "additionalProperties": false,
                                },
                            },
                            "multiSelect": ToolSchema::boolean(
                                "source-compatible multi-select toggle; unsupported in the Rust runtime",
                            ),
                        },
                        "required": ["question", "header", "options"],
                        "additionalProperties": false,
                    }
                },
                "answers": {
                    "type": "object",
                    "description": "source-compatible answer payload; unsupported as input in the Rust runtime",
                },
                "annotations": {
                    "type": "object",
                    "description": "source-compatible annotations payload; unsupported in the Rust runtime",
                },
                "metadata": {
                    "type": "object",
                    "description": "source-compatible metadata payload; unsupported in the Rust runtime",
                }
            },
            "additionalProperties": false
        });
        spec.aliases.push("AskUserQuestion".into());
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_ask_user_input(input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_ask_user_input(&input)?;
        input.validate()?;
        let input = input.into_basic_input();

        // TUI mode: use the interaction channel so we don't conflict with
        // crossterm's raw-mode event reader.
        if let Some(rx) = context.interaction_rx {
            let answer = rx
                .lock()
                .map_err(|_| WonderError::internal("ask_user: interaction mutex poisoned"))?
                .recv()
                .map_err(|_| {
                    WonderError::validation(
                        "ask_user: interaction channel closed before answer arrived",
                    )
                })?;
            return Ok(ToolResult::success(use_id, answer));
        }

        // Non-TUI (CLI / headless) mode: use stdio directly.
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = stdin.lock();
        let mut writer = stdout.lock();
        let response = ask_with_io(&input, &mut reader, &mut writer)?;
        Ok(ToolResult::success(use_id, response))
    }
}

impl ParsedAskUserInput {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Basic(input) => input.validate(),
            Self::Compat(input) => input.validate(),
        }
    }

    fn into_basic_input(self) -> AskUserInput {
        match self {
            Self::Basic(input) => input,
            Self::Compat(input) => input.into_basic_input(),
        }
    }
}

fn parse_ask_user_input(input: &Value) -> Result<ParsedAskUserInput> {
    let object = input
        .as_object()
        .ok_or_else(|| WonderError::validation("invalid ask_user input: expected an object"))?;
    let has_question = object.contains_key("question");
    let has_questions = object.contains_key("questions");

    if has_question && has_questions {
        return Err(WonderError::validation(
            "ask_user accepts either `question` or source-compatible `questions`, not both",
        ));
    }

    if has_questions {
        return parse_input::<AskUserCompatInput>("ask_user", input)
            .map(ParsedAskUserInput::Compat);
    }

    parse_input::<AskUserInput>("ask_user", input).map(ParsedAskUserInput::Basic)
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
    use std::{collections::BTreeSet, io::Cursor};

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
    fn ask_user_accepts_source_compatible_single_question() {
        let mut reader = Cursor::new(b"option a\n".to_vec());
        let mut writer = Vec::new();
        let response = ask_with_io(
            &parse_ask_user_input(&json!({
                "questions": [
                    {
                        "question": "Which path should we take?",
                        "header": "Approach",
                        "options": [
                            { "label": "option a", "description": "Take path A" },
                            { "label": "option b", "description": "Take path B" }
                        ]
                    }
                ]
            }))
            .expect("parse source input")
            .into_basic_input(),
            &mut reader,
            &mut writer,
        )
        .expect("response");

        assert_eq!(response, "option a");
        assert!(
            String::from_utf8(writer)
                .expect("prompt")
                .contains("Options: option a, option b")
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

    #[test]
    fn ask_user_validation_rejects_source_multi_select() {
        let tool = AskUserTool;
        let error = tool
            .validate_input(&json!({
                "questions": [
                    {
                        "question": "Which features should we enable?",
                        "header": "Features",
                        "multi_select": true,
                        "options": [
                            { "label": "a", "description": "A" },
                            { "label": "b", "description": "B" }
                        ]
                    }
                ]
            }))
            .expect_err("unsupported multi-select");

        assert!(error.to_string().contains("multiSelect"));
    }

    #[test]
    fn ask_user_validation_rejects_multiple_source_questions() {
        let tool = AskUserTool;
        let error = tool
            .validate_input(&json!({
                "questions": [
                    {
                        "question": "Question 1?",
                        "header": "One",
                        "options": [
                            { "label": "a", "description": "A" },
                            { "label": "b", "description": "B" }
                        ]
                    },
                    {
                        "question": "Question 2?",
                        "header": "Two",
                        "options": [
                            { "label": "c", "description": "C" },
                            { "label": "d", "description": "D" }
                        ]
                    }
                ]
            }))
            .expect_err("unsupported question count");

        assert!(error.to_string().contains("exactly one question"));
    }

    #[test]
    fn ask_user_validation_rejects_source_answers_payload() {
        let tool = AskUserTool;
        let error = tool
            .validate_input(&json!({
                "questions": [
                    {
                        "question": "Question?",
                        "header": "One",
                        "options": [
                            { "label": "a", "description": "A" },
                            { "label": "b", "description": "B" }
                        ]
                    }
                ],
                "answers": { "Question?": "a" }
            }))
            .expect_err("unsupported answers");

        assert!(error.to_string().contains("output-only"));
    }

    #[test]
    fn ask_user_spec_exposes_source_alias() {
        let tool = AskUserTool;
        let aliases = tool.spec().aliases.into_iter().collect::<BTreeSet<_>>();

        assert_eq!(aliases, BTreeSet::from(["AskUserQuestion".to_string()]));
    }
}

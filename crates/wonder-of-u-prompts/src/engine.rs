use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A variable in a prompt template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemplateVariable {
    /// Variable name used in the template (`{{name}}`).
    pub name: String,
    /// Human-readable description of this variable.
    pub description: String,
    /// Whether this variable is required.
    pub required: bool,
    /// Default value when not provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// A prompt template with variable substitution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Template name (e.g. "system", "memory-extraction", "planning").
    pub name: String,
    /// Target model or model family (e.g. "claude", "gpt", "*" for all).
    pub model: String,
    /// Template version for tracking changes.
    pub version: String,
    /// The template text with `{{variable}}` placeholders.
    pub content: String,
    /// Variables declared by this template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<TemplateVariable>,
}

/// Engine that renders prompt templates by substituting variables.
#[derive(Clone, Debug, Default)]
pub struct TemplateEngine {
    templates: Vec<PromptTemplate>,
}

impl TemplateEngine {
    /// Create a new engine with the given templates.
    #[must_use]
    pub fn new(templates: Vec<PromptTemplate>) -> Self {
        Self { templates }
    }

    /// Find the best matching template for a given name and model.
    #[must_use]
    pub fn find(&self, name: &str, model: &str) -> Option<&PromptTemplate> {
        let model_lower = model.to_lowercase();

        self.templates
            .iter()
            .find(|t| t.name == name && t.model.to_lowercase() == model_lower)
            .or_else(|| {
                self.templates
                    .iter()
                    .find(|t| t.name == name && t.model == "*")
            })
    }

    /// Render a template with the given variables.
    pub fn render(&self, name: &str, model: &str, vars: &HashMap<String, String>) -> Option<String> {
        let template = self.find(name, model)?;
        Some(render_template(&template.content, vars))
    }

    /// Add templates to the engine.
    pub fn add_templates(&mut self, templates: Vec<PromptTemplate>) {
        self.templates.extend(templates);
    }
}

fn render_template(content: &str, vars: &HashMap<String, String>) -> String {
    let mut result = content.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{key}}}}}");
        result = result.replace(&placeholder, value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_simple() {
        let template = PromptTemplate {
            name: "test".into(),
            model: "*".into(),
            version: "1".into(),
            content: "Hello {{name}}!".into(),
            variables: vec![],
        };
        let engine = TemplateEngine::new(vec![template]);
        let mut vars = HashMap::new();
        vars.insert("name".into(), "World".into());
        let result = engine.render("test", "claude", &vars);
        assert_eq!(result, Some("Hello World!".into()));
    }

    #[test]
    fn model_specific() {
        let generic = PromptTemplate {
            name: "system".into(),
            model: "*".into(),
            version: "1".into(),
            content: "generic".into(),
            variables: vec![],
        };
        let claude = PromptTemplate {
            name: "system".into(),
            model: "claude".into(),
            version: "1".into(),
            content: "claude-specific".into(),
            variables: vec![],
        };
        let engine = TemplateEngine::new(vec![generic, claude]);
        assert_eq!(
            engine.render("system", "claude", &HashMap::new()),
            Some("claude-specific".into())
        );
        assert_eq!(
            engine.render("system", "gpt-4", &HashMap::new()),
            Some("generic".into())
        );
    }
}

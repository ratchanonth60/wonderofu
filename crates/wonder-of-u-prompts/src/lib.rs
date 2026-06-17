//! Prompt template engine for model system prompts.
//!
//! Defines a template system that separates prompt text from code,
//! allowing model-specific and versioned prompt templates.
//!
//! Templates use a simple `{{variable}}` substitution syntax.

#![warn(missing_docs)]

mod builtin;
mod engine;

pub use builtin::BUILTIN_TEMPLATES;
pub use engine::{PromptTemplate, TemplateEngine, TemplateVariable};

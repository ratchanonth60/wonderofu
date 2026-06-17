use crate::PromptTemplate;

/// Built-in prompt templates bundled with the binary.
pub static BUILTIN_TEMPLATES: &[PromptTemplate] = &[];

/// Define a prompt template statically.
#[macro_export]
macro_rules! prompt_template {
    (
        name: $name:expr,
        model: $model:expr,
        version: $version:expr,
        variables: [$($var:ident = $desc:expr $(, required = $req:expr)? $(, default = $def:expr)?),* $(,)?],
        content: $content:expr
    ) => {
        $crate::PromptTemplate {
            name: $name.into(),
            model: $model.into(),
            version: $version.into(),
            content: $content.into(),
            variables: vec![
                $(
                    $crate::TemplateVariable {
                        name: stringify!($var).into(),
                        description: $desc.into(),
                        required: prompt_template!(@req $($req)?),
                        default: prompt_template!(@def $($def)?),
                    }
                ),*
            ],
        }
    };
    (@req) => (false);
    (@req $req:expr) => ($req);
    (@def) => (None);
    (@def $def:expr) => (Some($def.into()));
}

#[cfg(test)]
mod build_slash_suggestions_tests {
    use super::super::build_slash_suggestions;
    use async_trait::async_trait;
    use std::sync::Arc;
    use wonder_of_u_core::{
        Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandRegistry,
        CommandSpec, Result,
    };

    struct FakeCmd(CommandSpec);

    #[async_trait]
    impl Command for FakeCmd {
        fn spec(&self) -> CommandSpec {
            self.0.clone()
        }

        async fn execute(&self, _: CommandContext, _: CommandInvocation) -> Result<CommandOutput> {
            Ok(CommandOutput::Noop)
        }
    }

    fn registry_with(specs: impl IntoIterator<Item = CommandSpec>) -> CommandRegistry {
        let mut reg = CommandRegistry::new();
        for spec in specs {
            reg.register(Arc::new(FakeCmd(spec))).expect("register");
        }
        reg
    }

    #[test]
    fn no_hint_produces_slash_name_display_and_replacement() {
        let spec = CommandSpec::new("status", "show status", CommandKind::Local);
        let reg = registry_with([spec]);
        let suggestions = build_slash_suggestions(&reg);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].display_text, "/status");
        assert_eq!(suggestions[0].replacement, "/status");
    }

    #[test]
    fn hint_appended_to_display_text_but_not_replacement() {
        let spec = CommandSpec::new("fast", "fast mode", CommandKind::Local)
            .with_argument_hint("[on|off]");
        let reg = registry_with([spec]);
        let suggestions = build_slash_suggestions(&reg);
        assert_eq!(suggestions.len(), 1);
        // Display shows the hint so the user knows what to type.
        assert_eq!(suggestions[0].display_text, "/fast [on|off]");
        // Replacement stays bare so the cursor lands right after the command
        // name, ready for the user to type their argument.
        assert_eq!(suggestions[0].replacement, "/fast");
    }

    #[test]
    fn hidden_commands_are_excluded_from_suggestions() {
        let mut hidden = CommandSpec::new("internal", "internal cmd", CommandKind::Local);
        hidden.hidden = true;
        let visible = CommandSpec::new("help", "help", CommandKind::Local);
        let reg = registry_with([hidden, visible]);
        let suggestions = build_slash_suggestions(&reg);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].display_text, "/help");
    }

    #[test]
    fn aliases_become_keywords_in_suggestion() {
        let mut spec = CommandSpec::new("resume", "resume", CommandKind::Local);
        spec.aliases = vec!["continue".into()];
        let reg = registry_with([spec]);
        let suggestions = build_slash_suggestions(&reg);
        assert!(
            suggestions[0].keywords.contains(&"/continue".to_string()),
            "expected /continue in keywords"
        );
    }

    #[test]
    fn description_is_passed_through_to_suggestion() {
        let spec = CommandSpec::new("effort", "set effort level", CommandKind::Local)
            .with_argument_hint("[low|medium|high|max|auto]");
        let reg = registry_with([spec]);
        let suggestions = build_slash_suggestions(&reg);
        assert_eq!(
            suggestions[0].description.as_deref(),
            Some("set effort level")
        );
    }
}

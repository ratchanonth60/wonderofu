use std::path::PathBuf;

use async_trait::async_trait;
use wonder_of_u_agent::ProviderResolver;
use wonder_of_u_core::{
    AuthMaterialKind, Command, CommandContext, CommandInvocation, CommandKind, CommandOutput,
    CommandSpec, Result,
};

/// Represents doctor command
pub struct DoctorCommand {
    total_commands: usize,
    public_commands: usize,
    storage_dir: Option<PathBuf>,
}

impl DoctorCommand {
    /// Creates a new value
    pub fn new(
        total_commands: usize,
        public_commands: usize,
        storage_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            total_commands,
            public_commands,
            storage_dir,
        }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "doctor",
            "Run lightweight startup diagnostics",
            CommandKind::NonInteractive,
        )
    }
}

fn provider_id_list(providers: &[wonder_of_u_agent::ProviderDescriptor]) -> String {
    providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn provider_inventory_lines(report: &wonder_of_u_agent::ProviderStatusReport) -> Vec<String> {
    let configured_ids: std::collections::BTreeSet<&str> = report
        .configured_providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();
    let authenticated_ids: std::collections::BTreeSet<&str> = report
        .authenticated_providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();
    let ready_ids: std::collections::BTreeSet<&str> = report
        .ready_providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();

    let mut lines = vec![
        format!("providers_registered={}", report.available_providers.len()),
        format!("providers_configured={}", report.configured_providers.len()),
        format!(
            "providers_authenticated={}",
            report.authenticated_providers.len()
        ),
        format!("providers_ready={}", report.ready_providers.len()),
        format!(
            "registered_providers={}",
            provider_id_list(&report.available_providers)
        ),
        format!(
            "configured_providers={}",
            provider_id_list(&report.configured_providers)
        ),
        format!(
            "authenticated_providers={}",
            provider_id_list(&report.authenticated_providers)
        ),
        format!(
            "ready_providers={}",
            provider_id_list(&report.ready_providers)
        ),
    ];

    for provider in &report.available_providers {
        let hint = provider_env_hint(provider);
        lines.push(format!(
            "provider[{}]={};auth={};configured={};authenticated={};ready={};{}",
            provider.id,
            provider.display_name,
            auth_kind_label(provider.auth_kind),
            configured_ids.contains(provider.id.as_str()),
            authenticated_ids.contains(provider.id.as_str()),
            ready_ids.contains(provider.id.as_str()),
            hint
        ));
    }

    lines
}

#[async_trait]
impl Command for DoctorCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
        let selection = report
            .selection_label()
            .unwrap_or_else(|| "unconfigured".into());

        let mut lines = vec![
            "wonder-of-u foundation ok".to_string(),
            format!("cwd={}", context.cwd.display()),
            format!("features={}", context.features.iter().count()),
            format!(
                "commands={} total / {} public",
                self.total_commands, self.public_commands
            ),
            format!("provider_selection={selection}"),
            format!("provider_readiness={}", report.readiness.label()),
            format!("auth_kind={}", report.auth.kind_label()),
            format!("auth_status={}", report.auth.status_label()),
        ];
        lines.extend(provider_inventory_lines(&report));

        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Returns a short `key=value` hint string describing the env vars or auth
/// material needed to activate this provider.  Never includes secret values.
fn auth_kind_label(kind: AuthMaterialKind) -> &'static str {
    match kind {
        AuthMaterialKind::None => "none",
        AuthMaterialKind::ApiKey => "api_key",
        AuthMaterialKind::OAuth => "oauth",
        AuthMaterialKind::AwsSigV4 => "aws_sigv4",
        AuthMaterialKind::AwsBearer => "aws_bearer",
        AuthMaterialKind::AwsProfile => "aws_profile",
        AuthMaterialKind::GcpOAuth2 => "gcp_oauth2",
    }
}

fn provider_env_hint(p: &wonder_of_u_agent::ProviderDescriptor) -> String {
    match p.auth_kind {
        AuthMaterialKind::None => "hint=no_auth_required".into(),
        AuthMaterialKind::ApiKey => {
            let mut parts = Vec::new();
            if let Some(env) = &p.api_key_env {
                parts.push(format!("api_key_env={env}"));
            }
            if let Some(env) = &p.endpoint_env {
                parts.push(format!("endpoint_env={env}"));
            }
            if parts.is_empty() {
                "hint=set_api_key_via_login".into()
            } else {
                parts.join(";")
            }
        }
        AuthMaterialKind::OAuth => "hint=run_login_provider_copilot".into(),
        AuthMaterialKind::AwsSigV4 => {
            "hint=set_AWS_ACCESS_KEY_ID+AWS_SECRET_ACCESS_KEY_or_AWS_PROFILE".into()
        }
        AuthMaterialKind::AwsBearer => "hint=set_AWS_BEARER_TOKEN_BEDROCK".into(),
        AuthMaterialKind::AwsProfile => {
            "hint=set_AWS_PROFILE_and_configure_credentials_file".into()
        }
        AuthMaterialKind::GcpOAuth2 => {
            "hint=set_VERTEXAI_PROJECT+VERTEXAI_LOCATION+GOOGLE_BEARER_TOKEN_or_GOOGLE_APPLICATION_CREDENTIALS".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use wonder_of_u_agent::ProviderResolver;

    use super::*;

    #[test]
    fn provider_inventory_lines_label_registered_and_ready_providers_separately() {
        let report = ProviderResolver::builtin()
            .resolve_with_env(
                &wonder_of_u_agent::AgentSettings::default(),
                &wonder_of_u_agent::StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve report");

        let rendered = provider_inventory_lines(&report).join("\n");

        assert!(rendered.contains("providers_registered="));
        assert!(rendered.contains("providers_ready=1"));
        assert!(rendered.contains("ready_providers=local"));
        assert!(
            rendered.contains(
                "provider[openai]=OpenAI;auth=api_key;configured=false;authenticated=false;ready=false;api_key_env=OPENAI_API_KEY"
            )
        );
    }
}

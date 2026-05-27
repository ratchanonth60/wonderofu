use std::collections::BTreeMap;
use std::fmt::Write as _;

use wonder_of_u_core::Result;

use crate::{McpClient, McpClientIdentity, McpServerConfig};

/// Reuses stdio MCP sessions for repeated calls to the same server.
#[derive(Debug, Default)]
pub struct McpSessionPool {
    clients: BTreeMap<String, McpClient>,
}

impl McpSessionPool {
    /// Creates an empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `operation` with a pooled client, reconnecting on first use.
    pub fn with_client<T>(
        &mut self,
        server: &McpServerConfig,
        client_identity: &McpClientIdentity,
        fallback_protocol: &str,
        operation: impl FnOnce(&mut McpClient) -> Result<T>,
    ) -> Result<T> {
        // Use a content-aware key so config changes (env, command, cwd, …)
        // are never silently masked by a stale pooled session.
        let key = server_cache_key(server);
        if !self.clients.contains_key(&key) {
            let client = McpClient::connect(server, client_identity, fallback_protocol)?;
            self.clients.insert(key.clone(), client);
        }

        let result = {
            let client = self
                .clients
                .get_mut(&key)
                .expect("client was inserted before operation");
            operation(client)
        };
        if result.is_err() {
            self.clients.remove(&key);
        }
        result
    }

    /// Number of currently cached sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Returns true when no sessions are cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

/// Builds a stable string key that captures all config fields that affect
/// which process is launched and how it behaves.
///
/// Using the server name alone would allow a pool entry created for one
/// configuration (e.g. `TOKEN=abc`) to be reused after the config is changed
/// to a different value (e.g. `TOKEN=xyz`).  Including command, args, env,
/// cwd, and protocol version prevents that stale-session scenario.
fn server_cache_key(server: &McpServerConfig) -> String {
    // NUL and unit-separator bytes cannot appear in typical shell values,
    // making the key unambiguous without requiring a hash dependency.
    const SEP: char = '\x00';
    const FIELD: char = '\x01';

    let mut key = String::new();
    write!(
        &mut key,
        "name={}{FIELD}cmd={}{FIELD}args={}",
        server.name,
        server.command,
        server.args.join("\x1f")
    )
    .expect("fmt::Write on String is infallible");
    write!(&mut key, "{SEP}").expect("infallible");

    for (k, v) in &server.env {
        write!(&mut key, "env.{k}={v}{FIELD}").expect("infallible");
    }

    if let Some(cwd) = &server.cwd {
        write!(&mut key, "cwd={}{SEP}", cwd.display()).expect("infallible");
    }

    if let Some(pv) = &server.protocol_version {
        write!(&mut key, "proto={pv}{SEP}").expect("infallible");
    }

    key
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn base_server(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.into(),
            command: "echo".into(),
            args: vec!["--stdio".into()],
            env: BTreeMap::new(),
            enabled: true,
            cwd: None,
            protocol_version: None,
        }
    }

    #[test]
    fn same_name_same_config_produces_identical_keys() {
        let server = base_server("demo");
        assert_eq!(server_cache_key(&server), server_cache_key(&server));
    }

    #[test]
    fn same_name_different_env_produces_distinct_keys() {
        let mut a = base_server("demo");
        let mut b = base_server("demo");
        a.env.insert("TOKEN".into(), "abc".into());
        b.env.insert("TOKEN".into(), "xyz".into());

        assert_ne!(
            server_cache_key(&a),
            server_cache_key(&b),
            "differing env values must yield different cache keys to prevent stale reuse"
        );
    }

    #[test]
    fn same_name_different_command_produces_distinct_keys() {
        let mut a = base_server("demo");
        let mut b = base_server("demo");
        a.command = "server-v1".into();
        b.command = "server-v2".into();

        assert_ne!(server_cache_key(&a), server_cache_key(&b));
    }

    #[test]
    fn same_name_different_args_produces_distinct_keys() {
        let mut a = base_server("demo");
        let mut b = base_server("demo");
        a.args = vec!["--port".into(), "8080".into()];
        b.args = vec!["--port".into(), "9090".into()];

        assert_ne!(server_cache_key(&a), server_cache_key(&b));
    }

    #[test]
    fn pool_does_not_share_key_when_env_changes() {
        // Regression: the pool must treat two McpServerConfigs that share a
        // name but differ in env as distinct entries so that a changed API key
        // or endpoint env-var always results in a fresh connection rather than
        // reusing the stale one.
        let pool = McpSessionPool::new();

        let mut config_a = base_server("demo");
        config_a.env.insert("TOKEN".into(), "abc".into());

        let mut config_b = base_server("demo");
        config_b.env.insert("TOKEN".into(), "xyz".into());

        let key_a = server_cache_key(&config_a);
        let key_b = server_cache_key(&config_b);

        // Neither key is present in the empty pool.
        assert!(!pool.clients.contains_key(&key_a));
        assert!(!pool.clients.contains_key(&key_b));
        // And crucially the two keys must differ.
        assert_ne!(
            key_a,
            key_b,
            "distinct server configs must not share a cache key"
        );
    }
}

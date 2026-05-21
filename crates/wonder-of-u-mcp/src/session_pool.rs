use std::collections::BTreeMap;

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
        let key = server.name.clone();
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

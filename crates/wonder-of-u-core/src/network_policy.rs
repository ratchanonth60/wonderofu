//! Network egress policy — domain allowlist/denylist for web tools.
//!
//! Provides a configurable network policy that restricts which hosts
//! web tools (web_fetch, web_search) may access.

use serde::{Deserialize, Serialize};

/// Decision for a network host access.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkHostDecision {
    /// Allow access to this host.
    Allow,
    /// Deny access to this host.
    Deny,
    /// Ask the user before accessing this host.
    Prompt,
}

/// A single network policy rule for a domain pattern.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkPolicyRule {
    /// Domain pattern (e.g. `api.github.com`, `*.openai.com`).
    pub host: String,
    /// Access decision for this host pattern.
    pub decision: NetworkHostDecision,
}

/// Network egress policy configuration.
///
/// Rules are evaluated in order; the first matching rule wins.
/// If no rule matches, the default decision (Deny) is applied.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkPolicyConfig {
    /// Ordered list of domain rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<NetworkPolicyRule>,
    /// Default decision when no rule matches.
    #[serde(default = "default_network_default_decision")]
    pub default_decision: NetworkHostDecision,
    /// Whether the network proxy is enabled.
    #[serde(default)]
    pub enabled: bool,
}

fn default_network_default_decision() -> NetworkHostDecision {
    NetworkHostDecision::Deny
}

impl Default for NetworkPolicyConfig {
    fn default() -> Self {
        Self {
            rules: default_rules(),
            default_decision: NetworkHostDecision::Deny,
            enabled: false,
        }
    }
}

impl NetworkPolicyConfig {
    /// Create a policy with sensible defaults for development.
    #[must_use]
    pub fn permissive_defaults() -> Self {
        Self {
            rules: default_rules(),
            default_decision: NetworkHostDecision::Allow,
            enabled: false,
        }
    }

    /// Evaluate a host against this policy.
    /// Returns the decision for the first matching rule, or the default.
    #[must_use]
    pub fn check_host(&self, host: &str) -> NetworkHostDecision {
        for rule in &self.rules {
            if host_matches(host, &rule.host) {
                return rule.decision;
            }
        }
        self.default_decision
    }

    /// Check whether a URL's host is allowed.
    #[must_use]
    pub fn is_host_allowed(&self, host: &str) -> bool {
        matches!(self.check_host(host), NetworkHostDecision::Allow)
    }
}

fn default_rules() -> Vec<NetworkPolicyRule> {
    vec![
        NetworkPolicyRule {
            host: "platform.claude.com".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "api.openai.com".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "api.anthropic.com".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "api.github.com".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "github.com".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "*.docs.python.org".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "doc.rust-lang.org".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "developer.mozilla.org".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "*.wikipedia.org".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "localhost".into(),
            decision: NetworkHostDecision::Allow,
        },
        NetworkPolicyRule {
            host: "127.0.0.1".into(),
            decision: NetworkHostDecision::Allow,
        },
    ]
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let host = host.trim().to_lowercase();
    let pattern = pattern.trim().to_lowercase();

    if pattern == "*" {
        return true;
    }

    if let Some(suffix) = pattern.strip_prefix("**.") {
        return host == suffix || host.ends_with(&format!(".{suffix}"));
    }

    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host.ends_with(&format!(".{suffix}"));
    }

    host == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert!(host_matches("github.com", "github.com"));
        assert!(!host_matches("api.github.com", "github.com"));
    }

    #[test]
    fn wildcard_subdomain() {
        assert!(host_matches("api.github.com", "*.github.com"));
        assert!(host_matches("www.github.com", "*.github.com"));
        assert!(!host_matches("github.com", "*.github.com"));
    }

    #[test]
    fn super_wildcard() {
        assert!(host_matches("docs.python.org", "**.python.org"));
        assert!(host_matches("python.org", "**.python.org"));
        assert!(!host_matches("evil-python.org", "**.python.org"));
    }

    #[test]
    fn case_insensitive() {
        assert!(host_matches("GitHub.com", "github.com"));
        assert!(host_matches("API.GITHUB.COM", "*.github.com"));
    }

    #[test]
    fn policy_check() {
        let policy = NetworkPolicyConfig::default();
        assert!(policy.is_host_allowed("api.github.com"));
        assert!(policy.is_host_allowed("github.com"));
        assert!(!policy.is_host_allowed("evil.example.com"));
    }

    #[test]
    fn permissive_defaults_allows_unknown() {
        let policy = NetworkPolicyConfig::permissive_defaults();
        assert!(policy.is_host_allowed("unknown.example.com"));
    }
}

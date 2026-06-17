//! Async `ModelProvider` trait + mock implementation (Phase 1.2 of the codex-rs port).
//!
//! This module defines the seam between the agent loop and a concrete model
//! provider (Anthropic, OpenAI, GitHub Copilot, ...). Phase 1.2 ships the
//! trait and a `MockProvider` that emits a canned `EventMsg` sequence; the
//! real `reqwest` + `tokio` SSE implementations land in Phase 1.2b.
//!
//! Gated behind the `wonder-of-u-async` cargo feature (CLAUDE.md pitfall #1).

#![cfg(feature = "wonder-of-u-async")]

use std::pin::Pin;

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt};

use wonder_of_u_protocol::events::EventMsg;

use crate::error::Result;

/// A single input item in the model-format (post-resolution of `UserInput`).
///
/// Phase 1.2 keeps this minimal — text + image. Phase 1.3 will add tool-call
/// items and structured output once the turn driver needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelInput {
    /// Plain text the model should treat as user-visible content.
    Text(String),
    /// Image reference (local path or URL).
    Image(String),
}

/// Request sent to a [`ModelProvider`].
#[derive(Debug, Clone)]
pub struct ModelRequest {
    /// Input items, in order.
    pub input: Vec<ModelInput>,
    /// Model identifier (e.g. `"claude-opus-4"`).
    pub model: String,
}

impl ModelRequest {
    /// Construct a request with the given model and a single text input.
    pub fn from_text(model: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            input: vec![ModelInput::Text(text.into())],
            model: model.into(),
        }
    }
}

/// Stream of `EventMsg` items produced by a provider.
///
/// Errors yielded by the stream are surfaced to the client as
/// `EventMsg::Error(ErrorEvent { fatal: true, ... })` in Phase 1.3.
pub type EventMsgStream = Pin<Box<dyn Stream<Item = crate::Result<EventMsg>> + Send>>;

/// Async model provider.
///
/// Implementors stream `EventMsg`s in response to a `ModelRequest`. The trait
/// mirrors codex's `ModelProvider` shape minus the realtime/voice split we
/// will not implement.
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Stream a response for the given request.
    async fn stream(&self, request: ModelRequest) -> Result<EventMsgStream>;
}

// ---------------------------------------------------------------------------
// MockProvider
// ---------------------------------------------------------------------------

/// A canned `EventMsg` sequence, replayed verbatim for every request.
///
/// `MockProvider` is the test seam for Phase 1.3 (turn driver) and onward —
/// it lets us assert event sequences without a network or a real model.
#[derive(Debug, Clone, Default)]
pub struct MockProvider {
    /// The script to emit per request.
    script: Vec<EventMsg>,
}

impl MockProvider {
    /// Construct a mock provider that emits `script` for every request.
    pub fn new(script: Vec<EventMsg>) -> Self {
        Self { script }
    }

    /// Construct a mock provider that emits a single event for every request.
    pub fn single(event: EventMsg) -> Self {
        Self {
            script: vec![event],
        }
    }

    /// Borrow the canned script.
    pub fn script(&self) -> &[EventMsg] {
        &self.script
    }
}

#[async_trait]
impl ModelProvider for MockProvider {
    async fn stream(&self, _request: ModelRequest) -> Result<EventMsgStream> {
        // Collect into an owned `Vec` so the returned Stream is `'static`.
        // The `async_trait`-generated future borrows `&self`; an `iter` over
        // `&self.script` would tether the Stream to that borrow.
        let items: Vec<crate::Result<EventMsg>> = self.script.iter().cloned().map(Ok).collect();
        Ok(Box::pin(stream::iter(items)))
    }
}

// ---------------------------------------------------------------------------
// CollectingEventSink
// ---------------------------------------------------------------------------

/// Helper that drains an `EventMsgStream` into a `Vec<EventMsg>` for tests.
pub async fn collect(stream: EventMsgStream) -> Vec<EventMsg> {
    stream
        .filter_map(|res| async move { res.ok() })
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;
    use wonder_of_u_protocol::{
        events::{AgentMessageContentDeltaEvent, AgentMessageEvent, TokenCountEvent, TokenUsage},
        session::ThreadId,
    };

    #[test]
    fn model_request_from_text() {
        let req = ModelRequest::from_text("claude-opus-4", "hello");
        assert_eq!(req.model, "claude-opus-4");
        assert_eq!(req.input, vec![ModelInput::Text("hello".to_string())]);
    }

    #[test]
    fn mock_provider_empty_script_emits_nothing() {
        let rt = Runtime::new().unwrap();
        let events = rt.block_on(async {
            let p = MockProvider::default();
            let stream = p.stream(ModelRequest::from_text("m", "x")).await.unwrap();
            collect(stream).await
        });
        assert!(events.is_empty());
    }

    #[test]
    fn mock_provider_replays_script_in_order() {
        let rt = Runtime::new().unwrap();
        let events = rt.block_on(async {
            let p = MockProvider::new(vec![
                EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent {
                    delta: "hel".to_string(),
                }),
                EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent {
                    delta: "lo".to_string(),
                }),
                EventMsg::AgentMessage(AgentMessageEvent {
                    id: "msg_1".to_string(),
                    text: "hello".to_string(),
                }),
                EventMsg::TokenCount(TokenCountEvent {
                    info: Some(TokenUsage {
                        input_tokens: 1,
                        output_tokens: 1,
                        cached_input_tokens: 0,
                        total_tokens: 2,
                    }),
                }),
            ]);
            let stream = p.stream(ModelRequest::from_text("m", "x")).await.unwrap();
            collect(stream).await
        });

        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], EventMsg::AgentMessageContentDelta(_)));
        assert!(matches!(events[1], EventMsg::AgentMessageContentDelta(_)));
        assert!(matches!(events[2], EventMsg::AgentMessage(_)));
        assert!(matches!(events[3], EventMsg::TokenCount(_)));
    }

    #[test]
    fn mock_provider_single_event() {
        let rt = Runtime::new().unwrap();
        let events = rt.block_on(async {
            let p = MockProvider::single(EventMsg::SessionConfigured(
                wonder_of_u_protocol::events::SessionConfiguredEvent {
                    session_id: ThreadId::new(),
                    model: "m".to_string(),
                    approval_policy: wonder_of_u_protocol::config::AskForApproval::default(),
                    sandbox_policy:
                        wonder_of_u_protocol::config::SandboxPolicy::new_workspace_write_policy(),
                    reasoning_effort: None,
                },
            ));
            let stream = p.stream(ModelRequest::from_text("m", "x")).await.unwrap();
            collect(stream).await
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EventMsg::SessionConfigured(_)));
    }

    #[test]
    fn script_is_exposed_for_introspection() {
        let p = MockProvider::new(vec![EventMsg::ShutdownComplete]);
        assert_eq!(p.script().len(), 1);
    }
}

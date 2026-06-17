//! Async Anthropic Messages API provider (Phase 1.5 of the codex-rs port).
//!
//! Implements [`ModelProvider`] against `api.anthropic.com/v1/messages` using
//! `reqwest` + a hand-rolled SSE parser. Emits `EventMsg` deltas that match
//! the legacy `runtime::anthropic` behaviour, but streams them incrementally
//! instead of buffering into a finished string.
//!
//! Gated behind the `wonder-of-u-async` cargo feature — see CLAUDE.md
//! pitfall #1.

#![cfg(feature = "wonder-of-u-async")]

use std::collections::VecDeque;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, Stream, StreamExt, TryStreamExt};
use serde::Deserialize;
use serde_json::{Value, json};

use wonder_of_u_core::provider_async::{EventMsgStream, ModelInput, ModelProvider, ModelRequest};
use wonder_of_u_core::{Result, WonderError};
use wonder_of_u_protocol::events::{
    AgentMessageContentDeltaEvent, AgentMessageEvent, ErrorEvent, EventMsg, TokenCountEvent,
    TokenUsage,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default Anthropic Messages API base URL.
pub const DEFAULT_ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";

/// Default `anthropic-version` header value.
pub const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

/// Default `max_tokens` for streaming requests.
pub const DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS: u32 = 8_192;

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Anthropic Messages API provider (async, `reqwest` + tokio SSE).
#[derive(Debug, Clone)]
pub struct AnthropicMessagesProvider {
    api_base: String,
    api_key: String,
    anthropic_version: String,
    max_output_tokens: u32,
    client: reqwest::Client,
}

impl AnthropicMessagesProvider {
    /// Construct a provider pointing at the production Anthropic API.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::with_base_url(DEFAULT_ANTHROPIC_API_BASE, api_key)
    }

    /// Construct a provider pointing at a custom base URL (e.g. a test
    /// server or a private deployment).
    pub fn with_base_url(api_base: impl Into<String>, api_key: impl Into<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| WonderError::Internal(format!("reqwest client build: {e}")))?;
        Ok(Self {
            api_base: api_base.into(),
            api_key: api_key.into(),
            anthropic_version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            max_output_tokens: DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS,
            client,
        })
    }

    /// Override the default `max_tokens` for streaming requests.
    #[must_use]
    pub fn with_max_output_tokens(mut self, max_tokens: u32) -> Self {
        self.max_output_tokens = max_tokens;
        self
    }

    /// Borrow the configured API base URL.
    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    /// Borrow the `anthropic-version` header value.
    pub fn anthropic_version(&self) -> &str {
        &self.anthropic_version
    }

    /// Build the JSON request body that this provider would POST.
    pub fn build_body(&self, request: &ModelRequest) -> Value {
        let messages = vec![json!({
            "role": "user",
            "content": request
                .input
                .iter()
                .map(|item| match item {
                    ModelInput::Text(text) => json!({"type": "text", "text": text}),
                    // Phase 1.5 stub: image references are dropped. Phase 1.6
                    // will inline base64 attachments.
                    ModelInput::Image(_) => json!({"type": "text", "text": "[image]"}),
                })
                .collect::<Vec<_>>(),
        })];
        json!({
            "model": request.model,
            "max_tokens": self.max_output_tokens,
            "stream": true,
            "messages": messages,
        })
    }

    /// Issue the HTTP POST and return the response status + byte stream.
    ///
    /// Exposed for integration tests; production callers should use
    /// [`ModelProvider::stream`].
    pub async fn send_request(&self, request: &ModelRequest) -> Result<reqwest::Response> {
        let url = format!("{}/v1/messages", self.api_base);
        let body = self.build_body(request);
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.anthropic_version)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| WonderError::Internal(format!("anthropic send: {e}")))?;
        Ok(response)
    }
}

#[async_trait]
impl ModelProvider for AnthropicMessagesProvider {
    async fn stream(&self, request: ModelRequest) -> Result<EventMsgStream> {
        let response = match self.send_request(&request).await {
            Ok(r) => r,
            Err(e) => {
                let err = ErrorEvent {
                    message: e.to_string(),
                    details: None,
                    fatal: true,
                };
                return Ok(Box::pin(stream::once(
                    async move { Ok(EventMsg::Error(err)) },
                )));
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = match response.text().await {
                Ok(b) => b,
                Err(e) => format!("(failed to read body: {e})"),
            };
            let err = ErrorEvent {
                message: format!("anthropic HTTP {status}: {body}"),
                details: None,
                fatal: true,
            };
            return Ok(Box::pin(stream::once(
                async move { Ok(EventMsg::Error(err)) },
            )));
        }

        let byte_stream = response
            .bytes_stream()
            .map_err(|e| WonderError::Internal(format!("anthropic bytes: {e}")));
        let event_stream = parse_anthropic_sse_stream(byte_stream);

        let mapped = event_stream.filter_map(|res| async move {
            match res {
                Ok(event) => anthropic_event_to_event_msg(event).map(Ok),
                Err(e) => Some(Ok(EventMsg::Error(ErrorEvent {
                    message: format!("anthropic SSE parse error: {e}"),
                    details: None,
                    fatal: false,
                }))),
            }
        });

        Ok(Box::pin(mapped))
    }
}

// ---------------------------------------------------------------------------
// Anthropic SSE wire format
// ---------------------------------------------------------------------------

/// One Anthropic SSE event (`message_start`, `content_block_delta`, ...).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicEvent {
    /// Stream start; carries the message id, model, and initial usage.
    MessageStart {
        /// Message body.
        message: MessageStartBody,
    },
    /// Beginning of a content block.
    ContentBlockStart {
        /// Block index.
        index: u32,
        /// Block content.
        content_block: ContentBlock,
    },
    /// Incremental delta for a content block.
    ContentBlockDelta {
        /// Block index.
        index: u32,
        /// Delta payload.
        delta: ContentDelta,
    },
    /// End of a content block.
    ContentBlockStop {
        /// Block index.
        index: u32,
    },
    /// Stream-end metadata: stop reason + final usage.
    MessageDelta {
        /// Delta payload (stop reason, etc.).
        delta: MessageDeltaBody,
        /// Optional final usage.
        #[serde(default)]
        usage: Option<MessageDeltaUsage>,
    },
    /// Stream end.
    MessageStop,
    /// Keep-alive.
    Ping,
    /// Error from Anthropic.
    Error {
        /// Error payload.
        error: AnthropicErrorBody,
    },
}

/// Body of a `message_start` event.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MessageStartBody {
    /// Message id.
    pub id: String,
    /// Model identifier.
    pub model: String,
    /// Initial token usage.
    #[serde(default)]
    pub usage: Option<MessageStartUsage>,
}

/// Initial token usage (input tokens are known up front).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MessageStartUsage {
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens (typically 0 at start).
    #[serde(default)]
    pub output_tokens: u64,
}

/// Content block (only `text` is supported by this provider in Phase 1.5).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Text block.
    Text {
        /// Initial text (often empty at start).
        text: String,
    },
}

/// Delta payload for `content_block_delta`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    /// Text delta.
    TextDelta {
        /// Text chunk.
        text: String,
    },
}

/// Body of `message_delta`.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct MessageDeltaBody {
    /// Stop reason (e.g. `"end_turn"`, `"max_tokens"`).
    #[serde(default)]
    pub stop_reason: Option<String>,
}

/// Final usage reported with `message_delta`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MessageDeltaUsage {
    /// Output tokens (cumulative).
    pub output_tokens: u64,
}

/// Body of an `error` event.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AnthropicErrorBody {
    /// Error kind (e.g. `"invalid_request_error"`).
    #[serde(rename = "type")]
    pub kind: String,
    /// Human-readable message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// EventMsg mapping
// ---------------------------------------------------------------------------

/// Map an [`AnthropicEvent`] to one or zero [`EventMsg`]s.
///
/// Returns `None` for events that don't surface to the client
/// (`content_block_start`, `content_block_stop`, `ping`).
pub fn anthropic_event_to_event_msg(event: AnthropicEvent) -> Option<EventMsg> {
    match event {
        AnthropicEvent::MessageStart { message } => message.usage.map(|u| {
            EventMsg::TokenCount(TokenCountEvent {
                info: Some(TokenUsage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cached_input_tokens: 0,
                    total_tokens: u.input_tokens + u.output_tokens,
                }),
            })
        }),
        AnthropicEvent::ContentBlockDelta { delta, .. } => {
            let ContentDelta::TextDelta { text } = delta;
            if text.is_empty() {
                None
            } else {
                Some(EventMsg::AgentMessageContentDelta(
                    AgentMessageContentDeltaEvent { delta: text },
                ))
            }
        }
        AnthropicEvent::MessageDelta { usage, .. } => usage.map(|u| {
            EventMsg::TokenCount(TokenCountEvent {
                info: Some(TokenUsage {
                    input_tokens: 0,
                    output_tokens: u.output_tokens,
                    cached_input_tokens: 0,
                    total_tokens: u.output_tokens,
                }),
            })
        }),
        AnthropicEvent::MessageStop => Some(EventMsg::AgentMessage(AgentMessageEvent {
            // Phase 1.5: emit a marker `AgentMessage` so the TUI can render
            // a "stream done" cue. Phase 1.6 will accumulate the deltas and
            // emit a proper final text.
            id: String::new(),
            text: String::new(),
        })),
        AnthropicEvent::Error { error } => Some(EventMsg::Error(ErrorEvent {
            message: format!("anthropic {}: {}", error.kind, error.message),
            details: None,
            fatal: false,
        })),
        AnthropicEvent::ContentBlockStart { .. }
        | AnthropicEvent::ContentBlockStop { .. }
        | AnthropicEvent::Ping => None,
    }
}

// ---------------------------------------------------------------------------
// SSE byte-stream parser
// ---------------------------------------------------------------------------

/// Drain an SSE-formatted byte buffer, emitting any complete events.
///
/// `buf` is mutated in place; `current_event` and `current_data` track the
/// in-progress event. Any complete events are pushed into `out`.
pub fn parse_sse_chunk(
    buf: &mut Vec<u8>,
    current_event: &mut Option<String>,
    current_data: &mut String,
    out: &mut VecDeque<AnthropicEvent>,
) {
    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        let line_bytes: Vec<u8> = buf.drain(..pos).collect();
        buf.remove(0); // drop the newline
        let line = String::from_utf8_lossy(&line_bytes);
        let line = line.trim_end_matches('\r');

        if line.is_empty() {
            // Event boundary.
            if !current_data.is_empty() {
                if let Ok(evt) = serde_json::from_str::<AnthropicEvent>(current_data) {
                    out.push_back(evt);
                }
                current_data.clear();
            }
            current_event.take();
            continue;
        }

        if let Some(rest) = line.strip_prefix("event: ") {
            *current_event = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("data: ") {
            current_data.push_str(rest);
        }
        // Other lines (`id:`, `retry:`, comments starting with `:`) are
        // ignored for now.
    }
}

/// Stream of [`AnthropicEvent`]s parsed from a byte stream.
pub fn parse_anthropic_sse_stream<S>(bytes: S) -> impl Stream<Item = Result<AnthropicEvent>>
where
    S: Stream<Item = Result<Bytes>> + Unpin,
{
    struct State<S> {
        bytes: S,
        buf: Vec<u8>,
        event: Option<String>,
        data: String,
        pending: VecDeque<AnthropicEvent>,
        done: bool,
    }

    stream::unfold(
        State {
            bytes,
            buf: Vec::new(),
            event: None,
            data: String::new(),
            pending: VecDeque::new(),
            done: false,
        },
        |mut state| async move {
            loop {
                if let Some(evt) = state.pending.pop_front() {
                    return Some((Ok::<AnthropicEvent, WonderError>(evt), state));
                }
                if state.done {
                    return None;
                }
                match state.bytes.next().await {
                    Some(Ok(chunk)) => {
                        state.buf.extend_from_slice(&chunk);
                        parse_sse_chunk(
                            &mut state.buf,
                            &mut state.event,
                            &mut state.data,
                            &mut state.pending,
                        );
                    }
                    Some(Err(e)) => {
                        state.done = true;
                        return Some((Err(e), state));
                    }
                    None => {
                        state.done = true;
                        // Flush trailing data without a final newline.
                        if !state.data.is_empty() {
                            if let Ok(evt) = serde_json::from_str::<AnthropicEvent>(&state.data) {
                                state.pending.push_back(evt);
                                state.data.clear();
                            }
                        }
                    }
                }
            }
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use tokio::runtime::Runtime;

    fn rt() -> Runtime {
        Runtime::new().unwrap()
    }

    fn bytes_stream_from_str(s: &str) -> Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>> {
        let chunks: Vec<Result<Bytes>> = vec![Ok(Bytes::copy_from_slice(s.as_bytes()))];
        Box::pin(stream::iter(chunks))
    }

    fn bytes_stream_from_chunks(
        chunks: Vec<&'static str>,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>> {
        let chunks: Vec<Result<Bytes>> = chunks
            .into_iter()
            .map(|c| Ok(Bytes::copy_from_slice(c.as_bytes())))
            .collect();
        Box::pin(stream::iter(chunks))
    }

    // ----- synchronous parse_sse_chunk -----

    #[test]
    fn parse_sse_chunk_drains_complete_message_start_event() {
        let mut buf: Vec<u8> = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-opus-4\",\"usage\":{\"input_tokens\":42,\"output_tokens\":0}}}\n\n".to_vec();
        let mut event = None;
        let mut data = String::new();
        let mut out = VecDeque::new();

        parse_sse_chunk(&mut buf, &mut event, &mut data, &mut out);

        assert!(buf.is_empty(), "buffer should be empty after parse");
        assert_eq!(out.len(), 1);
        match out.pop_front().unwrap() {
            AnthropicEvent::MessageStart { message } => {
                assert_eq!(message.id, "msg_1");
                assert_eq!(message.model, "claude-opus-4");
                let usage = message.usage.unwrap();
                assert_eq!(usage.input_tokens, 42);
                assert_eq!(usage.output_tokens, 0);
            }
            other => panic!("expected MessageStart, got {:?}", other),
        }
    }

    #[test]
    fn parse_sse_chunk_buffers_partial_lines_until_newline() {
        // Send a partial line first, then the rest.
        let mut buf: Vec<u8> = Vec::new();
        let mut event = None;
        let mut data = String::new();
        let mut out = VecDeque::new();

        buf.extend_from_slice(b"event: content_block_delta\ndata: {\"type\":\"con");
        parse_sse_chunk(&mut buf, &mut event, &mut data, &mut out);
        assert_eq!(out.len(), 0, "no event should be yielded yet");

        buf.extend_from_slice(
            b"tent_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hel\"}}\n\n",
        );
        parse_sse_chunk(&mut buf, &mut event, &mut data, &mut out);
        assert_eq!(out.len(), 1);
        match out.pop_front().unwrap() {
            AnthropicEvent::ContentBlockDelta { delta, .. } => {
                let ContentDelta::TextDelta { text } = delta;
                assert_eq!(text, "hel");
            }
            other => panic!("expected ContentBlockDelta, got {:?}", other),
        }
    }

    #[test]
    fn parse_sse_chunk_ignores_ping_events() {
        let mut buf: Vec<u8> = b"event: ping\ndata: {\"type\":\"ping\"}\n\n".to_vec();
        let mut event = None;
        let mut data = String::new();
        let mut out = VecDeque::new();

        parse_sse_chunk(&mut buf, &mut event, &mut data, &mut out);

        // Ping parses but produces no EventMsg.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], AnthropicEvent::Ping);
    }

    #[test]
    fn parse_sse_chunk_handles_crlf_line_endings() {
        let mut buf: Vec<u8> =
            b"event: message_stop\r\ndata: {\"type\":\"message_stop\"}\r\n\r\n".to_vec();
        let mut event = None;
        let mut data = String::new();
        let mut out = VecDeque::new();
        parse_sse_chunk(&mut buf, &mut event, &mut data, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], AnthropicEvent::MessageStop);
    }

    // ----- async parse_anthropic_sse_stream -----

    #[test]
    fn sse_stream_emits_events_in_order() {
        let payload = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-opus-4\",\"usage\":{\"input_tokens\":42,\"output_tokens\":0}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hel\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let stream = bytes_stream_from_str(payload);
        let parsed = rt().block_on(async {
            parse_anthropic_sse_stream(stream)
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
        });
        assert_eq!(parsed.len(), 5);
        assert!(matches!(parsed[0], AnthropicEvent::MessageStart { .. }));
        assert!(matches!(
            parsed[1],
            AnthropicEvent::ContentBlockDelta { .. }
        ));
        assert!(matches!(
            parsed[2],
            AnthropicEvent::ContentBlockDelta { .. }
        ));
        assert!(matches!(parsed[3], AnthropicEvent::MessageDelta { .. }));
        assert_eq!(parsed[4], AnthropicEvent::MessageStop);
    }

    #[test]
    fn sse_stream_handles_split_chunks() {
        let stream = bytes_stream_from_chunks(vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"",
            "hel\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ]);
        let parsed = rt().block_on(async {
            parse_anthropic_sse_stream(stream)
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
        });
        assert_eq!(parsed.len(), 2);
        match &parsed[0] {
            AnthropicEvent::ContentBlockDelta { delta, .. } => {
                let ContentDelta::TextDelta { text } = delta;
                assert_eq!(text, "hel");
            }
            _ => panic!("expected ContentBlockDelta"),
        }
        assert_eq!(parsed[1], AnthropicEvent::MessageStop);
    }

    #[test]
    fn sse_stream_yields_nothing_for_empty_input() {
        let stream = bytes_stream_from_str("");
        let parsed = rt().block_on(async {
            parse_anthropic_sse_stream(stream)
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
        });
        assert!(parsed.is_empty());
    }

    #[test]
    fn sse_stream_propagates_byte_stream_error() {
        let chunks: Vec<Result<Bytes>> = vec![Err(WonderError::Internal("boom".to_string()))];
        let stream = Box::pin(stream::iter(chunks));
        let result = rt().block_on(async {
            parse_anthropic_sse_stream(stream)
                .try_collect::<Vec<_>>()
                .await
        });
        assert!(result.is_err());
    }

    // ----- anthropic_event_to_event_msg mapping -----

    #[test]
    fn message_start_maps_to_token_count_with_input_tokens() {
        let evt = AnthropicEvent::MessageStart {
            message: MessageStartBody {
                id: "msg_1".to_string(),
                model: "claude-opus-4".to_string(),
                usage: Some(MessageStartUsage {
                    input_tokens: 100,
                    output_tokens: 0,
                }),
            },
        };
        let mapped = anthropic_event_to_event_msg(evt).unwrap();
        match mapped {
            EventMsg::TokenCount(TokenCountEvent { info: Some(usage) }) => {
                assert_eq!(usage.input_tokens, 100);
                assert_eq!(usage.output_tokens, 0);
                assert_eq!(usage.total_tokens, 100);
            }
            other => panic!("expected TokenCount, got {:?}", other),
        }
    }

    #[test]
    fn content_block_delta_maps_to_text_delta() {
        let evt = AnthropicEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: "hi".to_string(),
            },
        };
        let mapped = anthropic_event_to_event_msg(evt).unwrap();
        match mapped {
            EventMsg::AgentMessageContentDelta(d) => assert_eq!(d.delta, "hi"),
            other => panic!("expected AgentMessageContentDelta, got {:?}", other),
        }
    }

    #[test]
    fn empty_text_delta_maps_to_none() {
        let evt = AnthropicEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: String::new(),
            },
        };
        assert!(anthropic_event_to_event_msg(evt).is_none());
    }

    #[test]
    fn message_delta_maps_to_token_count_with_output_tokens() {
        let evt = AnthropicEvent::MessageDelta {
            delta: MessageDeltaBody {
                stop_reason: Some("end_turn".to_string()),
            },
            usage: Some(MessageDeltaUsage { output_tokens: 42 }),
        };
        let mapped = anthropic_event_to_event_msg(evt).unwrap();
        match mapped {
            EventMsg::TokenCount(TokenCountEvent { info: Some(usage) }) => {
                assert_eq!(usage.output_tokens, 42);
                assert_eq!(usage.total_tokens, 42);
            }
            other => panic!("expected TokenCount, got {:?}", other),
        }
    }

    #[test]
    fn message_stop_maps_to_empty_agent_message_marker() {
        let evt = AnthropicEvent::MessageStop;
        let mapped = anthropic_event_to_event_msg(evt).unwrap();
        match mapped {
            EventMsg::AgentMessage(m) => {
                assert_eq!(m.id, "");
                assert_eq!(m.text, "");
            }
            other => panic!("expected AgentMessage, got {:?}", other),
        }
    }

    #[test]
    fn ping_maps_to_none() {
        assert!(anthropic_event_to_event_msg(AnthropicEvent::Ping).is_none());
        assert!(
            anthropic_event_to_event_msg(AnthropicEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::Text {
                    text: String::new(),
                },
            })
            .is_none()
        );
        assert!(
            anthropic_event_to_event_msg(AnthropicEvent::ContentBlockStop { index: 0 }).is_none()
        );
    }

    #[test]
    fn error_maps_to_error_event() {
        let evt = AnthropicEvent::Error {
            error: AnthropicErrorBody {
                kind: "invalid_request_error".to_string(),
                message: "bad model".to_string(),
            },
        };
        let mapped = anthropic_event_to_event_msg(evt).unwrap();
        match mapped {
            EventMsg::Error(e) => {
                assert!(e.message.contains("invalid_request_error"));
                assert!(e.message.contains("bad model"));
                assert!(!e.fatal);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    // ----- provider construction + body shape -----

    #[test]
    fn provider_construction_defaults_are_sensible() {
        let p = AnthropicMessagesProvider::new("sk-test").unwrap();
        assert_eq!(p.api_base(), DEFAULT_ANTHROPIC_API_BASE);
        assert_eq!(p.anthropic_version(), DEFAULT_ANTHROPIC_VERSION);
    }

    #[test]
    fn provider_with_base_url_overrides_endpoint() {
        let p =
            AnthropicMessagesProvider::with_base_url("http://localhost:1234", "sk-test").unwrap();
        assert_eq!(p.api_base(), "http://localhost:1234");
    }

    #[test]
    fn provider_with_max_output_tokens_overrides_default() {
        let p = AnthropicMessagesProvider::new("sk-test")
            .unwrap()
            .with_max_output_tokens(1024);
        let body = p.build_body(&ModelRequest::from_text("claude-opus-4", "hi"));
        assert_eq!(body["max_tokens"], 1024);
    }

    #[test]
    fn build_body_shape() {
        let p = AnthropicMessagesProvider::new("sk-test").unwrap();
        let req = ModelRequest {
            model: "claude-opus-4".to_string(),
            input: vec![
                ModelInput::Text("hello".to_string()),
                ModelInput::Image("/tmp/x.png".to_string()),
            ],
        };
        let body = p.build_body(&req);
        assert_eq!(body["model"], "claude-opus-4");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
        // Phase 1.5 stub: images become text placeholders.
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "[image]");
    }
}

//! Retry, backoff, and stream-resilience helpers for the provider transport.
//!
//! Ported from the retry semantics used by OpenAI's codex-rs CLI:
//! exponential backoff (`2^(attempt-1) * base_delay`) with ±10% jitter,
//! retrying HTTP 429, 5xx, and transport-level failures. Streaming bodies are
//! guarded by [`IdleTimeoutReader`], which fails the read when no bytes arrive
//! within the configured idle window.

use std::{
    io::Read,
    sync::mpsc::{Receiver, RecvTimeoutError, sync_channel},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use wonder_of_u_core::{Result, WonderError};

use super::{HttpRequest, HttpResponse, HttpTransport};

/// Default number of retries for non-streaming provider requests.
pub(super) const DEFAULT_REQUEST_MAX_RETRIES: u32 = 4;
/// Default number of retries when establishing a streaming connection.
pub(super) const DEFAULT_STREAM_MAX_RETRIES: u32 = 5;
/// Default idle timeout for streaming bodies (codex-rs default: 5 minutes).
pub(super) const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 300_000;
/// Default base delay for the exponential backoff schedule.
pub(super) const DEFAULT_BASE_DELAY_MS: u64 = 200;
/// Upper bound applied to server-provided `Retry-After` hints.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(60);

/// Backoff schedule for retried provider requests.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RetryPolicy {
    /// Number of retries after the initial attempt.
    pub max_retries: u32,
    /// Base delay; attempt `n` waits roughly `2^(n-1) * base_delay`.
    pub base_delay: Duration,
}

/// Computes the backoff delay for `attempt` (1-based) with ±10% jitter.
pub(super) fn backoff_delay(policy: &RetryPolicy, attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(16);
    let raw = policy.base_delay.saturating_mul(1u32 << exp);
    // Jitter in 0.9..=1.1 derived from the clock's sub-second noise; avoids
    // adding a `rand` dependency for a non-cryptographic use case.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let jitter = 0.9 + (f64::from(nanos % 1000) / 1000.0) * 0.2;
    Duration::from_secs_f64(raw.as_secs_f64() * jitter)
}

/// Returns whether an HTTP status is worth retrying (rate limit or server error).
pub(super) fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// Returns whether a transport-level error (no HTTP status available) should
/// be retried. Matches the message prefixes produced by our own transports.
pub(super) fn is_retryable_transport_error(error: &WonderError) -> bool {
    let message = error.to_string();
    message.contains("provider request failed")
        || message.contains("stream idle timeout")
        || message.contains("invalid provider response body")
}

/// Extracts a `Retry-After: <seconds>` hint from response headers, capped.
fn retry_after_hint(response: &HttpResponse) -> Option<Duration> {
    let value = response.headers.get("retry-after")?;
    let seconds: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(RETRY_AFTER_CAP))
}

/// Executes `request` with retries per `policy`, sleeping via `sleep` between
/// attempts. Returns the final [`HttpResponse`] (which may still carry an
/// error status — callers apply [`super::ensure_success`] afterwards) or the
/// final transport error once retries are exhausted.
pub(super) fn execute_with_retry(
    transport: &dyn HttpTransport,
    request: &HttpRequest,
    policy: &RetryPolicy,
    sleep: &mut dyn FnMut(Duration),
) -> Result<HttpResponse> {
    let mut attempt: u32 = 0;
    loop {
        let outcome = transport.execute(request);
        attempt += 1;
        let retries_left = attempt <= policy.max_retries;
        match outcome {
            Ok(response) if is_retryable_status(response.status) && retries_left => {
                let delay =
                    retry_after_hint(&response).unwrap_or_else(|| backoff_delay(policy, attempt));
                sleep(delay);
            }
            Ok(response) => return Ok(response),
            Err(error) if is_retryable_transport_error(&error) && retries_left => {
                sleep(backoff_delay(policy, attempt));
            }
            Err(error) => return Err(error),
        }
    }
}

/// A blocking [`Read`] adapter that fails when the underlying reader produces
/// no bytes within `idle_timeout`.
///
/// A detached helper thread pulls chunks from the inner reader and forwards
/// them over a bounded channel; the consuming side waits at most
/// `idle_timeout` per chunk. On timeout the helper thread stays parked until
/// the underlying socket dies — bounded by the transport's body-level socket
/// timeout, so threads cannot accumulate indefinitely.
pub(super) struct IdleTimeoutReader {
    receiver: Receiver<std::io::Result<Vec<u8>>>,
    idle_timeout: Duration,
    buffer: Vec<u8>,
    offset: usize,
    done: bool,
}

impl IdleTimeoutReader {
    pub(super) fn new(mut inner: Box<dyn Read + Send>, idle_timeout: Duration) -> Self {
        let (sender, receiver) = sync_channel::<std::io::Result<Vec<u8>>>(4);
        std::thread::Builder::new()
            .name("stream-idle-guard".into())
            .spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match inner.read(&mut chunk) {
                        Ok(0) => {
                            let _ = sender.send(Ok(Vec::new()));
                            break;
                        }
                        Ok(n) => {
                            if sender.send(Ok(chunk[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = sender.send(Err(e));
                            break;
                        }
                    }
                }
            })
            .expect("spawn stream idle guard thread");
        Self {
            receiver,
            idle_timeout,
            buffer: Vec::new(),
            offset: 0,
            done: false,
        }
    }
}

impl Read for IdleTimeoutReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.offset >= self.buffer.len() {
            if self.done {
                return Ok(0);
            }
            match self.receiver.recv_timeout(self.idle_timeout) {
                Ok(Ok(chunk)) if chunk.is_empty() => {
                    self.done = true;
                    return Ok(0);
                }
                Ok(Ok(chunk)) => {
                    self.buffer = chunk;
                    self.offset = 0;
                }
                Ok(Err(e)) => {
                    self.done = true;
                    return Err(e);
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.done = true;
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!(
                            "stream idle timeout after {}s without data",
                            self.idle_timeout.as_secs()
                        ),
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.done = true;
                    return Ok(0);
                }
            }
        }
        let available = &self.buffer[self.offset..];
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.offset += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use super::super::StreamingHttpResponse;
    use super::*;

    struct ScriptedTransport {
        outcomes: Mutex<Vec<Result<HttpResponse>>>,
        requests_seen: Mutex<usize>,
    }

    impl ScriptedTransport {
        fn new(outcomes: Vec<Result<HttpResponse>>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes),
                requests_seen: Mutex::new(0),
            }
        }

        fn seen(&self) -> usize {
            *self.requests_seen.lock().expect("lock seen")
        }
    }

    impl HttpTransport for ScriptedTransport {
        fn execute(&self, _request: &HttpRequest) -> Result<HttpResponse> {
            *self.requests_seen.lock().expect("lock seen") += 1;
            let mut outcomes = self.outcomes.lock().expect("lock outcomes");
            if outcomes.is_empty() {
                return Err(WonderError::internal("scripted transport exhausted"));
            }
            outcomes.remove(0)
        }

        fn execute_stream(&self, _request: &HttpRequest) -> Result<StreamingHttpResponse> {
            unimplemented!("not exercised by retry unit tests")
        }
    }

    fn request() -> HttpRequest {
        HttpRequest {
            method: "POST".into(),
            url: "https://example.test/v1".into(),
            headers: BTreeMap::new(),
            body: String::new(),
        }
    }

    fn response(status: u16) -> HttpResponse {
        HttpResponse {
            status,
            body: String::new(),
            headers: BTreeMap::new(),
        }
    }

    fn instant_policy(max_retries: u32) -> RetryPolicy {
        RetryPolicy {
            max_retries,
            base_delay: Duration::ZERO,
        }
    }

    #[test]
    fn retries_429_then_succeeds() {
        let transport = ScriptedTransport::new(vec![Ok(response(429)), Ok(response(200))]);
        let mut sleeps = Vec::new();
        let result = execute_with_retry(&transport, &request(), &instant_policy(4), &mut |d| {
            sleeps.push(d);
        })
        .expect("final response");
        assert_eq!(result.status, 200);
        assert_eq!(transport.seen(), 2);
        assert_eq!(sleeps.len(), 1);
    }

    #[test]
    fn terminal_401_is_not_retried() {
        let transport = ScriptedTransport::new(vec![Ok(response(401))]);
        let result = execute_with_retry(&transport, &request(), &instant_policy(4), &mut |_| {})
            .expect("response returned for caller to classify");
        assert_eq!(result.status, 401);
        assert_eq!(transport.seen(), 1);
    }

    #[test]
    fn retry_after_header_is_honored() {
        let mut rate_limited = response(429);
        rate_limited
            .headers
            .insert("retry-after".into(), "7".into());
        let transport = ScriptedTransport::new(vec![Ok(rate_limited), Ok(response(200))]);
        let mut sleeps = Vec::new();
        execute_with_retry(&transport, &request(), &instant_policy(4), &mut |d| {
            sleeps.push(d);
        })
        .expect("final response");
        assert_eq!(sleeps, vec![Duration::from_secs(7)]);
    }

    #[test]
    fn transport_errors_exhaust_retries_and_surface_last_error() {
        let transport = ScriptedTransport::new(vec![
            Err(WonderError::validation("provider request failed: refused")),
            Err(WonderError::validation("provider request failed: refused")),
            Err(WonderError::validation("provider request failed: refused")),
        ]);
        let error = execute_with_retry(&transport, &request(), &instant_policy(2), &mut |_| {})
            .expect_err("retries exhausted");
        assert!(error.to_string().contains("provider request failed"));
        assert_eq!(transport.seen(), 3);
    }

    #[test]
    fn non_retryable_transport_error_returns_immediately() {
        let transport = ScriptedTransport::new(vec![Err(WonderError::internal(
            "scripted transport exhausted",
        ))]);
        execute_with_retry(&transport, &request(), &instant_policy(4), &mut |_| {})
            .expect_err("internal error is terminal");
        assert_eq!(transport.seen(), 1);
    }

    #[test]
    fn backoff_schedule_is_exponential_and_bounded() {
        let policy = RetryPolicy {
            max_retries: 4,
            base_delay: Duration::from_millis(100),
        };
        // Jitter is ±10%, so check each delay against its expected window.
        for (attempt, expected_ms) in [(1u32, 100u64), (2, 200), (3, 400), (4, 800)] {
            let delay = backoff_delay(&policy, attempt);
            let low = Duration::from_millis(expected_ms * 9 / 10);
            let high = Duration::from_millis(expected_ms * 11 / 10);
            assert!(
                delay >= low && delay <= high,
                "attempt {attempt}: {delay:?} outside [{low:?}, {high:?}]"
            );
        }
    }

    #[test]
    fn retryable_status_classification() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(200));
    }

    #[test]
    fn idle_timeout_reader_times_out_without_data() {
        struct NeverReady;
        impl Read for NeverReady {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                std::thread::sleep(Duration::from_secs(5));
                Ok(0)
            }
        }
        let mut reader = IdleTimeoutReader::new(Box::new(NeverReady), Duration::from_millis(50));
        let mut buf = [0u8; 16];
        let err = reader.read(&mut buf).expect_err("expected idle timeout");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(err.to_string().contains("stream idle timeout"));
    }

    #[test]
    fn idle_timeout_reader_passes_data_through() {
        let data = b"hello stream".to_vec();
        let mut reader = IdleTimeoutReader::new(
            Box::new(std::io::Cursor::new(data.clone())),
            Duration::from_secs(1),
        );
        let mut out = Vec::new();
        reader.read_to_end(&mut out).expect("read");
        assert_eq!(out, data);
    }

    #[test]
    fn idle_timeout_reader_tolerates_slow_but_alive_source() {
        struct Slow {
            chunks: Vec<Vec<u8>>,
        }
        impl Read for Slow {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                std::thread::sleep(Duration::from_millis(20));
                match self.chunks.pop() {
                    Some(chunk) => {
                        buf[..chunk.len()].copy_from_slice(&chunk);
                        Ok(chunk.len())
                    }
                    None => Ok(0),
                }
            }
        }
        let source = Slow {
            chunks: vec![b"b".to_vec(), b"a".to_vec()],
        };
        let mut reader = IdleTimeoutReader::new(Box::new(source), Duration::from_millis(200));
        let mut out = Vec::new();
        reader.read_to_end(&mut out).expect("read");
        assert_eq!(out, b"ab");
    }
}

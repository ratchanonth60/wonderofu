use std::time::Duration;

use wonder_of_u_core::{Result, WonderError};

use super::{HttpRequest, HttpResponse, HttpTransport};

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client builder should not fail with default config"),
        }
    }
}

#[async_trait::async_trait]
impl HttpTransport for ReqwestTransport {
    async fn execute(&self, request: &HttpRequest) -> Result<HttpResponse> {
        let method = match request.method.as_str() {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            "PATCH" => reqwest::Method::PATCH,
            other => {
                return Err(WonderError::internal(format!(
                    "unsupported HTTP method: {other}"
                )))
            }
        };
        let mut req = self.client.request(method, &request.url);
        for (key, value) in &request.headers {
            req = req.header(key.as_str(), value.as_str());
        }
        if !request.body.is_empty() {
            req = req.body(request.body.clone());
        }
        let response = req
            .send()
            .await
            .map_err(|e| WonderError::validation(format!("provider request failed: {e}")))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| WonderError::validation(format!("provider response body failed: {e}")))?;
        if status >= 400 {
            Err(WonderError::validation(format!(
                "provider HTTP request failed with status {status}: {}",
                super::provider_error_message(&body)
            )))
        } else {
            Ok(HttpResponse { status, body })
        }
    }
}
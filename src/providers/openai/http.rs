use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{
    config::SecretString,
    providers::types::{ProviderError, ProviderRequest},
};

use super::request::map_request;

const API_BASE_URL: &str = "https://api.openai.com/v1/";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

pub struct OpenAiProvider {
    client: reqwest::Client,
    base_url: reqwest::Url,
    api_key: String,
}

impl OpenAiProvider {
    pub fn new(api_key: &SecretString) -> Result<Self, ProviderError> {
        let base_url =
            reqwest::Url::parse(API_BASE_URL).map_err(|_| ProviderError::InvalidRequest)?;
        Self::from_parts(api_key.expose_secret(), base_url, REQUEST_TIMEOUT)
    }

    #[cfg(test)]
    fn with_base_url(
        api_key: &str,
        base_url: reqwest::Url,
        timeout: Duration,
    ) -> Result<Self, ProviderError> {
        Self::from_parts(api_key, base_url, timeout)
    }

    fn from_parts(
        api_key: &str,
        base_url: reqwest::Url,
        timeout: Duration,
    ) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|_| ProviderError::Transport)?;
        Ok(Self {
            client,
            base_url,
            api_key: api_key.to_owned(),
        })
    }

    pub async fn send_request(
        &self,
        request: &ProviderRequest,
        cancellation: &CancellationToken,
    ) -> Result<reqwest::Response, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let body = map_request(request).map_err(|_| ProviderError::InvalidRequest)?;
        let url = self
            .base_url
            .join("responses")
            .map_err(|_| ProviderError::InvalidRequest)?;
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            result = self.client
                .post(url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send() => result.map_err(map_transport_error)?,
        };

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status().as_u16();
        let body = read_error_body(response, cancellation).await?;
        if let Ok(envelope) = serde_json::from_slice::<ApiErrorEnvelope>(&body) {
            return Err(ProviderError::Api {
                status,
                code: safe_error_code(envelope.error.code),
            });
        }
        Err(ProviderError::HttpStatus { status })
    }
}

#[derive(Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    code: Option<String>,
}

fn safe_error_code(code: Option<String>) -> Option<String> {
    code.filter(|code| {
        !code.is_empty()
            && code.len() <= 64
            && code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    })
}

fn map_transport_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Transport
    }
}

async fn read_error_body(
    response: reqwest::Response,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(MAX_ERROR_BODY_BYTES.min(4096));
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            return Ok(body);
        };
        let chunk = chunk.map_err(map_transport_error)?;
        let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            return Ok(body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OpenAiProvider;
    use crate::{
        agent::message::Message,
        providers::types::{ProviderError, ProviderRequest},
    };
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
        time::Duration,
    };
    use tokio_util::sync::CancellationToken;

    const TEST_API_KEY: &str = "test-only-api-key";

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "gpt-4.1".into(),
            messages: vec![Message::user("hello")],
            tools: vec![],
            max_output_tokens: Some(128),
        }
    }

    fn mock_server(
        status: u16,
        body: &'static str,
        delay: Duration,
    ) -> (reqwest::Url, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            thread::sleep(delay);
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            request
        });
        (
            reqwest::Url::parse(&format!("http://{address}/v1/")).unwrap(),
            server,
        )
    }

    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = stream.read(&mut buffer).unwrap_or(0);
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let body_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + body_length {
                    break;
                }
            }
        }
        request
    }

    fn provider(url: reqwest::Url, timeout: Duration) -> OpenAiProvider {
        OpenAiProvider::with_base_url(TEST_API_KEY, url, timeout).unwrap()
    }

    #[tokio::test]
    async fn sends_post_path_auth_and_mapped_json() {
        let (url, server) = mock_server(200, "{}", Duration::ZERO);
        let provider = provider(url, Duration::from_secs(1));

        let response = provider
            .send_request(&request(), &CancellationToken::new())
            .await
            .unwrap();
        let request = String::from_utf8(server.join().unwrap()).unwrap();

        assert_eq!(response.status(), 200);
        assert!(request.starts_with("POST /v1/responses HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {TEST_API_KEY}"))
        );
        let body_start = request.find("\r\n\r\n").unwrap() + 4;
        let body: serde_json::Value = serde_json::from_str(&request[body_start..]).unwrap();
        assert_eq!(body["model"], "gpt-4.1");
        assert_eq!(body["stream"], true);
    }

    #[tokio::test]
    async fn maps_api_error_body_without_retaining_its_message() {
        let body = r#"{"error":{"type":"authentication_error","code":"invalid_api_key","message":"do not retain this message"}}"#;
        let (url, server) = mock_server(401, body, Duration::ZERO);
        let provider = provider(url, Duration::from_secs(1));

        let error = provider
            .send_request(&request(), &CancellationToken::new())
            .await
            .unwrap_err();
        let _ = server.join().unwrap();

        assert_eq!(
            error,
            ProviderError::Api {
                status: 401,
                code: Some("invalid_api_key".into())
            }
        );
        assert!(!format!("{error:?}").contains("do not retain this message"));
    }

    #[tokio::test]
    async fn maps_rate_limit_error_code() {
        let body = r#"{"error":{"code":"rate_limit_exceeded"}}"#;
        let (url, server) = mock_server(429, body, Duration::ZERO);
        let provider = provider(url, Duration::from_secs(1));

        let error = provider
            .send_request(&request(), &CancellationToken::new())
            .await
            .unwrap_err();
        let _ = server.join().unwrap();

        assert_eq!(
            error,
            ProviderError::Api {
                status: 429,
                code: Some("rate_limit_exceeded".into())
            }
        );
    }

    #[tokio::test]
    async fn maps_unstructured_server_failure_to_status_error() {
        let (url, server) = mock_server(500, "server failure", Duration::ZERO);
        let provider = provider(url, Duration::from_secs(1));

        let error = provider
            .send_request(&request(), &CancellationToken::new())
            .await
            .unwrap_err();
        let _ = server.join().unwrap();

        assert_eq!(error, ProviderError::HttpStatus { status: 500 });
    }

    #[tokio::test]
    async fn maps_request_timeout_to_timeout_error() {
        let (url, server) = mock_server(200, "{}", Duration::from_millis(150));
        let provider = provider(url, Duration::from_millis(30));

        let error = provider
            .send_request(&request(), &CancellationToken::new())
            .await
            .unwrap_err();
        let _ = server.join().unwrap();

        assert_eq!(error, ProviderError::Timeout);
    }

    #[tokio::test]
    async fn maps_connection_failure_to_transport_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = reqwest::Url::parse(&format!("http://{address}/v1/")).unwrap();
        let provider = provider(url, Duration::from_secs(1));

        let error = provider
            .send_request(&request(), &CancellationToken::new())
            .await
            .unwrap_err();

        assert_eq!(error, ProviderError::Transport);
    }

    #[tokio::test]
    async fn cancels_a_request_while_waiting_for_response() {
        let (url, server) = mock_server(200, "{}", Duration::from_millis(250));
        let provider = provider(url, Duration::from_secs(1));
        let cancellation = CancellationToken::new();
        let request_cancellation = cancellation.clone();
        let request_task = tokio::spawn(async move {
            provider
                .send_request(&request(), &request_cancellation)
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancellation.cancel();

        let error = request_task.await.unwrap().unwrap_err();
        let _ = server.join().unwrap();

        assert_eq!(error, ProviderError::Cancelled);
    }
}

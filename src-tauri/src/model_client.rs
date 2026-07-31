use crate::{
    config::model::validate_base_url,
    error::{AppError, ErrorCode},
};
use serde_json::{json, Value};
use std::time::Duration;

pub async fn complete_model_prompt(
    base_url: &str,
    model: &str,
    api_key: &str,
    timeout_seconds: u64,
    prompt: &str,
) -> Result<String, AppError> {
    validate_base_url(base_url)?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(AppError::new(
            ErrorCode::ModelApiKeyMissing,
            "请先填写模型 API Key",
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|_| AppError::new(ErrorCode::ModelConnectionHttpFailed, "无法创建模型连接"))?;
    let response = client
        .post(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .json(&chat_request_body(base_url, model, prompt, None))
        .send()
        .await
        .map_err(connection_error)?;
    let response = ensure_success(response).await?;
    let response: Value = response.json().await.map_err(|_| {
        AppError::new(
            ErrorCode::ModelConnectionInvalidResponse,
            "模型服务返回了无效响应",
        )
    })?;
    response["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .and_then(|choice| choice["message"]["content"].as_str())
        .filter(|content| !content.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid_choices_response(&response))
}

pub async fn test_model_connection(
    base_url: &str,
    model: &str,
    api_key: &str,
    timeout_seconds: u64,
) -> Result<(), AppError> {
    validate_base_url(base_url)?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(AppError::new(
            ErrorCode::ModelApiKeyMissing,
            "请先填写模型 API Key",
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|_| AppError::new(ErrorCode::ModelConnectionHttpFailed, "无法创建模型连接"))?;
    let response = client
        .post(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .json(&chat_request_body(base_url, model, "只回复 OK", Some(32)))
        .send()
        .await
        .map_err(connection_error)?;
    let response = ensure_success(response).await?;
    let response: Value = response.json().await.map_err(|_| {
        AppError::new(
            ErrorCode::ModelConnectionInvalidResponse,
            "模型服务返回了无效响应",
        )
    })?;
    if response["choices"].as_array().is_some_and(|choices| {
        choices.iter().any(|choice| {
            choice["message"]["content"]
                .as_str()
                .is_some_and(|content| !content.trim().is_empty())
        })
    }) {
        Ok(())
    } else {
        Err(invalid_choices_response(&response))
    }
}

fn chat_request_body(base_url: &str, model: &str, prompt: &str, max_tokens: Option<u64>) -> Value {
    let mut body = json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "temperature": 0,
    });
    if let Some(max_tokens) = max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if is_deepseek_base_url(base_url) {
        body["thinking"] = json!({ "type": "disabled" });
    }
    body
}

fn is_deepseek_base_url(base_url: &str) -> bool {
    url::Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.deepseek.com"))
}

fn connection_error(error: reqwest::Error) -> AppError {
    let code = if error.is_timeout() {
        ErrorCode::ModelConnectionTimeout
    } else {
        ErrorCode::ModelConnectionHttpFailed
    };
    AppError::new(code, "模型服务连接失败")
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, AppError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    let detail = model_error_detail(&body);
    let message = if detail.is_empty() {
        format!("模型服务连接失败：HTTP {status}")
    } else {
        format!("模型服务连接失败：HTTP {status}；{detail}")
    };
    Err(AppError::new(ErrorCode::ModelConnectionHttpFailed, message))
}

fn model_error_detail(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.pointer("/message"))
                .or_else(|| value.pointer("/error"))
                .and_then(|item| item.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| trimmed.chars().take(500).collect())
}

fn invalid_choices_response(response: &Value) -> AppError {
    AppError::new(
        ErrorCode::ModelConnectionInvalidResponse,
        format!(
            "模型服务未返回有效的 choices；响应摘要：{}",
            model_error_detail(&response.to_string())
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    type ServerResult = Result<(), String>;

    fn is_expected_disconnect(error: &std::io::Error) -> bool {
        matches!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
        )
    }

    #[test]
    fn treats_timeout_disconnect_write_errors_as_expected() {
        assert!(is_expected_disconnect(&std::io::Error::from(
            std::io::ErrorKind::BrokenPipe,
        )));
        assert!(is_expected_disconnect(&std::io::Error::from(
            std::io::ErrorKind::ConnectionReset,
        )));
    }

    fn read_request(stream: &mut TcpStream) -> Result<String, String> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            let size = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if size == 0 {
                return Err("request closed before headers completed".into());
            }
            bytes.extend_from_slice(&buffer[..size]);
            let Some(headers_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers =
                std::str::from_utf8(&bytes[..headers_end]).map_err(|error| error.to_string())?;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .or_else(|| line.strip_prefix("Content-Length:"))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= headers_end + 4 + content_length {
                return String::from_utf8(bytes).map_err(|error| error.to_string());
            }
        }
    }

    fn test_server(
        response: &'static str,
        delay: Duration,
    ) -> (String, thread::JoinHandle<ServerResult>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || -> ServerResult {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            read_request(&mut stream)?;
            thread::sleep(delay);
            match stream.write_all(response.as_bytes()) {
                Ok(()) => Ok(()),
                Err(error) if is_expected_disconnect(&error) => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        });
        (format!("http://{address}"), handle)
    }

    fn successful_test_server() -> (
        String,
        mpsc::Receiver<String>,
        thread::JoinHandle<ServerResult>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || -> ServerResult {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let request = read_request(&mut stream)?;
            sender.send(request).map_err(|error| error.to_string())?;
            let body = r#"{"choices":[{"message":{"content":"OK"}}]}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .map_err(|error| error.to_string())
        });
        (format!("http://{address}"), receiver, handle)
    }

    #[test]
    fn rejects_a_missing_api_key_before_connecting() {
        let error = tauri::async_runtime::block_on(test_model_connection(
            "http://127.0.0.1:1/v1",
            "model",
            "   ",
            1,
        ))
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ModelApiKeyMissing);
    }

    #[test]
    fn posts_the_minimal_connection_prompt_with_bearer_auth_and_accepts_choices() {
        let (base_url, request, server) = successful_test_server();

        tauri::async_runtime::block_on(test_model_connection(
            &base_url,
            "connection-model",
            "connection-key",
            1,
        ))
        .unwrap();

        let request = request.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap().unwrap();
        assert!(request.starts_with("POST /chat/completions HTTP/1.1"));
        assert!(request.contains("authorization: Bearer connection-key"));
        assert!(request.contains(r#""model":"connection-model""#));
        assert!(request.contains("只回复 OK"));
        assert!(!request.contains(r#""thinking""#));
    }

    #[test]
    fn deepseek_requests_explicitly_disable_thinking_mode() {
        let body = chat_request_body(
            "https://api.deepseek.com",
            "deepseek-v4-flash",
            "只回复 OK",
            Some(32),
        );

        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["max_tokens"], 32);
    }

    #[test]
    fn maps_non_success_http_responses_to_a_stable_code() {
        let (base_url, server) = test_server(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n",
            Duration::ZERO,
        );
        let error = tauri::async_runtime::block_on(test_model_connection(
            &base_url, "model", "test-key", 1,
        ))
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ModelConnectionHttpFailed);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn includes_http_status_and_body_when_model_service_rejects_the_request() {
        let body = r#"{"error":{"message":"Invalid API key"}}"#;
        let response = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let response = Box::leak(response.into_boxed_str());
        let (base_url, server) = test_server(response, Duration::ZERO);
        let error = tauri::async_runtime::block_on(test_model_connection(
            &base_url, "model", "test-key", 1,
        ))
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ModelConnectionHttpFailed);
        assert!(error.message.contains("HTTP 401"));
        assert!(error.message.contains("Invalid API key"));
        server.join().unwrap().unwrap();
    }

    #[test]
    fn maps_request_timeouts_to_a_stable_code() {
        let (base_url, server) = test_server(
            "HTTP/1.1 200 OK\r\nContent-Length: 14\r\n\r\n{\"choices\":[]}",
            Duration::from_secs(2),
        );
        let error = tauri::async_runtime::block_on(test_model_connection(
            &base_url, "model", "test-key", 1,
        ))
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ModelConnectionTimeout);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn rejects_a_response_without_choices() {
        let body = "{}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let response = Box::leak(response.into_boxed_str());
        let (base_url, server) = test_server(response, Duration::ZERO);
        let error = tauri::async_runtime::block_on(test_model_connection(
            &base_url, "model", "test-key", 1,
        ))
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ModelConnectionInvalidResponse);
        assert!(error.message.contains("响应摘要"));
        assert!(error.message.contains("{}"));
        server.join().unwrap().unwrap();
    }

    #[test]
    fn rejects_invalid_json_and_empty_choice_content_as_invalid_responses() {
        for body in ["not-json", r#"{"choices":[{}]}"#] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let response = Box::leak(response.into_boxed_str());
            let (base_url, server) = test_server(response, Duration::ZERO);
            let error = tauri::async_runtime::block_on(test_model_connection(
                &base_url, "model", "test-key", 1,
            ))
            .unwrap_err();

            assert_eq!(error.code, ErrorCode::ModelConnectionInvalidResponse);
            server.join().unwrap().unwrap();
        }
    }

    #[test]
    fn rejects_tampered_remote_http_endpoints_before_sending_a_bearer_token() {
        let error = tauri::async_runtime::block_on(test_model_connection(
            "http://api.example.com/v1",
            "model",
            "test-key",
            1,
        ))
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidEndpoint);
    }
}

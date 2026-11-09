//! HTTP-level integration tests. The server should expose OpenCode-shaped JSON.
use std::io::{Read, Write};
use std::net::TcpStream;

use tinycode::cli::VERSION;
use tinycode::srv::{start_server, ServerConfig};

fn config() -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".into(),
        port: 0,
        auto: true,
        version: VERSION.into(),
    }
}

fn http_request_stream(
    addr: std::net::SocketAddr,
    method: &'static str,
    path: &str,
    body_json: &str,
    expected: Option<&str>,
) -> String {
    let mut buf = Vec::new();
    let mut stream = TcpStream::connect(addr).expect("socket");
    let body = body_json.to_string();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nhost: 127.0.0.1\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("write head");
    stream.write_all(body.as_bytes()).expect("write body");
    stream.flush().expect("flush");
    stream.read_to_end(&mut buf).expect("body");
    let received = String::from_utf8_lossy(&buf).to_string();
    if let Some(expected) = expected {
        assert!(
            received.contains(expected),
            "expected {expected:?} missing in: {received:?}"
        );
    }
    received
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_session_flow_works() {
    std::env::set_var("TINYCODE_PROVIDER", "openai");
    std::env::set_var("TINYCODE_MODEL", "openai/gpt-4.1-mini");
    std::env::remove_var("TINYCODE_BASE_URL");
    let handle = start_server(config()).expect("server");
    let addr = handle.local_addr;
    let accept_loop_handle = tokio::spawn(handle.run());
    // Server info should come back correct and shaped.
    let _info = http_request_stream(addr, "GET", "/api/info", "{}", None);
    assert!(
        _info.contains(VERSION),
        "version must be in /api/info: {_info:?}"
    );
    let model = http_request_stream(addr, "GET", "/api/model", "{}", None);
    assert!(
        model.contains("\"id\":\"gpt-4.1-mini\""),
        "model list must keep the model id: {model:?}"
    );
    let session_create = http_request_stream(
        addr,
        "POST",
        "/api/session",
        r#"{"model":{"providerID":"openai","id":"gpt-4.1-mini"}}"#,
        None,
    );
    assert!(
        session_create.contains("ses_"),
        "session create must be OpenCode-shaped: {session_create:?}"
    );
    assert!(
        session_create.contains("gpt-4.1-mini"),
        "session create must keep model id: {session_create:?}"
    );
    // Prompt and message list should produce an OpenCode-shaped history.
    let prompt = http_request_stream(
        addr,
        "POST",
        "/api/session/ses_1/prompt",
        r#"{"text": "hello"}"#,
        None,
    );
    assert!(
        prompt.contains("assistant"),
        "prompt must be assistant-shaped"
    );
    let messages = http_request_stream(addr, "GET", "/api/session/ses_1/message", "{}", None);
    assert!(
        messages.contains("\"assistant\""),
        "history must contain an assistant entry: {messages:?}"
    );
    let _ = accept_loop_handle.await;
}

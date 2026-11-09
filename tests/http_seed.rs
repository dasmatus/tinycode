//! HTTP seed test without any real provider drive.
use std::io::{Read, Write};
use std::net::TcpStream;

use tinycode::cli::VERSION;
use tinycode::srv::{start_server, ServerConfig};

fn config() -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".into(),
        port: 0,
        auto: false,
        version: VERSION.into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_list_reports_the_configured_model() {
    std::env::set_var("TINYCODE_MODEL", "opencode-go/glm-5.3-flash");
    std::env::set_var("TINYCODE_PROVIDER", "opencode-go");
    let handle = start_server(config()).expect("server");
    let addr = handle.local_addr;
    let accept_loop_handle = tokio::spawn(handle.run());
    let response = stream_get(addr, "/api/model", None);
    assert!(
        accept_loop_handle.await.is_ok(),
        "server accept loop should finish"
    );
    assert!(
        response.contains("glm-5.3-flash"),
        "model list should include the configured model: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_info_reports_the_opencode_2_version() {
    let handle = start_server(config()).expect("server");
    // The accept loop must be running before the first socket read.
    let addr = handle.local_addr;
    let accept_loop_handle = tokio::spawn(handle.run());
    stream_get(addr, "/api/info", Some(VERSION));
    assert!(
        accept_loop_handle.await.is_ok(),
        "server accept loop should finish"
    );
}

fn stream_get(addr: std::net::SocketAddr, path: &str, expected: Option<&str>) -> String {
    let mut buf = Vec::new();
    let mut stream = TcpStream::connect(addr).expect("socket");
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n").as_bytes())
        .expect("write");
    stream.flush().expect("flush");
    stream.read_to_end(&mut buf).expect("read body");
    let received = String::from_utf8_lossy(&buf).to_string();
    if let Some(expected) = expected {
        assert!(
            received.contains(expected),
            "expected {expected:?} missing in: {received:?}"
        );
    }
    received
}

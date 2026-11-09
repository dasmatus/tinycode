pub mod http;
pub mod routes;
pub mod state;
pub mod wire;

pub use state::Room;

use miette::Result;
use serde_json::json;
use std::net::{SocketAddr, TcpListener, TcpStream};

use crate::srv::state::Response;

#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub auto: bool,
    pub version: String,
}

pub struct ServerHandle {
    pub local_addr: SocketAddr,
    pub listener: TcpListener,
    pub config: ServerConfig,
}

pub fn start_server(config: ServerConfig) -> Result<ServerHandle> {
    let listener = TcpListener::bind((config.host.as_str(), config.port))
        .map_err(|e| miette::miette!("bind {}/{}: {e}", config.host, config.port))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| miette::miette!("local address: {e}"))?;
    Ok(ServerHandle {
        local_addr,
        listener,
        config,
    })
}

pub fn server_once(
    room: &mut Room,
    config: ServerConfig,
    mut stream: TcpStream,
) -> Result<(), String> {
    let request = http::read_request(&mut stream)?;
    let method = request.method.to_string();
    let path = request.path.to_string();
    let body = serde_json::from_slice::<serde_json::Value>(&request.body).unwrap_or_default();

    // OpenCode-shaped routes, without worrying about path escapes yet.
    if request.path.starts_with("/api/session/") && request.path.ends_with("/prompt") {
        let sid = path.split('/').nth(3).unwrap_or_default().to_string();
        let mut body = body;
        body["sessionID"] = json!(sid);
        let response = room.session_prompt(&body);
        return http::write_json(&mut stream, response.status, &response.body);
    }
    if let Some(id) = request.path.strip_prefix("/api/session/") {
        if let Some(rest) = id.strip_suffix("/message") {
            let response = room.session_message_list(rest);
            return http::write_json(&mut stream, response.status, &response.body);
        }
    }
    if request.path == "/api/session" && request.method == "POST" {
        let response = room.session_create(&body);
        return http::write_json(&mut stream, response.status, &response.body);
    }
    if request.path == "/api/session" && request.method == "GET" {
        let response = room.session_list();
        return http::write_json(&mut stream, response.status, &response.body);
    }

    // Other endpoints.
    let response = match (method.as_str(), path.as_str()) {
        ("GET", "/api/info") => Response::ok(
            200,
            routes::server_info(&config, &format!("http://{}:{}", config.host, config.port)),
        ),
        ("GET", "/api/model") => room.model_list(),
        _ => Response::ok(404, routes::not_found()),
    };
    http::write_json(&mut stream, response.status, &response.body)
}

impl ServerHandle {
    /// The actual server loop, callable directly in tests.
    pub async fn run(self) -> Result<()> {
        // The listener blocks on `accept`, so keep it on its own OS thread:
        // tokio::spawn(worker tasks) stays responsive in tests and T3.
        let listener = self.listener;
        let config = self.config;
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(stream) => stream,
                    Err(e) => {
                        eprintln!("server accept error: {e}");
                        return;
                    }
                };
                let server_config = config.clone();
                std::thread::spawn(move || {
                    if let Err(e) = Self::serve_once(server_config, stream) {
                        eprintln!("server request error: {e}");
                    }
                });
            }
        });
        Ok(())
    }

    /// Blocks the caller until the listener stops.
    pub async fn run_forever(&self) -> Result<()> {
        loop {
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) => return Err(miette::miette!("accept error: {e}")),
            };
            if let Err(e) = Self::serve_once(self.config.clone(), stream) {
                eprintln!("server request error: {e}");
            }
        }
    }

    fn serve_once(config: ServerConfig, mut stream: TcpStream) -> Result<(), String> {
        let request = http::read_request(&mut stream)?;
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/info") => {
                let addr = format!("http://{}:{}", config.host, config.port);
                let body = routes::server_info(&config, &addr);
                http::write_json(&mut stream, 200, &body)
            }
            ("GET", "/api/model") => http::write_json(&mut stream, 200, &routes::model_list()),
            _ => http::write_json(&mut stream, 404, &routes::not_found()),
        }
    }
}

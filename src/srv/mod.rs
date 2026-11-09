pub mod http;
pub mod routes;

use miette::Result;
use std::net::{SocketAddr, TcpListener, TcpStream};

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

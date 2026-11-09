use serde_json::{json, Value};

use crate::srv::ServerConfig;

pub fn server_info(cfg: &ServerConfig, addr: &str) -> Value {
    json!({
        "version": cfg.version.clone(),
        "pid": std::process::id(),
        "urls": [addr.to_string()],
        "paths": {"tmp": std::env::temp_dir().join("tinycode").display().to_string()},
    })
}

pub fn not_found() -> Value {
    json!({"error": "not found", "path": "unknown", "method": "unknown"})
}

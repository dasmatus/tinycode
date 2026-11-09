use serde_json::{json, Value};

use crate::config::{resolve_model_spec, ProviderKind};
use crate::srv::ServerConfig;

pub fn server_info(cfg: &ServerConfig, addr: &str) -> Value {
    json!({
        "version": cfg.version.clone(),
        "pid": std::process::id(),
        "urls": [addr.to_string()],
        "paths": {"tmp": std::env::temp_dir().join("tinycode").display().to_string()},
    })
}

/// The model catalog T3's OpenCode 2 client requires.
pub fn model_list() -> Value {
    let configured = std::env::var("TINYCODE_MODEL").ok();
    let spec = configured
        .as_deref()
        .and_then(resolve_model_spec)
        .map(|(_kind, provider, _base, model)| (provider, model));
    let (provider_id, model_id, model_name) = match spec {
        Some((provider, model)) => (provider, model.clone(), model),
        None => (
            ProviderKind::Anthropic.name().to_string(),
            ProviderKind::Anthropic.default_model().to_string(),
            "tinycode default".to_string(),
        ),
    };
    json!({
        "location": {"directory": "."},
        "data": [{
            "id": model_id.clone(),
            "modelID": model_id.clone(),
            "providerID": provider_id,
            "type": "chat",
            "name": model_name,
            "capabilities": {"tools": true, "input": ["text"], "output": ["text"], "temperature": false},
            "variants": [],
            "time": {"released": 0},
            "cost": [{"tier": {"type": "context", "size": 0}, "input": 0, "output": 0, "cache": {"read": 0, "write": 0}}],
            "status": "active",
            "enabled": true,
            "limit": {"context": 200000, "output": 16384},
        }],
    })
}

pub fn not_found() -> Value {
    json!({"error": "not found", "path": "unknown", "method": "unknown"})
}

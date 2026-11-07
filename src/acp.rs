use crate::agent::Agent;
use crate::config::{self, Config};
use miette::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize, Debug)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize, Debug)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

struct AcpServer {
    sessions: Arc<Mutex<HashMap<String, (Agent, Config)>>>,
}

impl AcpServer {
    fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone()?;
        let result = match req.method.as_str() {
            "initialize" => self.initialize(req.params),
            "session/new" => self.session_new(req.params),
            "session/prompt" => self.session_prompt(req.params),
            "session/cancel" => {
                // notification style but might come as request
                return Some(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(json!({})),
                    error: None,
                });
            }
            _ => Err(JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
                data: None,
            }),
        };

        Some(match result {
            Ok(r) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(r),
                error: None,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(e),
            },
        })
    }

    fn initialize(&self, _params: Option<Value>) -> std::result::Result<Value, JsonRpcError> {
        Ok(json!({
            "protocolVersion": 1,
            "agentCapabilities": {
                "loadSession": false,
                "promptCapabilities": {
                    "image": false,
                    "audio": false,
                    "embeddedContext": false
                }
            },
            "agentInfo": {
                "name": "tinycode",
                "title": "TinyCode",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    fn session_new(&self, params: Option<Value>) -> std::result::Result<Value, JsonRpcError> {
        let params = params.unwrap_or(json!({}));
        let cwd = params
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);

        let cfg = build_config_from_env(cwd.clone()).map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Internal error: {}", e),
            data: None,
        })?;

        let agent = Agent::new(cfg.clone()).map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Internal error: {}", e),
            data: None,
        })?;

        let session_id = agent
            .session_path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("session")
            .to_string();

        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), (agent, cfg));

        Ok(json!({
            "sessionId": session_id
        }))
    }

    fn session_prompt(&self, params: Option<Value>) -> std::result::Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;
        let session_id = params
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing sessionId".to_string(),
                data: None,
            })?
            .to_string();

        // Extract prompt text
        let mut task = String::new();
        if let Some(prompt) = params.get("prompt") {
            if let Some(arr) = prompt.as_array() {
                for block in arr {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        task.push_str(text);
                        task.push('\n');
                    } else if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(content) = block.get("content").and_then(|v| v.as_str()) {
                            task.push_str(content);
                            task.push('\n');
                        } else if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            task.push_str(text);
                            task.push('\n');
                        }
                    }
                }
            } else if let Some(s) = prompt.as_str() {
                task.push_str(s);
            }
        }
        let task = task.trim().to_string();

        let mut sessions = self.sessions.lock().unwrap();
        let (agent, _cfg) = sessions.get_mut(&session_id).ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Session not found".to_string(),
            data: None,
        })?;

        if !task.is_empty() {
            agent.messages.push(serde_json::json!({"role": "user", "content": task}));
        }

        let mut final_text = String::new();
        let _code = agent
            .run(&mut |event: &crate::agent::Event| {
                match event {
                    crate::agent::Event::Assistant(text) => {
                        final_text.push_str(text);
                    }
                    crate::agent::Event::Done { text, .. } => {
                        final_text = text.clone();
                    }
                    _ => {}
                }
            })
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Internal error: {}", e),
                data: None,
            })?;

        Ok(json!({
            "stopReason": "end_turn"
        }))
    }
}

fn build_config_from_env(root: PathBuf) -> std::result::Result<Config, Box<dyn std::error::Error>> {
    let (oc_config_path, oc_auth_path) = config::opencode_paths();
    let oc = config::load_opencode(&oc_config_path, &oc_auth_path);

    let provider = if let Ok(p) = std::env::var("TINYCODE_PROVIDER") {
        config::ProviderKind::parse(&p).unwrap_or(config::ProviderKind::Anthropic)
    } else if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        config::ProviderKind::Anthropic
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        config::ProviderKind::OpenAi
    } else if let Some(p) = oc.provider {
        p
    } else {
        config::ProviderKind::Anthropic
    };

    let api_key = std::env::var("TINYCODE_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .or_else(|_| {
            if oc.provider == Some(provider) {
                oc.api_key.clone().ok_or(std::env::VarError::NotPresent)
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .unwrap_or_default();

    let model = std::env::var("TINYCODE_MODEL")
        .or_else(|_| {
            if let Some((k, m)) = &oc.model {
                if *k == provider {
                    Ok(m.clone())
                } else {
                    Err(std::env::VarError::NotPresent)
                }
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .unwrap_or_else(|_| provider.default_model().to_string());

    let base_url = std::env::var("TINYCODE_BASE_URL")
        .or_else(|_| {
            if oc.provider == Some(provider) {
                oc.base_url.clone().ok_or(std::env::VarError::NotPresent)
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .unwrap_or_else(|_| provider.default_base_url().to_string());

    let system_prompt = std::env::var("TINYCODE_SYSTEM_PROMPT").ok().filter(|s| !s.is_empty());

    Ok(Config {
        provider,
        api_key,
        model,
        base_url,
        provider_id: if oc.provider == Some(provider) {
            oc.provider_id.clone()
        } else {
            None
        },
        usage_file: None,
        root,
        auto: true,
        max_turns: 24,
        json: false,
        quiet: true,
        session_file: None,
        task: String::new(),
        system_prompt,
    })
}

pub async fn run_acp() -> Result<()> {
    let server = AcpServer::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(req) = serde_json::from_str::<JsonRpcRequest>(&line) {
            if let Some(resp) = server.handle_request(req) {
                let resp_str = serde_json::to_string(&resp).unwrap();
                let _ = writeln!(stdout, "{}", resp_str);
                let _ = stdout.flush();
            }
            // If it's a notification (no id), we don't respond
        }
    }
    Ok(())
}

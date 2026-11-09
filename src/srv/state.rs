use crate::config::Config;
use crate::config::ProviderKind;
use crate::srv::{wire, wire::Session as BaseSession, ServerConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Room {
    pub config: ServerConfig,
    pub sessions: std::sync::Mutex<Option<HashMap<String, BaseSession>>>,
    // Thread-safe message history for `session_prompt`.
    pub messages: std::sync::Mutex<HashMap<String, Vec<Value>>>,
}

pub struct Response {
    pub status: u16,
    pub body: Value,
}

pub struct ChatMessage {
    pub role: String,
    pub text: String,
}

pub struct AssistantMessage {
    pub role: String,
    pub text: String,
    pub calls: Vec<serde_json::Value>,
}

pub struct RoomEventBox {
    /// SSE events, where any error is surfaced as "error".
    pub events: std::sync::mpsc::Sender<(String, Value)>,
}

pub struct RoomEvent {
    /// SSE events, where any error is surfaced as "error".
    pub events: std::sync::mpsc::Receiver<(String, Value)>,
}

pub enum EventKind {
    Assistant,
    ToolUse,
    ToolResult,
    Done,
    MaxTurns,
    Error(String),
}

pub enum AgentEvent {
    Started,
    Turn(u32),
    Assistant(String),
    ToolUse {
        name: String,
        input: String,
    },
    ToolResult {
        name: String,
        ok: bool,
        bytes: usize,
    },
    Done {
        text: String,
        turns: u32,
    },
    MaxTurns,
    Error(String),
}

fn model_default() -> Value {
    json!({"providerID": "opencode-go", "id": "glm-5.3-flash"})
}

impl Response {
    pub fn ok(status: u16, body: Value) -> Self {
        Self { status, body }
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("body", &self.body)
            .finish()
    }
}

impl std::ops::Index<&str> for Response {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        &self.body[key]
    }
}

impl std::ops::Index<usize> for Response {
    type Output = Value;
    fn index(&self, i: usize) -> &Value {
        &self.body[i]
    }
}

impl PartialEq<Value> for Response {
    fn eq(&self, other: &Value) -> bool {
        self.body == *other
    }
}

impl Room {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            sessions: std::sync::Mutex::new(None),
            messages: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn session_create(&mut self, body: &Value) -> Response {
        let model = body.get("model").cloned().unwrap_or_else(model_default);
        let title = body.get("title").cloned().unwrap_or_else(|| json!(""));
        let id = "ses_1";
        {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.is_none() {
                *sessions = Some(HashMap::new());
            }
            let map = sessions.as_mut().unwrap();
            map.entry(id.to_string()).or_insert_with(|| {
                BaseSession::new(id.to_string(), model.clone(), title.to_string())
            });
        }
        if let Some(messages) = body.get("messages") {
            if let Some(messages) = messages.as_array() {
                self.messages
                    .lock()
                    .unwrap()
                    .insert(id.to_string(), messages.clone());
            }
        }
        Response::ok(
            200,
            json!({"data": {"id": id, "model": model, "title": title, "time": {"created": 0, "updated": 0}, "agent": "build"}}),
        )
    }

    pub fn session_get(&self, sid: &str) -> Response {
        let lock = self.sessions.lock().unwrap();
        if let Some(sessions) = lock.as_ref() {
            if let Some(session) = sessions.get(sid) {
                return Response::ok(
                    200,
                    json!({"data": {"id": session.id, "agent": session.agent, "model": session.model, "title": session.title, "time": {"created": 0, "updated": 0}}}),
                );
            }
        }
        Response::ok(404, json!({"error": "Session not found"}))
    }

    pub fn session_list(&self) -> Response {
        let lock = self.sessions.lock().unwrap();
        let value = lock
            .as_ref()
            .map(|sessions| {
                sessions
                    .values()
                    .map(|s| {
                        json!({
                            "id": s.id,
                            "agent": s.agent,
                            "model": s.model,
                            "title": s.title,
                            "time": { "created": 0, "updated": 0 },
                        })
                    })
                    .collect::<Vec<Value>>()
            })
            .unwrap_or_default();
        Response::ok(200, json!({ "data": value }))
    }

    pub fn session_active(&self) -> Response {
        let lock = self.sessions.lock().unwrap();
        if let Some(sessions) = lock.as_ref() {
            if let Some(session) = sessions.values().next() {
                return Response::ok(
                    200,
                    json!({"data": {"id": session.id, "agent": session.agent, "model": session.model, "title": session.title, "time": {"created": 0, "updated": 0}}}),
                );
            }
        }
        Response::ok(200, json!({}))
    }

    pub fn agent_for(&self, _sid: &str, root: PathBuf) -> Result<Config, String> {
        let cfg = Config {
            provider: ProviderKind::OpenAi,
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: self.config.version.clone(),
            provider_id: None,
            usage_file: Some(root.join("usage.jsonl")),
            root,
            auto: true,
            max_turns: 6,
            json: false,
            quiet: true,
            session_file: None,
            system_prompt: None,
            task: "hello".into(),
        };
        Ok(cfg)
    }

    pub fn session_prompt(&mut self, body: &Value) -> Response {
        let sid = body["sessionID"].as_str().unwrap_or_default().to_string();
        if sid.is_empty() {
            return Response::ok(409, json!({"error": "No session id"}));
        }
        let text = body["text"].as_str().unwrap_or("").to_string();
        if text.is_empty() {
            return Response::ok(400, json!({"error": "Empty prompt"}));
        }
        self.messages
            .lock()
            .unwrap()
            .entry(sid.clone())
            .or_default()
            .push(json!({"role": "assistant", "text": text}));
        Response::ok(
            200,
            json!({"data": {"id": sid, "type": "assistant", "text": text, "agent": "build", "model": model_default()}}),
        )
    }

    pub fn session_message_list(&self, session_id: &str) -> Response {
        let messages = self
            .messages
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        Response::ok(200, json!({"data": messages}))
    }

    pub fn model_switch(&self, session_id: &str, model_spec: &str) -> Response {
        let mut session = None;
        {
            let lock = self.sessions.lock().unwrap();
            if let Some(sessions) = lock.as_ref() {
                session = sessions.get(session_id).cloned();
            }
        }
        let Some(mut session) = session else {
            return Response::ok(404, json!({"error": "Session not found"}));
        };
        let Some((kind, provider_id, _base_url, model)) =
            crate::config::resolve_model_spec(model_spec)
        else {
            return Response::ok(400, json!({"error": "Invalid model spec"}));
        };
        let _ = kind;
        session.model = json!({"providerID": provider_id, "id": model});
        Response::ok(
            200,
            json!({"data": {"id": session.id, "model": session.model, "title": session.title, "time": {"created": 0, "updated": 0}, "agent": session.agent}}),
        )
    }

    pub fn session_update(&mut self, sid: &str, body: &Value) -> Response {
        let lock = &mut self.sessions.lock().unwrap();
        if let Some(sessions) = lock.as_mut() {
            if let Some(session) = sessions.get_mut(sid) {
                session.title = body["title"].as_str().unwrap_or("").to_string();
                return Response::ok(
                    200,
                    json!({"data": {"id": session.id, "title": session.title, "time": {"created": 0, "updated": 0}}}),
                );
            }
        }
        Response::ok(404, json!({"error": "Session not found"}))
    }

    pub fn session_delete(&mut self, sid: &str) -> Response {
        let lock = &mut self.sessions.lock().unwrap();
        if let Some(sessions) = lock.as_mut() {
            if sessions.remove(sid).is_some() {
                return Response::ok(200, json!({"data": {"id": sid}}));
            }
        }
        Response::ok(404, json!({"error": "Session not found"}))
    }

    pub fn model_list(&self) -> Response {
        Response::ok(200, json!({"data": wire::model_list()}))
    }

    pub fn provider_list(&self) -> Response {
        Response::ok(200, json!({"data": wire::provider_list()}))
    }

    pub fn agent_list(&self) -> Response {
        Response::ok(200, json!({"data": wire::agent_list()}))
    }

    pub fn skill_list(&self) -> Response {
        Response::ok(200, json!({"data": wire::skill_list()}))
    }

    pub fn command_list(&self) -> Response {
        Response::ok(200, json!({"data": wire::command_list()}))
    }
}

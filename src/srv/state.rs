use serde_json::{json, Value};

use crate::srv::{wire, wire::Session as BaseSession, ServerConfig};
use std::collections::HashMap;

pub struct Room {
    pub config: ServerConfig,
    pub sessions: std::sync::Mutex<Option<HashMap<String, BaseSession>>>,
}

pub struct Response {
    pub status: u16,
    pub body: Value,
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
        }
    }

    pub fn session_create(&mut self, body: &Value) -> Response {
        let model = body
            .get("model")
            .cloned()
            .unwrap_or_else(|| json!({"providerID": "opencode-go", "id": "glm-5.3-flash"}));
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

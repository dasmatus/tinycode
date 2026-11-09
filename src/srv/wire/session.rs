use serde_json::{json, Value};

pub struct Session {
    pub id: String,
    pub agent: String,
    pub model: Value,
    pub title: String,
    pub file: std::path::PathBuf,
    pub status: u16,
}

impl Session {
    pub fn new(id: String, model: Value, title: String) -> Self {
        Self {
            id,
            agent: "build".into(),
            model,
            title,
            file: std::path::PathBuf::from("tmp/sessions.json"),
            status: 200,
        }
    }
}

pub fn session_get(sid: &str) -> Value {
    json!({
        "id": sid,
        "agent": "build",
        "model": json!({"providerID": "opencode-go", "id": "glm-5.3-flash"}),
        "time": {"created": 0, "updated": 0},
        "status": 200,
    })
}

pub fn session_delete(sid: &str) -> Value {
    json!({"id": sid, "status": 200})
}

pub fn session_update(sid: &str) -> Value {
    json!({"id": sid, "status": 200})
}
pub fn session_list() -> Value {
    json!([])
}
pub fn session_active() -> Value {
    json!({})
}

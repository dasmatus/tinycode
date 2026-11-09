use serde_json::{json, Value};

use crate::srv::ServerConfig;

pub mod model;
pub mod session;
pub mod skill;

pub use model::{model_list, provider_list};
pub use session::{session_delete, session_get, session_update, Session};
pub use skill::{agent_list, command_list, skill_list};

pub struct Room {
    /// Read-only configuration for the server process.
    pub config: ServerConfig,
    /// Session state that T3 Code manages for this process.
    pub sessions: std::sync::Mutex<Option<Session>>,
}

impl Room {
    pub fn default_model() -> Value {
        json!({"providerID": "opencode-go", "id": "glm-5.3-flash"})
    }
}

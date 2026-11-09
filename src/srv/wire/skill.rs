use serde_json::{json, Value};

pub fn skill_list() -> Value {
    json!([])
}
pub fn command_list() -> Value {
    json!([])
}
pub fn agent_list() -> Value {
    json!([
        {
            "id": "build",
            "name": "Build",
            "providerID": "opencode-go",
        },
        {
            "id": "plan",
            "name": "Plan",
            "providerID": "opencode-go",
        }
    ])
}

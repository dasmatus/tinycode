//! Catalog endpoints exposed by TinyCode's OpenCode 2 surface.
//!
//! The OpenCode 2 protocol requires these to be flat JSON with `id`,
//! `providerID`, `name`, `capabilities`, `variants`, `time`, `cost`, `status`,
//! `enabled`, and `limit`.

use serde_json::{json, Value};

pub fn model_list() -> Value {
    json!([
        {
            "id": "glm-5.3-flash",
            "modelID": "glm-5.3-flash",
            "providerID": "opencode-go",
            "name": "GLM-5.3-Flash",
            "capabilities": {"tools": true, "input": ["text"], "output": ["text"], "temperature": false},
            "variants": [],
            "time": {"released": 0},
            "cost": [{"tier": {"type": "context", "size": 0}, "input": 0, "output": 0, "cache": {"read": 0, "write": 0}}],
            "status": "active",
            "enabled": true,
            "limit": {"context": 200000, "output": 16384},
        }
    ])
}

pub fn provider_list() -> Value {
    json!([
        {
            "id": "opencode-go",
            "name": "OpenCode Go",
            "providerID": "opencode-go",
            "modelRef": "opencode-go/glm-5.3-flash",
        }
    ])
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

pub fn skill_list() -> Value {
    json!([])
}

pub fn command_list() -> Value {
    json!([])
}

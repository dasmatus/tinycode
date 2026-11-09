//! Prompt and agent event flow tests. Real ProviderKind is exposed through the
//! room, while mock provider SSE fills the event stream.
use serde_json::{json, Value};

use tinycode::cli::VERSION;
use tinycode::config::ProviderKind;
use tinycode::srv::{Room, ServerConfig};

fn cfg() -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".into(),
        port: 0,
        auto: true,
        version: VERSION.to_string(),
    }
}

fn model_ref(kind: ProviderKind, model: &str) -> Value {
    json!({"providerID": kind.name(), "id": model})
}

#[test]
fn prompt_and_message_list_shapes_are_consistent() {
    let mut room = Room::new(cfg());
    let created =
        room.session_create(&json!({"model": model_ref(ProviderKind::OpenAi, "test-model")}));
    let sid = created["data"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let prompt = room.session_prompt(&json!({"sessionID": sid, "text": "please reply with alfa"}));
    assert_eq!(prompt.status, 200);
    let messages = room.session_message_list(&sid);
    assert_eq!(messages.status, 200);
    let list = messages.body["data"].as_array().expect("messages");
    assert!(!list.is_empty(), "assistant history should not be empty");
    assert_eq!(list[0]["role"], "assistant");
}

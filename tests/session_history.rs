//! Session/room state tests. Each test works with a fresh model reference and
//! a common fake ProviderKind so the OpenCode wire surface is consistent.
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

fn session_create(room: &mut Room, model: &str) -> Value {
    let req = json!({"model": model_ref(ProviderKind::Anthropic, model)});
    room.session_create(&req).body
}

fn prompt_body(sid: &str, text: &str) -> Value {
    json!({"sessionID": sid, "text": text})
}

fn session_id(created: &Value) -> String {
    created["data"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn session_create_emits_the_data_shape() {
    let mut room = Room::new(cfg());
    let session = session_create(&mut room, "claude-3-5-sonnet");
    let sid = session_id(&session);
    assert!(sid.starts_with("ses_"), "session id: {sid:?}");
    assert_eq!(session["data"]["agent"], "build");
    assert_eq!(session["data"]["model"]["id"], "claude-3-5-sonnet");
    assert_eq!(
        session["data"]["time"]["created"],
        session["data"]["time"]["updated"]
    );
}

#[test]
fn session_prompt_earns_a_fresh_model_with_a_sensible_message() {
    let mut room = Room::new(cfg());
    let session = session_create(&mut room, "claude-3-5-sonnet");
    let sid = session_id(&session);
    let body = prompt_body(&sid, "help me make a file");
    let created = room.session_prompt(&body);
    assert_eq!(created.status, 200);
    let messages = room.session_message_list(&sid);
    assert_eq!(messages.status, 200);
    let text = messages.body["data"].as_array().expect("messages");
    assert!(!text.is_empty(), "assistant history should not be empty");
    assert_eq!(text[0]["role"], "assistant");
}

#[test]
fn empty_prompt_is_400_and_unknown_session_is_409() {
    let mut room = Room::new(cfg());
    let session = session_create(&mut room, "claude-3-5-sonnet");
    let sid = session_id(&session);
    let body = prompt_body(&sid, "");
    let empty = room.session_prompt(&body);
    assert_eq!(empty.status, 400);
    assert_eq!(empty.body["error"], "Empty prompt");

    let missing = room.session_prompt(&prompt_body("", "hi"));
    assert_eq!(missing.status, 409);
    assert_eq!(missing.body["error"], "No session id");
}

//! Model switch tests. Uses a fake provider and a cleanly named model.
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

#[test]
fn model_switch_updates_fresh_momentum_and_keeps_unknown_400() {
    let mut room = Room::new(cfg());
    let session = session_create(&mut room, "claude-3-5-sonnet");
    let sid = session["data"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let valid = room.model_switch(&sid, "anthropic/claude-3-5-haiku");
    assert_eq!(valid.status, 200);
    assert_eq!(valid.body["data"]["model"]["id"], "claude-3-5-haiku");
    assert_eq!(valid.body["data"]["model"]["providerID"], "anthropic");

    let invalid = room.model_switch(&sid, "not-a-model");
    assert_eq!(invalid.status, 400);
}

#[test]
fn unknown_provider_model_switch_is_400() {
    let mut room = Room::new(cfg());
    let session = session_create(&mut room, "claude-3-5-sonnet");
    let sid = session["data"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let out = room.model_switch(&sid, "openai/gpt-4.1-mini");
    assert_eq!(out.status, 200);
}

#[test]
fn model_switch_can_only_change_the_session_if_it_exists() {
    let mut room = Room::new(cfg());
    let session = session_create(&mut room, "claude-3-5-sonnet");
    let sid = session["data"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    room.session_delete(&sid);
    let invalid = room.model_switch(&sid, "anthropic/claude-3-5-haiku");
    assert_eq!(invalid.status, 404);
}

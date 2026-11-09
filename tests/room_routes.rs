//! Concrete session shapes across the HTTP surface and OpenCode 2 wire.
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

fn session_body(created: &Value) -> Value {
    created["data"].clone()
}

fn session_id(created: &Value) -> String {
    session_body(created)["id"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn session_create_makes_an_opencode_id_and_wrapped_model() {
    let cfg = cfg();
    let req = json!({
        "model": model_ref(ProviderKind::OpenAi, "test-model"),
        "title": "alpha thread"
    });
    let mut room = Room::new(cfg);
    let created = room.session_create(&req);
    assert_eq!(created.status, 200);

    let body = session_body(&created.body);
    assert!(body["id"].as_str().is_some_and(|id| id.starts_with("ses_")));
    assert_eq!(body["agent"], "build");
    assert_eq!(body["title"], "alpha thread");
    assert_eq!(body["time"]["created"], body["time"]["updated"]);
}

#[test]
fn session_get_returns_status_200_for_a_live_session() {
    let mut room = Room::new(cfg());
    let req = json!({"model": model_ref(ProviderKind::OpenAi, "test-model")});
    let created = room.session_create(&req);
    let sid = session_id(&created.body);
    let fetched = room.session_get(&sid);
    assert_eq!(fetched.status, 200);
    assert_eq!(fetched.body["data"]["id"], sid);
    assert_eq!(fetched.body["data"]["agent"], "build");
}

#[test]
fn session_list_and_active_shapes_are_consistent() {
    let room = Room::new(cfg());
    let list = room.session_list();
    assert_eq!(list.status, 200);
    assert_eq!(list.body["data"], json!([]));
    let active = room.session_active();
    assert_eq!(active.status, 200);
    assert_eq!(active.body, json!({}));
}

#[test]
fn unknown_session_returns_404_and_no_body() {
    let room = Room::new(cfg());
    let missing = room.session_get("ses_missing");
    assert_eq!(missing.status, 404);
    assert_eq!(missing.body, json!({"error": "Session not found"}));
}

#[test]
fn session_update_and_delete_are_wired() {
    let mut room = Room::new(cfg());
    let created =
        room.session_create(&json!({"model": model_ref(ProviderKind::OpenAi, "test-model")}));
    let sid = session_id(&created.body);
    let update = room.session_update(&sid, &json!({"title": "alpha thread"}));
    assert_eq!(update.status, 200);
    assert_eq!(update.body["data"]["title"], "alpha thread");

    let deleted = room.session_delete(&sid);
    assert_eq!(deleted.status, 200);
    assert_eq!(deleted.body["data"]["id"], sid);
}

#[test]
fn model_list_exposes_a_nonempty_catalog() {
    let room = Room::new(cfg());
    let list = room.model_list();
    assert_eq!(list.status, 200);
    let models = list.body["data"].as_array().expect("models");
    assert!(
        !models.is_empty(),
        "model list must not be empty: {models:?}"
    );
    assert!(models[0]["id"].as_str().is_some());
    assert!(models[0]["providerID"].as_str().is_some());
    assert!(models[0]["name"].as_str().is_some());
    assert!(models[0]["capabilities"].is_object());
    assert!(models[0]["time"]["released"].as_i64().unwrap_or(0) >= 0);
    assert!(models[0]["cost"].is_array());
    assert!(models[0]["limit"].as_object().is_some());
}

#[test]
fn provider_list_exposes_a_nonempty_catalog() {
    let room = Room::new(cfg());
    let list = room.provider_list();
    assert_eq!(list.status, 200);
    assert!(!list.body["data"].as_array().unwrap().is_empty());
    assert!(list.body["data"][0]["id"].as_str().is_some());
}

#[test]
fn agent_list_exposes_a_nonempty_catalog() {
    let room = Room::new(cfg());
    let list = room.agent_list();
    assert_eq!(list.status, 200);
    assert!(list.body["data"].as_array().unwrap().len() >= 2);
    assert!(list.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == "build"));
    assert!(list.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == "plan"));
}

#[test]
fn skill_list_exposes_an_empty_catalog() {
    let room = Room::new(cfg());
    let list = room.skill_list();
    assert_eq!(list.status, 200);
    assert_eq!(list.body["data"], json!([]));
}

#[test]
fn command_list_exposes_an_empty_catalog() {
    let room = Room::new(cfg());
    let list = room.command_list();
    assert_eq!(list.status, 200);
    assert_eq!(list.body["data"], json!([]));
}

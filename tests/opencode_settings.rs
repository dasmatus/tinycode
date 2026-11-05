//! Reusing OpenCode's own settings: the `opencode-go` gateway in
//! auth.json / opencode.json resolves to the OpenAI-compatible Go
//! endpoint, and `tinycode usage` reporting is wired into config docs.
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use tinycode::config::{self, ProviderKind, OPENCODE_GO_BASE_URL};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// These tests trade TINYCODE_OPENCODE_* env vars process-wide; the
/// resolver reads them inside parse_args. Serialize the whole file.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn tempdir(tag: &str) -> TempDir {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("tinycode-oc-settings-{tag}-{nanos}"));
    fs::create_dir_all(&path).expect("mkdir");
    TempDir(path)
}

fn parse_with_config(config_json: &str, auth_json: &str) -> config::Config {
    let dir = tempdir("parse");
    let config_path = dir.0.join("opencode.json");
    let auth_path = dir.0.join("auth.json");
    fs::write(&config_path, config_json).expect("write opencode.json");
    fs::write(&auth_path, auth_json).expect("write auth.json");
    // Point the resolver at our temp files, one at a time.
    let cfg = {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("TINYCODE_OPENCODE_CONFIG", &config_path);
        std::env::set_var("TINYCODE_OPENCODE_AUTH", &auth_path);
        let cfg = config::parse_args(&["run".to_string(), "demo".to_string()]).expect("config");
        std::env::remove_var("TINYCODE_OPENCODE_CONFIG");
        std::env::remove_var("TINYCODE_OPENCODE_AUTH");
        cfg
    };
    cfg
}

#[test]
fn opencode_go_auth_json_resolves_gateway() {
    // Model comes from OpenCode's default; key from auth.json's opencode-go entry.
    let json_text = r#"{"model": "opencode-go/glm-5.3-flash"}"#;
    let auth_text = r#"{"opencode-go": {"type": "api", "key": "sk-test-go"}}"#;
    let cfg = parse_with_config(json_text, auth_text);
    assert_eq!(cfg.provider, ProviderKind::OpenAi, "opencode-go is OpenAI-compatible");
    assert_eq!(cfg.provider_id.as_deref(), Some("opencode-go"));
    assert_eq!(cfg.base_url, OPENCODE_GO_BASE_URL);
    assert_eq!(cfg.model, "glm-5.3-flash");
    assert_eq!(cfg.api_key, "sk-test-go");
}

#[test]
fn auth_json_plain_kinds_still_resolve_first() {
    let auth_text = r#"{
        "anthropic": {"type": "api", "key": "sk-ant-test"},
        "opencode-go": {"type": "api", "key": "sk-test-go"}
    }"#;
    let cfg = parse_with_config("{}", auth_text);
    assert_eq!(cfg.provider, ProviderKind::Anthropic, "plain anthropic wins");
    assert_eq!(cfg.api_key, "sk-ant-test");
    assert_eq!(cfg.base_url, ProviderKind::Anthropic.default_base_url());
}

#[test]
fn explicit_flags_override_reused_settings() {
    let json_text = r#"{"model": "opencode-go/glm-5.3-flash"}"#;
    let auth_text = r#"{"opencode-go": {"type": "api", "key": "sk-test-go"}}"#;
    let dir = tempdir("override");
    let config_path = dir.0.join("opencode.json");
    let auth_path = dir.0.join("auth.json");
    fs::write(&config_path, json_text).expect("write");
    fs::write(&auth_path, auth_text).expect("write");
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var("TINYCODE_OPENCODE_CONFIG", &config_path);
    std::env::set_var("TINYCODE_OPENCODE_AUTH", &auth_path);
    std::env::remove_var("TINYCODE_MODEL");
    std::env::remove_var("ANTHROPIC_API_KEY");
    std::env::remove_var("OPENAI_API_KEY");
    let cfg = config::parse_args(&[
        "run".to_string(),
        "--model".to_string(),
        "kimi-k3".to_string(),
        "--base-url".to_string(),
        "http://127.0.0.1:9".to_string(),
        "demo".to_string(),
    ])
    .expect("config");
    std::env::remove_var("TINYCODE_OPENCODE_CONFIG");
    std::env::remove_var("TINYCODE_OPENCODE_AUTH");
    assert_eq!(cfg.model, "kimi-k3");
    assert_eq!(cfg.base_url, "http://127.0.0.1:9");
    assert_eq!(cfg.provider_id.as_deref(), Some("opencode-go"), "id labels usage records even when overridden");
}

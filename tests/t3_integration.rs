//! Integration tests for T3 Code / OpenCode-style model selection.
//!
//! T3 Code's model selector can provide qualified model IDs like
//! `opencode-go/glm-5.2`. tinycode must resolve these to the correct
//! provider, gateway endpoint, provider id, and bare model name without
//! requiring extra flags.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use tinycode::acp;
use tinycode::config::{self, ProviderKind, OPENCODE_GO_BASE_URL};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tempdir(tag: &str) -> TempDir {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("tinycode-t3-int-{tag}-{nanos}"));
    fs::create_dir_all(&path).expect("mkdir");
    TempDir(path)
}

fn with_isolated_env<F, R>(f: F) -> R
where
    F: FnOnce(&PathBuf) -> R,
{
    let dir = tempdir("env");
    let config_path = dir.0.join("opencode.json");
    let auth_path = dir.0.join("auth.json");
    fs::write(&config_path, "{}").expect("write config");
    fs::write(&auth_path, "{}").expect("write auth");
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let saved: Vec<(String, Option<String>)> = [
        "TINYCODE_OPENCODE_CONFIG",
        "TINYCODE_OPENCODE_AUTH",
        "TINYCODE_API_KEY",
        "OPENCODE_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "TINYCODE_PROVIDER",
        "TINYCODE_MODEL",
        "TINYCODE_BASE_URL",
        "TINYCODE_SYSTEM_PROMPT",
    ]
    .into_iter()
    .map(|k| (k.to_string(), std::env::var(k).ok()))
    .collect();
    std::env::set_var("TINYCODE_OPENCODE_CONFIG", &config_path);
    std::env::set_var("TINYCODE_OPENCODE_AUTH", &auth_path);
    for (k, _) in &saved {
        if k != "TINYCODE_OPENCODE_CONFIG" && k != "TINYCODE_OPENCODE_AUTH" {
            std::env::remove_var(k);
        }
    }
    let result = f(&dir.0);
    for (k, v) in saved {
        match v {
            Some(val) => std::env::set_var(k, val),
            None => std::env::remove_var(k),
        }
    }
    result
}

#[test]
fn cli_resolves_opencode_go_model_selector() {
    with_isolated_env(|dir| {
        let root = fs::canonicalize(dir).expect("canonicalize");
        let cfg = config::parse_args(&[
            "run".into(),
            "--root".into(),
            root.display().to_string(),
            "--model".into(),
            "opencode-go/glm-5.2".into(),
            "hello".into(),
        ])
        .expect("parse");
        assert_eq!(cfg.provider, ProviderKind::OpenAi);
        assert_eq!(cfg.provider_id.as_deref(), Some("opencode-go"));
        assert_eq!(cfg.base_url, OPENCODE_GO_BASE_URL);
        assert_eq!(cfg.model, "glm-5.2");
    });
}

#[test]
fn cli_resolves_opencode_go_from_env_model() {
    with_isolated_env(|dir| {
        std::env::set_var("TINYCODE_MODEL", "opencode-go/minimax-m3");
        let root = fs::canonicalize(dir).expect("canonicalize");
        let cfg = config::parse_args(&[
            "run".into(),
            "--root".into(),
            root.display().to_string(),
            "hello".into(),
        ])
        .expect("parse");
        assert_eq!(cfg.provider, ProviderKind::OpenAi);
        assert_eq!(cfg.provider_id.as_deref(), Some("opencode-go"));
        assert_eq!(cfg.base_url, OPENCODE_GO_BASE_URL);
        assert_eq!(cfg.model, "minimax-m3");
    });
}

#[test]
fn cli_explicit_base_url_overrides_resolved_gateway() {
    with_isolated_env(|dir| {
        let root = fs::canonicalize(dir).expect("canonicalize");
        let cfg = config::parse_args(&[
            "run".into(),
            "--root".into(),
            root.display().to_string(),
            "--model".into(),
            "opencode-go/glm-5.2".into(),
            "--base-url".into(),
            "https://proxy.example.test/v1".into(),
            "hello".into(),
        ])
        .expect("parse");
        assert_eq!(cfg.provider, ProviderKind::OpenAi);
        assert_eq!(cfg.model, "glm-5.2");
        assert_eq!(cfg.base_url, "https://proxy.example.test/v1");
    });
}

#[test]
fn cli_resolves_anthropic_and_openai_qualified_models() {
    with_isolated_env(|dir| {
        let root = fs::canonicalize(dir).expect("canonicalize");
        let cfg = config::parse_args(&[
            "run".into(),
            "--root".into(),
            root.display().to_string(),
            "--model".into(),
            "anthropic/claude-sonnet-4-5".into(),
            "hello".into(),
        ])
        .expect("parse");
        assert_eq!(cfg.provider, ProviderKind::Anthropic);
        assert_eq!(cfg.model, "claude-sonnet-4-5");
        assert_eq!(cfg.base_url, ProviderKind::Anthropic.default_base_url());

        let cfg = config::parse_args(&[
            "run".into(),
            "--root".into(),
            root.display().to_string(),
            "--model".into(),
            "openai/gpt-4.1-mini".into(),
            "hello".into(),
        ])
        .expect("parse");
        assert_eq!(cfg.provider, ProviderKind::OpenAi);
        assert_eq!(cfg.model, "gpt-4.1-mini");
        assert_eq!(cfg.base_url, ProviderKind::OpenAi.default_base_url());
    });
}

#[test]
fn acp_resolves_opencode_go_model_selector() {
    with_isolated_env(|dir| {
        std::env::set_var("TINYCODE_MODEL", "opencode-go/kimi-k3");
        std::env::set_var("OPENCODE_API_KEY", "test-key");
        let cfg = acp::build_config_from_env(dir.clone()).expect("acp config");
        assert_eq!(cfg.provider, ProviderKind::OpenAi);
        assert_eq!(cfg.provider_id.as_deref(), Some("opencode-go"));
        assert_eq!(cfg.base_url, OPENCODE_GO_BASE_URL);
        assert_eq!(cfg.model, "kimi-k3");
        assert_eq!(cfg.api_key, "test-key");
        assert!(cfg.auto, "ACP sessions run tools without interactive prompts");
    });
}

#[test]
fn resolve_model_spec_handles_known_provider_ids() {
    let (kind, id, base, model) = config::resolve_model_spec("opencode-go/glm-5.2").expect("spec");
    assert_eq!(kind, ProviderKind::OpenAi);
    assert_eq!(id, "opencode-go");
    assert_eq!(base.as_deref(), Some(OPENCODE_GO_BASE_URL));
    assert_eq!(model, "glm-5.2");

    assert!(config::resolve_model_spec("unknown-provider/model").is_none());
    assert!(config::resolve_model_spec("not-qualified").is_none());
}

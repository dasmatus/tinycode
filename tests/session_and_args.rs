//! Sessions (save/load roundtrip, provider guard) and argument parsing
//! edge cases. No test here may depend on ambient API keys, since tests
//! run in parallel threads.
use std::fs;
use std::path::PathBuf;
use tinycode::config::{self, USAGE};
use tinycode::store::{self, Session};
use tinycode::agent;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tempdir(tag: &str) -> TempDir {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("tinycode-store-{tag}-{nanos}"));
    fs::create_dir_all(&path).expect("mkdir");
    TempDir(path)
}

fn sample_session(root: &PathBuf) -> Session {
    Session {
        provider: "openai".into(),
        model: "test-model".into(),
        base_url: "http://127.0.0.1:1".into(),
        root: root.display().to_string(),
        turns: 2,
        messages: vec![
            serde_json::json!({"role": "user", "content": "first"}),
            serde_json::json!({"role": "assistant", "content": null, "tool_calls": []}),
        ],
    }
}

#[test]
fn session_save_load_roundtrip() {
    let dir = tempdir("save");
    let path = store::new_path(&dir.0).expect("new session path");
    let s = sample_session(&dir.0);
    store::save(&path, &s).expect("save");

    // Atomic saves must not leave tmp files behind.
    let tmp_leftovers: Vec<PathBuf> = fs::read_dir(&dir.0.join(".tinycode/sessions"))
        .expect("list.")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains(".tmp."))
        .collect();
    assert!(tmp_leftovers.is_empty(), "no tmp files left behind");

    let loaded = store::load(&path).expect("load");
    assert_eq!(loaded.provider, "openai");
    assert_eq!(loaded.turns, 2);
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[1]["role"], "assistant");
}

#[test]
fn agent_guards_session_provider_mismatch() {
    let dir = tempdir("mismatch");
    let root = fs::canonicalize(&dir.0).expect("canonicalize");
    let path = dir.0.join("tc-test.json");
    let provider_agnostic = sample_session(&root); // provider: openai
    store::save(&path, &provider_agnostic).expect("save");

    let mut cfg = config::Config {
        provider: tinycode::config::ProviderKind::Anthropic, // session says openai → mismatch
        api_key: "k".into(),
        model: "m".into(),
        base_url: "http://127.0.0.1:1".into(),
        root,
        auto: true,
        max_turns: 3,
        json: false,
        quiet: true,
        session_file: Some(path),
        task: "continue".into(),
    };
    let _ = &mut cfg;
    let agent = agent::Agent::new(cfg);
    let Err(err) = agent else {
        unreachable!("provider mismatch refuses the session; Agent::new returned Ok");
    };
    assert!(err.to_string().contains("provider"), "{err}");
}

#[test]
fn args_reject_unknown_options() {
    let args = vec!["--bogus".to_string()];
    let err = config::parse_args(&args).expect_err("unknown option");
    assert!(err.contains("--bogus"), "{err}");
}

#[test]
fn leading_run_subcommand_is_not_a_task() {
    let args = vec![
        "run".to_string(),
        "--provider".to_string(),
        "openai".to_string(),
        "write the readme".to_string(),
    ];
    let cfg = config::parse_args(&args).expect("run subcommand drops cleanly");
    assert_eq!(cfg.task, "write the readme");
}

#[test]
fn args_parse_flags_values_and_prompt_file() {
    let dir = tempdir("args");
    let root = fs::canonicalize(&dir.0).expect("canonicalize");
    let prompt_file = dir.0.join("task.txt");
    fs::write(&prompt_file, "do the thing\n").expect("prompt file");

    let args = vec![
        "run".to_string(),
        "--root".to_string(),
        root.display().to_string(),
        "--max-turns".to_string(),
        "7".to_string(),
        "--auto".to_string(),
        "--provider".to_string(),
        "openai".to_string(),
        "--task-file".to_string(),
        prompt_file.display().to_string(),
    ];
    let cfg = config::parse_args(&args).expect("parses");
    assert_eq!(cfg.task, "do the thing", "trailing newline trimmed");
    assert_eq!(cfg.max_turns, 7);
    assert!(cfg.auto);
    assert_eq!(cfg.provider.name(), "openai");
    assert_eq!(cfg.root, root);
}

#[test]
fn session_flag_requires_a_file_value() {
    let dir = tempdir("sessionflag");
    let root = fs::canonicalize(&dir.0).expect("canonicalize");
    let session = dir.0.join(".tinycode/sessions/tc-a.json");
    let args = vec![
        "--root".to_string(),
        root.display().to_string(),
        "--session".to_string(),
        session.display().to_string(),
        "continue".to_string(),
    ];
    let cfg = config::parse_args(&args).expect("parse");
    assert_eq!(cfg.task, "continue", "prompt not swallowed by --session");
    assert_eq!(cfg.session_file.expect("session path"), session);

    let args = vec!["--session".to_string()];
    let err = config::parse_args(&args).expect_err("session needs a value");
    assert!(err.contains("--session"), "{err}");
}

#[test]
fn usage_documents_the_t3_surface() {
    assert!(USAGE.contains("--json"));
    assert!(USAGE.contains("TINYCODE_API_KEY"));
}

// --- OpenCode config integration -----------------------------------------

use tinycode::config::load_opencode;

fn oc_paths(dir: &PathBuf, with_config: bool, with_auth: bool) -> (PathBuf, PathBuf) {
    let cfg_file = dir.join("opencode.json");
    let auth_file = dir.join("auth.json");
    if with_config {
        fs::write(&cfg_file, "").ok();
    }
    if with_auth {
        fs::write(&auth_file, "").ok();
    }
    (cfg_file, auth_file)
}

#[test]
fn auth_json_api_keys_are_picked_up() {
    let dir = tempdir("ocauth");
    let (cfg_path, auth_path) = oc_paths(&dir.0, false, true);
    std::fs::write(
        &auth_path,
        r#"{"anthropic": {"type": "api", "key": "sk-ant-test"}, "openai": {"type": "api", "key": "sk-oai-test"}}"#,
    )
    .expect("auth fixture");

    let env = load_opencode(&cfg_path, &auth_path);
    assert_eq!(env.provider, Some(tinycode::config::ProviderKind::Anthropic), "first listed provider wins");
    assert_eq!(env.api_key.as_deref(), Some("sk-ant-test"));
}

#[test]
fn opencode_default_model_and_provider_key_are_picked_up() {
    let dir = tempdir("occfg");
    let (cfg_path, auth_path) = oc_paths(&dir.0, true, true);
    std::fs::write(
        &cfg_path,
        r#"{"$schema": "x", "model": "openai/gpt-4.1-mini", "provider": {"openai": {"apiKey": "sk-cfgtest"}}}"#,
    )
    .expect("opencode fixture");

    let env = load_opencode(&cfg_path, &auth_path);
    assert_eq!(
        env.model,
        Some((tinycode::config::ProviderKind::OpenAi, "gpt-4.1-mini".to_string())),
        "provider/model split"
    );
    assert_eq!(env.provider, Some(tinycode::config::ProviderKind::OpenAi));
    assert_eq!(env.api_key.as_deref(), Some("sk-cfgtest"));
}

#[test]
fn missing_or_garbage_opencode_files_are_ignored() {
    let dir = tempdir("ocnone");
    let env = load_opencode(&dir.0.join("nope.json"), &dir.0.join("nopeak.json"));
    assert_eq!(env.provider, None);
    assert_eq!(env.api_key, None);
    assert_eq!(env.model, None);

    fs::write(dir.0.join("broken.json"), "not json {").expect("garbage");
    let env = load_opencode(&dir.0.join("broken.json"), &dir.0.join("also-gone.json"));
    assert_eq!(env.provider, None);
    assert_eq!(env.model, None);
    assert_eq!(env.api_key, None);
}

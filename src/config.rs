use std::io::Read;
use std::path::{Path, PathBuf};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
}

impl ProviderKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Some(Self::Anthropic),
            "openai" | "openai-compat" | "openai-compatible" => Some(Self::OpenAi),
            _ => None,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Anthropic => "claude-sonnet-4-5",
            Self::OpenAi => "gpt-4.1-mini",
        }
    }

    /// Base URL without the versioned endpoint path; `provider::chat_url` appends it.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com/v1",
        }
    }
}

/// Read OpenCode's own configuration so tinycode is set up by the same
/// files — `~/.config/opencode/opencode.json` for the default model (and
/// optional per-provider keys) and `~/.local/share/opencode/auth.json` for
/// API keys. Env (`TINYCODE_*`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`)
/// still wins; these fill the gaps so nothing has to be configured twice.
#[derive(Debug, Default)]
pub struct OpenCodeEnv {
    pub provider: Option<ProviderKind>,
    pub api_key: Option<String>,
    /// OpenCode's default model, split as (its provider, bare model id).
    pub model: Option<(ProviderKind, String)>,
}

pub fn load_opencode(config_path: &Path, auth_path: &Path) -> OpenCodeEnv {
    let mut env = OpenCodeEnv::default();

    // auth.json: {"anthropic": {"type": "api", "key": "sk-…"}, "openai": {…}}
    if let Ok(text) = std::fs::read_to_string(auth_path) {
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) {
            for name in ["anthropic", "openai"] {
                let Some(entry) = map.get(name) else { continue };
                let key = entry.get("key").and_then(Value::as_str).filter(|k| !k.is_empty());
                if let Some(key) = key {
                    if env.api_key.is_none() {
                        env.provider =
                            Some(if name == "anthropic" { ProviderKind::Anthropic } else { ProviderKind::OpenAi });
                        env.api_key = Some(key.to_string());
                    }
                }
            }
        }
    }

    // opencode.json: {"model": "anthropic/claude-…", "provider": {"anthropic": {"apiKey": …}}}
    if let Ok(text) = std::fs::read_to_string(config_path) {
        if let Ok(cfg) = serde_json::from_str::<Value>(&text) {
            if env.model.is_none() {
                env.model = cfg
                    .get("model")
                    .and_then(Value::as_str)
                    .and_then(split_opencode_model);
            }
            if env.api_key.is_none() {
                if let Some(providers) = cfg.get("provider").and_then(Value::as_object) {
                    for name in ["anthropic", "openai"] {
                        let key = providers
                            .get(name)
                            .and_then(|p| p.get("apiKey"))
                            .and_then(Value::as_str)
                            .filter(|k| !k.is_empty());
                        if let Some(key) = key {
                            env.provider =
                                Some(if name == "anthropic" { ProviderKind::Anthropic } else { ProviderKind::OpenAi });
                            env.api_key = Some(key.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    env
}

fn split_opencode_model(raw: &str) -> Option<(ProviderKind, String)> {
    let (provider, model) = raw.trim().split_once('/')?;
    let model = model.trim();
    if model.is_empty() {
        None
    } else {
        Some((ProviderKind::parse(provider)?, model.to_string()))
    }
}

/// XDG-aware default locations of OpenCode's configuration.
pub fn opencode_paths() -> (PathBuf, PathBuf) {
    let dir = |env: &str, fallback: &str| {
        std::env::var(env)
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(fallback))
                    .unwrap_or_default()
            })
    };
    (
        dir("XDG_CONFIG_HOME", ".config").join("opencode").join("opencode.json"),
        dir("XDG_DATA_HOME", ".local/share").join("opencode").join("auth.json"),
    )
}

#[derive(Debug)]
pub struct Config {
    pub provider: ProviderKind,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    /// Workspace root, canonicalized; every tool path must stay inside it.
    pub root: PathBuf,
    /// When false, mutating tools (write/edit/run) are refused.
    pub auto: bool,
    pub max_turns: u32,
    pub json: bool,
    pub quiet: bool,
    pub session_file: Option<PathBuf>,
    pub task: String,
}

pub const USAGE: &str = "\
tinycode — a minimal coding agent, meant to be driven from T3 Code.

USAGE:
    tinycode run [OPTIONS] \"<task prompt>\"

OPTIONS:
    -f, --task-file <PATH>   read the task prompt from a file (\"-\" for stdin)
        --root <DIR>         workspace root (default: current directory)
        --model <NAME>       model name (default: TINYCODE_MODEL or provider default)
        --provider <NAME>    anthropic | openai  (default: detected from env)
        --base-url <URL>     API base URL without the versioned endpoint path
        --auto               allow mutating tools (write/edit/run)
        --max-turns <N>      model turns per run (default 24)
    -s, --session <FILE>     continue the session file shown in the last run,
                             or listed in .tinycode/sessions/
        --json               NDJSON events on stdout instead of human output
        --quiet              suppress stderr narration

ENV:
    TINYCODE_API_KEY   |  ANTHROPIC_API_KEY  |  OPENAI_API_KEY
    TINYCODE_MODEL, TINYCODE_PROVIDER, TINYCODE_BASE_URL

OPCODE-STYLE CONFIG (defaults existing per provider are picked up automatically):
    ~/.local/share/opencode/auth.json     — API keys
    ~/.config/opencode/opencode.json      — \"model\": \"provider/model-id\"
    override single paths with TINYCODE_OPENCODE_CONFIG / TINYCODE_OPENCODE_AUTH
";

pub fn parse_args(args: &[String]) -> Result<Config, String> {
    // `tinycode run <prompt>`: the optional subcommand is spelled for
    // humans and shell completions; the parser drops it.
    let args = if args.first().map(|a| a == "run").unwrap_or(false) {
        &args[1..]
    } else {
        args
    };

    let mut provider: Option<ProviderKind> = None;
    let mut model: Option<String> = None;
    let mut base_url: Option<String> = None;
    let mut root: Option<PathBuf> = None;
    let mut auto = false;
    let mut max_turns: u32 = 24;
    let mut json_mode = false;
    let mut quiet = false;
    let mut session_file: Option<PathBuf> = None;
    let mut task_file: Option<PathBuf> = None;
    let mut task: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--auto" => auto = true,
            "--json" => json_mode = true,
            "--quiet" => quiet = true,
            "--model" | "--base-url" | "--provider" | "--root" | "--max-turns" | "-f"
            | "--task-file" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("missing value for {arg}"))?;
                i += 1;
                match arg {
                    "--model" => model = Some(value.clone()),
                    "--base-url" => base_url = Some(value.clone()),
                    "--provider" => {
                        provider = Some(
                            ProviderKind::parse(value)
                                .ok_or_else(|| format!("unknown provider '{value}' (anthropic|openai)"))?,
                        )
                    }
                    "--root" => root = Some(PathBuf::from(value)),
                    "--max-turns" => {
                        max_turns = value
                            .parse()
                            .map_err(|_| format!("--max-turns: '{value}' is not a number"))?
                    }
                    _ => task_file = Some(PathBuf::from(value)),
                }
            }
            "-s" | "--session" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing value for --session (pass the session file path shown in the last run)".to_string())?;
                i += 1;
                session_file = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            _ => {
                if task.is_none() {
                    task = Some(arg.to_string());
                } else {
                    return Err("only one task prompt may be given".into());
                }
            }
        }
        i += 1;
    }

    let task = match (task_file, task) {
        (Some(tf), _) => {
            let mut text = String::new();
            if tf == PathBuf::from("-") {
                std::io::stdin()
                    .read_to_string(&mut text)
                    .map_err(|e| format!("stdin: {e}"))?;
            } else {
                text = std::fs::read_to_string(&tf)
                    .map_err(|e| format!("--task-file {}: {e}", tf.display()))?;
            }
            let text = text.trim_end().to_string();
            if text.is_empty() {
                return Err("task prompt is empty".into());
            }
            text
        }
        (None, Some(t)) => t,
        (None, None) => return Err("no task given (pass a prompt or --task-file)".into()),
    };

    // Provider detection: flag > TINYCODE_PROVIDER > which API key exists
    // > OpenCode's own config (auth.json / opencode.json).
    let (oc_config_path, oc_auth_path) = opencode_paths();
    let (oc_config_path, oc_auth_path) = (
        std::env::var("TINYCODE_OPENCODE_CONFIG").map(PathBuf::from).unwrap_or(oc_config_path),
        std::env::var("TINYCODE_OPENCODE_AUTH").map(PathBuf::from).unwrap_or(oc_auth_path),
    );
    let oc = load_opencode(&oc_config_path, &oc_auth_path);
    let env_provider = std::env::var("TINYCODE_PROVIDER")
        .ok()
        .and_then(|p| ProviderKind::parse(&p))
        .or_else(|| {
            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                Some(ProviderKind::Anthropic)
            } else if std::env::var("OPENAI_API_KEY").is_ok() {
                Some(ProviderKind::OpenAi)
            } else {
                None
            }
        })
        .or(oc.provider);
    let provider = provider.or(env_provider).unwrap_or(ProviderKind::Anthropic);

    let env_key = std::env::var("TINYCODE_API_KEY").ok();
    let key_from_env = match (env_key, provider) {
        (Some(k), _) => Some(k),
        (None, ProviderKind::Anthropic) => std::env::var("ANTHROPIC_API_KEY").ok(),
        (None, ProviderKind::OpenAi) => std::env::var("OPENAI_API_KEY").ok(),
    };
    // OpenCode's key applies when it belongs to the provider we resolved to.
    let oc_key = oc.api_key.filter(|_| oc.provider == Some(provider));
    let api_key = key_from_env.or(oc_key).unwrap_or_default();

    let base_url = base_url.or_else(|| std::env::var("TINYCODE_BASE_URL").ok());

    let model = model
        .or_else(|| std::env::var("TINYCODE_MODEL").ok())
        // OpenCode's default model applies only when its provider matches
        // the provider this run actually resolves to.
        .or_else(|| {
            oc.model
                .filter(|(kind, _)| *kind == provider)
                .map(|(_, id)| id)
        })
        .unwrap_or_else(|| provider.default_model().to_string());
    let base_url = base_url.unwrap_or_else(|| provider.default_base_url().to_string());

    let root = root.unwrap_or_else(|| PathBuf::from("."));
    let root = std::fs::canonicalize(&root).map_err(|e| format!("--root {}: {e}", root.display()))?;

    Ok(Config {
        provider,
        api_key,
        model,
        base_url,
        root,
        auto,
        max_turns: max_turns.clamp(1, 1000),
        json: json_mode,
        quiet,
        session_file,
        task,
    })
}

/// Session files live under the workspace root by default; `--root` decides.
pub fn session_dir(root: &Path) -> PathBuf {
    root.join(".tinycode").join("sessions")
}

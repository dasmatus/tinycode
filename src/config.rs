use std::io::Read;
use std::path::{Path, PathBuf};

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

    // Provider detection: flag > TINYCODE_PROVIDER > which API key exists.
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
        });
    let provider = provider.or(env_provider).unwrap_or(ProviderKind::Anthropic);

    let env_key = std::env::var("TINYCODE_API_KEY").ok();
    let api_key = match (env_key, provider) {
        (Some(k), _) => k,
        (None, ProviderKind::Anthropic) => std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
        (None, ProviderKind::OpenAi) => std::env::var("OPENAI_API_KEY").unwrap_or_default(),
    };
    let base_url = base_url.or_else(|| std::env::var("TINYCODE_BASE_URL").ok());

    let model = model
        .or_else(|| std::env::var("TINYCODE_MODEL").ok())
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

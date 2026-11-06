use crate::config::Config;
use crate::provider::{self, ResultMsg};
use crate::store::{self, Session};
use crate::tools::{self, ToolCtx};
use crate::usage;
use serde_json::{json, Value};
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

pub enum Event {
    Started { session: String },
    Turn(u32),
    Assistant(String),
    ToolUse { name: String, input: String },
    ToolResult { name: String, ok: bool, bytes: usize },
    Done { text: String, turns: u32, session: String },
    MaxTurns { session: String },
    Error(String),
}

impl Event {
    pub fn to_json(&self) -> Value {
        match self {
            Event::Started { session } => json!({"type": "start", "session": session}),
            Event::Turn(n) => json!({"type": "turn", "n": n}),
            Event::Assistant(t) => json!({"type": "text", "text": t}),
            Event::ToolUse { name, input } => json!({"type": "tool", "name": name, "input": input}),
            Event::ToolResult { name, ok, bytes } => {
                json!({"type": "tool_result", "name": name, "ok": ok, "bytes": bytes})
            }
            Event::Done { text, turns, session } => {
                json!({"type": "done", "turns": turns, "text": text, "session": session})
            }
            Event::MaxTurns { session } => json!({"type": "max_turns", "session": session}),
            Event::Error(message) => json!({"type": "error", "message": message}),
        }
    }
}

/// When the serialized history exceeds this, tool results from older turns
/// (all but the last 8 messages) are shrunk to a stub. That keeps peak
/// request size bounded instead of letting long sessions grow without limit.
const COMPACT_THRESHOLD: usize = 280_000;
const TOOL_RESULT_KEEP: usize = 400;

pub struct Agent {
    cfg: Config,
    tool_ctx: ToolCtx,
    system: String,
    pub messages: Vec<Value>,
    session_path: PathBuf,
    turns: u32,
}

impl Agent {
    pub fn new(cfg: Config) -> Result<Self, Box<dyn Error>> {
        let session_path = match &cfg.session_file {
            Some(p) => p.clone(),
            None => store::new_path(&cfg.root).map_err(|e| -> Box<dyn Error> { e.into() })?,
        };
        let messages = if cfg.session_file.is_some() {
            let session =
                store::load(&session_path).map_err(|e| -> Box<dyn Error> { e.into() })?;
            if session.provider != cfg.provider.name() {
                return Err(format!(
                    "session was written with provider '{}', run with --provider {} (or --base-url/--model)",
                    session.provider, session.provider
                )
                .into());
            }
            session.messages
        } else {
            Vec::new()
        };
        let system = system_prompt(&cfg);
        Ok(Self {
            tool_ctx: ToolCtx { root: cfg.root.clone(), auto: cfg.auto },
            cfg,
            system,
            messages,
            session_path,
            turns: 0,
        })
    }

    pub fn session_path(&self) -> &PathBuf {
        &self.session_path
    }

    /// Short per-run tag sent to session-routing gateways and kept in usage
    /// records: the session file's file name (unique per run, no paths).
    fn session_tag(&self) -> String {
        self.session_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("tinycode")
            .to_string()
    }

    /// Append one usage record for this provider turn. Recording never
    /// fails the run: a logging error is only reported as a warning.
    fn record_turn(&self, started: Instant, ok: bool, error: Option<String>, u: usage::Usage) {
        let rec = usage::TurnRecord {
            provider: self.provider_label(),
            kind: self.cfg.provider.name().to_string(),
            model: self.cfg.model.clone(),
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            turn: self.turns,
            duration_ms: started.elapsed().as_millis() as u64,
            ok,
            error,
            session: self.session_tag(),
        };
        let path = self.cfg.usage_file.clone().unwrap_or_else(usage::file_path);
        if let Err(e) = usage::record_to(&path, &rec) {
            eprintln!("warn: usage recording failed: {e}");
        }
    }

    /// Provider identifier for usage records: OpenCode ids flow through when
    /// the settings reuse resolved one ("opencode-go"), else the wire kind.
    fn provider_label(&self) -> String {
        self.cfg.provider_id.clone().unwrap_or_else(|| self.cfg.provider.name().to_string())
    }

    /// Returns the process exit code: 0 done, 1 hard error, 2 max turns hit.
    pub fn run(&mut self, report: &mut dyn FnMut(&Event)) -> Result<i32, Box<dyn Error>> {
        report(&Event::Started { session: self.session_path.display().to_string() });
        let tools_spec = tools::definitions();

        // If this is a fresh session with no messages, add the task as the first user message
        if self.messages.is_empty() && !self.cfg.task.is_empty() {
            self.messages.push(json!({"role": "user", "content": self.cfg.task.clone()}));
        }

        loop {
            self.turns += 1;
            if self.turns > self.cfg.max_turns {
                report(&Event::MaxTurns { session: self.session_path.display().to_string() });
                return Ok(2);
            }
            report(&Event::Turn(self.turns));
            compact_history(&mut self.messages);

            let kind = self.cfg.provider;
            let started = Instant::now();
            let reply = match provider::stream(
                kind,
                &self.cfg.api_key,
                &self.cfg.model,
                &self.cfg.base_url,
                &self.system,
                &tools_spec,
                &self.messages,
                &self.session_tag(),
                None,
                |_delta: &str| {},
            ) {
                Ok(reply) => {
                    self.record_turn(started, true, None, reply.usage.clone());
                    reply
                }
                Err(e) => {
                    self.record_turn(started, false, Some(e.to_string()), usage::Usage::default());
                    return Err(e);
                }
            };
            self.messages.push(provider::assistant_message(kind, &reply));
            if !reply.text.is_empty() {
                report(&Event::Assistant(reply.text.clone()));
            }

            if reply.calls.is_empty() {
                self.save()?;
                report(&Event::Done {
                    text: reply.text,
                    turns: self.turns,
                    session: self.session_path.display().to_string(),
                });
                return Ok(0);
            }

            let mut results = Vec::new();
            for call in &reply.calls {
                report(&Event::ToolUse { name: call.name.clone(), input: call.arguments.clone() });
                let out = tools::dispatch(&self.tool_ctx, &call.name, &call.arguments);
                report(&Event::ToolResult {
                    name: call.name.clone(),
                    ok: out.ok,
                    bytes: out.output.len(),
                });
                results.push(ResultMsg { id: call.id.clone(), ok: out.ok, output: out.output });
            }
            self.messages.extend(provider::tool_results_messages(kind, &results));
            self.save()?;
        }
    }

    /// Sessions are written at the end of every turn, so a crash mid-turn
    /// means resume simply replays that turn; nothing is torn.
    fn save(&self) -> Result<(), Box<dyn Error>> {
        let session = Session {
            provider: self.cfg.provider.name().to_string(),
            model: self.cfg.model.clone(),
            base_url: self.cfg.base_url.clone(),
            root: self.cfg.root.display().to_string(),
            turns: self.turns,
            messages: self.messages.clone(),
        };
        store::save(&self.session_path, &session).map_err(|e| -> Box<dyn Error> { e.into() })
    }
}

fn system_prompt(cfg: &Config) -> String {
    let root = cfg.root.display().to_string();
    if let Some(custom) = &cfg.system_prompt {
        return custom.replace("{root}", &root);
    }
    format!(
        "You are tinycode, a minimal coding agent. You work inside the workspace root {root}.\n\
         Tools may only touch paths inside that root. Rules:\n\
         - Inspect before editing: use read/ls/grep; never guess file contents.\n\
         - Edit precisely with edit; create files with write; check compile/test results with run.\n\
         - Keep 'run' output small (filters, pipes); timestamps will be capped anyway.\n\
         - Mutating tools are only permitted when the operator passed --auto; if refused, say so.\n\
         - Do not stop until the task is fully complete and verified. Prefer using the run tool for git, tests, and any operations that T3 Code can track (worktrees, commits, pull requests via gh).\n\
         - Once the task is complete, reply with a short plain-text summary and no tool calls."
    )
}

fn compact_history(messages: &mut Vec<Value>) {
    let total: usize = messages.iter().map(|m| m.to_string().len()).sum();
    if total <= COMPACT_THRESHOLD {
        return;
    }
    let keep_recent = messages.len().saturating_sub(8);
    for message in &mut messages[..keep_recent] {
        shrink_tool_results(message);
    }
}

fn shrink_tool_results(message: &mut Value) {
    match message {
        // OpenAI: {"role": "tool", "content": "..."}
        // Anthropic: {"role": "user", "content": [{"type": "tool_result", "content": "..."}]}
        Value::Object(map) => {
            let is_tool = map
                .get("role")
                .and_then(Value::as_str)
                .map(|r| r == "tool")
                .unwrap_or(false);
            if is_tool {
                if let Some(Value::String(content)) = map.get_mut("content") {
                    truncate_string_val(content);
                }
            }
            for (_key, v) in map.iter_mut() {
                shrink_tool_results(v);
            }
        }
        Value::Array(items) => {
            for item in items {
                let shaped = item
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|t| t == "tool_result")
                    .unwrap_or(false);
                if shaped {
                    if let Some(Value::String(content)) = item.get_mut("content") {
                        truncate_string_val(content);
                    }
                }
            }
        }
        _ => {}
    }
}

fn truncate_string_val(content: &mut String) {
    if content.len() > TOOL_RESULT_KEEP {
        let mut keep = TOOL_RESULT_KEEP;
        while keep > 0 && !content.is_char_boundary(keep) {
            keep -= 1;
        }
        *content = format!("{} [...compacted away by tinycode...]", &content[..keep]);
    }
}

use crate::config::{self, Config};
use agent_client_protocol::{
    Agent, AgentCapabilities, AgentSideConnection, AuthenticateRequest, AuthenticateResponse,
    CancelNotification, Client, ContentBlock, Error, ExtNotification, ExtRequest, ExtResponse,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, McpCapabilities,
    NewSessionRequest, NewSessionResponse, PromptCapabilities, PromptRequest, PromptResponse,
    SessionId, SessionNotification, SessionUpdate, SetSessionModeRequest, SetSessionModeResponse,
    StopReason, TextContent, V1,
};
use miette::{miette, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

struct SessionState {
    agent: crate::agent::Agent,
    _config: Config,
}

#[derive(Clone)]
struct TinyAcpAgent {
    inner: Rc<Inner>,
}

struct Inner {
    sessions: RefCell<HashMap<Arc<str>, SessionState>>,
    conn: RefCell<Option<Rc<AgentSideConnection>>>,
}

impl TinyAcpAgent {
    fn new() -> Self {
        Self {
            inner: Rc::new(Inner {
                sessions: RefCell::new(HashMap::new()),
                conn: RefCell::new(None),
            }),
        }
    }

    fn set_conn(&self, conn: Rc<AgentSideConnection>) {
        *self.inner.conn.borrow_mut() = Some(conn);
    }

    fn conn(&self) -> std::result::Result<Rc<AgentSideConnection>, Error> {
        self.inner
            .conn
            .borrow()
            .clone()
            .ok_or_else(|| Error::internal_error().with_data("connection not ready"))
    }

    fn make_session_id(agent: &crate::agent::Agent) -> SessionId {
        let name = agent
            .session_path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("session")
            .to_string();
        SessionId(Arc::from(name))
    }
}

#[async_trait::async_trait(?Send)]
impl Agent for TinyAcpAgent {
    async fn initialize(
        &self,
        _args: InitializeRequest,
    ) -> std::result::Result<InitializeResponse, Error> {
        Ok(InitializeResponse {
            protocol_version: V1,
            agent_capabilities: AgentCapabilities {
                load_session: false,
                prompt_capabilities: PromptCapabilities {
                    image: false,
                    audio: false,
                    embedded_context: false,
                    meta: None,
                },
                mcp_capabilities: McpCapabilities {
                    http: false,
                    sse: false,
                    meta: None,
                },
                meta: None,
            },
            auth_methods: Vec::new(),
            meta: None,
        })
    }

    async fn authenticate(
        &self,
        _args: AuthenticateRequest,
    ) -> std::result::Result<AuthenticateResponse, Error> {
        Ok(AuthenticateResponse::default())
    }

    async fn new_session(
        &self,
        args: NewSessionRequest,
    ) -> std::result::Result<NewSessionResponse, Error> {
        let cwd = if args.cwd.as_os_str().is_empty() {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            args.cwd
        };
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        let cfg = build_config_from_env(cwd).map_err(|e| {
            Error::internal_error().with_data(format!("failed to build config: {e}"))
        })?;
        let agent = crate::agent::Agent::new(cfg.clone()).map_err(|e| {
            Error::internal_error().with_data(format!("failed to create agent: {e}"))
        })?;
        let session_id = Self::make_session_id(&agent);
        self.inner.sessions.borrow_mut().insert(
            session_id.0.clone(),
            SessionState {
                agent,
                _config: cfg,
            },
        );
        Ok(NewSessionResponse {
            session_id,
            modes: None,
            meta: None,
        })
    }

    async fn prompt(
        &self,
        args: PromptRequest,
    ) -> std::result::Result<PromptResponse, Error> {
        let session_id = args.session_id.0.clone();
        let task = prompt_to_text(&args.prompt);
        if task.trim().is_empty() {
            return Ok(PromptResponse {
                stop_reason: StopReason::EndTurn,
                meta: None,
            });
        }

        let conn = self.conn()?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEventMsg>();

        // Move the session agent out for blocking execution, then put it back.
        let mut state = {
            let mut sessions = self.inner.sessions.borrow_mut();
            sessions
                .remove(&session_id)
                .ok_or_else(|| Error::invalid_params().with_data("session not found"))?
        };

        let handle = tokio::task::spawn_blocking(move || {
            state.agent.messages.push(serde_json::json!({"role": "user", "content": task}));
            let mut stop_reason = StopReason::EndTurn;
            let code = state
                .agent
                .run(&mut |event: &crate::agent::Event| {
                    match event {
                        crate::agent::Event::Assistant(text) => {
                            let _ = tx.send(AgentEventMsg::Text(text.clone()));
                        }
                        crate::agent::Event::Done { text, .. } => {
                            let _ = tx.send(AgentEventMsg::DoneText(text.clone()));
                        }
                        crate::agent::Event::MaxTurns { .. } => {
                            stop_reason = StopReason::MaxTurnRequests;
                        }
                        crate::agent::Event::Error(msg) => {
                            let _ = tx.send(AgentEventMsg::Error(msg.clone()));
                        }
                        _ => {}
                    }
                });
            match code {
                Ok(2) => stop_reason = StopReason::MaxTurnRequests,
                Ok(_) => {}
                Err(e) => {
                    let _ = tx.send(AgentEventMsg::Error(e.to_string()));
                    stop_reason = StopReason::EndTurn;
                }
            }
            (state, stop_reason)
        });

        // Forward streamed text to the client as session updates.
        let notify_session = SessionId(session_id.clone());
        while let Some(msg) = rx.recv().await {
            let text = match msg {
                AgentEventMsg::Text(t) | AgentEventMsg::DoneText(t) => t,
                AgentEventMsg::Error(e) => format!("[error] {e}"),
            };
            if !text.is_empty() {
                let _ = conn
                    .session_notification(SessionNotification {
                        session_id: notify_session.clone(),
                        update: SessionUpdate::AgentMessageChunk {
                            content: ContentBlock::Text(TextContent {
                                annotations: None,
                                text,
                                meta: None,
                            }),
                        },
                        meta: None,
                    })
                    .await;
            }
        }

        let (state, stop_reason) = handle
            .await
            .map_err(|e| Error::internal_error().with_data(format!("agent task failed: {e}")))?;
        self.inner.sessions.borrow_mut().insert(session_id, state);

        Ok(PromptResponse {
            stop_reason,
            meta: None,
        })
    }

    async fn cancel(&self, _args: CancelNotification) -> std::result::Result<(), Error> {
        // Cooperative cancellation is not yet wired into the agent loop.
        Ok(())
    }

    async fn load_session(
        &self,
        _args: LoadSessionRequest,
    ) -> std::result::Result<LoadSessionResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn set_session_mode(
        &self,
        _args: SetSessionModeRequest,
    ) -> std::result::Result<SetSessionModeResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn ext_method(
        &self,
        _args: ExtRequest,
    ) -> std::result::Result<ExtResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn ext_notification(
        &self,
        _args: ExtNotification,
    ) -> std::result::Result<(), Error> {
        Ok(())
    }
}

enum AgentEventMsg {
    Text(String),
    DoneText(String),
    Error(String),
}

fn prompt_to_text(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text(t) => {
                out.push_str(&t.text);
                out.push('\n');
            }
            ContentBlock::ResourceLink(r) => {
                out.push_str(&format!("[resource: {}]", r.uri));
                out.push('\n');
            }
            ContentBlock::Resource(r) => {
                // Best-effort: embedded resources may carry text; keep a marker.
                out.push_str(&format!("[embedded resource: {:?}]", r.resource));
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

fn build_config_from_env(root: PathBuf) -> std::result::Result<Config, Box<dyn std::error::Error>> {
    let (oc_config_path, oc_auth_path) = config::opencode_paths();
    let oc = config::load_opencode(&oc_config_path, &oc_auth_path);

    let provider = if let Ok(p) = std::env::var("TINYCODE_PROVIDER") {
        config::ProviderKind::parse(&p).unwrap_or(config::ProviderKind::Anthropic)
    } else if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        config::ProviderKind::Anthropic
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        config::ProviderKind::OpenAi
    } else if let Some(p) = oc.provider {
        p
    } else {
        config::ProviderKind::Anthropic
    };

    let api_key = std::env::var("TINYCODE_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .or_else(|_| {
            if oc.provider == Some(provider) {
                oc.api_key.clone().ok_or(std::env::VarError::NotPresent)
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .unwrap_or_default();

    let model = std::env::var("TINYCODE_MODEL")
        .or_else(|_| {
            if let Some((k, m)) = &oc.model {
                if *k == provider {
                    Ok(m.clone())
                } else {
                    Err(std::env::VarError::NotPresent)
                }
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .unwrap_or_else(|_| provider.default_model().to_string());

    let base_url = std::env::var("TINYCODE_BASE_URL")
        .or_else(|_| {
            if oc.provider == Some(provider) {
                oc.base_url.clone().ok_or(std::env::VarError::NotPresent)
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .unwrap_or_else(|_| provider.default_base_url().to_string());

    let system_prompt = std::env::var("TINYCODE_SYSTEM_PROMPT").ok().filter(|s| !s.is_empty());

    Ok(Config {
        provider,
        api_key,
        model,
        base_url,
        provider_id: if oc.provider == Some(provider) {
            oc.provider_id.clone()
        } else {
            None
        },
        usage_file: None,
        root,
        auto: true,
        max_turns: 24,
        json: false,
        quiet: true,
        session_file: None,
        task: String::new(),
        system_prompt,
    })
}

/// Run the ACP (Agent Client Protocol) server over stdio using the
/// `agent-client-protocol` library.
pub async fn run_acp() -> Result<()> {
    let agent = TinyAcpAgent::new();
    let stdin = tokio::io::stdin().compat();
    let stdout = tokio::io::stdout().compat_write();

    let local = LocalSet::new();
    local
        .run_until(async move {
            let (conn, io_task) = AgentSideConnection::new(
                agent.clone(),
                stdout,
                stdin,
                |fut| {
                    tokio::task::spawn_local(fut);
                },
            );
            let conn = Rc::new(conn);
            agent.set_conn(conn);
            io_task.await.map_err(|e| miette!("ACP I/O error: {e}"))
        })
        .await
}

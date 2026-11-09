use agent_client_protocol::{
    Agent, Client, ClientCapabilities, ClientSideConnection, ContentBlock, CreateTerminalRequest,
    CreateTerminalResponse, Error, ExtNotification, ExtRequest, ExtResponse, InitializeRequest,
    KillTerminalCommandRequest, KillTerminalCommandResponse, NewSessionRequest, PromptRequest,
    ReadTextFileRequest, ReadTextFileResponse, ReleaseTerminalRequest, ReleaseTerminalResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SessionNotification, SessionUpdate, TerminalOutputRequest, TerminalOutputResponse, TextContent,
    WaitForTerminalExitRequest, WaitForTerminalExitResponse, WriteTextFileRequest,
    WriteTextFileResponse, V1,
};
use miette::{miette, IntoDiagnostic, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::process::Command;
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// A minimal ACP client handler that prints session updates and auto-approves
/// permission requests. Used to test `tinycode acp` server functionality.
#[derive(Clone, Default)]
struct TestClient {
    got_update: Arc<AtomicBool>,
}

#[async_trait::async_trait(?Send)]
impl Client for TestClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> std::result::Result<RequestPermissionResponse, Error> {
        // Auto-approve: pick the first option if any, otherwise cancel.
        let outcome = args
            .options
            .first()
            .map(|opt| RequestPermissionOutcome::Selected {
                option_id: opt.id.clone(),
            })
            .unwrap_or(RequestPermissionOutcome::Cancelled);
        Ok(RequestPermissionResponse {
            outcome,
            meta: None,
        })
    }

    async fn session_notification(
        &self,
        args: SessionNotification,
    ) -> std::result::Result<(), Error> {
        self.got_update.store(true, Ordering::Relaxed);
        match args.update {
            SessionUpdate::AgentMessageChunk { content } => {
                if let ContentBlock::Text(TextContent { text, .. }) = content {
                    print!("{text}");
                }
            }
            other => {
                // Keep the test client simple; surface any other update kinds for
                // visibility without depending on every SessionUpdate variant name.
                eprintln!("\n[session update] {other:?}");
            }
        }
        Ok(())
    }

    async fn write_text_file(
        &self,
        _args: WriteTextFileRequest,
    ) -> std::result::Result<WriteTextFileResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn read_text_file(
        &self,
        _args: ReadTextFileRequest,
    ) -> std::result::Result<ReadTextFileResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn create_terminal(
        &self,
        _args: CreateTerminalRequest,
    ) -> std::result::Result<CreateTerminalResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn terminal_output(
        &self,
        _args: TerminalOutputRequest,
    ) -> std::result::Result<TerminalOutputResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn release_terminal(
        &self,
        _args: ReleaseTerminalRequest,
    ) -> std::result::Result<ReleaseTerminalResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn wait_for_terminal_exit(
        &self,
        _args: WaitForTerminalExitRequest,
    ) -> std::result::Result<WaitForTerminalExitResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn kill_terminal_command(
        &self,
        _args: KillTerminalCommandRequest,
    ) -> std::result::Result<KillTerminalCommandResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn ext_method(&self, _args: ExtRequest) -> std::result::Result<ExtResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn ext_notification(&self, _args: ExtNotification) -> std::result::Result<(), Error> {
        Ok(())
    }
}

pub struct AcpClientConfig {
    pub server_cmd: PathBuf,
    pub server_args: Vec<String>,
    pub cwd: PathBuf,
    pub prompt: Option<String>,
    pub handshake_only: bool,
}

/// Run a simple ACP client against a `tinycode acp` server process.
///
/// This spawns the server, performs the ACP initialize/new-session handshake,
/// optionally sends a prompt, and prints streamed updates. It is intended as a
/// simple functional test of the ACP server implementation.
pub async fn run_acp_client(cfg: AcpClientConfig) -> Result<()> {
    let mut child = Command::new(&cfg.server_cmd)
        .args(&cfg.server_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .into_diagnostic()
        .map_err(|e| miette!("failed to spawn ACP server {:?}: {e}", cfg.server_cmd))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| miette!("server stdin not captured"))?
        .compat_write();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| miette!("server stdout not captured"))?
        .compat();

    let client = TestClient::default();
    let local = LocalSet::new();
    let result = local
        .run_until(async move {
            let (conn, io_task) = ClientSideConnection::new(
                client.clone(),
                stdin,
                stdout,
                |fut| {
                    tokio::task::spawn_local(fut);
                },
            );

            // Drive I/O in the background.
            let io = tokio::task::spawn_local(async move {
                if let Err(e) = io_task.await {
                    eprintln!("[acp io] {e}");
                }
            });

            // 1) Initialize
            eprintln!("→ initialize");
            let init = conn
                .initialize(InitializeRequest {
                    protocol_version: V1,
                    client_capabilities: ClientCapabilities::default(),
                    meta: None,
                })
                .await
                .map_err(|e| miette!("initialize failed: {e}"))?;
            eprintln!(
                "← initialized (protocol {:?}, agent caps: {:?})",
                init.protocol_version, init.agent_capabilities
            );

            // 2) New session
            let cwd = std::fs::canonicalize(&cfg.cwd).unwrap_or(cfg.cwd.clone());
            eprintln!("→ new_session (cwd: {})", cwd.display());
            let session = conn
                .new_session(NewSessionRequest {
                    cwd,
                    mcp_servers: Vec::new(),
                    meta: None,
                })
                .await
                .map_err(|e| miette!("new_session failed: {e}"))?;
            eprintln!("← session created: {}", session.session_id.0);

            if cfg.handshake_only {
                eprintln!("✓ ACP handshake OK (handshake-only mode)");
            } else if let Some(prompt) = cfg.prompt {
                // 3) Prompt
                eprintln!("→ prompt: {}", prompt);
                let resp = conn
                    .prompt(PromptRequest {
                        session_id: session.session_id.clone(),
                        prompt: vec![ContentBlock::Text(TextContent {
                            annotations: None,
                            text: prompt,
                            meta: None,
                        })],
                        meta: None,
                    })
                    .await
                    .map_err(|e| miette!("prompt failed: {e}"))?;
                println!();
                eprintln!("← prompt complete (stop reason: {:?})", resp.stop_reason);
            } else {
                eprintln!("✓ ACP handshake OK (no prompt provided; pass --prompt or use --handshake-only)");
            }

            // Shut down: dropping the connection closes the transport; abort the
            // I/O driver so we don't block forever on a half-open stdio pipe.
            drop(conn);
            io.abort();
            let _ = io.await;
            Ok::<(), miette::Report>(())
        })
        .await;

    // Don't leave a zombie.
    let _ = child.start_kill();
    let _ = child.wait().await;

    result
}

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use tinycode::agent::Event;
use tinycode::cli::ServeArgs;
use tinycode::{agent, cli, config, srv, usage};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    dispatch(cli.command, cli.global).await
}

async fn dispatch(command: Option<cli::Commands>, global: cli::GlobalArgs) -> Result<()> {
    match command {
        Some(cli::Commands::Acp) => tinycode::acp::run_acp().await,
        Some(cli::Commands::AcpClient(args)) => {
            let server_cmd = match args.server {
                Some(p) => p,
                None => std::env::current_exe().into_diagnostic()?,
            };
            let cfg = tinycode::acp_client::AcpClientConfig {
                server_cmd,
                server_args: vec!["acp".to_string()],
                cwd: args.cwd,
                prompt: args.prompt,
                handshake_only: args.handshake_only,
            };
            tinycode::acp_client::run_acp_client(cfg).await
        }
        Some(cli::Commands::Usage(args)) => {
            let _ = args;
            let code = usage::cli(&[]);
            std::process::exit(code);
        }
        Some(cli::Commands::Run(run_args)) => {
            let cfg = build_config(run_args, global)?;
            run_agent(cfg)?;
            Ok(())
        }
        Some(cli::Commands::Serve(args)) => serve(args).await,
        None => Err(miette::miette!("no task given; see --help")),
    }
}

async fn serve(args: ServeArgs) -> Result<()> {
    let config = srv::ServerConfig {
        host: args.hostname,
        port: args.port,
        auto: args.auto,
        version: cli::VERSION.to_string(),
    };
    let handle = srv::start_server(config)?;
    if let Err(e) = handle.run().await {
        return Err(miette::miette!("server error: {e}"));
    }
    Ok(())
}

fn build_config(run_args: cli::RunArgs, global: cli::GlobalArgs) -> Result<config::Config> {
    // Consistent with clap, not a manual fallback.
    let mut args: Vec<String> = vec![];
    if let Some(t) = run_args.task {
        args.push(t);
    }
    if let Some(tf) = run_args.task_file {
        args.push("--task-file".into());
        args.push(tf.to_string_lossy().to_string());
    }
    args.push("--root".into());
    args.push(run_args.root.to_string_lossy().to_string());
    if let Some(m) = run_args.model {
        args.push("--model".into());
        args.push(m);
    }
    if let Some(p) = run_args.provider {
        args.push("--provider".into());
        args.push(p);
    }
    if let Some(b) = run_args.base_url {
        args.push("--base-url".into());
        args.push(b);
    }
    if run_args.auto {
        args.push("--auto".into());
    }
    args.push("--max-turns".into());
    args.push(run_args.max_turns.to_string());
    if let Some(s) = run_args.session {
        args.push("--session".into());
        args.push(s.to_string_lossy().to_string());
    }
    if run_args.json {
        args.push("--json".into());
    }
    if run_args.quiet {
        args.push("--quiet".into());
    }
    let mut cfg = config::parse_args(&args).map_err(|e| miette::miette!("{}", e))?;
    cfg.system_prompt = global.system_prompt.or_else(|| {
        global.system_prompt_file.and_then(|path| {
            std::fs::read_to_string(path)
                .ok()
                .map(|s| s.trim_end().to_string())
        })
    });
    Ok(cfg)
}

fn run_agent(cfg: config::Config) -> Result<()> {
    let json_mode = cfg.json;
    let quiet = cfg.quiet;

    let mut ag = agent::Agent::new(cfg).map_err(|e| miette::miette!("{}", e))?;

    let code = ag
        .run(&mut |event: &Event| {
            if json_mode {
                println!("{}", event.to_json());
                return;
            }
            match event {
                Event::Started { session } => {
                    if !quiet {
                        eprintln!("session: {session}");
                    }
                }
                Event::ToolUse { name, input } => {
                    eprintln!("➜ {name} {}", preview(input, 160));
                }
                Event::ToolResult { name, ok, .. } => {
                    if !ok {
                        eprintln!("  ✗ {name} failed (see session)");
                    }
                }
                Event::Done { text, turns, session } => {
                    println!("{text}");
                    if !quiet {
                        eprintln!();
                        eprintln!("done in {turns} turn(s) — session: {session}");
                    }
                }
                Event::MaxTurns { session } => {
                    eprintln!(
                        "hit --max-turns; resume: tinycode run \"<next instruction>\" --session {session}"
                    );
                }
                _ => {}
            }
        })
        .map_err(|e| miette::miette!("{}", e))?;

    std::process::exit(code);
}

fn preview(text: &str, cap: usize) -> String {
    let flat = text.lines().take(1).next().unwrap_or_default();
    let mut out: String = flat.chars().take(cap).collect();
    if flat.chars().count() > cap || text.lines().count() > 1 {
        out.push('…');
    }
    out
}

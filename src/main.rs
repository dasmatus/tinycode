use clap::Parser;
use miette::{IntoDiagnostic, Result};
use tinycode::agent::Event;
use tinycode::{agent, config, usage};
use tinycode::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Acp) => {
            return tinycode::acp::run_acp().await;
        }
        Some(Commands::AcpClient(args)) => {
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
            return tinycode::acp_client::run_acp_client(cfg).await;
        }
        Some(Commands::Usage(args)) => {
            let _ = args;
            let code = usage::cli(&[]); // usage might need adjustment
            std::process::exit(code);
        }
        Some(Commands::Run(run_args)) => {
            // Build config from args
            let cfg = build_config(run_args, cli.global)?;
            run_agent(cfg)?;
            return Ok(());
        }
        None => {
            // Try to be compatible with old style - if no subcommand, maybe check if it's acp mode
            // or just run as before
            // For backward compat, maybe parse as old style
        }
    }

    // Fallback to old behavior for backward compat
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "usage").unwrap_or(false) {
        std::process::exit(usage::cli(&args[1..]));
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", config::USAGE);
        std::process::exit(0);
    }
    if args.is_empty() {
        eprintln!("error: no task given\n\n{}", config::USAGE);
        std::process::exit(2);
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("tinycode {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let cfg = config::parse_args(&args).map_err(|e| miette::miette!("{}", e))?;

    run_agent(cfg)?;
    Ok(())
}

fn build_config(run_args: tinycode::cli::RunArgs, global: tinycode::cli::GlobalArgs) -> Result<config::Config> {
    // Build config similar to parse_args but using clap args
    let args = vec!["run".to_string()]; // dummy
    // Easier to adapt - use parse_args style logic
    let mut cfg_args: Vec<String> = vec!["run".to_string()];
    if let Some(ref task) = run_args.task {
        cfg_args.push(task.clone());
    }
    if let Some(ref tf) = run_args.task_file {
        cfg_args.push("--task-file".to_string());
        cfg_args.push(tf.to_string_lossy().to_string());
    }
    cfg_args.push("--root".to_string());
    cfg_args.push(run_args.root.to_string_lossy().to_string());
    if let Some(ref model) = run_args.model {
        cfg_args.push("--model".to_string());
        cfg_args.push(model.clone());
    }
    if let Some(ref provider) = run_args.provider {
        cfg_args.push("--provider".to_string());
        cfg_args.push(provider.clone());
    }
    if let Some(ref base_url) = run_args.base_url {
        cfg_args.push("--base-url".to_string());
        cfg_args.push(base_url.clone());
    }
    if run_args.auto {
        cfg_args.push("--auto".to_string());
    }
    cfg_args.push("--max-turns".to_string());
    cfg_args.push(run_args.max_turns.to_string());
    if let Some(ref session) = run_args.session {
        cfg_args.push("--session".to_string());
        cfg_args.push(session.to_string_lossy().to_string());
    }
    if run_args.json {
        cfg_args.push("--json".to_string());
    }
    if run_args.quiet {
        cfg_args.push("--quiet".to_string());
    }
    if let Some(ref spf) = global.system_prompt_file {
        cfg_args.push("--system-prompt-file".to_string());
        cfg_args.push(spf.to_string_lossy().to_string());
    }
    // Also handle global.system_prompt
    let mut cfg = config::parse_args(&cfg_args).map_err(|e| miette::miette!("{}", e))?;
    if let Some(sp) = global.system_prompt {
        cfg.system_prompt = Some(sp);
    }
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

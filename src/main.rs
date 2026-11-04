use tinycode::agent::Event;
use tinycode::{agent, config};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

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
        return;
    }

    let cfg = match config::parse_args(&args) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("error: {e}\n\n{}", config::USAGE);
            std::process::exit(2);
        }
    };

    let json_mode = cfg.json;
    let quiet = cfg.quiet;

    let mut ag = match agent::Agent::new(cfg) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let code = match ag.run(&mut |event: &Event| {
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
    }) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    };
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

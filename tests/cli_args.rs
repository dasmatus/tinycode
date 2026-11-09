//! Clap is the single CLI path: OpenCode-style version, a `serve` subcommand,
//! and no hand-rolled argument fallback.
use clap::Parser;
use tinycode::cli::{Commands, ServeArgs};

#[test]
fn serve_args_parse_and_two_bit_version() {
    let cli = tinycode::cli::Cli::try_parse_from([
        "tinycode",
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        "4096",
    ])
    .expect("parses serve");
    assert!(
        matches!(
            cli.command,
            Some(Commands::Serve(ServeArgs { ref hostname, port, auto }))
                if hostname == "127.0.0.1" && port == 4096 && !auto
        ),
        "serve args: {:?}",
        cli.command
    );
    assert_eq!(tinycode::VERSION, "2.0.18-tinycode.0.1.0");
}

#[test]
fn build_config_does_not_decline_after_the_clap_arguments() {
    let cli = tinycode::cli::Cli::try_parse_from([
        "tinycode",
        "run",
        "--root",
        "tmp",
        "--auto",
        "--model",
        // A valid kind and model should parse.
        "anthropic/claude-3-5-haiku",
        "hello",
    ])
    .expect("parses run");
    // Impl waits until the first turn.
    let debug = format!("{:?}", cli.command);
    assert!(
        debug.contains("Run"),
        "Commands::Run must handle the args itself: {debug}"
    );
}

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "tinycode")]
#[command(about = "A minimal, resource-efficient coding agent CLI")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[command(flatten)]
    pub global: GlobalArgs,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the agent with a task
    Run(RunArgs),
    /// Summarize recorded provider usage
    Usage(UsageArgs),
    /// Run as ACP (Agent Client Protocol) server over stdio
    Acp,
    /// Run a simple ACP client to test the ACP server
    AcpClient(AcpClientArgs),
}

#[derive(Parser, Debug)]
pub struct GlobalArgs {
    #[arg(long)]
    pub system_prompt: Option<String>,
    #[arg(long)]
    pub system_prompt_file: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Task prompt
    #[arg(value_name = "TASK")]
    pub task: Option<String>,

    #[arg(short = 'f', long = "task-file")]
    pub task_file: Option<PathBuf>,

    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub provider: Option<String>,

    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long)]
    pub auto: bool,

    #[arg(long, default_value = "24")]
    pub max_turns: u32,

    #[arg(short = 's', long = "session")]
    pub session: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct UsageArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Parser, Debug)]
pub struct AcpClientArgs {
    /// Working directory for the ACP session
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Prompt to send after session creation (omit with --handshake-only)
    #[arg(long)]
    pub prompt: Option<String>,

    /// Only perform initialize + new_session handshake, then exit
    #[arg(long)]
    pub handshake_only: bool,

    /// Path to the tinycode binary to run as the ACP server (defaults to current executable)
    #[arg(long)]
    pub server: Option<PathBuf>,
}

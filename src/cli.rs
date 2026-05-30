use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "shortcut-forge")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Shortcut Forge build/sign server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Clone, Debug)]
pub enum Commands {
    Serve(ServeArgs),
    Gc(GcArgs),
    Init(InitArgs),
    Doctor(DoctorArgs),
    Start(OperatorArgs),
    Stop(OperatorArgs),
    Restart(OperatorArgs),
    Status(StatusArgs),
    Logs(LogsArgs),
    #[command(subcommand)]
    Config(ConfigCmd),
    #[command(subcommand)]
    Token(TokenCmd),
    Smoke(SmokeArgs),
    Build(BuildArgs),
}

#[derive(Args, Clone, Debug)]
pub struct ServeArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub public_base_url: Option<String>,
    #[arg(long)]
    pub storage: Option<PathBuf>,
    #[arg(long)]
    pub max_source_bytes: Option<usize>,
    #[arg(long)]
    pub build_timeout_seconds: Option<u64>,
    #[arg(long)]
    pub max_build_concurrency: Option<usize>,
    #[arg(long)]
    pub auth_token: Option<String>,
    #[arg(long)]
    pub health_cache_ttl_seconds: Option<u64>,
    #[arg(long)]
    pub cherri_bin: Option<String>,
    #[arg(long)]
    pub shortcuts_bin: Option<String>,
}

#[derive(Args, Clone, Debug)]
pub struct GcArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub storage: Option<PathBuf>,
    #[arg(long)]
    pub expired_before: Option<String>,
}

#[derive(Args, Clone, Debug)]
pub struct InitArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
    #[arg(long, default_value_t = 8787)]
    pub port: u16,
    #[arg(long)]
    pub public_base_url: Option<String>,
    #[arg(long)]
    pub storage: Option<PathBuf>,
    #[arg(long)]
    pub non_interactive: bool,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Clone, Debug)]
pub struct DoctorArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone, Debug)]
pub struct OperatorArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub struct StatusArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone, Debug)]
pub struct LogsArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long, default_value_t = 80)]
    pub lines: usize,
    #[arg(long)]
    pub follow: bool,
}

#[derive(Subcommand, Clone, Debug)]
pub enum ConfigCmd {
    Show(ConfigShowArgs),
    Set(ConfigSetArgs),
}

#[derive(Args, Clone, Debug)]
pub struct ConfigShowArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub struct ConfigSetArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    pub key: String,
    pub value: String,
}

#[derive(Subcommand, Clone, Debug)]
pub enum TokenCmd {
    Rotate(TokenRotateArgs),
}

#[derive(Args, Clone, Debug)]
pub struct TokenRotateArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub print: bool,
}

#[derive(Args, Clone, Debug)]
pub struct SmokeArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub request: Option<PathBuf>,
    #[arg(long, default_value = "/tmp/minimal.signed.shortcut")]
    pub output: PathBuf,
}

#[derive(Args, Clone, Debug)]
pub struct BuildArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
    pub request_path: PathBuf,
}

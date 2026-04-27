use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "warden",
    version,
    about = "Orchestrator for Dyson agents in CubeSandbox MicroVMs",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Path to the config TOML.
    #[arg(long, default_value = "/etc/dyson-warden/config.toml", global = true)]
    pub config: PathBuf,

    /// Disable the admin-token check on /v1/* routes. Loud and dangerous;
    /// see startup banner for details.
    #[arg(long = "dangerous-no-auth", default_value_t = false, global = true)]
    pub dangerous_no_auth: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the HTTP server (default action when no subcommand is given).
    Serve,

    /// Per-instance secret material.
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum SecretsAction {
    /// Set or overwrite a secret on an instance.
    Set {
        instance: String,
        name: String,
        value: String,
    },
    /// Remove a secret from an instance.
    Clear { instance: String, name: String },
}

/// Five-line warning emitted when `--dangerous-no-auth` is active.
pub const DANGEROUS_BANNER: &str = "\
=================================================================
WARNING: --dangerous-no-auth is set.
The admin API at /v1/* will accept requests with no bearer token.
Every authenticated response carries X-Warden-Insecure.
Do not run this configuration outside a trusted network.
=================================================================";

pub fn print_dangerous_banner() {
    eprintln!("{DANGEROUS_BANNER}");
}

use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "warden",
    version,
    about = "Orchestrator for Dyson agents in CubeSandbox MicroVMs",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Path to the config TOML.
    #[arg(long, default_value = "/etc/dyson-warden/config.toml")]
    pub config: PathBuf,

    /// Disable the admin-token check on /v1/* routes. Loud and dangerous;
    /// see startup banner for details.
    #[arg(long = "dangerous-no-auth", default_value_t = false)]
    pub dangerous_no_auth: bool,
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

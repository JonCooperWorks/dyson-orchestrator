use std::collections::BTreeMap;
use std::process::ExitCode;

use clap::Parser;

use dyson_warden::{cli, config, db, logging};

fn collect_env() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(k, _)| k.starts_with("WARDEN_"))
        .collect()
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = cli::Cli::parse();
    if args.dangerous_no_auth {
        cli::print_dangerous_banner();
    }
    logging::init();

    let cfg = match config::Config::load(&args.config, &collect_env(), args.dangerous_no_auth) {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(error = %err, config = %args.config.display(), "config load failed");
            return ExitCode::from(2);
        }
    };

    let _pool = match db::open(&cfg.db_path).await {
        Ok(p) => p,
        Err(err) => {
            tracing::error!(error = %err, db = %cfg.db_path.display(), "db open failed");
            return ExitCode::from(2);
        }
    };

    tracing::info!(bind = %cfg.bind, db = %cfg.db_path.display(), "warden started");

    if let Err(err) = wait_for_shutdown().await {
        tracing::warn!(error = %err, "signal handler error");
    }

    tracing::info!("warden stopped");
    ExitCode::SUCCESS
}

async fn wait_for_shutdown() -> std::io::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
    Ok(())
}

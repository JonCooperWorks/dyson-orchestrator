use std::collections::BTreeMap;
use std::process::ExitCode;

use clap::Parser;
use reqwest::Method;

use dyson_warden::{
    api_client::ApiClient,
    cli::{self, Command, SecretsAction},
    config, db, logging,
};

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

    match args.command.unwrap_or(Command::Serve) {
        Command::Serve => run_server(cfg).await,
        Command::Secrets { action } => run_secrets(&cfg, args.dangerous_no_auth, action).await,
    }
}

async fn run_server(cfg: config::Config) -> ExitCode {
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

async fn run_secrets(
    cfg: &config::Config,
    dangerous_no_auth: bool,
    action: SecretsAction,
) -> ExitCode {
    let token = if dangerous_no_auth {
        None
    } else {
        Some(cfg.admin_token.clone())
    };
    let client = match ApiClient::from_bind(&cfg.bind, token) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("error: {err:#}");
            return ExitCode::FAILURE;
        }
    };
    let result = match action {
        SecretsAction::Set { instance, name, value } => {
            let path = format!("/v1/instances/{instance}/secrets/{name}");
            client
                .send_json(Method::PUT, &path, &serde_json::json!({"value": value}))
                .await
        }
        SecretsAction::Clear { instance, name } => {
            let path = format!("/v1/instances/{instance}/secrets/{name}");
            client.send_no_body(Method::DELETE, &path).await
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
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

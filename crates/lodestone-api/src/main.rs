use std::net::SocketAddr;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod auth;
mod queries;
mod server;

#[derive(Parser, Debug)]
#[command(name = "api", about = "Knowledge graph query API")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7700", env = "API_LISTEN")]
    listen: SocketAddr,

    #[arg(long, default_value = "http://127.0.0.1:8123", env = "CLICKHOUSE_URL")]
    clickhouse_url: String,

    #[arg(long, env = "CLICKHOUSE_DB")]
    clickhouse_db: String,

    #[arg(long, env = "CLICKHOUSE_USER")]
    clickhouse_user: String,

    /// ClickHouse password. Required; there is no default.
    #[arg(long, env = "CLICKHOUSE_PASSWORD", hide_env_values = true)]
    clickhouse_password: String,

    /// Bearer token required on every request except /healthz.
    #[arg(long, env = "LODESTONE_API_TOKEN", hide_env_values = true)]
    api_token: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let args = Args::parse();
    let client = clickhouse::Client::default()
        .with_url(&args.clickhouse_url)
        .with_database(&args.clickhouse_db)
        .with_user(&args.clickhouse_user)
        .with_password(&args.clickhouse_password);

    // Smoke-test the connection so we fail fast.
    let _: u8 = client.query("SELECT 1").fetch_one().await?;
    tracing::info!(url = %args.clickhouse_url, db = %args.clickhouse_db, "connected to ClickHouse");

    if args.api_token.len() < 16 {
        anyhow::bail!("LODESTONE_API_TOKEN must be at least 16 chars; generate one with `openssl rand -hex 32`");
    }
    let app = server::router(client, args.api_token);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(addr = %args.listen, "api listening");
    axum::serve(listener, app).await?;
    Ok(())
}

use std::net::SocketAddr;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod queries;
mod server;

#[derive(Parser, Debug)]
#[command(name = "api", about = "Knowledge graph query API")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:7700", env = "API_LISTEN")]
    listen: SocketAddr,

    #[arg(long, default_value = "http://127.0.0.1:8123", env = "CLICKHOUSE_URL")]
    clickhouse_url: String,

    #[arg(long, default_value = "kg", env = "CLICKHOUSE_DB")]
    clickhouse_db: String,

    #[arg(long, default_value = "kg", env = "CLICKHOUSE_USER")]
    clickhouse_user: String,

    #[arg(long, default_value = "kg", env = "CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
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

    let app = server::router(client);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(addr = %args.listen, "api listening");
    axum::serve(listener, app).await?;
    Ok(())
}

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod parse;
mod publish;
mod walk;

#[derive(Parser, Debug)]
#[command(name = "indexer", about = "Walk a Rust repo and emit graph events to NATS")]
struct Args {
    /// Path to the repository to index
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Logical name of the repo (used as the `repo` column in storage)
    #[arg(long)]
    repo_name: Option<String>,

    /// NATS server URL
    #[arg(long, default_value = "nats://127.0.0.1:4222", env = "NATS_URL")]
    nats_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let args = Args::parse();
    let repo_path = args.repo.canonicalize()?;
    let repo_name = args
        .repo_name
        .unwrap_or_else(|| {
            repo_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("repo")
                .to_string()
        });

    tracing::info!(repo = %repo_path.display(), repo_name = %repo_name, "starting indexer");

    let client = async_nats::connect(&args.nats_url).await?;
    let publisher = publish::Publisher::new(client);

    let mut nodes_emitted = 0u64;
    let mut edges_emitted = 0u64;

    for entry in walk::rust_files(&repo_path) {
        let path = entry.path().to_owned();
        let rel = path.strip_prefix(&repo_path).unwrap_or(&path).to_owned();
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to read");
                continue;
            }
        };

        let extracted = match parse::extract(&repo_name, &rel, &source) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to parse");
                continue;
            }
        };

        for node in &extracted.nodes {
            publisher.publish_code_node(node).await?;
            nodes_emitted += 1;
        }
        for edge in &extracted.edges {
            publisher.publish_code_edge(edge).await?;
            edges_emitted += 1;
        }
    }

    publisher.flush().await?;
    tracing::info!(nodes_emitted, edges_emitted, "indexer done");
    Ok(())
}

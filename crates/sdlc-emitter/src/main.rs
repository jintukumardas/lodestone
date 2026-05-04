//! Reads a JSON fixture of issues + MRs and publishes them to NATS as if
//! Siphon were streaming SDLC change events into the knowledge graph.

use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use kg_core::{
    ids::{edge_id, node_id},
    subjects::{SDLC_EDGE_SUBJECT, SDLC_NODE_SUBJECT},
    Edge, Node, SdlcEvent,
};
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "sdlc-emitter")]
struct Args {
    /// Path to a JSON file containing an array of SdlcEvent objects.
    #[arg(long)]
    file: PathBuf,

    /// NATS server URL.
    #[arg(long, default_value = "nats://127.0.0.1:4222", env = "NATS_URL")]
    nats_url: String,

    /// Default repo name to attribute events to (overrides per-event repo if set).
    #[arg(long)]
    repo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FixtureEntry {
    Tagged(SdlcEvent),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let args = Args::parse();
    let raw = std::fs::read_to_string(&args.file)?;
    let entries: Vec<FixtureEntry> = serde_json::from_str(&raw)?;

    let client = async_nats::connect(&args.nats_url).await?;
    let mut nodes = 0u64;
    let mut edges = 0u64;

    for entry in entries {
        let FixtureEntry::Tagged(event) = entry;
        match event {
            SdlcEvent::Issue {
                id,
                title,
                author,
                state,
                repo,
                ts,
            } => {
                let repo = args.repo.clone().unwrap_or(repo);
                let qname = format!("{repo}:issue:{id}");
                let nid = node_id(&repo, "issue", &qname);
                let attrs = serde_json::json!({
                    "external_id": id,
                    "author": author,
                    "state": state,
                })
                .to_string();
                let node = Node {
                    id: nid,
                    kind: "issue".into(),
                    name: title,
                    qualified_name: qname,
                    repo,
                    file_path: String::new(),
                    start_line: 0,
                    end_line: 0,
                    attrs,
                    ts,
                };
                publish_node(&client, &node).await?;
                nodes += 1;
            }
            SdlcEvent::Mr {
                id,
                title,
                author,
                state,
                repo,
                touches,
                closes,
                ts,
            } => {
                let repo = args.repo.clone().unwrap_or(repo);
                let qname = format!("{repo}:mr:{id}");
                let mr_id = node_id(&repo, "mr", &qname);
                let attrs = serde_json::json!({
                    "external_id": id,
                    "author": author,
                    "state": state,
                })
                .to_string();
                let mr_node = Node {
                    id: mr_id.clone(),
                    kind: "mr".into(),
                    name: title,
                    qualified_name: qname,
                    repo: repo.clone(),
                    file_path: String::new(),
                    start_line: 0,
                    end_line: 0,
                    attrs,
                    ts,
                };
                publish_node(&client, &mr_node).await?;
                nodes += 1;

                for closed_issue in closes {
                    let issue_qname = format!("{repo}:issue:{closed_issue}");
                    let issue_id = node_id(&repo, "issue", &issue_qname);
                    let edge = Edge {
                        id: edge_id(&mr_id, &issue_id, "closes"),
                        src_id: mr_id.clone(),
                        dst_id: issue_id,
                        kind: "closes".into(),
                        repo: repo.clone(),
                        attrs: "{}".into(),
                        ts,
                    };
                    publish_edge(&client, &edge).await?;
                    edges += 1;
                }

                for path in touches {
                    let file_qname = format!("{repo}:{path}");
                    let file_id = node_id(&repo, "file", &file_qname);
                    let edge = Edge {
                        id: edge_id(&mr_id, &file_id, "touches"),
                        src_id: mr_id.clone(),
                        dst_id: file_id,
                        kind: "touches".into(),
                        repo: repo.clone(),
                        attrs: "{}".into(),
                        ts,
                    };
                    publish_edge(&client, &edge).await?;
                    edges += 1;
                }
            }
        }
    }

    client.flush().await?;
    let _ = Utc::now();
    tracing::info!(nodes, edges, "sdlc-emitter done");
    Ok(())
}

async fn publish_node(client: &async_nats::Client, node: &Node) -> Result<()> {
    let payload = serde_json::to_vec(node)?;
    client.publish(SDLC_NODE_SUBJECT, payload.into()).await?;
    Ok(())
}

async fn publish_edge(client: &async_nats::Client, edge: &Edge) -> Result<()> {
    let payload = serde_json::to_vec(edge)?;
    client.publish(SDLC_EDGE_SUBJECT, payload.into()).await?;
    Ok(())
}

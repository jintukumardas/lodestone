//! Reads a JSON fixture of issues + MRs and publishes them to NATS as if
//! Siphon were streaming SDLC change events into the Lodestone graph.

use std::path::PathBuf;

use anyhow::{Context, Result};
use async_nats::jetstream::{self, stream};
use chrono::Utc;
use clap::Parser;
use lodestone_core::{
    ids::{edge_id, node_id},
    subjects::{SDLC_EDGE_SUBJECT, SDLC_NODE_SUBJECT, STREAM_NAME, STREAM_SUBJECTS},
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
    let js = jetstream::new(client);
    ensure_stream(&js).await?;
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
                publish_node(&js, &node).await?;
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
                publish_node(&js, &mr_node).await?;
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
                    publish_edge(&js, &edge).await?;
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
                    publish_edge(&js, &edge).await?;
                    edges += 1;
                }
            }
        }
    }

    let _ = Utc::now();
    tracing::info!(nodes, edges, "sdlc-emitter done");
    Ok(())
}

async fn ensure_stream(js: &jetstream::Context) -> Result<()> {
    js.get_or_create_stream(stream::Config {
        name: STREAM_NAME.to_string(),
        subjects: STREAM_SUBJECTS.iter().map(|s| (*s).to_string()).collect(),
        storage: stream::StorageType::File,
        retention: stream::RetentionPolicy::Limits,
        max_age: std::time::Duration::from_secs(7 * 24 * 3600),
        ..Default::default()
    })
    .await
    .context("get_or_create_stream LODESTONE")?;
    Ok(())
}

async fn publish_node(js: &jetstream::Context, node: &Node) -> Result<()> {
    let payload = serde_json::to_vec(node)?;
    let ack = js
        .publish(SDLC_NODE_SUBJECT, payload.into())
        .await
        .context("jetstream publish failed (sdlc node)")?;
    ack.await.context("jetstream ack failed (sdlc node)")?;
    Ok(())
}

async fn publish_edge(js: &jetstream::Context, edge: &Edge) -> Result<()> {
    let payload = serde_json::to_vec(edge)?;
    let ack = js
        .publish(SDLC_EDGE_SUBJECT, payload.into())
        .await
        .context("jetstream publish failed (sdlc edge)")?;
    ack.await.context("jetstream ack failed (sdlc edge)")?;
    Ok(())
}

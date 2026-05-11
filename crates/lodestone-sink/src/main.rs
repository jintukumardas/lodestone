//! lodestone-sink: durable bridge from JetStream → ClickHouse.
//!
//! This is the production replacement for the old ClickHouse NATS table
//! engine. Four durable JetStream consumers (one per subject) batch messages
//! and write them to `lodestone.nodes` / `lodestone.edges` over ClickHouse's
//! HTTP interface. Messages are acked only after a successful insert, so a
//! sink crash replays unacked work from JetStream on restart.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use async_nats::jetstream::{
    self,
    consumer::{pull, AckPolicy},
    stream,
};
use chrono::{DateTime, Utc};
use clap::Parser;
use futures::StreamExt;
use lodestone_core::{
    subjects::{
        CODE_EDGE_SUBJECT, CODE_NODE_SUBJECT, SDLC_EDGE_SUBJECT, SDLC_NODE_SUBJECT, STREAM_NAME,
        STREAM_SUBJECTS,
    },
    Edge, Node,
};
use serde::Serialize;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "lodestone-sink", about = "JetStream → ClickHouse sink")]
struct Args {
    #[arg(long, default_value = "nats://127.0.0.1:4222", env = "NATS_URL")]
    nats_url: String,

    #[arg(long, default_value = "http://127.0.0.1:8123", env = "CLICKHOUSE_URL")]
    clickhouse_url: String,

    #[arg(long, env = "CLICKHOUSE_DB")]
    clickhouse_db: String,

    #[arg(long, env = "CLICKHOUSE_USER")]
    clickhouse_user: String,

    #[arg(long, env = "CLICKHOUSE_PASSWORD", hide_env_values = true)]
    clickhouse_password: String,

    /// Liveness/readiness HTTP listen address.
    #[arg(long, default_value = "127.0.0.1:7710", env = "SINK_LISTEN")]
    listen: SocketAddr,

    /// Maximum messages per insert batch.
    #[arg(long, default_value_t = 256)]
    batch_size: usize,

    /// Maximum time to wait while filling a batch.
    #[arg(long, default_value_t = 500)]
    batch_ms: u64,
}

#[derive(Debug, Clone, Serialize, clickhouse::Row)]
struct NodeRow {
    id: String,
    kind: String,
    name: String,
    qualified_name: String,
    repo: String,
    file_path: String,
    start_line: u32,
    end_line: u32,
    attrs: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, clickhouse::Row)]
struct EdgeRow {
    id: String,
    src_id: String,
    dst_id: String,
    kind: String,
    repo: String,
    attrs: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    ts: DateTime<Utc>,
}

impl From<Node> for NodeRow {
    fn from(n: Node) -> Self {
        Self {
            id: n.id,
            kind: n.kind,
            name: n.name,
            qualified_name: n.qualified_name,
            repo: n.repo,
            file_path: n.file_path,
            start_line: n.start_line,
            end_line: n.end_line,
            attrs: n.attrs,
            ts: n.ts,
        }
    }
}

impl From<Edge> for EdgeRow {
    fn from(e: Edge) -> Self {
        Self {
            id: e.id,
            src_id: e.src_id,
            dst_id: e.dst_id,
            kind: e.kind,
            repo: e.repo,
            attrs: e.attrs,
            ts: e.ts,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Node,
    Edge,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let ch = clickhouse::Client::default()
        .with_url(&args.clickhouse_url)
        .with_database(&args.clickhouse_db)
        .with_user(&args.clickhouse_user)
        .with_password(&args.clickhouse_password);
    let _: u8 = ch
        .query("SELECT 1")
        .fetch_one()
        .await
        .context("clickhouse smoke test")?;
    tracing::info!(url = %args.clickhouse_url, "connected to ClickHouse");

    let nats = async_nats::connect(&args.nats_url)
        .await
        .context("nats connect")?;
    let js = jetstream::new(nats);
    ensure_stream(&js).await?;
    tracing::info!(url = %args.nats_url, "connected to NATS");

    let cfg = ConsumerCfg {
        js: js.clone(),
        ch: ch.clone(),
        batch_size: args.batch_size,
        batch_dur: Duration::from_millis(args.batch_ms),
    };

    let mut handles = Vec::new();
    handles.push(tokio::spawn(consume(
        cfg.clone(),
        "sink-code-nodes",
        CODE_NODE_SUBJECT,
        Kind::Node,
    )));
    handles.push(tokio::spawn(consume(
        cfg.clone(),
        "sink-code-edges",
        CODE_EDGE_SUBJECT,
        Kind::Edge,
    )));
    handles.push(tokio::spawn(consume(
        cfg.clone(),
        "sink-sdlc-nodes",
        SDLC_NODE_SUBJECT,
        Kind::Node,
    )));
    handles.push(tokio::spawn(consume(
        cfg.clone(),
        "sink-sdlc-edges",
        SDLC_EDGE_SUBJECT,
        Kind::Edge,
    )));

    // Tiny health server. The sink is healthy as long as every consumer task
    // is still running; if any panics we drop the readiness state.
    let health = tokio::spawn(serve_health(args.listen));
    handles.push(health);

    // First completed task wins — sink should run forever, so any return is
    // either a panic propagation or a fatal config error.
    let (res, _idx, _rest) = futures::future::select_all(handles).await;
    res.context("consumer task panicked")?
}

#[derive(Clone)]
struct ConsumerCfg {
    js: jetstream::Context,
    ch: clickhouse::Client,
    batch_size: usize,
    batch_dur: Duration,
}

async fn consume(cfg: ConsumerCfg, durable: &str, subject: &str, kind: Kind) -> Result<()> {
    let stream = cfg
        .js
        .get_stream(STREAM_NAME)
        .await
        .with_context(|| format!("get_stream {STREAM_NAME}"))?;

    let consumer = stream
        .get_or_create_consumer(
            durable,
            pull::Config {
                durable_name: Some(durable.into()),
                filter_subject: subject.into(),
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(60),
                max_deliver: 16,
                ..Default::default()
            },
        )
        .await
        .with_context(|| format!("create consumer {durable}"))?;

    tracing::info!(durable, subject, ?kind, "consumer attached");

    let mut messages = consumer
        .messages()
        .await
        .with_context(|| format!("open message stream for {durable}"))?;

    let mut batch: Vec<async_nats::jetstream::Message> = Vec::with_capacity(cfg.batch_size);
    loop {
        // Wait for at least one message, then opportunistically drain up to
        // batch_size or batch_dur, whichever comes first.
        match messages.next().await {
            Some(Ok(msg)) => batch.push(msg),
            Some(Err(e)) => return Err(e).context("message stream error"),
            None => return Ok(()),
        }
        let deadline = tokio::time::Instant::now() + cfg.batch_dur;
        while batch.len() < cfg.batch_size {
            match tokio::time::timeout_at(deadline, messages.next()).await {
                Ok(Some(Ok(m))) => batch.push(m),
                Ok(Some(Err(e))) => return Err(e).context("message stream error"),
                Ok(None) => break,
                Err(_) => break, // batch_dur elapsed
            }
        }

        match flush_batch(&cfg.ch, kind, &batch).await {
            Ok(n) => {
                for m in &batch {
                    if let Err(e) = m.ack().await {
                        tracing::warn!(durable, error = ?e, "ack failed");
                    }
                }
                tracing::debug!(durable, inserted = n, "batch flushed");
            }
            Err(e) => {
                // Don't ack — JetStream will redeliver after ack_wait.
                tracing::error!(durable, error = ?e, "insert failed; messages will redeliver");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        batch.clear();
    }
}

async fn flush_batch(
    ch: &clickhouse::Client,
    kind: Kind,
    batch: &[async_nats::jetstream::Message],
) -> Result<usize> {
    match kind {
        Kind::Node => {
            let mut insert = ch.insert::<NodeRow>("lodestone.nodes")?;
            let mut n = 0usize;
            for m in batch {
                let node: Node = serde_json::from_slice(&m.payload)
                    .context("decode node payload")?;
                insert.write(&NodeRow::from(node)).await?;
                n += 1;
            }
            insert.end().await?;
            Ok(n)
        }
        Kind::Edge => {
            let mut insert = ch.insert::<EdgeRow>("lodestone.edges")?;
            let mut n = 0usize;
            for m in batch {
                let edge: Edge = serde_json::from_slice(&m.payload)
                    .context("decode edge payload")?;
                insert.write(&EdgeRow::from(edge)).await?;
                n += 1;
            }
            insert.end().await?;
            Ok(n)
        }
    }
}

async fn ensure_stream(js: &jetstream::Context) -> Result<()> {
    js.get_or_create_stream(stream::Config {
        name: STREAM_NAME.to_string(),
        subjects: STREAM_SUBJECTS.iter().map(|s| (*s).to_string()).collect(),
        storage: stream::StorageType::File,
        retention: stream::RetentionPolicy::Limits,
        max_age: Duration::from_secs(7 * 24 * 3600),
        ..Default::default()
    })
    .await
    .context("ensure stream")?;
    Ok(())
}

async fn serve_health(addr: SocketAddr) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(addr).await.context("bind health server")?;
    tracing::info!(%addr, "health server listening");
    loop {
        let (mut stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
        });
    }
}

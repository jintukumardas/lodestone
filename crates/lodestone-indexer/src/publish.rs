//! JetStream publisher for code nodes/edges.
//!
//! Messages are persisted in the `LODESTONE` stream so they survive a sink
//! restart. Each `publish_*` call awaits the JetStream ack, so the binary
//! exits only after every event is durable.

use anyhow::{Context, Result};
use async_nats::jetstream::{self, stream};
use async_nats::Client;
use lodestone_core::{
    subjects::{CODE_EDGE_SUBJECT, CODE_NODE_SUBJECT, STREAM_NAME, STREAM_SUBJECTS},
    Edge, Node,
};

pub struct Publisher {
    js: jetstream::Context,
}

impl Publisher {
    pub async fn new(client: Client) -> Result<Self> {
        let js = jetstream::new(client);
        ensure_stream(&js).await?;
        Ok(Self { js })
    }

    pub async fn publish_code_node(&self, node: &Node) -> Result<()> {
        let payload = serde_json::to_vec(node)?;
        let ack = self
            .js
            .publish(CODE_NODE_SUBJECT, payload.into())
            .await
            .context("jetstream publish failed (node)")?;
        ack.await.context("jetstream ack failed (node)")?;
        Ok(())
    }

    pub async fn publish_code_edge(&self, edge: &Edge) -> Result<()> {
        let payload = serde_json::to_vec(edge)?;
        let ack = self
            .js
            .publish(CODE_EDGE_SUBJECT, payload.into())
            .await
            .context("jetstream publish failed (edge)")?;
        ack.await.context("jetstream ack failed (edge)")?;
        Ok(())
    }

    pub async fn flush(&self) -> Result<()> {
        // JetStream publishes are ack'd inline, so there's nothing to flush.
        Ok(())
    }
}

/// Idempotently create the LODESTONE stream that captures both `code.>` and
/// `sdlc.>` subjects.
pub async fn ensure_stream(js: &jetstream::Context) -> Result<()> {
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

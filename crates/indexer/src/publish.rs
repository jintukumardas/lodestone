//! NATS publisher for code nodes/edges.

use anyhow::Result;
use async_nats::Client;
use kg_core::{
    subjects::{CODE_EDGE_SUBJECT, CODE_NODE_SUBJECT},
    Edge, Node,
};

pub struct Publisher {
    client: Client,
}

impl Publisher {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn publish_code_node(&self, node: &Node) -> Result<()> {
        let payload = serde_json::to_vec(node)?;
        self.client
            .publish(CODE_NODE_SUBJECT, payload.into())
            .await?;
        Ok(())
    }

    pub async fn publish_code_edge(&self, edge: &Edge) -> Result<()> {
        let payload = serde_json::to_vec(edge)?;
        self.client
            .publish(CODE_EDGE_SUBJECT, payload.into())
            .await?;
        Ok(())
    }

    pub async fn flush(&self) -> Result<()> {
        self.client.flush().await?;
        Ok(())
    }
}

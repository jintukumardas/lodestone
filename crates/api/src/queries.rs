//! ClickHouse query layer.
//!
//! `FINAL` is used on every read so ReplacingMergeTree returns the latest
//! version of each row. For a real-world workload this would be expensive;
//! the demo uses small data so it's fine.

use clickhouse::Client;
use serde::Serialize;

#[derive(Debug, Clone, serde::Deserialize, Serialize, clickhouse::Row)]
pub struct NodeRow {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub repo: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub attrs: String,
}

#[derive(Debug, Clone, serde::Deserialize, Serialize, clickhouse::Row)]
pub struct EdgeRow {
    pub id: String,
    pub src_id: String,
    pub dst_id: String,
    pub kind: String,
    pub repo: String,
    pub attrs: String,
}

#[derive(Debug, Serialize)]
pub struct Subgraph {
    pub nodes: Vec<NodeRow>,
    pub edges: Vec<EdgeRow>,
}

const NODE_COLS: &str =
    "id, kind, name, qualified_name, repo, file_path, start_line, end_line, attrs";
const EDGE_COLS: &str = "id, src_id, dst_id, kind, repo, attrs";

/// Functions whose `calls` edges target the given function id.
pub async fn callers_of(client: &Client, function_id: &str) -> anyhow::Result<Vec<NodeRow>> {
    let sql = format!(
        "SELECT {NODE_COLS} FROM kg.nodes FINAL \
         WHERE id IN ( \
            SELECT src_id FROM kg.edges FINAL WHERE kind = 'calls' AND dst_id = ? \
         )"
    );
    Ok(client.query(&sql).bind(function_id).fetch_all().await?)
}

/// Code entities impacted by an MR: walk MR --touches--> file --contains--> *.
pub async fn impacted_by(client: &Client, mr_id: &str) -> anyhow::Result<Vec<NodeRow>> {
    let sql = format!(
        "SELECT {NODE_COLS} FROM kg.nodes FINAL \
         WHERE id IN ( \
            SELECT dst_id FROM kg.edges FINAL \
            WHERE kind = 'contains' AND src_id IN ( \
                SELECT dst_id FROM kg.edges FINAL \
                WHERE kind = 'touches' AND src_id = ? \
            ) \
         )"
    );
    Ok(client.query(&sql).bind(mr_id).fetch_all().await?)
}

/// BFS subgraph around `seed_id` to depth `depth`. Iterative expansion (no
/// recursive CTE), bidirectional. Capped at `max_nodes` to keep responses
/// small for the demo.
pub async fn subgraph_around(
    client: &Client,
    seed_id: &str,
    depth: u32,
    max_nodes: usize,
) -> anyhow::Result<Subgraph> {
    use std::collections::HashSet;

    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(seed_id.to_string());
    let mut frontier: Vec<String> = vec![seed_id.to_string()];
    let mut all_edges: Vec<EdgeRow> = Vec::new();

    for _ in 0..depth {
        if frontier.is_empty() || visited.len() >= max_nodes {
            break;
        }
        // Collect edges where any endpoint is in the frontier.
        let placeholders = std::iter::repeat("?")
            .take(frontier.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT {EDGE_COLS} FROM kg.edges FINAL \
             WHERE src_id IN ({ph}) OR dst_id IN ({ph})",
            ph = placeholders
        );
        let mut q = client.query(&sql);
        for id in &frontier {
            q = q.bind(id);
        }
        for id in &frontier {
            q = q.bind(id);
        }
        let rows: Vec<EdgeRow> = q.fetch_all().await?;

        let mut next_frontier = Vec::new();
        for e in &rows {
            for endpoint in [&e.src_id, &e.dst_id] {
                if visited.insert(endpoint.clone()) {
                    next_frontier.push(endpoint.clone());
                }
            }
        }
        all_edges.extend(rows);
        frontier = next_frontier;
    }

    // Now fetch all nodes by id.
    let ids: Vec<String> = visited.into_iter().collect();
    let nodes = if ids.is_empty() {
        Vec::new()
    } else {
        let placeholders = std::iter::repeat("?")
            .take(ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT {NODE_COLS} FROM kg.nodes FINAL WHERE id IN ({ph})",
            ph = placeholders
        );
        let mut q = client.query(&sql);
        for id in &ids {
            q = q.bind(id);
        }
        q.fetch_all::<NodeRow>().await?
    };

    // Dedupe edges by id (multiple frontier passes can re-collect the same edge).
    let mut seen = std::collections::HashSet::new();
    all_edges.retain(|e| seen.insert(e.id.clone()));

    Ok(Subgraph {
        nodes,
        edges: all_edges,
    })
}

/// Look up a node by its qualified name within a repo.
pub async fn find_by_qname(
    client: &Client,
    repo: &str,
    qualified_name: &str,
) -> anyhow::Result<Option<NodeRow>> {
    let sql = format!(
        "SELECT {NODE_COLS} FROM kg.nodes FINAL \
         WHERE repo = ? AND qualified_name = ? LIMIT 1"
    );
    let rows: Vec<NodeRow> = client
        .query(&sql)
        .bind(repo)
        .bind(qualified_name)
        .fetch_all()
        .await?;
    Ok(rows.into_iter().next())
}

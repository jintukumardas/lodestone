//! ClickHouse query layer.
//!
//! `ReplacingMergeTree` keeps multiple physical rows per logical key until a
//! background merge collapses them, so a naive `SELECT *` can return stale
//! versions. The classic fix — `SELECT … FINAL` — works but materializes the
//! merge on every query, which is fine for a demo and miserable at scale.
//! Here we collapse manually with `argMax(col, _version) GROUP BY id`, which
//! lets ClickHouse scan parts in parallel and skip the merge plan.

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

// Latest-row projections via argMax. The trailing `id` is the grouping key
// and stays un-aggregated.
const NODE_LATEST: &str = "SELECT id, \
    argMax(kind, _version)            AS kind, \
    argMax(name, _version)            AS name, \
    argMax(qualified_name, _version)  AS qualified_name, \
    argMax(repo, _version)            AS repo, \
    argMax(file_path, _version)       AS file_path, \
    argMax(start_line, _version)      AS start_line, \
    argMax(end_line, _version)        AS end_line, \
    argMax(attrs, _version)           AS attrs \
    FROM lodestone.nodes";

const EDGE_LATEST: &str = "SELECT id, \
    argMax(src_id, _version) AS src_id, \
    argMax(dst_id, _version) AS dst_id, \
    argMax(kind, _version)   AS kind, \
    argMax(repo, _version)   AS repo, \
    argMax(attrs, _version)  AS attrs \
    FROM lodestone.edges";

/// Functions whose `calls` edges target the given function id.
///
/// Edge attribute columns (kind, src_id, dst_id) are stable per id in our
/// model, so a raw subquery filter is safe and avoids the WHERE-vs-argMax
/// alias clash that ClickHouse refuses.
pub async fn callers_of(client: &Client, function_id: &str) -> anyhow::Result<Vec<NodeRow>> {
    let sql = format!(
        "{NODE_LATEST} \
         WHERE id IN ( \
            SELECT src_id FROM lodestone.edges \
            WHERE kind = 'calls' AND dst_id = ? \
         ) \
         GROUP BY id"
    );
    Ok(client.query(&sql).bind(function_id).fetch_all().await?)
}

/// Code entities impacted by an MR: walk MR --touches--> file --contains--> *.
pub async fn impacted_by(client: &Client, mr_id: &str) -> anyhow::Result<Vec<NodeRow>> {
    let sql = format!(
        "{NODE_LATEST} \
         WHERE id IN ( \
            SELECT dst_id FROM lodestone.edges \
            WHERE kind = 'contains' AND src_id IN ( \
                SELECT dst_id FROM lodestone.edges \
                WHERE kind = 'touches' AND src_id = ? \
            ) \
         ) \
         GROUP BY id"
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
        let placeholders = std::iter::repeat("?")
            .take(frontier.len())
            .collect::<Vec<_>>()
            .join(",");
        // Filter on raw columns via a subquery to avoid the argMax-alias
        // clash CH otherwise reports as ILLEGAL_AGGREGATION.
        let sql = format!(
            "{EDGE_LATEST} \
             WHERE id IN ( \
                SELECT id FROM lodestone.edges \
                WHERE src_id IN ({ph}) OR dst_id IN ({ph}) \
             ) \
             GROUP BY id",
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

    let ids: Vec<String> = visited.into_iter().collect();
    let nodes = if ids.is_empty() {
        Vec::new()
    } else {
        let placeholders = std::iter::repeat("?")
            .take(ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "{NODE_LATEST} WHERE id IN ({ph}) GROUP BY id",
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
        "{NODE_LATEST} \
         WHERE id IN ( \
            SELECT id FROM lodestone.nodes WHERE repo = ? AND qualified_name = ? \
         ) \
         GROUP BY id \
         LIMIT 1"
    );
    let rows: Vec<NodeRow> = client
        .query(&sql)
        .bind(repo)
        .bind(qualified_name)
        .fetch_all()
        .await?;
    Ok(rows.into_iter().next())
}

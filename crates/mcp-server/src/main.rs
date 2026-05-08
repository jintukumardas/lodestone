//! MCP server that wraps the knowledge graph HTTP API.
//!
//! Exposes three tools to MCP clients (Claude Desktop, mcp-inspector, etc.):
//!   - `get_function_callers` — given a repo + function name, list callers
//!   - `get_impacted` — given an MR external id, list code entities impacted
//!   - `get_subgraph` — given a node id, return the local subgraph
//!
//! Communicates over stdio so it can be launched as a child process by an
//! MCP client.

use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use kg_core::ids::node_id;
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::{CallToolResult, Content, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(name = "mcp-server")]
struct Args {
    /// Base URL of the knowledge graph HTTP API.
    #[arg(long, default_value = "http://127.0.0.1:7700", env = "KG_API_URL")]
    api_url: String,
}

#[derive(Clone)]
struct KgServer {
    api_url: Arc<String>,
    http: reqwest::Client,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CallersArgs {
    /// Repo name (e.g. "knowledge-graph").
    repo: String,
    /// File path of the function, relative to the repo root.
    file_path: String,
    /// Bare function name.
    function_name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ImpactedArgs {
    /// Repo name.
    repo: String,
    /// External MR id (e.g. "MR-101").
    mr_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SubgraphArgs {
    /// Node id (32-char hex hash from kg_core::ids).
    node_id: String,
    /// Hops to expand from the seed. Default 2.
    #[serde(default = "default_depth")]
    depth: u32,
}
fn default_depth() -> u32 {
    2
}

#[tool_router(router = tool_router)]
impl KgServer {
    fn new(api_url: String) -> Self {
        Self {
            api_url: Arc::new(api_url),
            http: reqwest::Client::new(),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List functions that call the named function within the given file. Use when asked who-calls / what-uses a specific function in a repo."
    )]
    async fn get_function_callers(
        &self,
        Parameters(args): Parameters<CallersArgs>,
    ) -> Result<CallToolResult, rmcp::Error> {
        let qname = format!("{}:{}:{}", args.repo, args.file_path, args.function_name);
        let id = node_id(&args.repo, "function", &qname);
        let url = format!("{}/callers/{}", self.api_url, id);
        let body = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(internal)?
            .text()
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(
        description = "List code entities impacted by an SDLC change. Walks MR --touches--> file --contains--> functions/structs."
    )]
    async fn get_impacted(
        &self,
        Parameters(args): Parameters<ImpactedArgs>,
    ) -> Result<CallToolResult, rmcp::Error> {
        let qname = format!("{}:mr:{}", args.repo, args.mr_id);
        let id = node_id(&args.repo, "mr", &qname);
        let url = format!("{}/impacted/{}", self.api_url, id);
        let body = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(internal)?
            .text()
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(
        description = "Return the local neighborhood of a node up to N hops. Use to explore architecture around an entity (file, function, MR, issue)."
    )]
    async fn get_subgraph(
        &self,
        Parameters(args): Parameters<SubgraphArgs>,
    ) -> Result<CallToolResult, rmcp::Error> {
        let url = format!(
            "{}/subgraph/{}?depth={}",
            self.api_url, args.node_id, args.depth
        );
        let body = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(internal)?
            .text()
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for KgServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Knowledge graph for a local Rust monorepo. Index code via the `indexer` \
                 binary, then use these tools to explore call graphs and SDLC impact. \
                 Node IDs are stable hashes; resolve them via `find` (HTTP) or by name."
                    .into(),
            ),
            ..Default::default()
        }
    }
}

fn internal<E: std::fmt::Display>(e: E) -> rmcp::Error {
    rmcp::Error::internal_error(format!("api request failed: {e}"), None)
}

#[tokio::main]
async fn main() -> Result<()> {
    // MCP runs over stdio, so logs MUST go to stderr (default for tracing
    // when stdout is JSON-RPC).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let server = KgServer::new(args.api_url);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

# Lodestone

A local SDLC + code knowledge graph for a monorepo. Indexes Rust source and
lightweight SDLC metadata (issues, MRs) into ClickHouse via NATS, exposes graph
queries over an HTTP API, and surfaces them through MCP so an AI agent can
answer architecture questions about the repo.

Inspired by **GitLab's Knowledge Graph** and mirrors the GitLab **Orbit** /
**Siphon** production architecture in miniature.

**Author:** Jintu Kumar Das

## Architecture

```
  lodestone-indexer (tree-sitter)  ─┐
                                    ├─►  NATS  ─►  ClickHouse NATS engine  ─►  MV  ─►  lodestone.nodes
  lodestone-sdlc-emitter (CLI)     ─┘                                                    lodestone.edges
                                                                                          │
                                                                                          ▼
                                                                          lodestone-api (Axum, iterative BFS)
                                                                                          │
                                                                                          ▼
                                                                          lodestone-mcp-server (rmcp / stdio)
```

- **Subjects:** `code.node.upserted`, `code.edge.upserted`, `sdlc.node.upserted`, `sdlc.edge.upserted`
- **Storage:** two `ReplacingMergeTree` tables (`lodestone.nodes`, `lodestone.edges`); IDs are
  blake3-hashed from `(repo, kind, qualified_name)` so re-publishing dedupes.
- **Materialized views** bridge each NATS source table into the storage tables.

## Quick start

```bash
docker compose up -d                                          # NATS + ClickHouse
cargo build --workspace --release

./target/release/lodestone-indexer --repo . --repo-name lodestone
./target/release/lodestone-sdlc-emitter --file fixtures/issues.json --repo lodestone

./target/release/lodestone-api &                              # HTTP API on :7700
./target/release/lodestone-mcp-server                         # MCP over stdio
```

> The pipeline uses **core NATS** (not JetStream): messages published before
> ClickHouse attaches are lost. If `lodestone.nodes` is empty, just re-run the
> indexer/emitter — they're idempotent.

## Using it on your own local git repo

The indexer takes any path — nothing in the pipeline is specific to this
repo. Workflow for a real project on disk (say `~/code/my-service`):

```bash
# 1. Bring up infra (once)
docker compose up -d

# 2. Index the repo. Pick a stable --repo-name; it becomes the `repo` column
#    in ClickHouse and the `repo` argument to every MCP tool. Re-running is
#    idempotent (IDs are content-hashed), so you can re-index after changes.
./target/release/lodestone-indexer \
    --repo ~/code/my-service \
    --repo-name my-service

# 3. (Optional) Feed in SDLC metadata. Write a JSON file with the same shape
#    as fixtures/issues.json, using the same --repo-name so MR `touches` edges
#    link to the file nodes the indexer produced. Paths must be repo-relative.
./target/release/lodestone-sdlc-emitter \
    --file ~/code/my-service/.lodestone/issues.json \
    --repo my-service

# 4. Query
./target/release/lodestone-api &        # http://127.0.0.1:7700
./target/release/lodestone-mcp-server   # or wire into Claude Desktop, see below
```

A few things to know:

- **`.gitignore` is honored** by the walker (via the `ignore` crate), so
  `target/`, `node_modules/`, etc. are skipped automatically.
- **Only Rust source is parsed** in V1 (`*.rs` files via `tree-sitter-rust`).
  Non-Rust files are ignored. Adding a language is a tree-sitter grammar +
  a few match arms in `crates/lodestone-indexer/src/parse.rs`.
- **No incremental re-indexing.** Each run is a full sweep. Because IDs are
  deterministic, re-runs dedupe correctly via `ReplacingMergeTree`. To pick up
  changes, just re-run the indexer (it's fast — ~40 ms for this 27-file repo).
- **Multiple repos coexist** in the same ClickHouse instance. Index each with
  a distinct `--repo-name`; queries that take a `repo` argument scope correctly.
- **SDLC adapter is up to you.** `lodestone-sdlc-emitter` reads a static JSON file. To
  pull from a real tracker, either generate that JSON from the GitHub/GitLab
  API, or write a sibling crate that publishes the same NATS subjects directly
  using `lodestone-core`'s types.
- **File-path conventions matter for SDLC linkage.** `MR --touches--> file`
  edges work by hashing `(repo, "file", "<repo>:<path>")`. The indexer uses
  paths relative to the repo root; your `touches` array must match that exact
  string for the edge to land on a real file node.
- **Auto-reindex on commit (one-liner):** drop this in `.git/hooks/post-commit`:
  ```sh
  #!/bin/sh
  /path/to/target/release/lodestone-indexer --repo "$(git rev-parse --show-toplevel)" --repo-name my-service >/dev/null 2>&1 &
  ```

### Asking questions about your repo

Once data is loaded, useful patterns from the MCP side:

- *"What functions does `parse_request` call into?"* → `get_subgraph` on its
  `node_id`, depth 1, then filter for `calls` edges.
- *"Who calls `validate_token`?"* → `get_function_callers`.
- *"What files would MR-42 touch and what functions live in them?"* →
  `get_impacted` with the MR id.
- *"Show me the neighborhood around the `auth` module"* → resolve via `/find`
  with `qname=my-service:src/auth/mod.rs:auth`, then `get_subgraph` depth 2.

## HTTP API (port 7700)

| Endpoint | Description |
|---|---|
| `GET /healthz` | liveness |
| `GET /find?repo=&qname=` | look up a node id by `repo` + qualified name |
| `GET /callers/{function_id}` | functions that call this one (1-hop reverse on `calls`) |
| `GET /impacted/{mr_id}` | code entities reachable from `mr --touches--> file --contains--> *` |
| `GET /subgraph/{node_id}?depth=2&max=200` | iterative BFS, bidirectional, capped |

Example:

```bash
ID=$(curl -s "http://127.0.0.1:7700/find?repo=lodestone&qname=lodestone:crates/lodestone-indexer/src/parse.rs:extract" \
     | jq -r .node.id)
curl -s "http://127.0.0.1:7700/subgraph/$ID?depth=2" | jq '{nodes: (.nodes|length), edges: (.edges|length)}'
```

## MCP tools

`lodestone-mcp-server` speaks MCP 2024-11-05 over stdio. Three tools:

| Tool | Arguments | What it answers |
|---|---|---|
| `get_function_callers` | `repo`, `file_path`, `function_name` | who calls this function |
| `get_impacted` | `repo`, `mr_id` (e.g. `"MR-101"`) | which code entities an SDLC change touches |
| `get_subgraph` | `node_id`, `depth` | local neighborhood of any node |

Wire it into Claude Desktop with:

```json
{
  "mcpServers": {
    "lodestone": {
      "command": "/absolute/path/to/target/release/lodestone-mcp-server",
      "env": { "LODESTONE_API_URL": "http://127.0.0.1:7700" }
    }
  }
}
```

## Layout

- `crates/lodestone-core/` — shared `Node`/`Edge` types, NATS subject constants, deterministic ID hashing, ClickHouse-friendly datetime serde
- `crates/lodestone-indexer/` — `tree-sitter-rust` AST walker; emits `code.node.*` / `code.edge.*` to NATS
- `crates/lodestone-sdlc-emitter/` — reads a JSON fixture, emits `sdlc.node.*` / `sdlc.edge.*` (file IDs computed with the same hash so cross-source linkage works)
- `crates/lodestone-api/` — Axum HTTP server querying ClickHouse
- `crates/lodestone-mcp-server/` — `rmcp` 0.2 stdio server wrapping the HTTP API
- `clickhouse/migrations/` — `01_database` → `02_storage` → `03_nats_sources` → `04_materialized_views`
- `fixtures/issues.json` — small set of issues + MRs for the demo

## Captured graph (V1 scope)

| Node kinds | Edge kinds |
|---|---|
| `file`, `function`, `struct`, `enum`, `trait`, `module`, `issue`, `mr` | `contains`, `calls`, `references`, `closes`, `touches` |

V1 deliberately skips a cross-file resolver: `calls` edges target a hash of
`(repo, "function", "<repo>:fn:<callee_name>")`, so within-file calls usually
resolve and cross-file calls dangle. That's intentional and surfaces honestly
in queries (you'll see `calls` edges to non-existent function nodes).

## Design notes & known limitations

- **rustc 1.86 compatibility:** `time` is pinned via `cargo update -p time --precise 0.3.41 && cargo update -p time-core --precise 0.1.4`.
- **Datetime format:** ClickHouse JSONEachRow rejects RFC3339's trailing `Z`. `lodestone-core::model::ch_datetime` emits `YYYY-MM-DD HH:MM:SS.fff` instead.
- **`FINAL` on every read:** acceptable for the demo, expensive at scale; switch to `argMax(...)` aggregations for production.
- **Iterative BFS, not recursive CTE:** simpler and version-portable; ClickHouse 24.x recursive CTEs would let `subgraph_around` collapse to a single SQL.

## Contributing

Bug reports, language extractors, query endpoints, MCP tools, and docs are
all welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions,
project layout, and the workflow for opening issues and PRs.

## License

Lodestone is released under the [MIT License](LICENSE). You are free to use,
modify, and distribute it for any purpose — personal, academic, or commercial.

For questions, collaboration, or commercial support, contact
**Jintu Kumar Das** at jintukumardas@gmail.com.

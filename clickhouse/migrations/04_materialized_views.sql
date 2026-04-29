-- Bridges: each MV reads from a NATS source and writes into the appropriate
-- ReplacingMergeTree. Both code.* and sdlc.* nodes land in `kg.nodes`; both
-- code.* and sdlc.* edges land in `kg.edges`. The kind column distinguishes.

CREATE MATERIALIZED VIEW IF NOT EXISTS kg.mv_code_nodes TO kg.nodes AS
SELECT id, kind, name, qualified_name, repo, file_path, start_line, end_line, attrs, ts
FROM kg.nats_code_nodes;

CREATE MATERIALIZED VIEW IF NOT EXISTS kg.mv_code_edges TO kg.edges AS
SELECT id, src_id, dst_id, kind, repo, attrs, ts
FROM kg.nats_code_edges;

CREATE MATERIALIZED VIEW IF NOT EXISTS kg.mv_sdlc_nodes TO kg.nodes AS
SELECT id, kind, name, qualified_name, repo, file_path, start_line, end_line, attrs, ts
FROM kg.nats_sdlc_nodes;

CREATE MATERIALIZED VIEW IF NOT EXISTS kg.mv_sdlc_edges TO kg.edges AS
SELECT id, src_id, dst_id, kind, repo, attrs, ts
FROM kg.nats_sdlc_edges;

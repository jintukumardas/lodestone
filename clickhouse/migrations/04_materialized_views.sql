-- Bridges: each MV reads from a NATS source and writes into the appropriate
-- ReplacingMergeTree. Both code.* and sdlc.* nodes land in `lodestone.nodes`; both
-- code.* and sdlc.* edges land in `lodestone.edges`. The kind column distinguishes.

CREATE MATERIALIZED VIEW IF NOT EXISTS lodestone.mv_code_nodes TO lodestone.nodes AS
SELECT id, kind, name, qualified_name, repo, file_path, start_line, end_line, attrs, ts
FROM lodestone.nats_code_nodes;

CREATE MATERIALIZED VIEW IF NOT EXISTS lodestone.mv_code_edges TO lodestone.edges AS
SELECT id, src_id, dst_id, kind, repo, attrs, ts
FROM lodestone.nats_code_edges;

CREATE MATERIALIZED VIEW IF NOT EXISTS lodestone.mv_sdlc_nodes TO lodestone.nodes AS
SELECT id, kind, name, qualified_name, repo, file_path, start_line, end_line, attrs, ts
FROM lodestone.nats_sdlc_nodes;

CREATE MATERIALIZED VIEW IF NOT EXISTS lodestone.mv_sdlc_edges TO lodestone.edges AS
SELECT id, src_id, dst_id, kind, repo, attrs, ts
FROM lodestone.nats_sdlc_edges;

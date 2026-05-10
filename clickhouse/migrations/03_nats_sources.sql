-- NATS source tables. These don't store rows themselves — they expose the
-- NATS subscription as a stream of rows. A materialized view (next file) then
-- pumps each row into the appropriate storage table.
--
-- Subjects:
--   code.node.upserted   → nats_code_nodes
--   code.edge.upserted   → nats_code_edges
--   sdlc.node.upserted   → nats_sdlc_nodes
--   sdlc.edge.upserted   → nats_sdlc_edges

CREATE TABLE IF NOT EXISTS lodestone.nats_code_nodes
(
    id              String,
    kind            String,
    name            String,
    qualified_name  String,
    repo            String,
    file_path       String,
    start_line      UInt32,
    end_line        UInt32,
    attrs           String,
    ts              DateTime64(3, 'UTC')
)
ENGINE = NATS
SETTINGS
    nats_url = 'nats:4222',
    nats_subjects = 'code.node.upserted',
    nats_format = 'JSONEachRow',
    nats_queue_group = 'lodestone-clickhouse-code-nodes',
    nats_skip_broken_messages = 100;

CREATE TABLE IF NOT EXISTS lodestone.nats_code_edges
(
    id      String,
    src_id  String,
    dst_id  String,
    kind    String,
    repo    String,
    attrs   String,
    ts      DateTime64(3, 'UTC')
)
ENGINE = NATS
SETTINGS
    nats_url = 'nats:4222',
    nats_subjects = 'code.edge.upserted',
    nats_format = 'JSONEachRow',
    nats_queue_group = 'lodestone-clickhouse-code-edges',
    nats_skip_broken_messages = 100;

CREATE TABLE IF NOT EXISTS lodestone.nats_sdlc_nodes
(
    id              String,
    kind            String,
    name            String,
    qualified_name  String,
    repo            String,
    file_path       String,
    start_line      UInt32,
    end_line        UInt32,
    attrs           String,
    ts              DateTime64(3, 'UTC')
)
ENGINE = NATS
SETTINGS
    nats_url = 'nats:4222',
    nats_subjects = 'sdlc.node.upserted',
    nats_format = 'JSONEachRow',
    nats_queue_group = 'lodestone-clickhouse-sdlc-nodes',
    nats_skip_broken_messages = 100;

CREATE TABLE IF NOT EXISTS lodestone.nats_sdlc_edges
(
    id      String,
    src_id  String,
    dst_id  String,
    kind    String,
    repo    String,
    attrs   String,
    ts      DateTime64(3, 'UTC')
)
ENGINE = NATS
SETTINGS
    nats_url = 'nats:4222',
    nats_subjects = 'sdlc.edge.upserted',
    nats_format = 'JSONEachRow',
    nats_queue_group = 'lodestone-clickhouse-sdlc-edges',
    nats_skip_broken_messages = 100;

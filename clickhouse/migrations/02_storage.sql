-- Storage tables: ReplacingMergeTree dedupes on (ORDER BY) using `_version`.
-- The indexer/emitter compute deterministic `id`s, so re-publishing the same
-- entity yields the same row and the highest `_version` wins.

CREATE TABLE IF NOT EXISTS kg.nodes
(
    id              String,
    kind            LowCardinality(String),
    name            String,
    qualified_name  String,
    repo            LowCardinality(String),
    file_path       String,
    start_line      UInt32,
    end_line        UInt32,
    attrs           String,
    ts              DateTime64(3, 'UTC'),
    _version        UInt64 MATERIALIZED toUInt64(toUnixTimestamp64Milli(ts))
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY (repo, kind, id);

CREATE TABLE IF NOT EXISTS kg.edges
(
    id          String,
    src_id      String,
    dst_id      String,
    kind        LowCardinality(String),
    repo        LowCardinality(String),
    attrs       String,
    ts          DateTime64(3, 'UTC'),
    _version    UInt64 MATERIALIZED toUInt64(toUnixTimestamp64Milli(ts))
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY (repo, kind, id);

-- Helper indices for traversal queries
ALTER TABLE kg.edges ADD INDEX IF NOT EXISTS edges_src_idx src_id TYPE bloom_filter GRANULARITY 1;
ALTER TABLE kg.edges ADD INDEX IF NOT EXISTS edges_dst_idx dst_id TYPE bloom_filter GRANULARITY 1;

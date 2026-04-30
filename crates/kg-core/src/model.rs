//! Wire types for events flowing through NATS.
//!
//! These serialize to JSON. The ClickHouse NATS table engine reads them via
//! `JSONEachRow` format, so field names here must match the column names
//! declared in the migration SQL.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Custom (de)serializer for `DateTime<Utc>` that emits a format ClickHouse's
/// JSONEachRow input parser accepts directly: `YYYY-MM-DD HH:MM:SS.fff`. The
/// default chrono RFC3339 form (with trailing `Z`) is rejected by CH.
mod ch_datetime {
    use super::*;
    use chrono::NaiveDateTime;

    pub fn serialize<S: Serializer>(v: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let raw = String::deserialize(d)?;
        // Accept both our format and RFC3339 on the read path.
        if let Ok(dt) = DateTime::parse_from_rfc3339(&raw) {
            return Ok(dt.with_timezone(&Utc));
        }
        let naive = NaiveDateTime::parse_from_str(&raw, "%Y-%m-%d %H:%M:%S%.3f")
            .or_else(|_| NaiveDateTime::parse_from_str(&raw, "%Y-%m-%d %H:%M:%S"))
            .map_err(serde::de::Error::custom)?;
        Ok(DateTime::from_naive_utc_and_offset(naive, Utc))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    File,
    Module,
    Function,
    Struct,
    Enum,
    Trait,
    Issue,
    Mr,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Module => "module",
            NodeKind::Function => "function",
            NodeKind::Struct => "struct",
            NodeKind::Enum => "enum",
            NodeKind::Trait => "trait",
            NodeKind::Issue => "issue",
            NodeKind::Mr => "mr",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Contains,
    Calls,
    References,
    Closes,
    Touches,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Contains => "contains",
            EdgeKind::Calls => "calls",
            EdgeKind::References => "references",
            EdgeKind::Closes => "closes",
            EdgeKind::Touches => "touches",
        }
    }
}

/// A graph node. Maps 1:1 to a row in the `nodes` ReplacingMergeTree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub repo: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    /// JSON-encoded extra attributes. Stored as a `String` column in CH.
    pub attrs: String,
    #[serde(with = "ch_datetime")]
    pub ts: DateTime<Utc>,
}

/// A graph edge. Maps 1:1 to a row in the `edges` ReplacingMergeTree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub src_id: String,
    pub dst_id: String,
    pub kind: String,
    pub repo: String,
    pub attrs: String,
    #[serde(with = "ch_datetime")]
    pub ts: DateTime<Utc>,
}

/// Wrapper for SDLC events. Carries either a node or a link to existing nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SdlcEvent {
    Issue {
        id: String,
        title: String,
        author: String,
        state: String,
        repo: String,
        ts: DateTime<Utc>,
    },
    Mr {
        id: String,
        title: String,
        author: String,
        state: String,
        repo: String,
        /// File paths touched by this MR
        touches: Vec<String>,
        /// Issue IDs this MR closes
        closes: Vec<String>,
        ts: DateTime<Utc>,
    },
}

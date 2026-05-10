//! NATS subject conventions.
//!
//! All graph events flow through two subject hierarchies:
//! - `code.>` — code-derived nodes/edges from the indexer
//! - `sdlc.>` — issues/MRs/etc from the (fake) Siphon emitter

pub const CODE_NODE_SUBJECT: &str = "code.node.upserted";
pub const CODE_EDGE_SUBJECT: &str = "code.edge.upserted";

pub const SDLC_NODE_SUBJECT: &str = "sdlc.node.upserted";
pub const SDLC_EDGE_SUBJECT: &str = "sdlc.edge.upserted";

pub const STREAM_NAME: &str = "LODESTONE";
pub const STREAM_SUBJECTS: &[&str] = &["code.>", "sdlc.>"];

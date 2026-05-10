//! Stable, content-addressed IDs.
//!
//! The indexer must be deterministic across runs so re-indexing the same code
//! produces the same node IDs (allowing `ReplacingMergeTree` to dedupe). We
//! hash a tuple of `(repo, kind, qualified_name)` with blake3 → hex.

pub fn node_id(repo: &str, kind: &str, qualified_name: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(repo.as_bytes());
    hasher.update(b"\0");
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(qualified_name.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash.as_bytes()[..16])
}

pub fn edge_id(src: &str, dst: &str, kind: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(src.as_bytes());
    hasher.update(b"\0");
    hasher.update(dst.as_bytes());
    hasher.update(b"\0");
    hasher.update(kind.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash.as_bytes()[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_is_stable() {
        let a = node_id("repo", "function", "foo::bar");
        let b = node_id("repo", "function", "foo::bar");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn node_id_differs_per_input() {
        assert_ne!(
            node_id("a", "function", "x"),
            node_id("b", "function", "x")
        );
    }
}

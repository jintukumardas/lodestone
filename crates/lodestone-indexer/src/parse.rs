//! Tree-sitter walk that extracts a small set of node and edge kinds from a
//! single Rust source file.
//!
//! Two-pass design:
//!   1. `extract_defs` walks each file and emits definition nodes (`file`,
//!      `function`, `struct`, `enum`, `trait`, `module`), `contains` edges,
//!      and a list of unresolved `CallSite`s (one per call expression inside
//!      every function body).
//!   2. After every file has been visited the caller builds a [`DefIndex`]
//!      from the accumulated function nodes and feeds it back through
//!      [`resolve_call_sites`], which produces real `calls` edges that target
//!      the actual function-node IDs whenever resolution is possible.
//!
//! Resolution policy is intentionally simple:
//!   * if the callee name is defined in the same file, prefer that definition;
//!   * else if it is defined in exactly one other file across the repo, target
//!     that definition;
//!   * else fall back to the legacy `<repo>:fn:<name>` hash so the edge
//!     dangles honestly (ambiguous or external call).
//!
//! This is enough to make in-repo cross-file calls resolve while still being
//! truthful when the name is ambiguous.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use lodestone_core::{
    ids::{edge_id, node_id},
    Edge, Node,
};
use tree_sitter::{Node as TsNode, Parser, Tree};

/// Definitions and unresolved call sites extracted from one file.
pub struct FileDefs {
    pub nodes: Vec<Node>,
    pub contains_edges: Vec<Edge>,
    pub call_sites: Vec<CallSite>,
}

/// A single call expression we haven't yet resolved to a target node.
#[derive(Debug, Clone)]
pub struct CallSite {
    pub src_fn_id: String,
    pub src_file_path: String,
    pub callee_name: String,
    pub ts: DateTime<Utc>,
}

/// Index of every function definition in the repo, keyed by bare name.
#[derive(Debug, Default)]
pub struct DefIndex {
    by_name: HashMap<String, Vec<DefEntry>>,
}

#[derive(Debug, Clone)]
struct DefEntry {
    file_path: String,
    fn_id: String,
}

impl DefIndex {
    /// Build the index from the function nodes collected during pass 1.
    pub fn build(nodes: &[Node]) -> Self {
        let mut by_name: HashMap<String, Vec<DefEntry>> = HashMap::new();
        for n in nodes {
            if n.kind == "function" {
                by_name.entry(n.name.clone()).or_default().push(DefEntry {
                    file_path: n.file_path.clone(),
                    fn_id: n.id.clone(),
                });
            }
        }
        Self { by_name }
    }

    /// Resolve a callee name from a given source file.
    ///
    /// Returns `Some((target_id, resolved))` — `resolved == true` when the
    /// edge points to a real function node, `false` when it falls back to the
    /// legacy bare-name hash.
    pub fn resolve(&self, repo: &str, src_file: &str, callee: &str) -> (String, bool) {
        if let Some(defs) = self.by_name.get(callee) {
            // Same-file definition wins outright.
            if let Some(same) = defs.iter().find(|d| d.file_path == src_file) {
                return (same.fn_id.clone(), true);
            }
            // Exactly one cross-file definition: resolve to it.
            if defs.len() == 1 {
                return (defs[0].fn_id.clone(), true);
            }
            // Ambiguous (multiple cross-file defs, none in this file): dangle.
        }
        let qname = format!("{repo}:fn:{callee}");
        (node_id(repo, "function", &qname), false)
    }
}

/// First pass: extract definitions and gather call sites without resolving.
pub fn extract_defs(repo: &str, rel_path: &Path, source: &str) -> Result<FileDefs> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| anyhow!("failed to set language: {e}"))?;

    let tree: Tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter returned no tree"))?;

    let rel_str = rel_path.to_string_lossy().into_owned();
    let now = Utc::now();

    let file_qname = format!("{repo}:{rel_str}");
    let file_id = node_id(repo, "file", &file_qname);

    let file_node = Node {
        id: file_id.clone(),
        kind: "file".into(),
        name: rel_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string(),
        qualified_name: file_qname,
        repo: repo.to_string(),
        file_path: rel_str.clone(),
        start_line: 0,
        end_line: source.lines().count() as u32,
        attrs: "{}".into(),
        ts: now,
    };

    let mut nodes = vec![file_node];
    let mut contains_edges = Vec::new();
    let mut call_sites = Vec::new();

    let root = tree.root_node();
    walk(
        root,
        source.as_bytes(),
        repo,
        &rel_str,
        &file_id,
        &mut nodes,
        &mut contains_edges,
        &mut call_sites,
        now,
    );

    Ok(FileDefs {
        nodes,
        contains_edges,
        call_sites,
    })
}

/// Second pass: turn each [`CallSite`] into a `calls` edge using [`DefIndex`].
///
/// Returns `(edges, resolved_count)`.
pub fn resolve_call_sites(repo: &str, index: &DefIndex, sites: &[CallSite]) -> (Vec<Edge>, u64) {
    let mut edges = Vec::with_capacity(sites.len());
    let mut resolved = 0u64;
    for s in sites {
        let (target_id, ok) = index.resolve(repo, &s.src_file_path, &s.callee_name);
        if ok {
            resolved += 1;
        }
        edges.push(Edge {
            id: edge_id(&s.src_fn_id, &target_id, "calls"),
            src_id: s.src_fn_id.clone(),
            dst_id: target_id,
            kind: "calls".into(),
            repo: repo.into(),
            attrs: format!(
                r#"{{"callee_name":"{}","resolved":{}}}"#,
                escape_json(&s.callee_name),
                ok
            ),
            ts: s.ts,
        });
    }
    (edges, resolved)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    n: TsNode<'_>,
    src: &[u8],
    repo: &str,
    rel_path: &str,
    file_id: &str,
    nodes: &mut Vec<Node>,
    contains_edges: &mut Vec<Edge>,
    call_sites: &mut Vec<CallSite>,
    now: chrono::DateTime<Utc>,
) {
    match n.kind() {
        "function_item" => {
            if let Some(name) = name_of(n, src, "name") {
                let qname = format!("{repo}:{rel_path}:{name}");
                let id = node_id(repo, "function", &qname);
                nodes.push(Node {
                    id: id.clone(),
                    kind: "function".into(),
                    name: name.clone(),
                    qualified_name: qname,
                    repo: repo.into(),
                    file_path: rel_path.into(),
                    start_line: n.start_position().row as u32,
                    end_line: n.end_position().row as u32,
                    attrs: "{}".into(),
                    ts: now,
                });
                contains_edges.push(Edge {
                    id: edge_id(file_id, &id, "contains"),
                    src_id: file_id.into(),
                    dst_id: id.clone(),
                    kind: "contains".into(),
                    repo: repo.into(),
                    attrs: "{}".into(),
                    ts: now,
                });

                if let Some(body) = n.child_by_field_name("body") {
                    collect_calls(body, src, rel_path, &id, call_sites, now);
                }
                return;
            }
        }
        "struct_item" | "enum_item" | "trait_item" | "mod_item" => {
            if let Some(name) = name_of(n, src, "name") {
                let kind = match n.kind() {
                    "struct_item" => "struct",
                    "enum_item" => "enum",
                    "trait_item" => "trait",
                    "mod_item" => "module",
                    _ => unreachable!(),
                };
                let qname = format!("{repo}:{rel_path}:{name}");
                let id = node_id(repo, kind, &qname);
                nodes.push(Node {
                    id: id.clone(),
                    kind: kind.into(),
                    name: name.clone(),
                    qualified_name: qname,
                    repo: repo.into(),
                    file_path: rel_path.into(),
                    start_line: n.start_position().row as u32,
                    end_line: n.end_position().row as u32,
                    attrs: "{}".into(),
                    ts: now,
                });
                contains_edges.push(Edge {
                    id: edge_id(file_id, &id, "contains"),
                    src_id: file_id.into(),
                    dst_id: id.clone(),
                    kind: "contains".into(),
                    repo: repo.into(),
                    attrs: "{}".into(),
                    ts: now,
                });
            }
        }
        _ => {}
    }

    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        walk(
            child,
            src,
            repo,
            rel_path,
            file_id,
            nodes,
            contains_edges,
            call_sites,
            now,
        );
    }
}

fn collect_calls(
    n: TsNode<'_>,
    src: &[u8],
    rel_path: &str,
    src_function_id: &str,
    call_sites: &mut Vec<CallSite>,
    now: chrono::DateTime<Utc>,
) {
    if n.kind() == "call_expression" {
        if let Some(fn_node) = n.child_by_field_name("function") {
            if let Some(name) = callee_name(fn_node, src) {
                call_sites.push(CallSite {
                    src_fn_id: src_function_id.into(),
                    src_file_path: rel_path.into(),
                    callee_name: name,
                    ts: now,
                });
            }
        }
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        collect_calls(child, src, rel_path, src_function_id, call_sites, now);
    }
}

fn name_of(n: TsNode<'_>, src: &[u8], field: &str) -> Option<String> {
    n.child_by_field_name(field)
        .and_then(|c| c.utf8_text(src).ok())
        .map(|s| s.to_string())
}

/// Resolve the callee name for a `call_expression`'s `function` child.
/// Handles plain identifiers, scoped paths (`a::b::c` → `c`), and field/method
/// access (`foo.bar()` → `bar`).
fn callee_name(n: TsNode<'_>, src: &[u8]) -> Option<String> {
    match n.kind() {
        "identifier" => n.utf8_text(src).ok().map(|s| s.to_string()),
        "scoped_identifier" => n
            .child_by_field_name("name")
            .and_then(|c| c.utf8_text(src).ok())
            .map(|s| s.to_string()),
        "field_expression" => n
            .child_by_field_name("field")
            .and_then(|c| c.utf8_text(src).ok())
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn extracts_function_and_call_sites() {
        let src = r#"
fn greet() { hello(); }
fn hello() {}
"#;
        let out = extract_defs("r", Path::new("a.rs"), src).unwrap();
        let kinds: Vec<&str> = out.nodes.iter().map(|n| n.kind.as_str()).collect();
        assert!(kinds.contains(&"file"));
        assert_eq!(kinds.iter().filter(|k| **k == "function").count(), 2);
        assert_eq!(out.call_sites.len(), 1);
        assert_eq!(out.call_sites[0].callee_name, "hello");
        assert_eq!(
            out.contains_edges.iter().filter(|e| e.kind == "contains").count(),
            2
        );
    }

    #[test]
    fn extracts_struct_and_trait() {
        let src = r#"
struct Foo;
trait Bar {}
"#;
        let out = extract_defs("r", Path::new("a.rs"), src).unwrap();
        let kinds: Vec<String> = out.nodes.iter().map(|n| n.kind.clone()).collect();
        assert!(kinds.iter().any(|k| k == "struct"));
        assert!(kinds.iter().any(|k| k == "trait"));
    }

    #[test]
    fn resolves_same_file_call() {
        let src = r#"
fn greet() { hello(); }
fn hello() {}
"#;
        let a = extract_defs("r", Path::new("a.rs"), src).unwrap();
        let idx = DefIndex::build(&a.nodes);
        let (edges, resolved) = resolve_call_sites("r", &idx, &a.call_sites);
        assert_eq!(resolved, 1);
        let hello_id = node_id("r", "function", "r:a.rs:hello");
        assert_eq!(edges[0].dst_id, hello_id);
        assert!(edges[0].attrs.contains("\"resolved\":true"));
    }

    #[test]
    fn resolves_unique_cross_file_call() {
        let a = extract_defs("r", Path::new("a.rs"), "fn caller() { target(); }").unwrap();
        let b = extract_defs("r", Path::new("b.rs"), "fn target() {}").unwrap();

        let mut all = a.nodes.clone();
        all.extend(b.nodes.clone());
        let idx = DefIndex::build(&all);

        let (edges, resolved) = resolve_call_sites("r", &idx, &a.call_sites);
        assert_eq!(resolved, 1);
        let target_id = node_id("r", "function", "r:b.rs:target");
        assert_eq!(edges[0].dst_id, target_id);
    }

    #[test]
    fn ambiguous_call_dangles() {
        let a = extract_defs("r", Path::new("a.rs"), "fn caller() { dup(); }").unwrap();
        let b = extract_defs("r", Path::new("b.rs"), "fn dup() {}").unwrap();
        let c = extract_defs("r", Path::new("c.rs"), "fn dup() {}").unwrap();

        let mut all = a.nodes.clone();
        all.extend(b.nodes.clone());
        all.extend(c.nodes.clone());
        let idx = DefIndex::build(&all);

        let (edges, resolved) = resolve_call_sites("r", &idx, &a.call_sites);
        assert_eq!(resolved, 0);
        let dangle = node_id("r", "function", "r:fn:dup");
        assert_eq!(edges[0].dst_id, dangle);
        assert!(edges[0].attrs.contains("\"resolved\":false"));
    }
}

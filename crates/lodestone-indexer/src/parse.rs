//! Tree-sitter walk that extracts a small set of node and edge kinds from a
//! single Rust source file.
//!
//! Scope cap (V1):
//! - Nodes: `file`, `function`, `struct`, `enum`, `trait`, `module`
//! - Edges: `contains` (file → item), `calls` (function → callee by name within file)
//!
//! No cross-file resolution. The `dst_id` of a `calls` edge is hashed from the
//! callee's bare name within the same `repo`, so multiple call sites referring
//! to the same name collapse onto the same target — which may or may not
//! exist as a `function` node. That's intentional: dangling edges are fine for
//! the demo and surface honestly in queries.

use std::path::Path;

use anyhow::{anyhow, Result};
use chrono::Utc;
use lodestone_core::{
    ids::{edge_id, node_id},
    Edge, Node,
};
use tree_sitter::{Node as TsNode, Parser, Tree};

pub struct Extracted {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

pub fn extract(repo: &str, rel_path: &Path, source: &str) -> Result<Extracted> {
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
    let mut edges = Vec::new();

    let root = tree.root_node();
    walk(
        root,
        source.as_bytes(),
        repo,
        &rel_str,
        &file_id,
        &mut nodes,
        &mut edges,
        now,
    );

    Ok(Extracted { nodes, edges })
}

#[allow(clippy::too_many_arguments)]
fn walk(
    n: TsNode<'_>,
    src: &[u8],
    repo: &str,
    rel_path: &str,
    file_id: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
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
                edges.push(Edge {
                    id: edge_id(file_id, &id, "contains"),
                    src_id: file_id.into(),
                    dst_id: id.clone(),
                    kind: "contains".into(),
                    repo: repo.into(),
                    attrs: "{}".into(),
                    ts: now,
                });

                // Walk the function body specifically to collect call_expressions
                // attributed to this function as the source.
                if let Some(body) = n.child_by_field_name("body") {
                    collect_calls(body, src, repo, &id, edges, now);
                }
                // Don't recurse further into the function via the generic walker.
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
                edges.push(Edge {
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
        walk(child, src, repo, rel_path, file_id, nodes, edges, now);
    }
}

fn collect_calls(
    n: TsNode<'_>,
    src: &[u8],
    repo: &str,
    src_function_id: &str,
    edges: &mut Vec<Edge>,
    now: chrono::DateTime<Utc>,
) {
    if n.kind() == "call_expression" {
        if let Some(fn_node) = n.child_by_field_name("function") {
            if let Some(name) = callee_name(fn_node, src) {
                // Target qname is just `<repo>:fn:<name>` — stable across call
                // sites referring to the same bare callee. May dangle.
                let target_qname = format!("{repo}:fn:{name}");
                let target_id = node_id(repo, "function", &target_qname);
                edges.push(Edge {
                    id: edge_id(src_function_id, &target_id, "calls"),
                    src_id: src_function_id.into(),
                    dst_id: target_id,
                    kind: "calls".into(),
                    repo: repo.into(),
                    attrs: format!(r#"{{"callee_name":"{}"}}"#, escape_json(&name)),
                    ts: now,
                });
            }
        }
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        collect_calls(child, src, repo, src_function_id, edges, now);
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
    fn extracts_function_and_call() {
        let src = r#"
fn greet() { hello(); }
fn hello() {}
"#;
        let out = extract("r", Path::new("a.rs"), src).unwrap();
        let kinds: Vec<&str> = out.nodes.iter().map(|n| n.kind.as_str()).collect();
        assert!(kinds.contains(&"file"));
        assert_eq!(kinds.iter().filter(|k| **k == "function").count(), 2);
        assert!(out.edges.iter().any(|e| e.kind == "calls"));
        assert_eq!(
            out.edges.iter().filter(|e| e.kind == "contains").count(),
            2
        );
    }

    #[test]
    fn extracts_struct_and_trait() {
        let src = r#"
struct Foo;
trait Bar {}
"#;
        let out = extract("r", Path::new("a.rs"), src).unwrap();
        let kinds: Vec<String> = out.nodes.iter().map(|n| n.kind.clone()).collect();
        assert!(kinds.iter().any(|k| k == "struct"));
        assert!(kinds.iter().any(|k| k == "trait"));
    }
}

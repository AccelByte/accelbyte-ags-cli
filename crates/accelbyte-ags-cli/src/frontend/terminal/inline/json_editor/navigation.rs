//! Flat-navigation index over an expanded/collapsed node tree.
//!
//! The tree itself is hierarchical, but ↑/↓ row navigation needs the
//! visible-row order as a flat list of paths. [`flatten`] produces that
//! list honouring each node's `expanded` flag; [`get`] / [`get_mut`]
//! resolve a path back to a node reference.

use super::node::{Node, NodeKind};

/// Path to a node from the root, indexed by child position at each
/// level. An empty path refers to the root node itself.
pub type NodePath = Vec<usize>;

/// Visible (in the current expand state) ordered list of node paths.
pub fn flatten(root: &Node) -> Vec<NodePath> {
    let mut out = Vec::new();
    walk(root, &mut Vec::new(), &mut out);
    out
}

/// Depth-first walk collecting the path to every visible node into `out`,
/// descending into a container only when it is expanded.
fn walk(node: &Node, prefix: &mut NodePath, out: &mut Vec<NodePath>) {
    out.push(prefix.clone());
    if !node.expanded {
        return;
    }
    let children: &[Node] = match &node.kind {
        NodeKind::Object { children } | NodeKind::Array { children } => children,
        _ => return,
    };
    for (i, child) in children.iter().enumerate() {
        prefix.push(i);
        walk(child, prefix, out);
        prefix.pop();
    }
}

/// Resolve `path` from `root` to a node reference, or `None` if the path
/// runs off the end of a non-container kind.
pub fn get<'a>(root: &'a Node, path: &NodePath) -> Option<&'a Node> {
    let mut node = root;
    for &i in path {
        let children = match &node.kind {
            NodeKind::Object { children } | NodeKind::Array { children } => children,
            _ => return None,
        };
        node = children.get(i)?;
    }
    Some(node)
}

/// Mutable counterpart of [`get`].
pub fn get_mut<'a>(root: &'a mut Node, path: &NodePath) -> Option<&'a mut Node> {
    let mut node = root;
    for &i in path {
        let children = match &mut node.kind {
            NodeKind::Object { children } | NodeKind::Array { children } => children,
            _ => return None,
        };
        node = children.get_mut(i)?;
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::json_editor::node::from_schema;

    #[test]
    fn test_flatten_expanded_object_includes_all_descendants() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" }, "b": { "type": "integer" } }
        });
        let root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let paths = flatten(&root);
        // Root + 2 children = 3 visible rows.
        assert_eq!(paths.len(), 3);
    }

    #[test]
    fn test_flatten_collapsed_object_hides_children() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } }
        });
        let mut root = from_schema("root", &schema, &serde_json::Value::Null, true);
        root.expanded = false;
        let paths = flatten(&root);
        assert_eq!(paths.len(), 1); // just root
    }

    #[test]
    fn test_flatten_root_path_is_empty() {
        let schema = serde_json::json!({ "type": "string" });
        let root = from_schema("name", &schema, &serde_json::Value::Null, true);
        let paths = flatten(&root);
        assert_eq!(paths, vec![Vec::<usize>::new()]);
    }

    #[test]
    fn test_get_resolves_nested_path() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "object", "properties": { "y": { "type": "string" } } }
            }
        });
        let root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let inner = get(&root, &vec![0, 0]).expect("nested path resolves");
        assert_eq!(inner.name, "y");
    }

    #[test]
    fn test_get_returns_none_for_path_off_end_of_scalar() {
        let schema = serde_json::json!({ "type": "string" });
        let root = from_schema("name", &schema, &serde_json::Value::Null, true);
        assert!(get(&root, &vec![0]).is_none());
    }

    #[test]
    fn test_get_mut_allows_in_place_edit() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } }
        });
        let mut root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let child = get_mut(&mut root, &vec![0]).expect("child present");
        child.expanded = false;
        assert!(!get(&root, &vec![0]).unwrap().expanded);
    }

    #[test]
    fn test_flatten_array_children_are_flattened_when_array_expanded() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let value = serde_json::json!(["a", "b"]);
        let root = from_schema("items", &schema, &value, false);
        let paths = flatten(&root);
        assert_eq!(paths.len(), 3); // root + 2 array entries
    }
}

use tree_sitter::Node;

use super::{extract_node_text, BindingDescriptor};

pub(super) fn extract_rust_binding_descriptors<'tree>(
    node: Node<'tree>,
    source_bytes: &[u8],
) -> Vec<BindingDescriptor<'tree>> {
    if node.kind() != "let_declaration" {
        return Vec::new();
    }

    let Some(pattern_node) = node.child_by_field_name("pattern") else {
        return Vec::new();
    };
    if pattern_node.kind() != "identifier" {
        return Vec::new();
    }
    if extract_node_text(pattern_node, source_bytes).is_empty() {
        return Vec::new();
    }

    vec![BindingDescriptor {
        display_node: node,
        initializer_node: node.child_by_field_name("value"),
        name_node: pattern_node,
    }]
}

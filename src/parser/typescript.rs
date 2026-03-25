use tree_sitter::Node;

use super::{extract_node_text, BindingDescriptor};

pub(super) fn extract_typescript_binding_descriptors<'tree>(
    node: Node<'tree>,
    source_bytes: &[u8],
) -> Vec<BindingDescriptor<'tree>> {
    if node.kind() != "variable_declarator" {
        return Vec::new();
    }

    let Some(name_node) = node.child_by_field_name("name") else {
        return Vec::new();
    };
    if name_node.kind() != "identifier" {
        return Vec::new();
    }

    let mut display_node = node;
    if let Some(parent_node) = node.parent() {
        if parent_node.kind() == "variable_declaration" {
            display_node = parent_node;
            if let Some(grand_parent_node) = parent_node.parent() {
                if grand_parent_node.kind() == "lexical_declaration" {
                    display_node = grand_parent_node;
                }
            }
        } else if parent_node.kind() == "lexical_declaration" {
            display_node = parent_node;
        }
    }

    if extract_node_text(name_node, source_bytes).is_empty() {
        return Vec::new();
    }

    vec![BindingDescriptor {
        display_node,
        initializer_node: node.child_by_field_name("value"),
        name_node,
    }]
}

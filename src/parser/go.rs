use tree_sitter::Node;

use super::{collect_named_children, extract_node_text, BindingDescriptor};

pub(super) fn extract_go_binding_descriptors<'tree>(
    node: Node<'tree>,
    source_bytes: &[u8],
) -> Vec<BindingDescriptor<'tree>> {
    match node.kind() {
        "short_var_declaration" => {
            let Some(left_node) = node.child_by_field_name("left") else {
                return Vec::new();
            };
            let Some(right_node) = node.child_by_field_name("right") else {
                return Vec::new();
            };
            let left_items = collect_named_children(left_node);
            let right_items = collect_named_children(right_node);

            left_items
                .into_iter()
                .enumerate()
                .filter_map(|(index, name_node)| {
                    if name_node.kind() != "identifier" {
                        return None;
                    }
                    if extract_node_text(name_node, source_bytes).is_empty() {
                        return None;
                    }

                    Some(BindingDescriptor {
                        display_node: node,
                        initializer_node: right_items.get(index).copied(),
                        name_node,
                    })
                })
                .collect()
        }
        "var_spec" => {
            let value_items = node
                .child_by_field_name("value")
                .map(collect_named_children)
                .unwrap_or_default();
            let mut value_index = 0usize;
            let mut descriptors = Vec::new();

            for child in collect_named_children(node) {
                if child.kind() != "identifier" {
                    continue;
                }
                if extract_node_text(child, source_bytes).is_empty() {
                    continue;
                }

                descriptors.push(BindingDescriptor {
                    display_node: node,
                    initializer_node: value_items.get(value_index).copied(),
                    name_node: child,
                });
                value_index += 1;
            }

            descriptors
        }
        _ => Vec::new(),
    }
}

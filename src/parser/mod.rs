mod go;
mod rust;
mod typescript;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use self::go::extract_go_binding_descriptors;
use self::rust::extract_rust_binding_descriptors;
use self::typescript::extract_typescript_binding_descriptors;
use tree_sitter::{Node, Parser};

use crate::model::{
    build_target_id, NamedText, SearchTarget, SearchTargetKind, SupportedLanguage,
    TraceReference, TraceStep,
};
use crate::text::{condense_whitespace, tokenize_text, truncate_text};

pub(crate) struct FileAnalysis {
    pub targets: Vec<SearchTarget>,
    pub warning_count: usize,
}

struct SourceContext<'a> {
    language: SupportedLanguage,
    relative_file_path: &'a Path,
    source_bytes: &'a [u8],
    source_lines: &'a [String],
    source: &'a str,
}

#[derive(Clone, Copy)]
pub(super) struct BindingDescriptor<'tree> {
    pub(super) display_node: Node<'tree>,
    pub(super) initializer_node: Option<Node<'tree>>,
    pub(super) name_node: Node<'tree>,
}

struct CallReference {
    display_label: String,
    simple_name: String,
    line_start: usize,
    line_end: usize,
    snippet: String,
}

pub(crate) fn analyze_file(
    search_root_path: &Path,
    absolute_file_path: &Path,
) -> Result<FileAnalysis, std::io::Error> {
    let language = match SupportedLanguage::from_path(absolute_file_path) {
        Some(language) => language,
        None => {
            return Ok(FileAnalysis {
                targets: Vec::new(),
                warning_count: 0,
            });
        }
    };

    let source = fs::read_to_string(absolute_file_path)?;
    let relative_file_path = make_relative_path(search_root_path, absolute_file_path);
    let source_lines = source.lines().map(str::to_string).collect::<Vec<_>>();
    let source_bytes = source.as_bytes();
    let context = SourceContext {
        language,
        relative_file_path: &relative_file_path,
        source_bytes,
        source_lines: &source_lines,
        source: &source,
    };

    let mut parser = Parser::new();
    let grammar = language_grammar(language);
    parser
        .set_language(&grammar)
        .expect("supported tree-sitter grammar should load");

    let parsed_tree = match parser.parse(&source, None) {
        Some(parsed_tree) => parsed_tree,
        None => {
            return Ok(FileAnalysis {
                targets: vec![build_fallback_target(&context)],
                warning_count: 1,
            });
        }
    };

    let mut targets = Vec::new();
    collect_targets(parsed_tree.root_node(), &context, &mut targets);

    if targets.is_empty() {
        targets.push(build_fallback_target(&context));
    }

    populate_sibling_context(parsed_tree.root_node(), &context, &mut targets);

    let warning_count = if parsed_tree.root_node().has_error() { 1 } else { 0 };

    Ok(FileAnalysis {
        targets,
        warning_count,
    })
}

fn collect_targets(
    node: Node<'_>,
    context: &SourceContext<'_>,
    targets: &mut Vec<SearchTarget>,
) {
    if let Some(target_kind) = classify_primary_target_kind(node, context.language) {
        if let Some(primary_target) = build_primary_target(node, target_kind, context) {
            let local_binding_targets = if target_kind.is_callable() {
                build_local_binding_targets(node, &primary_target, context)
            } else {
                Vec::new()
            };

            targets.push(primary_target);
            targets.extend(local_binding_targets);
        }
    }

    for child in collect_named_children(node) {
        collect_targets(child, context, targets);
    }
}

fn build_primary_target(
    node: Node<'_>,
    target_kind: SearchTargetKind,
    context: &SourceContext<'_>,
) -> Option<SearchTarget> {
    let symbol_name = extract_symbol_name(node, context.source_bytes)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let node_text = extract_node_text(node, context.source_bytes);
    let comment_text = collect_preceding_comments(context.source_lines, node.start_position().row + 1);
    let signature_text = extract_signature_text(&node_text);
    let declaration_snippet = build_single_line_snippet(&signature_text);
    let token_text = tokenize_text(&node_text).join(" ");
    let parameter_descriptions = collect_parameter_descriptions(
        node.child_by_field_name("parameters"),
        context.source_bytes,
    );
    let call_references = if target_kind.is_callable() {
        match node.child_by_field_name("body") {
            Some(body_node) => collect_call_references_in_scope(body_node, context),
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let outgoing_dependencies = call_references
        .iter()
        .map(call_reference_to_dependency)
        .collect::<Vec<_>>();
    let call_names = dedup_strings_preserve_order(
        call_references
            .iter()
            .map(|reference| reference.simple_name.clone())
            .collect::<Vec<_>>(),
    );

    let mut signature_search_parts = vec![signature_text.clone()];
    signature_search_parts.extend(
        parameter_descriptions
            .iter()
            .map(|description| description.text.clone()),
    );
    let mut context_search_parts = vec![comment_text.clone(), token_text];
    context_search_parts.extend(call_names.iter().cloned());

    let doc_comment = if comment_text.is_empty() {
        None
    } else {
        Some(comment_text.clone())
    };
    let semantic_role = classify_semantic_role(node, &symbol_name, context.language, context.source_bytes);
    let import_hint = if matches!(target_kind, SearchTargetKind::Function | SearchTargetKind::Type)
    {
        build_import_hint(
            &symbol_name,
            context.relative_file_path,
            context.language,
            target_kind,
        )
    } else {
        None
    };
    let symbol_name_search_text = build_search_text(&[symbol_name.clone()]);
    let signature_search_text = build_search_text(&signature_search_parts);
    let context_search_text = build_search_text(&context_search_parts);
    let target_id = build_target_id(
        context.relative_file_path,
        line_start,
        line_end,
        target_kind,
        &symbol_name,
    );

    Some(SearchTarget {
        target_id,
        file_path: context.relative_file_path.to_path_buf(),
        language: context.language,
        target_kind,
        symbol_name,
        parent_symbol_name: None,
        line_start,
        line_end,
        symbol_name_search_text,
        signature_search_text,
        context_search_text,
        declaration_snippet,
        signature_text: if signature_text.is_empty() {
            None
        } else {
            Some(signature_text)
        },
        return_type_hint: extract_return_type_hint(node, context.source_bytes),
        parameter_descriptions,
        incoming_dependencies: Vec::new(),
        outgoing_dependencies,
        flow_steps: Vec::new(),
        call_names,
        doc_comment,
        semantic_role,
        sibling_symbol_names: Vec::new(),
        container_name: None,
        import_hint,
    })
}

fn build_local_binding_targets(
    callable_node: Node<'_>,
    callable_target: &SearchTarget,
    context: &SourceContext<'_>,
) -> Vec<SearchTarget> {
    let Some(body_node) = callable_node.child_by_field_name("body") else {
        return Vec::new();
    };

    let parameter_descriptions = collect_parameter_descriptions(
        callable_node.child_by_field_name("parameters"),
        context.source_bytes,
    );
    let parameter_map = parameter_descriptions
        .iter()
        .map(|description| (description.name.clone(), description.text.clone()))
        .collect::<HashMap<_, _>>();

    let mut binding_descriptors = Vec::new();
    collect_local_binding_descriptors(body_node, body_node, context, &mut binding_descriptors);

    binding_descriptors
        .into_iter()
        .filter_map(|descriptor| {
            build_local_binding_target(
                body_node,
                callable_target,
                descriptor,
                &parameter_map,
                context,
            )
        })
        .collect()
}

fn build_local_binding_target(
    body_node: Node<'_>,
    callable_target: &SearchTarget,
    descriptor: BindingDescriptor<'_>,
    parameter_map: &HashMap<String, String>,
    context: &SourceContext<'_>,
) -> Option<SearchTarget> {
    let symbol_name = extract_node_text(descriptor.name_node, context.source_bytes);
    if symbol_name.is_empty() {
        return None;
    }

    let line_start = descriptor.display_node.start_position().row + 1;
    let line_end = descriptor.display_node.end_position().row + 1;
    let declaration_text = extract_node_text(descriptor.display_node, context.source_bytes);
    let declaration_snippet = build_single_line_snippet(&declaration_text);
    let explicit_type_hint = extract_explicit_binding_type_hint(descriptor.display_node, context.source_bytes);
    let initializer_text = descriptor
        .initializer_node
        .map(|node| extract_node_text(node, context.source_bytes))
        .unwrap_or_default();
    let outgoing_call_references = descriptor
        .initializer_node
        .map(|node| collect_call_references_in_expression(node, context))
        .unwrap_or_default();
    let outgoing_dependencies = outgoing_call_references
        .iter()
        .map(call_reference_to_dependency)
        .collect::<Vec<_>>();
    let call_names = dedup_strings_preserve_order(
        outgoing_call_references
            .iter()
            .map(|reference| reference.simple_name.clone())
            .collect::<Vec<_>>(),
    );
    let incoming_dependencies = descriptor
        .initializer_node
        .map(|node| collect_incoming_dependencies(node, &symbol_name, parameter_map, context))
        .unwrap_or_default();
    let flow_steps = collect_local_binding_flow_steps(
        body_node,
        descriptor.display_node,
        &symbol_name,
        context,
    );

    let mut context_search_parts = vec![
        declaration_text.clone(),
        initializer_text.clone(),
        callable_target.symbol_name.clone(),
    ];
    if let Some(type_hint) = explicit_type_hint.clone() {
        context_search_parts.push(type_hint);
    }
    context_search_parts.extend(flow_steps.iter().map(|step| step.label.clone()));
    context_search_parts.extend(
        incoming_dependencies
            .iter()
            .map(|dependency| dependency.label.clone()),
    );
    context_search_parts.extend(
        outgoing_dependencies
            .iter()
            .map(|dependency| dependency.label.clone()),
    );
    context_search_parts.push(tokenize_text(&declaration_text).join(" "));
    context_search_parts.push(tokenize_text(&initializer_text).join(" "));

    let symbol_name_search_text = build_search_text(&[symbol_name.clone()]);
    let signature_search_text = build_search_text(
        &explicit_type_hint
            .clone()
            .into_iter()
            .collect::<Vec<_>>(),
    );
    let context_search_text = build_search_text(&context_search_parts);
    let target_id = build_target_id(
        context.relative_file_path,
        line_start,
        line_end,
        SearchTargetKind::LocalBinding,
        &symbol_name,
    );

    Some(SearchTarget {
        target_id,
        file_path: context.relative_file_path.to_path_buf(),
        language: context.language,
        target_kind: SearchTargetKind::LocalBinding,
        symbol_name,
        parent_symbol_name: Some(callable_target.symbol_name.clone()),
        line_start,
        line_end,
        symbol_name_search_text,
        signature_search_text,
        context_search_text,
        declaration_snippet,
        signature_text: None,
        return_type_hint: explicit_type_hint,
        parameter_descriptions: Vec::new(),
        incoming_dependencies,
        outgoing_dependencies,
        flow_steps,
        call_names,
        doc_comment: None,
        semantic_role: None,
        sibling_symbol_names: Vec::new(),
        container_name: None,
        import_hint: None,
    })
}

fn collect_local_binding_descriptors<'tree>(
    node: Node<'tree>,
    scope_root: Node<'tree>,
    context: &SourceContext<'_>,
    binding_descriptors: &mut Vec<BindingDescriptor<'tree>>,
) {
    if node != scope_root && is_nested_scope(node, context.language) {
        return;
    }

    binding_descriptors.extend(extract_binding_descriptors(node, context));

    for child in collect_named_children(node) {
        collect_local_binding_descriptors(child, scope_root, context, binding_descriptors);
    }
}

fn extract_binding_descriptors<'tree>(
    node: Node<'tree>,
    context: &SourceContext<'_>,
) -> Vec<BindingDescriptor<'tree>> {
    match context.language {
        SupportedLanguage::TypeScript => extract_typescript_binding_descriptors(node, context.source_bytes),
        SupportedLanguage::Go => extract_go_binding_descriptors(node, context.source_bytes),
        SupportedLanguage::Rust => extract_rust_binding_descriptors(node, context.source_bytes),
    }
}

fn collect_local_binding_flow_steps(
    body_node: Node<'_>,
    declaration_node: Node<'_>,
    binding_name: &str,
    context: &SourceContext<'_>,
) -> Vec<TraceStep> {
    let mut flow_steps = Vec::new();
    collect_direct_binding_use_steps(
        body_node,
        body_node,
        declaration_node,
        binding_name,
        context,
        &mut flow_steps,
    );

    if context.language == SupportedLanguage::TypeScript {
        collect_typescript_chain_flow_steps(
            body_node,
            body_node,
            declaration_node,
            binding_name,
            context,
            &mut flow_steps,
        );
    }

    dedup_trace_steps(flow_steps)
}

fn collect_direct_binding_use_steps(
    node: Node<'_>,
    scope_root: Node<'_>,
    declaration_node: Node<'_>,
    binding_name: &str,
    context: &SourceContext<'_>,
    flow_steps: &mut Vec<TraceStep>,
) {
    if node != scope_root && is_nested_scope(node, context.language) {
        return;
    }

    if is_identifier_like(node.kind()) {
        let identifier_text = extract_node_text(node, context.source_bytes);
        if identifier_text == binding_name && !node_is_within(node, declaration_node) {
            if let Some(container_node) = find_relevant_flow_container(node, scope_root) {
                if container_node.start_position().row + 1 >= declaration_node.start_position().row + 1 {
                    if let Some(flow_step) = build_flow_step_from_node(container_node, context) {
                        flow_steps.push(flow_step);
                    }
                }
            }
        }
    }

    for child in collect_named_children(node) {
        collect_direct_binding_use_steps(
            child,
            scope_root,
            declaration_node,
            binding_name,
            context,
            flow_steps,
        );
    }
}

fn collect_typescript_chain_flow_steps(
    node: Node<'_>,
    scope_root: Node<'_>,
    declaration_node: Node<'_>,
    binding_name: &str,
    context: &SourceContext<'_>,
    flow_steps: &mut Vec<TraceStep>,
) {
    if node != scope_root && is_nested_scope(node, context.language) {
        return;
    }

    if node.kind() == "identifier"
        && extract_node_text(node, context.source_bytes) == binding_name
        && !node_is_within(node, declaration_node)
    {
        flow_steps.extend(collect_typescript_chain_steps_for_identifier(
            node,
            scope_root,
            context,
        ));
    }

    for child in collect_named_children(node) {
        collect_typescript_chain_flow_steps(
            child,
            scope_root,
            declaration_node,
            binding_name,
            context,
            flow_steps,
        );
    }
}

fn collect_typescript_chain_steps_for_identifier(
    identifier_node: Node<'_>,
    scope_root: Node<'_>,
    context: &SourceContext<'_>,
) -> Vec<TraceStep> {
    let Some(combine_latest_call) = find_ancestor_call_with_name(identifier_node, "combineLatest", context) else {
        return Vec::new();
    };
    let Some(array_node) = find_ancestor_kind(identifier_node, "array", scope_root) else {
        return Vec::new();
    };
    let Some(binding_position) = find_child_position(array_node, identifier_node) else {
        return Vec::new();
    };
    let Some(outer_container) = find_outer_flow_container(combine_latest_call, scope_root) else {
        return Vec::new();
    };

    let mut flow_steps = Vec::new();
    let operator_calls = collect_flow_operator_calls(outer_container, context);
    for operator_call in operator_calls {
        if let Some(flow_step) = build_flow_step_from_node(operator_call, context) {
            flow_steps.push(flow_step);
        }
    }

    if let Some((alias_name, callback_body)) =
        resolve_typescript_map_alias(outer_container, binding_position, context)
    {
        collect_alias_use_steps(
            callback_body,
            callback_body,
            &alias_name,
            context,
            &mut flow_steps,
        );
    }

    flow_steps
}

fn collect_alias_use_steps(
    node: Node<'_>,
    scope_root: Node<'_>,
    alias_name: &str,
    context: &SourceContext<'_>,
    flow_steps: &mut Vec<TraceStep>,
) {
    if node != scope_root && is_nested_scope(node, context.language) {
        return;
    }

    if is_identifier_like(node.kind()) && extract_node_text(node, context.source_bytes) == alias_name {
        if let Some(container_node) = find_relevant_flow_container(node, scope_root) {
            if let Some(flow_step) = build_flow_step_from_node(container_node, context) {
                flow_steps.push(flow_step);
            }
        }
    }

    for child in collect_named_children(node) {
        collect_alias_use_steps(child, scope_root, alias_name, context, flow_steps);
    }
}

fn collect_incoming_dependencies(
    expression_node: Node<'_>,
    binding_name: &str,
    parameter_map: &HashMap<String, String>,
    context: &SourceContext<'_>,
) -> Vec<TraceReference> {
    let mut dependencies = Vec::new();
    collect_incoming_dependencies_in_node(
        expression_node,
        binding_name,
        parameter_map,
        context,
        &mut dependencies,
    );

    dedup_trace_references(dependencies)
}

fn collect_incoming_dependencies_in_node(
    node: Node<'_>,
    binding_name: &str,
    parameter_map: &HashMap<String, String>,
    context: &SourceContext<'_>,
    dependencies: &mut Vec<TraceReference>,
) {
    match node.kind() {
        "call_expression" => {
            if let Some(arguments_node) = node.child_by_field_name("arguments") {
                collect_incoming_dependencies_in_node(
                    arguments_node,
                    binding_name,
                    parameter_map,
                    context,
                    dependencies,
                );
            }
            return;
        }
        "member_expression" | "field_expression" | "selector_expression" => {
            if let Some(base_node) = member_base_node(node, context.language) {
                collect_incoming_dependencies_in_node(
                    base_node,
                    binding_name,
                    parameter_map,
                    context,
                    dependencies,
                );
            }
            return;
        }
        _ => {}
    }

    if is_identifier_like(node.kind()) {
        let identifier_text = extract_node_text(node, context.source_bytes);
        if identifier_text.is_empty()
            || identifier_text == binding_name
            || identifier_text == "this"
            || identifier_text == "self"
        {
            return;
        }

        let label = parameter_map
            .get(&identifier_text)
            .cloned()
            .unwrap_or(identifier_text.clone());
        dependencies.push(TraceReference {
            label: build_single_line_snippet(&label),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            snippet: build_single_line_snippet(&identifier_text),
            detail: None,
        });
        return;
    }

    for child in collect_named_children(node) {
        collect_incoming_dependencies_in_node(
            child,
            binding_name,
            parameter_map,
            context,
            dependencies,
        );
    }
}

fn collect_call_references_in_scope(
    scope_node: Node<'_>,
    context: &SourceContext<'_>,
) -> Vec<CallReference> {
    let mut call_references = Vec::new();
    collect_call_references_in_scope_node(scope_node, scope_node, context, &mut call_references);
    dedup_call_references(call_references)
}

fn collect_call_references_in_scope_node(
    node: Node<'_>,
    scope_root: Node<'_>,
    context: &SourceContext<'_>,
    call_references: &mut Vec<CallReference>,
) {
    if node != scope_root && is_nested_scope(node, context.language) {
        return;
    }

    if let Some(call_reference) = extract_call_reference(node, context) {
        call_references.push(call_reference);
    }

    for child in collect_named_children(node) {
        collect_call_references_in_scope_node(child, scope_root, context, call_references);
    }
}

fn collect_call_references_in_expression(
    node: Node<'_>,
    context: &SourceContext<'_>,
) -> Vec<CallReference> {
    let mut call_references = Vec::new();
    collect_call_references_in_expression_node(node, context, &mut call_references);
    dedup_call_references(call_references)
}

fn collect_call_references_in_expression_node(
    node: Node<'_>,
    context: &SourceContext<'_>,
    call_references: &mut Vec<CallReference>,
) {
    if let Some(call_reference) = extract_call_reference(node, context) {
        call_references.push(call_reference);
    }

    for child in collect_named_children(node) {
        collect_call_references_in_expression_node(child, context, call_references);
    }
}

fn extract_call_reference(
    node: Node<'_>,
    context: &SourceContext<'_>,
) -> Option<CallReference> {
    if node.kind() != "call_expression" {
        return None;
    }

    let function_node = node.child_by_field_name("function")?;
    let simple_name = extract_terminal_identifier(function_node, context.source_bytes)?;
    let display_name = normalize_call_display_name(&extract_node_text(function_node, context.source_bytes));

    Some(CallReference {
        display_label: format!("{display_name}()"),
        simple_name,
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        snippet: build_single_line_snippet(&extract_node_text(node, context.source_bytes)),
    })
}

fn call_reference_to_dependency(call_reference: &CallReference) -> TraceReference {
    TraceReference {
        label: call_reference.display_label.clone(),
        line_start: call_reference.line_start,
        line_end: call_reference.line_end,
        snippet: call_reference.snippet.clone(),
        detail: None,
    }
}

fn collect_parameter_descriptions(
    parameters_node: Option<Node<'_>>,
    source_bytes: &[u8],
) -> Vec<NamedText> {
    let Some(parameters_node) = parameters_node else {
        return Vec::new();
    };

    let mut descriptions = Vec::new();
    let mut seen_names = HashSet::new();

    for child in collect_named_children(parameters_node) {
        let Some(parameter_name) = extract_parameter_name(child, source_bytes) else {
            continue;
        };
        if !seen_names.insert(parameter_name.clone()) {
            continue;
        }

        descriptions.push(NamedText {
            name: parameter_name,
            text: build_single_line_snippet(&extract_node_text(child, source_bytes)),
        });
    }

    descriptions
}

fn extract_parameter_name(node: Node<'_>, source_bytes: &[u8]) -> Option<String> {
    for field_name in ["name", "pattern", "parameter"] {
        if let Some(field_node) = node.child_by_field_name(field_name) {
            if let Some(identifier) = extract_first_identifier(field_node, source_bytes) {
                return Some(identifier);
            }
        }
    }

    extract_first_identifier(node, source_bytes)
}

fn extract_return_type_hint(node: Node<'_>, source_bytes: &[u8]) -> Option<String> {
    for field_name in ["return_type", "result", "type"] {
        if let Some(type_node) = node.child_by_field_name(field_name) {
            let type_text = normalize_type_text(&extract_node_text(type_node, source_bytes));
            if !type_text.is_empty() {
                return Some(type_text);
            }
        }
    }

    None
}

fn extract_explicit_binding_type_hint(
    declaration_node: Node<'_>,
    source_bytes: &[u8],
) -> Option<String> {
    if let Some(type_node) = declaration_node.child_by_field_name("type") {
        let type_text = normalize_type_text(&extract_node_text(type_node, source_bytes));
        if !type_text.is_empty() {
            return Some(type_text);
        }
    }

    for child in collect_named_children(declaration_node) {
        if child.kind() == "variable_declarator" {
            if let Some(type_node) = child.child_by_field_name("type") {
                let type_text = normalize_type_text(&extract_node_text(type_node, source_bytes));
                if !type_text.is_empty() {
                    return Some(type_text);
                }
            }
        }
    }

    None
}

fn build_fallback_target(context: &SourceContext<'_>) -> SearchTarget {
    let source_lines = context.source.lines().collect::<Vec<_>>();
    let line_end = source_lines.len().max(1);
    let file_name = context
        .relative_file_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file")
        .to_string();

    let symbol_name_search_text = build_search_text(&[file_name.clone()]);
    let target_id = build_target_id(
        context.relative_file_path,
        1,
        line_end,
        SearchTargetKind::File,
        &file_name,
    );

    SearchTarget {
        target_id,
        file_path: context.relative_file_path.to_path_buf(),
        language: context.language,
        target_kind: SearchTargetKind::File,
        symbol_name: file_name,
        parent_symbol_name: None,
        line_start: 1,
        line_end,
        symbol_name_search_text,
        signature_search_text: String::new(),
        context_search_text: tokenize_text(context.source).join(" "),
        declaration_snippet: truncate_text(&build_single_line_snippet(context.source), 200),
        signature_text: None,
        return_type_hint: None,
        parameter_descriptions: Vec::new(),
        incoming_dependencies: Vec::new(),
        outgoing_dependencies: Vec::new(),
        flow_steps: Vec::new(),
        call_names: Vec::new(),
        doc_comment: None,
        semantic_role: None,
        sibling_symbol_names: Vec::new(),
        container_name: None,
        import_hint: None,
    }
}

fn classify_primary_target_kind(
    node: Node<'_>,
    language: SupportedLanguage,
) -> Option<SearchTargetKind> {
    match language {
        SupportedLanguage::Rust => match node.kind() {
            "function_item" => {
                if is_rust_impl_member(node) {
                    Some(SearchTargetKind::Method)
                } else {
                    Some(SearchTargetKind::Function)
                }
            }
            "struct_item" | "enum_item" | "trait_item" | "type_item" => Some(SearchTargetKind::Type),
            _ => None,
        },
        SupportedLanguage::Go => match node.kind() {
            "function_declaration" => Some(SearchTargetKind::Function),
            "method_declaration" => Some(SearchTargetKind::Method),
            "type_spec" => Some(SearchTargetKind::Type),
            _ => None,
        },
        SupportedLanguage::TypeScript => match node.kind() {
            "function_declaration" => Some(SearchTargetKind::Function),
            "method_definition" => Some(SearchTargetKind::Method),
            "class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration" => Some(SearchTargetKind::Type),
            _ => None,
        },
    }
}

fn is_nested_scope(node: Node<'_>, language: SupportedLanguage) -> bool {
    matches!(
        classify_primary_target_kind(node, language),
        Some(SearchTargetKind::Function | SearchTargetKind::Method)
    )
}

fn is_rust_impl_member(node: Node<'_>) -> bool {
    let Some(parent_node) = node.parent() else {
        return false;
    };
    if parent_node.kind() == "impl_item" {
        return true;
    }

    parent_node
        .parent()
        .is_some_and(|grand_parent_node| grand_parent_node.kind() == "impl_item")
}

fn language_grammar(language: SupportedLanguage) -> tree_sitter::Language {
    match language {
        SupportedLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        SupportedLanguage::Go => tree_sitter_go::LANGUAGE.into(),
        SupportedLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    }
}

fn make_relative_path(search_root_path: &Path, absolute_file_path: &Path) -> PathBuf {
    absolute_file_path
        .strip_prefix(search_root_path)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| absolute_file_path.to_path_buf())
}

fn extract_symbol_name(node: Node<'_>, source_bytes: &[u8]) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let symbol_name = extract_node_text(name_node, source_bytes);
        if !symbol_name.is_empty() {
            return Some(symbol_name);
        }
    }

    extract_first_identifier(node, source_bytes)
}

fn extract_first_identifier(node: Node<'_>, source_bytes: &[u8]) -> Option<String> {
    if is_identifier_like(node.kind()) {
        let identifier = extract_node_text(node, source_bytes);
        if !identifier.is_empty() {
            return Some(identifier);
        }
    }

    for child in collect_named_children(node) {
        if let Some(identifier) = extract_first_identifier(child, source_bytes) {
            return Some(identifier);
        }
    }

    None
}

fn extract_terminal_identifier(node: Node<'_>, source_bytes: &[u8]) -> Option<String> {
    let mut terminal_identifier = None;
    collect_terminal_identifier(node, source_bytes, &mut terminal_identifier);
    terminal_identifier
}

fn collect_terminal_identifier(
    node: Node<'_>,
    source_bytes: &[u8],
    terminal_identifier: &mut Option<String>,
) {
    if is_identifier_like(node.kind()) {
        let identifier = extract_node_text(node, source_bytes);
        if !identifier.is_empty() {
            *terminal_identifier = Some(identifier);
        }
    }

    for child in collect_named_children(node) {
        collect_terminal_identifier(child, source_bytes, terminal_identifier);
    }
}

fn find_relevant_flow_container<'tree>(
    node: Node<'tree>,
    scope_root: Node<'tree>,
) -> Option<Node<'tree>> {
    let mut current_node = node;

    while current_node != scope_root {
        if matches!(
            current_node.kind(),
            "call_expression"
                | "pair"
                | "return_statement"
                | "return_expression"
                | "variable_declarator"
                | "let_declaration"
                | "short_var_declaration"
                | "var_spec"
                | "assignment_expression"
                | "augmented_assignment_expression"
                | "expression_statement"
        ) {
            return Some(current_node);
        }

        current_node = current_node.parent()?;
    }

    Some(scope_root)
}

fn find_outer_flow_container<'tree>(
    node: Node<'tree>,
    scope_root: Node<'tree>,
) -> Option<Node<'tree>> {
    let mut current_node = node;
    let mut outer_container = node;

    while current_node != scope_root {
        if matches!(
            current_node.kind(),
            "return_statement" | "return_expression" | "expression_statement" | "lexical_declaration"
        ) {
            outer_container = current_node;
            break;
        }

        current_node = current_node.parent()?;
    }

    Some(outer_container)
}

fn build_flow_step_from_node(
    node: Node<'_>,
    context: &SourceContext<'_>,
) -> Option<TraceStep> {
    if node.kind() == "call_expression" {
        let function_node = node.child_by_field_name("function")?;
        let function_name = extract_terminal_identifier(function_node, context.source_bytes)?;
        if matches!(function_name.as_str(), "from" | "pipe") {
            return None;
        }
    }

    let node_text = extract_node_text(node, context.source_bytes);
    let label = build_single_line_snippet(&node_text);
    if label.is_empty() {
        return None;
    }

    Some(TraceStep {
        label,
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        snippet: build_display_snippet("", &node_text),
    })
}

fn collect_flow_operator_calls<'tree>(
    node: Node<'tree>,
    context: &SourceContext<'_>,
) -> Vec<Node<'tree>> {
    let mut operator_calls = Vec::new();
    collect_flow_operator_calls_in_node(node, node, context, &mut operator_calls);
    operator_calls
}

fn collect_flow_operator_calls_in_node<'tree>(
    node: Node<'tree>,
    scope_root: Node<'tree>,
    context: &SourceContext<'_>,
    operator_calls: &mut Vec<Node<'tree>>,
) {
    if node != scope_root && is_nested_scope(node, context.language) {
        return;
    }

    if node.kind() == "call_expression" {
        if let Some(function_node) = node.child_by_field_name("function") {
            if let Some(function_name) = extract_terminal_identifier(function_node, context.source_bytes) {
                if matches!(
                    function_name.as_str(),
                    "combineLatest"
                        | "map"
                        | "filter"
                        | "mergeMap"
                        | "switchMap"
                        | "flatMap"
                        | "tap"
                        | "reduce"
                        | "scan"
                        | "withLatestFrom"
                ) {
                    operator_calls.push(node);
                }
            }
        }
    }

    for child in collect_named_children(node) {
        collect_flow_operator_calls_in_node(child, scope_root, context, operator_calls);
    }
}

fn resolve_typescript_map_alias<'tree>(
    outer_container: Node<'tree>,
    binding_position: usize,
    context: &SourceContext<'_>,
) -> Option<(String, Node<'tree>)> {
    for operator_call in collect_flow_operator_calls(outer_container, context) {
        let function_node = operator_call.child_by_field_name("function")?;
        let function_name = extract_terminal_identifier(function_node, context.source_bytes)?;
        if function_name != "map" {
            continue;
        }

        let arguments_node = operator_call.child_by_field_name("arguments")?;
        let callback_node = collect_named_children(arguments_node)
            .into_iter()
            .find(|node| matches!(node.kind(), "arrow_function" | "function_expression"))?;
        let callback_body = callback_node
            .child_by_field_name("body")
            .unwrap_or(callback_node);
        let array_pattern = find_descendant_kind(callback_node, "array_pattern")?;
        let identifiers = collect_named_children(array_pattern)
            .into_iter()
            .filter(|child| child.kind() == "identifier")
            .collect::<Vec<_>>();
        let alias_node = identifiers.get(binding_position).copied()?;
        let alias_name = extract_node_text(alias_node, context.source_bytes);
        if alias_name.is_empty() {
            return None;
        }

        return Some((alias_name, callback_body));
    }

    None
}

fn find_descendant_kind<'tree>(
    node: Node<'tree>,
    target_kind: &str,
) -> Option<Node<'tree>> {
    if node.kind() == target_kind {
        return Some(node);
    }

    for child in collect_named_children(node) {
        if let Some(found_node) = find_descendant_kind(child, target_kind) {
            return Some(found_node);
        }
    }

    None
}

fn find_ancestor_call_with_name<'tree>(
    node: Node<'tree>,
    target_name: &str,
    context: &SourceContext<'_>,
) -> Option<Node<'tree>> {
    let mut current_node = node.parent()?;

    loop {
        if current_node.kind() == "call_expression" {
            if let Some(function_node) = current_node.child_by_field_name("function") {
                if let Some(function_name) = extract_terminal_identifier(function_node, context.source_bytes) {
                    if function_name == target_name {
                        return Some(current_node);
                    }
                }
            }
        }

        current_node = current_node.parent()?;
    }
}

fn find_ancestor_kind<'tree>(
    node: Node<'tree>,
    target_kind: &str,
    stop_node: Node<'tree>,
) -> Option<Node<'tree>> {
    let mut current_node = node.parent()?;

    loop {
        if current_node.kind() == target_kind {
            return Some(current_node);
        }
        if current_node == stop_node {
            return None;
        }

        current_node = current_node.parent()?;
    }
}

fn find_child_position(
    parent_node: Node<'_>,
    child_node: Node<'_>,
) -> Option<usize> {
    collect_named_children(parent_node)
        .iter()
        .position(|candidate| node_is_within(child_node, *candidate))
}

fn member_base_node(
    node: Node<'_>,
    language: SupportedLanguage,
) -> Option<Node<'_>> {
    match language {
        SupportedLanguage::TypeScript => node.child_by_field_name("object").or_else(|| node.child(0)),
        SupportedLanguage::Go => node.child_by_field_name("operand").or_else(|| node.child(0)),
        SupportedLanguage::Rust => node.child_by_field_name("value").or_else(|| node.child(0)),
    }
}

fn node_is_within(node: Node<'_>, container_node: Node<'_>) -> bool {
    node.start_byte() >= container_node.start_byte() && node.end_byte() <= container_node.end_byte()
}

fn normalize_call_display_name(text: &str) -> String {
    condense_whitespace(text).trim_start_matches("this.").to_string()
}

fn normalize_type_text(text: &str) -> String {
    condense_whitespace(text).trim_start_matches(':').trim().to_string()
}

fn build_search_text(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| condense_whitespace(part))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_single_line_snippet(text: &str) -> String {
    truncate_text(&condense_whitespace(text), 180)
}

fn build_display_snippet(comment_text: &str, node_text: &str) -> String {
    let mut snippet_lines = Vec::new();

    if !comment_text.is_empty() {
        snippet_lines.extend(comment_text.lines().take(3).map(condense_whitespace));
    }

    snippet_lines.extend(
        node_text
            .lines()
            .map(condense_whitespace)
            .filter(|line| !line.is_empty())
            .take(4),
    );

    truncate_text(&snippet_lines.join("\n"), 400)
}

fn extract_signature_text(node_text: &str) -> String {
    let mut signature_lines = Vec::new();

    for line in node_text.lines().take(8) {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            if !signature_lines.is_empty() {
                break;
            }
            continue;
        }

        let signature_part = trimmed_line
            .split('{')
            .next()
            .unwrap_or(trimmed_line)
            .trim()
            .trim_end_matches(';')
            .trim();
        if !signature_part.is_empty() {
            signature_lines.push(signature_part.to_string());
        }

        if trimmed_line.contains('{') || trimmed_line.ends_with(';') {
            break;
        }
    }

    condense_whitespace(&signature_lines.join(" "))
}

fn collect_preceding_comments(source_lines: &[String], line_start: usize) -> String {
    if line_start <= 1 {
        return String::new();
    }

    let mut comment_lines = Vec::new();
    let mut current_index = line_start - 1;

    while current_index > 0 {
        let line = source_lines[current_index - 1].trim();
        if line.is_empty() {
            break;
        }

        if line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with("*/")
        {
            comment_lines.push(line.to_string());
            current_index -= 1;
            continue;
        }

        break;
    }

    comment_lines.reverse();
    comment_lines.join("\n")
}

pub(super) fn extract_node_text(node: Node<'_>, source_bytes: &[u8]) -> String {
    node.utf8_text(source_bytes)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

pub(super) fn collect_named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .collect::<Vec<_>>()
}

fn is_identifier_like(node_kind: &str) -> bool {
    matches!(
        node_kind,
        "identifier"
            | "field_identifier"
            | "property_identifier"
            | "private_property_identifier"
            | "type_identifier"
            | "package_identifier"
            | "scoped_identifier"
            | "statement_identifier"
    )
}

fn dedup_strings_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut seen_values = HashSet::new();
    let mut deduplicated_values = Vec::new();

    for value in values {
        if value.is_empty() || !seen_values.insert(value.clone()) {
            continue;
        }

        deduplicated_values.push(value);
    }

    deduplicated_values
}

fn dedup_trace_steps(mut trace_steps: Vec<TraceStep>) -> Vec<TraceStep> {
    trace_steps.sort_by(|left, right| {
        left.line_start
            .cmp(&right.line_start)
            .then_with(|| left.label.cmp(&right.label))
    });

    let mut seen_keys = HashSet::new();
    let mut deduplicated_steps = Vec::new();

    for trace_step in trace_steps {
        let step_key = format!("{}:{}", trace_step.line_start, trace_step.label);
        if !seen_keys.insert(step_key) {
            continue;
        }

        deduplicated_steps.push(trace_step);
    }

    deduplicated_steps
}

fn dedup_trace_references(mut trace_references: Vec<TraceReference>) -> Vec<TraceReference> {
    trace_references.sort_by(|left, right| {
        left.line_start
            .cmp(&right.line_start)
            .then_with(|| left.label.cmp(&right.label))
    });

    let mut seen_keys = HashSet::new();
    let mut deduplicated_references = Vec::new();

    for trace_reference in trace_references {
        let reference_key = trace_reference.label.clone();
        if !seen_keys.insert(reference_key) {
            continue;
        }

        deduplicated_references.push(trace_reference);
    }

    deduplicated_references
}

fn classify_semantic_role(
    node: Node<'_>,
    symbol_name: &str,
    language: SupportedLanguage,
    source_bytes: &[u8],
) -> Option<String> {
    // AST-based: Rust #[test] attribute
    if language == SupportedLanguage::Rust {
        if has_rust_test_attribute(node, source_bytes) {
            return Some("test".to_string());
        }
    }

    // AST-based: Go Test/Benchmark prefix
    if language == SupportedLanguage::Go {
        if symbol_name.starts_with("Test") {
            return Some("test".to_string());
        }
        if symbol_name.starts_with("Benchmark") {
            return Some("benchmark".to_string());
        }
    }

    // Name-pattern-based (universal)
    if symbol_name == "new" || symbol_name.starts_with("new_") || symbol_name == "constructor" {
        return Some("constructor".to_string());
    }
    if symbol_name.starts_with("test_") {
        return Some("test".to_string());
    }
    if symbol_name.starts_with("build") {
        return Some("builder".to_string());
    }
    if symbol_name.starts_with("handle_") || symbol_name.starts_with("on_") {
        return Some("handler".to_string());
    }
    if symbol_name.starts_with("from_") || symbol_name.starts_with("into_") || symbol_name.starts_with("to_") {
        return Some("converter".to_string());
    }
    if symbol_name.starts_with("get_") {
        return Some("getter".to_string());
    }
    if symbol_name.starts_with("set_") {
        return Some("setter".to_string());
    }

    None
}

fn has_rust_test_attribute(node: Node<'_>, source_bytes: &[u8]) -> bool {
    let mut sibling = node.prev_named_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "attribute_item" {
            let text = extract_node_text(sib, source_bytes);
            if text.contains("test") {
                return true;
            }
            sibling = sib.prev_named_sibling();
        } else {
            break;
        }
    }
    false
}

fn build_import_hint(
    symbol_name: &str,
    relative_file_path: &Path,
    language: SupportedLanguage,
    target_kind: SearchTargetKind,
) -> Option<String> {
    let path_str = relative_file_path.to_str()?;

    match language {
        SupportedLanguage::Rust => {
            if target_kind == SearchTargetKind::Method {
                return None;
            }
            // src/service.rs + CodeSearchService -> use crate::service::CodeSearchService;
            let module_path = path_str
                .trim_start_matches("src/")
                .trim_end_matches(".rs")
                .replace('/', "::");
            let module_path = module_path.trim_end_matches("::mod").to_string();
            let module_path = if module_path == "lib" || module_path == "main" {
                return Some(format!("use crate::{symbol_name};"));
            } else {
                module_path
            };
            Some(format!("use crate::{module_path}::{symbol_name};"))
        }
        SupportedLanguage::TypeScript => {
            if target_kind == SearchTargetKind::Method {
                return None;
            }
            let module_path = path_str
                .trim_end_matches(".ts")
                .trim_end_matches(".tsx");
            Some(format!("import {{ {symbol_name} }} from './{module_path}';"))
        }
        SupportedLanguage::Go => None,
    }
}

struct ContainerBlock {
    line_start: usize,
    line_end: usize,
    container_name: String,
    member_names: Vec<String>,
}

fn populate_sibling_context(
    root_node: Node<'_>,
    context: &SourceContext<'_>,
    targets: &mut Vec<SearchTarget>,
) {
    let container_blocks = collect_container_blocks(root_node, context);

    for target in targets.iter_mut() {
        for block in &container_blocks {
            if target.line_start >= block.line_start && target.line_end <= block.line_end {
                target.container_name = Some(block.container_name.clone());
                if target.target_kind == SearchTargetKind::Method {
                    target.parent_symbol_name = Some(block.container_name.clone());
                }
                target.sibling_symbol_names = block
                    .member_names
                    .iter()
                    .filter(|name| *name != &target.symbol_name)
                    .cloned()
                    .collect();
                break;
            }
        }
    }

    // Top-level targets that don't belong to any container: siblings are other top-level targets
    let top_level_names: Vec<String> = targets
        .iter()
        .filter(|t| t.container_name.is_none() && t.target_kind != SearchTargetKind::LocalBinding)
        .map(|t| t.symbol_name.clone())
        .collect();

    for target in targets.iter_mut() {
        if target.container_name.is_none()
            && target.sibling_symbol_names.is_empty()
            && target.target_kind != SearchTargetKind::LocalBinding
        {
            target.sibling_symbol_names = top_level_names
                .iter()
                .filter(|name| *name != &target.symbol_name)
                .cloned()
                .collect();
        }
    }
}

fn collect_container_blocks(
    root_node: Node<'_>,
    context: &SourceContext<'_>,
) -> Vec<ContainerBlock> {
    let mut blocks = Vec::new();
    collect_container_blocks_in_node(root_node, context, &mut blocks);
    blocks
}

fn collect_container_blocks_in_node(
    node: Node<'_>,
    context: &SourceContext<'_>,
    blocks: &mut Vec<ContainerBlock>,
) {
    match context.language {
        SupportedLanguage::Rust => {
            if node.kind() == "impl_item" {
                if let Some(type_node) = node.child_by_field_name("type") {
                    let container_name = extract_node_text(type_node, context.source_bytes);
                    if !container_name.is_empty() {
                        let mut member_names = Vec::new();
                        for child in collect_named_children(node) {
                            if child.kind() == "function_item" {
                                if let Some(name) = extract_symbol_name(child, context.source_bytes) {
                                    member_names.push(name);
                                }
                            }
                        }
                        // Also check declaration_list for impl items
                        if let Some(body) = node.child_by_field_name("body") {
                            for child in collect_named_children(body) {
                                if child.kind() == "function_item" {
                                    if let Some(name) = extract_symbol_name(child, context.source_bytes) {
                                        if !member_names.contains(&name) {
                                            member_names.push(name);
                                        }
                                    }
                                }
                            }
                        }
                        blocks.push(ContainerBlock {
                            line_start: node.start_position().row + 1,
                            line_end: node.end_position().row + 1,
                            container_name,
                            member_names,
                        });
                    }
                }
            }
        }
        SupportedLanguage::TypeScript => {
            if node.kind() == "class_declaration" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let container_name = extract_node_text(name_node, context.source_bytes);
                    if !container_name.is_empty() {
                        let mut member_names = Vec::new();
                        if let Some(body) = node.child_by_field_name("body") {
                            for child in collect_named_children(body) {
                                if child.kind() == "method_definition" {
                                    if let Some(name) = extract_symbol_name(child, context.source_bytes) {
                                        member_names.push(name);
                                    }
                                }
                            }
                        }
                        blocks.push(ContainerBlock {
                            line_start: node.start_position().row + 1,
                            line_end: node.end_position().row + 1,
                            container_name,
                            member_names,
                        });
                    }
                }
            }
        }
        SupportedLanguage::Go => {
            // Go uses receiver-based methods, grouping is complex; skip for now
        }
    }

    for child in collect_named_children(node) {
        collect_container_blocks_in_node(child, context, blocks);
    }
}

fn dedup_call_references(mut call_references: Vec<CallReference>) -> Vec<CallReference> {
    call_references.sort_by(|left, right| {
        left.line_start
            .cmp(&right.line_start)
            .then_with(|| left.display_label.cmp(&right.display_label))
    });

    let mut seen_keys = HashSet::new();
    let mut deduplicated_references = Vec::new();

    for call_reference in call_references {
        let reference_key = call_reference.display_label.clone();
        if !seen_keys.insert(reference_key) {
            continue;
        }

        deduplicated_references.push(call_reference);
    }

    deduplicated_references
}

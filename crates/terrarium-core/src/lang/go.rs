use super::*;

fn is_string(kind: &str) -> bool {
    kind == "interpreted_string_literal" || kind == "raw_string_literal"
}

pub fn extract(root: Node, src: &[u8]) -> FileFacts {
    let mut facts = FileFacts::default();
    let mut stack = DefStack::new();
    walk(root, src, &mut facts, &mut stack);
    facts
}

fn walk(node: Node, src: &[u8], facts: &mut FileFacts, stack: &mut DefStack) {
    let kind = node.kind();
    let mut pushed = false;
    match kind {
        "import_spec" => {
            if let Some(p) = node.child_by_field_name("path") {
                facts.imports.push(Import {
                    module: unquote(text(p, src)),
                    names: vec![],
                    line: line(node),
                });
            }
        }
        "function_declaration" => {
            if let Some(name) = field_text(node, "name", src) {
                stack.push(
                    facts,
                    Def {
                        name: name.into(),
                        kind: SymbolKind::Function,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: vec![],
                    },
                );
                pushed = true;
            }
        }
        "method_declaration" => {
            if let Some(name) = field_text(node, "name", src) {
                let recv = node
                    .child_by_field_name("receiver")
                    .map(|r| text(r, src))
                    .and_then(|t| {
                        t.trim_matches(|c| c == '(' || c == ')')
                            .split_whitespace()
                            .last()
                            .map(|s| s.trim_start_matches('*').to_string())
                    })
                    .unwrap_or_default();
                let full = if recv.is_empty() {
                    name.to_string()
                } else {
                    format!("{recv}.{name}")
                };
                stack.push(
                    facts,
                    Def {
                        name: full,
                        kind: SymbolKind::Method,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: vec![],
                    },
                );
                pushed = true;
            }
        }
        "type_spec" => {
            if let Some(name) = field_text(node, "name", src) {
                let sk = match node.child_by_field_name("type").map(|t| t.kind()) {
                    Some("struct_type") => SymbolKind::Struct,
                    Some("interface_type") => SymbolKind::Interface,
                    _ => SymbolKind::Type,
                };
                stack.push(
                    facts,
                    Def {
                        name: name.into(),
                        kind: sk,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: vec![],
                    },
                );
                pushed = true;
            }
        }
        "call_expression" => {
            if let Some(f) = node.child_by_field_name("function") {
                let callee = callee_text(f, src);
                let leaf = callee.rsplit('.').next().unwrap_or(&callee).to_string();
                push_call(
                    facts,
                    stack,
                    &callee,
                    &leaf,
                    node,
                    node.child_by_field_name("arguments"),
                    src,
                    is_string,
                );
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, src, facts, stack);
    }
    if pushed {
        stack.pop();
    }
}

fn callee_text(f: Node, src: &[u8]) -> String {
    match f.kind() {
        "selector_expression" => {
            let obj = f
                .child_by_field_name("operand")
                .map(|o| callee_text(o, src))
                .unwrap_or_default();
            let field = field_text(f, "field", src).unwrap_or("");
            if obj.is_empty() {
                field.to_string()
            } else {
                format!("{obj}.{field}")
            }
        }
        "call_expression" => f
            .child_by_field_name("function")
            .map(|n| format!("{}()", callee_text(n, src)))
            .unwrap_or_default(),
        "identifier" | "field_identifier" => text(f, src).to_string(),
        "parenthesized_expression" => f
            .named_child(0)
            .map(|n| callee_text(n, src))
            .unwrap_or_default(),
        _ => {
            let t = text(f, src);
            if t.len() > 60 {
                "<expr>".into()
            } else {
                t.to_string()
            }
        }
    }
}

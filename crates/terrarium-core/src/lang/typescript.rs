use super::*;

fn is_string(kind: &str) -> bool {
    kind == "string" || kind == "template_string"
}

pub fn extract(root: Node, src: &[u8]) -> FileFacts {
    let mut facts = FileFacts::default();
    let mut stack = DefStack::new();
    walk(root, src, &mut facts, &mut stack, 0);
    facts
}

fn decorators_before(node: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() == "decorator" {
            out.push(text(p, src).trim().to_string());
            prev = p.prev_named_sibling();
        } else if p.kind() == "comment" {
            prev = p.prev_named_sibling();
        } else {
            break;
        }
    }
    // decorators may also be children (class members)
    let mut c = node.walk();
    for ch in node.named_children(&mut c) {
        if ch.kind() == "decorator" {
            out.push(text(ch, src).trim().to_string());
        }
    }
    out.reverse();
    out
}

fn import_names(clause: Node, src: &[u8], out: &mut Vec<String>) {
    let mut c = clause.walk();
    for ch in clause.named_children(&mut c) {
        match ch.kind() {
            "identifier" => out.push(text(ch, src).to_string()),
            "named_imports" => {
                let mut c2 = ch.walk();
                for spec in ch.named_children(&mut c2) {
                    if spec.kind() == "import_specifier"
                        && let Some(n) = field_text(spec, "name", src)
                    {
                        out.push(n.to_string());
                    }
                }
            }
            "namespace_import" => {
                if let Some(id) = ch.named_child(0) {
                    out.push(text(id, src).to_string());
                }
            }
            _ => {}
        }
    }
}

fn walk(node: Node, src: &[u8], facts: &mut FileFacts, stack: &mut DefStack, depth: u32) {
    let kind = node.kind();
    let mut pushed = false;
    match kind {
        "import_statement" => {
            if let Some(source) = node.child_by_field_name("source") {
                let mut names = Vec::new();
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "import_clause" {
                        import_names(ch, src, &mut names);
                    }
                }
                facts.imports.push(Import {
                    module: unquote(text(source, src)),
                    names,
                    line: line(node),
                });
            }
        }
        "export_statement" => {
            if let Some(source) = node.child_by_field_name("source") {
                let mut names = Vec::new();
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "export_clause" {
                        let mut c2 = ch.walk();
                        for spec in ch.named_children(&mut c2) {
                            if let Some(n) = field_text(spec, "name", src) {
                                names.push(n.to_string());
                            }
                        }
                    }
                }
                facts.imports.push(Import {
                    module: unquote(text(source, src)),
                    names,
                    line: line(node),
                });
            }
        }
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            if let Some(name) = field_text(node, "name", src) {
                stack.push(
                    facts,
                    Def {
                        name: name.into(),
                        kind: SymbolKind::Function,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: decorators_before(node, src),
                    },
                );
                pushed = true;
            }
        }
        "class_declaration" | "abstract_class_declaration" | "class" => {
            if let Some(name) = field_text(node, "name", src) {
                stack.push(
                    facts,
                    Def {
                        name: name.into(),
                        kind: SymbolKind::Class,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: decorators_before(node, src),
                    },
                );
                pushed = true;
            }
        }
        "interface_declaration" => {
            if let Some(name) = field_text(node, "name", src) {
                stack.push(
                    facts,
                    Def {
                        name: name.into(),
                        kind: SymbolKind::Interface,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: vec![],
                    },
                );
                pushed = true;
            }
        }
        "type_alias_declaration" | "enum_declaration" => {
            if let Some(name) = field_text(node, "name", src) {
                let sk = if kind == "enum_declaration" {
                    SymbolKind::Enum
                } else {
                    SymbolKind::Type
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
        "method_definition" | "method_signature" | "public_field_definition" => {
            if let Some(name) = field_text(node, "name", src) {
                let is_fn_field = kind != "public_field_definition"
                    || node
                        .child_by_field_name("value")
                        .map(|v| v.kind() == "arrow_function" || v.kind() == "function_expression")
                        .unwrap_or(false);
                if is_fn_field {
                    let owner = stack.current().map(|i| facts.defs[i].name.clone());
                    let full = match owner {
                        Some(o) => format!("{o}.{name}"),
                        None => name.to_string(),
                    };
                    stack.push(
                        facts,
                        Def {
                            name: full,
                            kind: SymbolKind::Method,
                            start_line: line(node),
                            end_line: end_line(node),
                            parent: None,
                            attrs: decorators_before(node, src),
                        },
                    );
                    pushed = true;
                }
            }
        }
        "variable_declarator" => {
            if let (Some(name), Some(value)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("value"),
            ) && name.kind() == "identifier"
            {
                let vk = value.kind();
                let is_fn =
                    vk == "arrow_function" || vk == "function_expression" || vk == "function";
                // Top-level consts (depth: program > lexical_declaration > declarator, possibly under export_statement)
                if is_fn || depth <= 3 {
                    let sk = if is_fn {
                        SymbolKind::Function
                    } else {
                        SymbolKind::Const
                    };
                    stack.push(
                        facts,
                        Def {
                            name: text(name, src).into(),
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
        }
        "call_expression" => {
            if let Some(f) = node.child_by_field_name("function") {
                let callee = callee_text(f, src);
                let args = node.child_by_field_name("arguments");
                if callee == "require" || callee == "import" {
                    if let Some(a) = args.and_then(|a| a.named_child(0))
                        && is_string(a.kind())
                    {
                        facts.imports.push(Import {
                            module: unquote(text(a, src)),
                            names: vec![],
                            line: line(node),
                        });
                    }
                } else {
                    let leaf = callee.rsplit('.').next().unwrap_or(&callee).to_string();
                    push_call(facts, stack, &callee, &leaf, node, args, src, is_string);
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, src, facts, stack, depth + 1);
    }
    if pushed {
        stack.pop();
    }
}

fn callee_text(f: Node, src: &[u8]) -> String {
    match f.kind() {
        "member_expression" => {
            let obj = f
                .child_by_field_name("object")
                .map(|o| callee_text(o, src))
                .unwrap_or_default();
            let prop = field_text(f, "property", src).unwrap_or("");
            if obj.is_empty() {
                prop.to_string()
            } else {
                format!("{obj}.{prop}")
            }
        }
        "call_expression" => f
            .child_by_field_name("function")
            .map(|n| format!("{}()", callee_text(n, src)))
            .unwrap_or_default(),
        "identifier" | "this" | "super" | "import" | "property_identifier" => {
            text(f, src).to_string()
        }
        "await_expression"
        | "parenthesized_expression"
        | "non_null_expression"
        | "as_expression" => f
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

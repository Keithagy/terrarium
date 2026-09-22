use super::*;

fn is_string(kind: &str) -> bool {
    kind == "string" || kind == "concatenated_string"
}

pub fn extract(root: Node, src: &[u8]) -> FileFacts {
    let mut facts = FileFacts::default();
    let mut stack = DefStack::new();
    walk(root, src, &mut facts, &mut stack, &[]);
    facts
}

fn walk(
    node: Node,
    src: &[u8],
    facts: &mut FileFacts,
    stack: &mut DefStack,
    pending_decorators: &[String],
) {
    let kind = node.kind();
    let mut pushed = false;
    let mut decorators: Vec<String> = Vec::new();
    match kind {
        "import_statement" => {
            let mut c = node.walk();
            for ch in node.named_children(&mut c) {
                let module = match ch.kind() {
                    "dotted_name" => text(ch, src).to_string(),
                    "aliased_import" => field_text(ch, "name", src).unwrap_or("").to_string(),
                    _ => continue,
                };
                facts.imports.push(Import {
                    module,
                    names: vec![],
                    line: line(node),
                });
            }
        }
        "import_from_statement" => {
            let module = field_text(node, "module_name", src)
                .unwrap_or("")
                .to_string();
            let mut names = Vec::new();
            let mut c = node.walk();
            let mut seen_module = false;
            for ch in node.named_children(&mut c) {
                if !seen_module {
                    if ch.kind() == "dotted_name" || ch.kind() == "relative_import" {
                        seen_module = true;
                    }
                    continue;
                }
                match ch.kind() {
                    "dotted_name" => names.push(text(ch, src).to_string()),
                    "aliased_import" => {
                        if let Some(n) = field_text(ch, "name", src) {
                            names.push(n.to_string());
                        }
                    }
                    _ => {}
                }
            }
            facts.imports.push(Import {
                module,
                names,
                line: line(node),
            });
        }
        "decorated_definition" => {
            let mut c = node.walk();
            for ch in node.named_children(&mut c) {
                if ch.kind() == "decorator" {
                    decorators.push(text(ch, src).trim_start_matches('@').trim().to_string());
                }
            }
        }
        "function_definition" => {
            if let Some(name) = field_text(node, "name", src) {
                let owner = stack
                    .current()
                    .filter(|i| facts.defs[*i].kind == SymbolKind::Class)
                    .map(|i| facts.defs[i].name.clone());
                let (full, sk) = match owner {
                    Some(o) => (format!("{o}.{name}"), SymbolKind::Method),
                    None => (name.to_string(), SymbolKind::Function),
                };
                stack.push(
                    facts,
                    Def {
                        name: full,
                        kind: sk,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: pending_decorators.to_vec(),
                    },
                );
                pushed = true;
            }
        }
        "class_definition" => {
            if let Some(name) = field_text(node, "name", src) {
                stack.push(
                    facts,
                    Def {
                        name: name.into(),
                        kind: SymbolKind::Class,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: pending_decorators.to_vec(),
                    },
                );
                pushed = true;
            }
        }
        "call" => {
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
        if kind == "decorated_definition" && child.kind() == "decorator" {
            continue;
        }
        let d: &[String] = if kind == "decorated_definition" {
            &decorators
        } else {
            &[]
        };
        walk(child, src, facts, stack, d);
    }
    if pushed {
        stack.pop();
    }
}

fn callee_text(f: Node, src: &[u8]) -> String {
    match f.kind() {
        "attribute" => {
            let obj = f
                .child_by_field_name("object")
                .map(|o| callee_text(o, src))
                .unwrap_or_default();
            let attr = field_text(f, "attribute", src).unwrap_or("");
            if obj.is_empty() {
                attr.to_string()
            } else {
                format!("{obj}.{attr}")
            }
        }
        "call" => f
            .child_by_field_name("function")
            .map(|n| format!("{}()", callee_text(n, src)))
            .unwrap_or_default(),
        "identifier" => text(f, src).to_string(),
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

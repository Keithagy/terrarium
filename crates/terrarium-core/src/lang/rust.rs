use super::*;

fn is_string(kind: &str) -> bool {
    kind == "string_literal" || kind == "raw_string_literal"
}

pub fn extract(root: Node, src: &[u8]) -> FileFacts {
    let mut facts = FileFacts::default();
    let mut stack = DefStack::new();
    walk(root, src, &mut facts, &mut stack, None);
    facts
}

/// Collect `#[...]` attribute texts that precede `node`.
fn attrs_before(node: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() == "attribute_item" {
            out.push(text(p, src).trim().to_string());
            prev = p.prev_named_sibling();
        } else if p.kind() == "line_comment" || p.kind() == "block_comment" {
            prev = p.prev_named_sibling();
        } else {
            break;
        }
    }
    out.reverse();
    out
}

fn walk(
    node: Node,
    src: &[u8],
    facts: &mut FileFacts,
    stack: &mut DefStack,
    receiver: Option<&str>,
) {
    let kind = node.kind();
    let mut pushed = false;
    let mut next_receiver: Option<String> = receiver.map(|s| s.to_string());
    match kind {
        "use_declaration" => {
            if let Some(arg) = node.child_by_field_name("argument") {
                for path in expand_use_tree(text(arg, src)) {
                    let (module, name) = split_last(&path);
                    facts.imports.push(Import {
                        module,
                        names: name.into_iter().collect(),
                        line: line(node),
                    });
                }
            }
        }
        "function_item" | "function_signature_item" => {
            if let Some(name) = field_text(node, "name", src) {
                let (full, sk) = match receiver {
                    Some(r) => (format!("{r}::{name}"), SymbolKind::Method),
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
                        attrs: attrs_before(node, src),
                    },
                );
                pushed = true;
            }
        }
        "struct_item" | "enum_item" | "trait_item" | "type_item" | "const_item" | "static_item"
        | "union_item" => {
            if let Some(name) = field_text(node, "name", src) {
                let sk = match kind {
                    "struct_item" | "union_item" => SymbolKind::Struct,
                    "enum_item" => SymbolKind::Enum,
                    "trait_item" => SymbolKind::Trait,
                    "type_item" => SymbolKind::Type,
                    _ => SymbolKind::Const,
                };
                stack.push(
                    facts,
                    Def {
                        name: name.to_string(),
                        kind: sk,
                        start_line: line(node),
                        end_line: end_line(node),
                        parent: None,
                        attrs: attrs_before(node, src),
                    },
                );
                pushed = true;
            }
        }
        "mod_item" => {
            if let Some(name) = field_text(node, "name", src) {
                if node.child_by_field_name("body").is_none() {
                    // `mod foo;` — a file-level import of ./foo.rs or ./foo/mod.rs
                    facts.imports.push(Import {
                        module: format!("mod:{name}"),
                        names: vec![],
                        line: line(node),
                    });
                } else {
                    stack.push(
                        facts,
                        Def {
                            name: name.to_string(),
                            kind: SymbolKind::Module,
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
        "impl_item" => {
            if let Some(ty) = node.child_by_field_name("type") {
                let t = text(ty, src);
                let base = t.split('<').next().unwrap_or(t).trim();
                next_receiver = Some(match field_text(node, "trait", src) {
                    Some(tr) => format!("{base} as {}", tr.split('<').next().unwrap_or(tr)),
                    None => base.to_string(),
                });
            }
        }
        "call_expression" => {
            if let Some(f) = node.child_by_field_name("function") {
                let callee = callee_text(f, src);
                let leaf = callee
                    .rsplit([':', '.'])
                    .next()
                    .unwrap_or(&callee)
                    .to_string();
                let args = node.child_by_field_name("arguments");
                push_call(facts, stack, &callee, &leaf, node, args, src, is_string);
            }
        }
        "macro_invocation" => {
            if let Some(m) = node.child_by_field_name("macro") {
                let callee = format!("{}!", text(m, src));
                let leaf = callee.rsplit("::").next().unwrap_or(&callee).to_string();
                // token trees have no structured args; scan for string literals directly.
                let mut string_args = Vec::new();
                if let Some(tt) = node.child(node.child_count() - 1) {
                    let mut c = tt.walk();
                    for ch in tt.named_children(&mut c) {
                        if is_string(ch.kind()) && string_args.len() < 3 {
                            string_args.push(unquote(text(ch, src)));
                        }
                    }
                }
                facts.calls.push(Call {
                    callee,
                    leaf,
                    def: stack.current(),
                    line: line(node),
                    string_args,
                    ident_args: vec![],
                });
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, src, facts, stack, next_receiver.as_deref());
    }
    if pushed {
        stack.pop();
    }
}

fn callee_text(f: Node, src: &[u8]) -> String {
    match f.kind() {
        "field_expression" => {
            let value = f
                .child_by_field_name("value")
                .map(|v| callee_text(v, src))
                .unwrap_or_default();
            let field = field_text(f, "field", src).unwrap_or("");
            if value.is_empty() {
                field.to_string()
            } else {
                format!("{value}.{field}")
            }
        }
        "generic_function" => f
            .child_by_field_name("function")
            .map(|n| callee_text(n, src))
            .unwrap_or_default(),
        "call_expression" => f
            .child_by_field_name("function")
            .map(|n| callee_text(n, src))
            .unwrap_or_default(),
        "identifier" | "scoped_identifier" | "field_identifier" => text(f, src).to_string(),
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

fn split_last(path: &str) -> (String, Option<String>) {
    match path.rsplit_once("::") {
        Some((m, n)) if n != "*" && n != "self" => (m.to_string(), Some(n.to_string())),
        Some((m, _)) => (m.to_string(), None),
        None => (path.to_string(), None),
    }
}

/// Expand `a::b::{c, d::{e, f}, self}` into flat paths.
pub fn expand_use_tree(s: &str) -> Vec<String> {
    fn rec(s: &str, prefix: &str, out: &mut Vec<String>) {
        let s = s.trim();
        if let Some(open) = s.find('{') {
            let head = s[..open].trim().trim_end_matches("::");
            let inner = &s[open + 1..s.rfind('}').unwrap_or(s.len())];
            let new_prefix = join(prefix, head);
            for part in split_top(inner) {
                rec(part, &new_prefix, out);
            }
        } else if !s.is_empty() {
            // strip `as Alias`
            let s = match s.find(" as ") {
                Some(i) => &s[..i],
                None => s,
            };
            out.push(join(prefix, s.trim()));
        }
    }
    fn join(a: &str, b: &str) -> String {
        if a.is_empty() {
            b.to_string()
        } else if b.is_empty() {
            a.to_string()
        } else {
            format!("{a}::{b}")
        }
    }
    fn split_top(s: &str) -> Vec<&str> {
        let mut depth = 0;
        let mut start = 0;
        let mut out = Vec::new();
        for (i, c) in s.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        out.push(&s[start..]);
        out.into_iter().filter(|p| !p.trim().is_empty()).collect()
    }
    let mut out = Vec::new();
    rec(s, "", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expands_use_trees() {
        let v = expand_use_tree("crate::a::{b, c::{d, e as F}, self}");
        assert_eq!(
            v,
            vec![
                "crate::a::b",
                "crate::a::c::d",
                "crate::a::c::e",
                "crate::a::self"
            ]
        );
    }
}

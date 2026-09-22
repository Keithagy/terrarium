//! Per-language fact extraction on top of tree-sitter.
//!
//! Every language produces the same [`FileFacts`] shape; the scanner turns
//! facts into graph nodes and edges without knowing the language.

pub mod go;
pub mod python;
pub mod rust;
pub mod typescript;

use crate::model::{Lang, SymbolKind};
use tree_sitter::{Node, Parser, Tree};

#[derive(Debug, Clone, Default)]
pub struct Import {
    /// Raw module specifier: `./foo`, `crate::a::b`, `pkg.mod`, `github.com/x/y`.
    pub module: String,
    /// Imported names (empty for whole-module imports).
    pub names: Vec<String>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Def {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: u32,
    pub end_line: u32,
    /// Index into `FileFacts::defs` of the enclosing definition.
    pub parent: Option<usize>,
    /// Attribute / decorator texts attached to this definition.
    pub attrs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Call {
    /// Full callee text as written: `sqlx::query`, `app.get`, `http.HandleFunc`.
    pub callee: String,
    /// Last path segment: `query`, `get`, `HandleFunc`.
    pub leaf: String,
    /// Enclosing definition index.
    pub def: Option<usize>,
    pub line: u32,
    /// Static string literal arguments, in order (max 3).
    pub string_args: Vec<String>,
    /// Bare identifier arguments (used to link route handlers: `HandleFunc("/x", handler)`).
    pub ident_args: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FileFacts {
    pub imports: Vec<Import>,
    pub defs: Vec<Def>,
    pub calls: Vec<Call>,
    pub loc: u32,
}

impl FileFacts {
    pub fn def_of_line(&self, line: u32) -> Option<usize> {
        // Innermost def whose span covers the line.
        let mut best: Option<usize> = None;
        for (i, d) in self.defs.iter().enumerate() {
            if d.start_line <= line && line <= d.end_line {
                match best {
                    Some(b)
                        if self.defs[b].end_line - self.defs[b].start_line
                            <= d.end_line - d.start_line => {}
                    _ => best = Some(i),
                }
            }
        }
        best
    }
}

pub fn parse(lang: Lang, src: &[u8], path: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    let language = match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::TypeScript => {
            if path.ends_with(".tsx") {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            }
        }
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::Other => return None,
    };
    parser.set_language(&language).ok()?;
    parser.parse(src, None)
}

pub fn extract(lang: Lang, src: &[u8], path: &str) -> Option<FileFacts> {
    let tree = parse(lang, src, path)?;
    let root = tree.root_node();
    let mut facts = match lang {
        Lang::Rust => rust::extract(root, src),
        Lang::TypeScript | Lang::JavaScript => typescript::extract(root, src),
        Lang::Python => python::extract(root, src),
        Lang::Go => go::extract(root, src),
        Lang::Other => return None,
    };
    facts.loc = count_loc(src);
    Some(facts)
}

pub fn count_loc(src: &[u8]) -> u32 {
    let mut n = 0u32;
    for line in src.split(|b| *b == b'\n') {
        if line.iter().any(|b| !b.is_ascii_whitespace()) {
            n += 1;
        }
    }
    n
}

// ---- shared helpers ----------------------------------------------------

pub(crate) fn text<'a>(node: Node<'a>, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

pub(crate) fn field_text<'a>(node: Node<'a>, field: &str, src: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name(field).map(|n| text(n, src))
}

pub(crate) fn line(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

pub(crate) fn end_line(node: Node) -> u32 {
    node.end_position().row as u32 + 1
}

/// Strip quotes from a string literal's text.
pub(crate) fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('r').unwrap_or(s);
    let s = s.trim_start_matches('#').trim_end_matches('#');
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let (f, l) = (bytes[0], bytes[bytes.len() - 1]);
        if (f == b'"' && l == b'"') || (f == b'\'' && l == b'\'') || (f == b'`' && l == b'`') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

pub(crate) struct DefStack {
    stack: Vec<usize>,
}

impl DefStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }
    pub fn current(&self) -> Option<usize> {
        self.stack.last().copied()
    }
    pub fn push(&mut self, facts: &mut FileFacts, mut def: Def) -> usize {
        def.parent = self.current();
        facts.defs.push(def);
        let idx = facts.defs.len() - 1;
        self.stack.push(idx);
        idx
    }
    pub fn pop(&mut self) {
        self.stack.pop();
    }
}

/// Record a call and collect its literal / identifier arguments.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_call(
    facts: &mut FileFacts,
    stack: &DefStack,
    callee: &str,
    leaf: &str,
    node: Node,
    args: Option<Node>,
    src: &[u8],
    is_string: fn(&str) -> bool,
) {
    let mut string_args = Vec::new();
    let mut ident_args = Vec::new();
    if let Some(args) = args {
        let mut cursor = args.walk();
        for child in args.named_children(&mut cursor) {
            let kind = child.kind();
            if is_string(kind) {
                if string_args.len() < 3 {
                    string_args.push(unquote(text(child, src)));
                }
            } else if kind == "identifier" || kind == "field_identifier" {
                ident_args.push(text(child, src).to_string());
            } else if kind == "call_expression" || kind == "call" {
                // axum: `.route("/x", get(handler))` — dig one level for the handler ident.
                if let Some(inner) = child.child_by_field_name("arguments") {
                    let mut c2 = inner.walk();
                    for g in inner.named_children(&mut c2) {
                        if g.kind() == "identifier" {
                            ident_args.push(text(g, src).to_string());
                        }
                    }
                }
            } else if kind == "template_string" {
                // Keep the static prefix of a template so routes like `/api/users/${id}` still match.
                let t = unquote(text(child, src));
                if let Some(idx) = t.find("${") {
                    string_args.push(format!("{}*", &t[..idx]));
                } else {
                    string_args.push(t);
                }
            }
        }
    }
    facts.calls.push(Call {
        callee: callee.to_string(),
        leaf: leaf.to_string(),
        def: stack.current(),
        line: line(node),
        string_args,
        ident_args,
    });
}

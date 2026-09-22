//! Minimal TOON (Token-Oriented Object Notation) encoder over `serde_json::Value`.
//!
//! Covers the subset the CLI emits: objects, primitive arrays, tabular arrays of
//! flat objects, and list arrays for anything else.

use serde_json::Value;

pub fn encode(v: &Value) -> String {
    let mut out = String::new();
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                encode_field(&mut out, k, v, 0);
            }
        }
        other => encode_field(&mut out, "value", other, 0),
    }
    out
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn key(k: &str) -> String {
    if k.is_empty()
        || k.chars()
            .any(|c| c.is_whitespace() || matches!(c, ':' | ',' | '"' | '[' | ']' | '{' | '}'))
    {
        quote(k)
    } else {
        k.to_string()
    }
}

fn quote(s: &str) -> String {
    let mut q = String::with_capacity(s.len() + 2);
    q.push('"');
    for c in s.chars() {
        match c {
            '"' => q.push_str("\\\""),
            '\\' => q.push_str("\\\\"),
            '\n' => q.push_str("\\n"),
            '\r' => q.push_str("\\r"),
            '\t' => q.push_str("\\t"),
            c => q.push(c),
        }
    }
    q.push('"');
    q
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            let looks_special = s.is_empty()
                || s != s.trim()
                || s.contains([',', ':', '"', '\n', '\r', '\t', '[', ']', '{', '}'])
                || s.starts_with('-')
                || s == "true"
                || s == "false"
                || s == "null"
                || s.parse::<f64>().is_ok();
            if looks_special { quote(s) } else { s.clone() }
        }
        _ => quote(&v.to_string()),
    }
}

fn is_scalar(v: &Value) -> bool {
    !matches!(v, Value::Object(_) | Value::Array(_))
}

fn tabular_keys(items: &[Value]) -> Option<Vec<String>> {
    let first = items.first()?.as_object()?;
    let keys: Vec<String> = first.keys().cloned().collect();
    if keys.is_empty() {
        return None;
    }
    for it in items {
        let o = it.as_object()?;
        if o.len() != keys.len()
            || !keys
                .iter()
                .all(|k| o.get(k).map(is_scalar).unwrap_or(false))
        {
            return None;
        }
    }
    Some(keys)
}

fn encode_field(out: &mut String, k: &str, v: &Value, depth: usize) {
    match v {
        Value::Object(map) => {
            indent(out, depth);
            out.push_str(&key(k));
            out.push_str(":\n");
            for (kk, vv) in map {
                encode_field(out, kk, vv, depth + 1);
            }
        }
        Value::Array(items) => {
            indent(out, depth);
            out.push_str(&key(k));
            if items.is_empty() {
                out.push_str("[0]:\n");
            } else if items.iter().all(is_scalar) {
                out.push_str(&format!("[{}]: ", items.len()));
                out.push_str(&items.iter().map(scalar).collect::<Vec<_>>().join(","));
                out.push('\n');
            } else if let Some(keys) = tabular_keys(items) {
                out.push_str(&format!(
                    "[{}]{{{}}}:\n",
                    items.len(),
                    keys.iter().map(|k| key(k)).collect::<Vec<_>>().join(",")
                ));
                for it in items {
                    indent(out, depth + 1);
                    let o = it.as_object().unwrap();
                    out.push_str(
                        &keys
                            .iter()
                            .map(|k| scalar(&o[k]))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    out.push('\n');
                }
            } else {
                out.push_str(&format!("[{}]:\n", items.len()));
                for it in items {
                    encode_list_item(out, it, depth + 1);
                }
            }
        }
        scalar_v => {
            indent(out, depth);
            out.push_str(&key(k));
            out.push_str(": ");
            out.push_str(&scalar(scalar_v));
            out.push('\n');
        }
    }
}

fn encode_list_item(out: &mut String, v: &Value, depth: usize) {
    match v {
        Value::Object(map) => {
            let mut first = true;
            for (k, vv) in map {
                if first {
                    indent(out, depth);
                    out.push_str("- ");
                    let mut tmp = String::new();
                    encode_field(&mut tmp, k, vv, 0);
                    // re-indent continuation lines of a nested value
                    let mut lines = tmp.lines();
                    if let Some(l) = lines.next() {
                        out.push_str(l);
                        out.push('\n');
                    }
                    for l in lines {
                        indent(out, depth + 1);
                        out.push_str(l);
                        out.push('\n');
                    }
                    first = false;
                } else {
                    encode_field(out, k, vv, depth + 1);
                }
            }
        }
        Value::Array(_) => {
            indent(out, depth);
            out.push_str("- ");
            let mut tmp = String::new();
            encode_field(&mut tmp, "", v, 0);
            out.push_str(tmp.trim_start_matches("\"\""));
        }
        s => {
            indent(out, depth);
            out.push_str("- ");
            out.push_str(&scalar(s));
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encodes_tabular_and_scalars() {
        let v = json!({"count": "2 of 10", "nodes": [{"id": 1, "name": "a.rs", "kind": "file"}, {"id": 2, "name": "b, c", "kind": "file"}], "tags": ["db", "env:HOME"], "help": ["Run `x`"]});
        let s = encode(&v);
        assert_eq!(
            s,
            "count: 2 of 10\nnodes[2]{id,name,kind}:\n  1,a.rs,file\n  2,\"b, c\",file\ntags[2]: db,\"env:HOME\"\nhelp[1]: Run `x`\n"
        );
    }

    #[test]
    fn encodes_nested_and_empty() {
        let v = json!({"node": {"id": 3, "tags": []}, "list": [{"a": 1, "b": {"c": 2}}]});
        let s = encode(&v);
        assert_eq!(
            s,
            "node:\n  id: 3\n  tags[0]:\nlist[1]:\n  - a: 1\n    b:\n      c: 2\n"
        );
    }
}

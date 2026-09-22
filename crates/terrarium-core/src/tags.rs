//! Boundary heuristics: which calls talk to the outside world, and through what.
//!
//! Tags are cheap strings so the UI and CLI can filter on them without a schema:
//! `http-server`, `http-client`, `db`, `fs`, `env`, `ipc-server`, `ipc-client`, `queue`,
//! `process`, plus keyed tags `route:/api/x`, `ipc:cmd_name`, `env:NAME`, `topic:orders`.

use crate::lang::{Call, Def};
use crate::model::Lang;
use once_cell::sync::Lazy;
use regex::Regex;

/// A route or command binding discovered on the server side.
#[derive(Debug, Clone)]
pub struct Binding {
    /// `route:/api/x` or `ipc:cmd`
    pub key: String,
    /// Name of the handler symbol, if the call named one (`HandleFunc("/x", handler)`).
    pub handler: Option<String>,
    /// Definition that contains the binding call (fallback owner).
    pub def: Option<usize>,
}

#[derive(Debug, Default)]
pub struct CallTags {
    pub tags: Vec<String>,
    pub bindings: Vec<Binding>,
}

static HTTP_SERVER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?x)^(
        (app|router|r|api|server|srv|mux|e|g|group|route|blueprint|bp)\.(get|post|put|patch|delete|head|options|route|all|use|handle|handle_func|HandleFunc|Handle|GET|POST|PUT|PATCH|DELETE|Any|Group)
      | http\.(HandleFunc|Handle)
      | .*\.route$   # axum Router::route
      | web::(get|post|put|patch|delete|resource|scope)
      | (web::)?scope
    )$").unwrap()
});

static HTTP_CLIENT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?x)^(
        fetch | window\.fetch | globalThis\.fetch
      | (axios|ky|got|superagent|api|client|http|httpx|session|requests|urllib\.request|this\.http|this\.client|self\.client|self\.session)\.(get|post|put|patch|delete|request|fetch|head|urlopen)
      | axios | ky
      | http\.(Get|Post|Head|NewRequest|PostForm)
      | client\.Do | http\.DefaultClient\.Do
      | reqwest::(blocking::)?(get|post|Client::new) | .*\.send | ureq::(get|post)
      | urlopen
    )$").unwrap()
});

static DB: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?xi)^(
        sqlx::(query|query_as|query_scalar)!? | .*\.(execute|fetch_one|fetch_all|fetch_optional|query_row|prepare)
      | (db|conn|pool|tx|cursor|session|client|prisma|knex|sequelize|mongoose|mongo|collection|redis|sql|orm)\.(query|exec|execute|fetch|fetchall|fetchone|find|find_one|findOne|findMany|insert|insert_one|insertOne|update|updateOne|delete|deleteOne|save|create|aggregate|raw|select|from|Query|QueryRow|Exec|Prepare|First|Find|Create|Save|Delete|Where)
      | gorm\.Open | sql\.Open | sqlx\.Connect | sqlx::.*::connect | create_engine | psycopg2\.connect | sqlite3\.connect | pymongo\.MongoClient | MongoClient | createConnection | createPool | PrismaClient
      | diesel::.* | rusqlite::Connection::open
    )$").unwrap()
});

static FS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?x)^(
        (std::)?fs::(read|read_to_string|write|create_dir|create_dir_all|remove_file|remove_dir_all|copy|rename|File::open|File::create|read_dir)
      | File::(open|create) | .*\.write_all
      | fs\.(readFile|readFileSync|writeFile|writeFileSync|readdir|readdirSync|mkdir|mkdirSync|unlink|rm|stat|createReadStream|createWriteStream|promises\.\w+)
      | (fs\.promises|fsp|fsPromises)\.\w+
      | (Path|pathlib\.Path|path)\.(read_text|write_text|read_bytes|write_bytes|open|iterdir|glob)
      | os\.(Open|Create|ReadFile|WriteFile|ReadDir|Remove|Mkdir|MkdirAll|Stat) | ioutil\.(ReadFile|WriteFile|ReadDir) | filepath\.Walk
      | shutil\.\w+ | tokio::fs::\w+
    )$").unwrap()
});

static ENV: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)^(
        (std::)?env::(var|var_os|vars) | env::var
      | os\.(Getenv|LookupEnv|environ\.get|getenv) | os\.environ\.get | getenv
      | dotenv | dotenv\.config | dotenv::dotenv | load_dotenv
      | env!|option_env!|std::env::var
    )$",
    )
    .unwrap()
});

static IPC_CLIENT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(invoke|tauri\.invoke|ipcRenderer\.(invoke|send)|window\.__TAURI__\.\w+\.invoke)$",
    )
    .unwrap()
});
static IPC_SERVER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(ipcMain\.(handle|on)|listen)$").unwrap());

static QUEUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?xi)^(
        (producer|consumer|channel|ch|kafka|broker|queue|bus|nats|nc|js|sqs|pubsub|topic|subscriber|publisher|redis|rdb|client)\.(publish|subscribe|send|send_message|sendMessage|consume|produce|emit|basic_publish|basic_consume|Publish|Subscribe|SendMessage|ReceiveMessage|xadd|xread|lpush|rpush|brpop|blpop)
      | (celery|task|job)\.(delay|apply_async|enqueue) | \w+\.delay | \w+\.apply_async
    )$").unwrap()
});

static PROCESS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)^(
        (std::process::)?Command::new | tokio::process::Command::new
      | subprocess\.(run|Popen|call|check_output|check_call) | os\.system
      | exec\.Command | exec\.CommandContext
      | (child_process\.)?(spawn|exec|execSync|spawnSync|execFile|fork)
    )$",
    )
    .unwrap()
});

static ATTR_ROUTE: Lazy<Regex> = Lazy::new(|| {
    // `#[get("/x")]`, `@app.get("/x")`, `@Get('/x')`, `@router.post("/x")`, `#[route("/x", method = "GET")]`
    Regex::new(r#"(?i)(?:^|[\[\.@])(get|post|put|patch|delete|head|options|route|api_route|websocket|Get|Post|Put|Patch|Delete|All)\s*\(\s*["'`]([^"'`]*)["'`]"#).unwrap()
});

static ATTR_IPC: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"tauri::command|^command\b|#\[command").unwrap());

pub fn normalize_route(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return None;
    }
    // strip scheme + host
    if let Some(idx) = s.find("://") {
        let rest = &s[idx + 3..];
        s = match rest.find('/') {
            Some(i) => rest[i..].to_string(),
            None => "/".to_string(),
        };
    }
    if !s.starts_with('/') {
        return None;
    }
    if let Some(i) = s.find(['?', '#']) {
        s.truncate(i);
    }
    let segs: Vec<String> = s
        .split('/')
        .map(|seg| {
            let t = seg.trim();
            if t.is_empty() {
                return String::new();
            }
            let is_param = t.starts_with(':')
                || (t.starts_with('{') && t.ends_with('}'))
                || (t.starts_with('<') && t.ends_with('>'))
                || t.starts_with('*')
                || t.starts_with('$')
                || t.chars().all(|c| c.is_ascii_digit())
                || t.contains('*');
            if is_param {
                "*".to_string()
            } else {
                t.to_string()
            }
        })
        .collect();
    let mut out = segs.join("/");
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    Some(out)
}

/// Two normalized routes match if segment counts agree and every segment is equal or a wildcard.
pub fn routes_match(a: &str, b: &str) -> bool {
    let sa: Vec<&str> = a.split('/').collect();
    let sb: Vec<&str> = b.split('/').collect();
    if sa.len() != sb.len() {
        // allow a trailing wildcard from a template prefix (`/api/users/*`) to match deeper paths
        let (short, long) = if sa.len() < sb.len() {
            (&sa, &sb)
        } else {
            (&sb, &sa)
        };
        if short.last() != Some(&"*") {
            return false;
        }
        return short[..short.len() - 1]
            .iter()
            .zip(long.iter())
            .all(|(x, y)| x == y || *x == "*" || *y == "*");
    }
    sa.iter()
        .zip(sb.iter())
        .all(|(x, y)| x == y || *x == "*" || *y == "*")
}

pub fn classify_call(call: &Call, lang: Lang) -> CallTags {
    let mut out = CallTags::default();
    let c = call.callee.as_str();
    let push = |out: &mut CallTags, t: &str| {
        if !out.tags.iter().any(|x| x == t) {
            out.tags.push(t.to_string());
        }
    };
    if IPC_CLIENT.is_match(c) {
        push(&mut out, "ipc-client");
        if let Some(cmd) = call.string_args.first() {
            push(&mut out, &format!("ipc:{cmd}"));
        }
    } else if IPC_SERVER.is_match(c) && (lang == Lang::TypeScript || lang == Lang::JavaScript) {
        push(&mut out, "ipc-server");
        if let Some(cmd) = call.string_args.first() {
            out.bindings.push(Binding {
                key: format!("ipc:{cmd}"),
                handler: call.ident_args.first().cloned(),
                def: call.def,
            });
        }
    } else if HTTP_SERVER.is_match(c)
        && call
            .string_args
            .first()
            .map(|s| s.starts_with('/'))
            .unwrap_or(false)
    {
        push(&mut out, "http-server");
        if let Some(r) = call.string_args.first().and_then(|s| normalize_route(s)) {
            push(&mut out, &format!("route:{r}"));
            out.bindings.push(Binding {
                key: format!("route:{r}"),
                handler: call.ident_args.first().cloned(),
                def: call.def,
            });
        }
    } else if HTTP_CLIENT.is_match(c) {
        push(&mut out, "http-client");
        if let Some(r) = call.string_args.iter().find_map(|s| normalize_route(s)) {
            push(&mut out, &format!("route:{r}"));
        }
    } else if DB.is_match(c) {
        push(&mut out, "db");
    } else if ENV.is_match(c) {
        push(&mut out, "env");
        if let Some(name) = call.string_args.first()
            && name
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit())
            && !name.is_empty()
        {
            push(&mut out, &format!("env:{name}"));
        }
    } else if FS.is_match(c) || (c == "open" && lang == Lang::Python) {
        push(&mut out, "fs");
    } else if QUEUE.is_match(c) {
        push(&mut out, "queue");
        if let Some(t) = call.string_args.first() {
            push(&mut out, &format!("topic:{t}"));
        }
    } else if PROCESS.is_match(c) {
        push(&mut out, "process");
    }
    // `process.env.X` in JS is a member access, not a call; handled by the scanner's text pass.
    out
}

/// Tags derived from attributes / decorators on a definition.
pub fn classify_def(def: &Def) -> CallTags {
    let mut out = CallTags::default();
    for a in &def.attrs {
        if ATTR_IPC.is_match(a) {
            out.tags.push("ipc-server".into());
            let key = format!("ipc:{}", def.name.rsplit("::").next().unwrap_or(&def.name));
            out.tags.push(key.clone());
            out.bindings.push(Binding {
                key,
                handler: Some(def.name.clone()),
                def: None,
            });
        }
        if let Some(cap) = ATTR_ROUTE.captures(a)
            && let Some(r) = normalize_route(&cap[2])
        {
            out.tags.push("http-server".into());
            let key = format!("route:{r}");
            out.tags.push(key.clone());
            out.bindings.push(Binding {
                key,
                handler: Some(def.name.clone()),
                def: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_routes() {
        assert_eq!(
            normalize_route("http://localhost:3000/api/users/42?x=1").as_deref(),
            Some("/api/users/*")
        );
        assert_eq!(
            normalize_route("/api/users/:id").as_deref(),
            Some("/api/users/*")
        );
        assert_eq!(
            normalize_route("/api/users/{id}/").as_deref(),
            Some("/api/users/*")
        );
        assert_eq!(normalize_route("users"), None);
    }

    #[test]
    fn matches_wildcards() {
        assert!(routes_match("/api/users/*", "/api/users/*"));
        assert!(routes_match("/api/users/*", "/api/users/me"));
        assert!(!routes_match("/api/users", "/api/posts"));
        assert!(routes_match("/api/*", "/api/users/me"));
    }

    #[test]
    fn detects_decorator_routes() {
        let d = Def {
            name: "list_users".into(),
            kind: crate::model::SymbolKind::Function,
            start_line: 1,
            end_line: 2,
            parent: None,
            attrs: vec!["app.get(\"/users\")".into()],
        };
        let t = classify_def(&d);
        assert!(t.tags.contains(&"route:/users".to_string()));
        let d = Def {
            name: "scan".into(),
            kind: crate::model::SymbolKind::Function,
            start_line: 1,
            end_line: 2,
            parent: None,
            attrs: vec!["#[tauri::command]".into()],
        };
        assert!(classify_def(&d).tags.contains(&"ipc:scan".to_string()));
    }
}

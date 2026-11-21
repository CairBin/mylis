//! myls —— Mylis 语言服务器（LSP over stdio，零第三方依赖）。
//!
//! 能力：诊断（语法 + 语义，复用 mylc 的 loader/sema）、文档符号、悬停、跳转定义、补全。
//! 未落盘的缓冲区通过 `Loader::set_overlay` 注入，所以敲字时也能拿到准确的跨文件诊断。

use std::collections::HashMap;
use std::io::{Read, Write};

use mylc::diag::{Diag, Item};
use mylc::json::{self, Value};
use mylc::program::Loader;
use mylc::{ast, sema};

struct Doc {
    path: String,
    text: String,
    version: i64,
}

struct Server {
    /// 打开文档：uri -> Doc
    docs: HashMap<String, Doc>,
    /// uri 顺序（保持稳定）
    order: Vec<String>,
    root: String,
}

fn canon(p: &str) -> String {
    std::fs::canonicalize(p)
        .map(|x| x.to_string_lossy().to_string())
        .unwrap_or_else(|_| p.to_string())
}

fn uri_to_path(uri: &str) -> String {
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    let decoded = percent_decode(raw);
    // /C:/x -> C:/x
    if decoded.len() > 3 && decoded.starts_with('/') && decoded.as_bytes()[2] == b':' {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = (b[i + 1] as char).to_digit(16);
            let l = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn path_to_uri(p: &str) -> String {
    let norm = p.replace('\\', "/");
    let mut enc = String::from("file://");
    if !norm.starts_with('/') {
        enc.push('/');
    }
    enc.push_str(&norm);
    enc
}

impl Server {
    fn new() -> Server {
        Server {
            docs: HashMap::new(),
            order: Vec::new(),
            root: String::new(),
        }
    }

    fn std_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut cands: Vec<std::path::PathBuf> = Vec::new();
        if !self.root.is_empty() {
            let root = std::path::PathBuf::from(&self.root);
            cands.push(root.clone());
            cands.push(root.join("core"));
            cands.push(root.join("std/src"));
            cands.push(root.join("std"));
        }
        cands.push(std::path::PathBuf::from("."));
    cands.push(std::path::PathBuf::from("core"));
        cands.push(std::path::PathBuf::from("std/src"));
        cands.push(std::path::PathBuf::from("std"));
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                cands.push(dir.join("core"));
                cands.push(dir.join("std/src"));
                cands.push(dir.join("std"));
                if let Some(up) = dir.parent() {
                    cands.push(up.join("core"));
                    cands.push(up.join("std/src"));
                    cands.push(up.join("std"));
                }
            }
        }
        if let Ok(paths) = std::env::var("MYLIS_PATH") {
            for p in paths.split(if cfg!(windows) { ';' } else { ':' }) {
                if !p.is_empty() { cands.push(std::path::PathBuf::from(p)); }
            }
        }
        let mut out: Vec<std::path::PathBuf> = Vec::new();
        for c in cands { if !out.contains(&c) { out.push(c); } }
        out
    }

    /// 用当前所有打开文档做 overlay，检查 entry 指向的文件
    fn check(&self, entry: &str) -> Vec<Item> {
        let mut diags = Diag::new();
        diags.collect_mode();
        // 键同时放“原始路径”和“规范化路径”：Windows 上 canonicalize 会带 \\?\ 前缀
        let mut overlay: HashMap<String, String> = HashMap::new();
        for d in self.docs.values() {
            overlay.insert(d.path.clone(), d.text.clone());
            overlay.insert(canon(&d.path), d.text.clone());
        }
        let mut search: Vec<std::path::PathBuf> = Vec::new();
        search.extend(self.std_dirs());
        let mut prog = {
            let mut loader = Loader::new(&mut diags, search);
            loader.set_overlay(overlay);
            let _ = loader.load(&canon(entry));
            loader.prog
        };
        sema::analyze(&mut prog, &mut diags);
        diags.items.take().unwrap_or_default()
    }

    fn diagnostics_for(&self, uri: &str, path: &str) -> Value {
        let items = self.check(path);
        let mut arr = Vec::new();
        for it in items.iter().filter(|i| same_file(&i.file, path)) {
            let line = it.line.saturating_sub(1);
            let col = it.col.saturating_sub(1);
            arr.push(Value::Obj(vec![
                (
                    "range".to_string(),
                    Value::Obj(vec![
                        (
                            "start".to_string(),
                            Value::Obj(vec![
                                ("line".to_string(), Value::n(line as i64)),
                                ("character".to_string(), Value::n(col as i64)),
                            ]),
                        ),
                        (
                            "end".to_string(),
                            Value::Obj(vec![
                                ("line".to_string(), Value::n(line as i64)),
                                (
                                    "character".to_string(),
                                    Value::n((col as usize + 1) as i64),
                                ),
                            ]),
                        ),
                    ]),
                ),
                (
                    "severity".to_string(),
                    Value::n(if it.warning { 2 } else { 1 }),
                ),
                ("source".to_string(), Value::s("mylis")),
                (
                    "code".to_string(),
                    Value::Str(it.code.clone()),
                ),
                (
                    "message".to_string(),
                    Value::Str(format!("{}", it.msg)),
                ),
            ]));
        }
        let _ = uri;
        Value::Arr(arr)
    }

    fn publish(&self, uri: &str) -> Value {
        let path = match self.docs.get(uri) {
            Some(d) => d.path.clone(),
            None => uri_to_path(uri),
        };
        Value::Obj(vec![
            ("jsonrpc".to_string(), Value::s("2.0")),
        ])
        .make_notification(
            "textDocument/publishDiagnostics",
            Value::Obj(vec![
                ("uri".to_string(), Value::s(uri)),
                ("diagnostics".to_string(), self.diagnostics_for(uri, &path)),
            ]),
        )
    }
}

fn norm_path(s: &str) -> String {
    let mut t = s.replace('\\', "/");
    // Windows canonicalize 会带 \\?\ 前缀；wine 下也可能出现 //?/Z:
    for pre in ["//?/", "//./"] {
        if let Some(rest) = t.strip_prefix(pre) {
            t = rest.to_string();
        }
    }
    if t.len() > 2 && t.as_bytes()[0] == b'/' && t.as_bytes()[1].is_ascii_alphabetic() && t.as_bytes()[2] == b':' {
        t = t[1..].to_string();
    }
    while t.starts_with("./") {
        t = t[2..].to_string();
    }
    t.to_lowercase()
}

fn same_file(a: &str, b: &str) -> bool {
    let (na, nb) = (norm_path(a), norm_path(b));
    na == nb
}

trait MakeNotification {
    fn make_notification(self, method: &str, params: Value) -> Value;
}

impl MakeNotification for Value {
    fn make_notification(self, method: &str, params: Value) -> Value {
        let mut obj = match self {
            Value::Obj(kv) => kv,
            _ => Vec::new(),
        };
        obj.push(("method".to_string(), Value::s(method)));
        obj.push(("params".to_string(), params));
        Value::Obj(obj)
    }
}

fn read_message(stdin: &mut impl Read) -> Option<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    // 读 header
    loop {
        if stdin.read(&mut byte).ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8192 {
            return None;
        }
    }
    let header = String::from_utf8_lossy(&buf).to_string();
    let mut len = 0usize;
    for line in header.split("\r\n") {
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            len = rest.trim().parse().ok()?;
        }
    }
    let mut body = vec![0u8; len];
    stdin.read_exact(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).to_string())
}

fn write_message(out: &mut impl Write, v: &Value) {
    let mut s = String::new();
    json::write(v, &mut s);
    let bytes = s.as_bytes();
    let _ = write!(out, "Content-Length: {}\r\n\r\n", bytes.len());
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

fn response(id: &Value, result: Value) -> Value {
    Value::Obj(vec![
        ("jsonrpc".to_string(), Value::s("2.0")),
        (
            "id".to_string(),
            match id {
                Value::Num(_) => id.clone(),
                Value::Str(s) => Value::Str(s.clone()),
                _ => Value::Null,
            },
        ),
        ("result".to_string(), result),
    ])
}

fn error_response(id: &Value, code: i64, msg: &str) -> Value {
    Value::Obj(vec![
        ("jsonrpc".to_string(), Value::s("2.0")),
        ("id".to_string(), id.clone()),
        (
            "error".to_string(),
            Value::Obj(vec![
                ("code".to_string(), Value::n(code)),
                ("message".to_string(), Value::s(msg)),
            ]),
        ),
    ])
}

/// 位置 → 偏移（按 UTF-16 code unit 近似用 char 计）
fn word_at(text: &str, line: u32, character: u32) -> Option<(String, u32, u32)> {
    let line_text = text.split('\n').nth(line as usize)?;
    let chars: Vec<char> = line_text.chars().collect();
    let mut i = (character as usize).min(chars.len());
    if i == chars.len() && i > 0 {
        i -= 1;
    }
    if !(chars.get(i).map_or(false, |c| is_word(*c))) {
        return None;
    }
    let mut start = i;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = i;
    while end + 1 < chars.len() && is_word(chars[end + 1]) {
        end += 1;
    }
    let word: String = chars[start..=end].iter().collect();
    Some((word, start as u32, end as u32))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn decl_signature(d: &ast::Decl) -> String {
    let mut s = String::new();
    match d.kind {
        ast::DeclKind::Class => {
            s.push_str(&format!("class {}", d.name));
        }
        ast::DeclKind::Iface => {
            s.push_str(&format!("interface {}", d.name));
        }
        ast::DeclKind::Enum => {
            s.push_str(&format!("enum {}", d.name));
        }
        ast::DeclKind::Alias => {
            s.push_str(&format!("type {} = {}", d.name, d.alias_target));
        }
        ast::DeclKind::Const => {
            s.push_str(&format!("const {} {}", d.ret, d.name));
        }
        ast::DeclKind::Fn => {
            s.push_str(&format!("fn {}(", d.name));
            for (i, p) in d.params.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&format!("{} {}", p.ty, p.name));
            }
            s.push_str(&format!(") -> {}", d.ret));
        }
    }
    s
}

fn collect_symbols(mods: &[(String, usize)], decls: &[ast::Decl], path: &str) -> Value {
    let mut arr = Vec::new();
    for (di, d) in decls.iter().enumerate() {
        let _ = di;
        let file = mods
            .iter()
            .find(|(_, id)| *id == d.mod_id)
            .map(|(p, _)| p.as_str())
            .unwrap_or("");
        if !same_file(file, path) {
            continue;
        }
        let kind = match d.kind {
            ast::DeclKind::Class => 5,
            ast::DeclKind::Iface => 11,
            ast::DeclKind::Enum => 10,
            ast::DeclKind::Alias => 26,
            ast::DeclKind::Const => 14,
            ast::DeclKind::Fn => 12,
        };
        arr.push(Value::Obj(vec![
            ("name".to_string(), Value::Str(d.name.clone())),
            ("kind".to_string(), Value::n(kind)),
            (
                "range".to_string(),
                range_value(d.line, d.col, d.name.chars().count()),
            ),
            (
                "selectionRange".to_string(),
                range_value(d.line, d.col, d.name.chars().count()),
            ),
            ("detail".to_string(), Value::Str(decl_signature(d))),
        ]));
    }
    Value::Arr(arr)
}

fn range_value(line: u32, col: u32, len: usize) -> Value {
    let l = line.saturating_sub(1);
    let c = col.saturating_sub(1);
    Value::Obj(vec![
        (
            "start".to_string(),
            Value::Obj(vec![
                ("line".to_string(), Value::n(l as i64)),
                ("character".to_string(), Value::n(c as i64)),
            ]),
        ),
        (
            "end".to_string(),
            Value::Obj(vec![
                ("line".to_string(), Value::n(l as i64)),
                (
                    "character".to_string(),
                    Value::n((c as usize + len) as i64),
                ),
            ]),
        ),
    ])
}

fn needs_io() -> bool {
    true
}

fn main() {
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut srv = Server::new();

    while let Some(body) = read_message(&mut stdin) {
        let msg = match json::parse(&body) {
            Some(v) => v,
            None => continue,
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let has_id = msg.get("id").is_some();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        match method.as_str() {
            "initialize" => {
                if let Some(root) = params.get("rootPath").and_then(|v| v.as_str()) {
                    srv.root = root.to_string();
                } else if let Some(root) = params
                    .get("rootUri")
                    .and_then(|v| v.as_str())
                    .map(uri_to_path)
                {
                    srv.root = root;
                }
                let caps = Value::Obj(vec![
                    ("textDocumentSync".to_string(), Value::n(1)),
                    ("hoverProvider".to_string(), Value::Bool(true)),
                    ("definitionProvider".to_string(), Value::Bool(true)),
                    ("documentSymbolProvider".to_string(), Value::Bool(true)),
                    (
                        "completionProvider".to_string(),
                        Value::Obj(vec![(
                            "triggerCharacters".to_string(),
                            Value::Arr(vec![Value::s("."), Value::s(":")]),
                        )]),
                    ),
                ]);
                let info = Value::Obj(vec![
                    ("name".to_string(), Value::s("myls")),
                    ("version".to_string(), Value::s("0.1.0")),
                ]);
                let result = Value::Obj(vec![
                    ("capabilities".to_string(), caps),
                    ("serverInfo".to_string(), info),
                ]);
                write_message(&mut stdout, &response(&id, result));
            }
            "initialized" => {}
            "textDocument/didOpen" => {
                if let Some(td) = params.get("textDocument") {
                    let uri = td.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let text = td.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let version = td.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
                    let path = uri_to_path(&uri);
                    if !srv.docs.contains_key(&uri) {
                        srv.order.push(uri.clone());
                    }
                    srv.docs.insert(uri.clone(), Doc { path, text, version });
                    write_message(&mut stdout, &srv.publish(&uri));
                }
            }
            "textDocument/didChange" => {
                if let Some(td) = params.get("textDocument") {
                    let uri = td.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let version = td.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
                    let mut new_text = None;
                    if let Some(changes) = params
                        .get("contentChanges")
                        .and_then(|c| c.as_arr())
                    {
                        for ch in changes {
                            if let Some(t) = ch.get("text").and_then(|v| v.as_str()) {
                                new_text = Some(t.to_string());
                            }
                        }
                    }
                    if let (Some(t), Some(d)) = (new_text, srv.docs.get_mut(&uri)) {
                        d.text = t;
                        d.version = version;
                    }
                    write_message(&mut stdout, &srv.publish(&uri));
                }
            }
            "textDocument/didSave" => {
                if let Some(td) = params.get("textDocument") {
                    let uri = td.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    write_message(&mut stdout, &srv.publish(&uri));
                }
            }
            "textDocument/didClose" => {
                if let Some(td) = params.get("textDocument") {
                    let uri = td.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    srv.docs.remove(&uri);
                    srv.order.retain(|u| u != &uri);
                    let v = Value::Obj(vec![("jsonrpc".to_string(), Value::s("2.0"))])
                        .make_notification(
                            "textDocument/publishDiagnostics",
                            Value::Obj(vec![
                                ("uri".to_string(), Value::s(&uri)),
                                ("diagnostics".to_string(), Value::Arr(Vec::new())),
                            ]),
                        );
                    write_message(&mut stdout, &v);
                }
            }
            "textDocument/hover" => {
                let uri = params
                    .get("textDocument")
                    .and_then(|v| v.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let pos = params.get("position").cloned().unwrap_or(Value::Null);
                let line = pos.get("line").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
                let ch = pos.get("character").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
                let word = srv
                    .docs
                    .get(&uri)
                    .and_then(|d| word_at(&d.text, line, ch))
                    .map(|(w, _, _)| w);
                let hover = match word {
                    Some(w) => {
                        let path = srv.docs.get(&uri).map(|d| d.path.clone()).unwrap_or_default();
                        let items = srv.check(&path);
                        let _ = items;
                        let text = hover_text(&path, &w);
                        match text {
                            Some(t) => Value::Obj(vec![
                                (
                                    "contents".to_string(),
                                    Value::Obj(vec![
                                        ("kind".to_string(), Value::s("markdown")),
                                        ("value".to_string(), Value::Str(format!("```mylis\n{}\n```", t))),
                                    ]),
                                ),
                                ("range".to_string(), range_value(line + 1, ch + 1, w.chars().count())),
                            ]),
                            None => Value::Null,
                        }
                    }
                    None => Value::Null,
                };
                write_message(&mut stdout, &response(&id, hover));
            }
            "textDocument/definition" => {
                let uri = params
                    .get("textDocument")
                    .and_then(|v| v.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let pos = params.get("position").cloned().unwrap_or(Value::Null);
                let line = pos.get("line").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
                let ch = pos.get("character").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
                let word = srv
                    .docs
                    .get(&uri)
                    .and_then(|d| word_at(&d.text, line, ch))
                    .map(|(w, _, _)| w);
                let mut result = Value::Null;
                if let Some(w) = word {
                    let path = srv.docs.get(&uri).map(|d| d.path.clone()).unwrap_or_default();
                    if let Some((file, l, c, len)) = find_decl(&path, &w) {
                        result = Value::Obj(vec![
                            ("uri".to_string(), Value::Str(path_to_uri(&file))),
                            ("range".to_string(), range_value(l, c, len)),
                        ]);
                    }
                }
                write_message(&mut stdout, &response(&id, result));
            }
            "textDocument/documentSymbol" => {
                let uri = params
                    .get("textDocument")
                    .and_then(|v| v.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let path = srv.docs.get(&uri).map(|d| d.path.clone()).unwrap_or_else(|| uri_to_path(&uri));
                let result = {
                    let mut diags = Diag::new();
                    diags.collect_mode();
                    let mut search: Vec<std::path::PathBuf> = Vec::new();
                    search.extend(srv.std_dirs());
                    let overlay: HashMap<String, String> = srv
                        .docs
                        .values()
                        .map(|d| (d.path.clone(), d.text.clone()))
                        .collect();
                    let mut loader = Loader::new(&mut diags, search);
                    loader.set_overlay(overlay);
                    let _ = loader.load(&path);
                    let prog = loader.prog;
                    let mods: Vec<(String, usize)> = prog
                        .modules
                        .iter()
                        .enumerate()
                        .map(|(i, m)| (m.path.clone(), i))
                        .collect();
                    collect_symbols(&mods, &prog.decls, &path)
                };
                write_message(&mut stdout, &response(&id, result));
            }
            "textDocument/completion" => {
                let mut items: Vec<Value> = Vec::new();
                for kw in KEYWORDS {
                    items.push(Value::Obj(vec![
                        ("label".to_string(), Value::s(kw)),
                        ("kind".to_string(), Value::n(14)),
                    ]));
                }
                let path = params
                    .get("textDocument")
                    .and_then(|v| v.get("uri"))
                    .and_then(|v| v.as_str())
                    .map(uri_to_path)
                    .unwrap_or_default();
                for (name, detail, kind) in visible_symbols(&path) {
                    items.push(Value::Obj(vec![
                        ("label".to_string(), Value::Str(name)),
                        ("detail".to_string(), Value::Str(detail)),
                        ("kind".to_string(), Value::n(kind)),
                    ]));
                }
                write_message(&mut stdout, &response(&id, Value::Obj(vec![("isIncomplete".to_string(), Value::Bool(false)), ("items".to_string(), Value::Arr(items))])));
            }
            "shutdown" => {
                write_message(&mut stdout, &response(&id, Value::Null));
            }
            "exit" => break,
            _ => {
                if has_id {
                    write_message(&mut stdout, &error_response(&id, -32601, "未实现的方法"));
                }
            }
        }
    }
    let _ = needs_io();
}

const KEYWORDS: &[&str] = &[
    "fn", "class", "interface", "enum", "pub", "import", "static", "unsafe", "new", "match",
    "while", "for", "if", "else", "return", "break", "continue", "true", "false", "null",
    "this", "void", "bool", "string", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64",
    "f32", "f64",
];

fn lsp_search_dirs(anchor: Option<&str>) -> Vec<std::path::PathBuf> {
    let mut cands: Vec<std::path::PathBuf> = Vec::new();
    if let Some(a) = anchor {
        if let Some(parent) = std::path::Path::new(a).parent() {
            cands.push(parent.to_path_buf());
            if let Some(grand) = parent.parent() {
                cands.push(grand.to_path_buf());
            }
            cands.push(parent.join("core"));
            cands.push(parent.join("std/src"));
            cands.push(parent.join("std"));
            cands.push(parent.join("../core"));
            cands.push(parent.join("../std/src"));
            cands.push(parent.join("../std"));
        }
    }
    cands.push(std::path::PathBuf::from("core"));
    cands.push(std::path::PathBuf::from("std/src"));
    cands.push(std::path::PathBuf::from("std"));
    if let Ok(paths) = std::env::var("MYLIS_PATH") {
        for p in paths.split(if cfg!(windows) { ';' } else { ':' }) {
            if !p.is_empty() { cands.push(std::path::PathBuf::from(p)); }
        }
    }
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    for c in cands { if !out.contains(&c) { out.push(c); } }
    out
}

fn visible_symbols(path: &str) -> Vec<(String, String, i64)> {
    let mut diags = Diag::new();
    diags.collect_mode();
    let search = lsp_search_dirs(Some(path));
    let mut loader = Loader::new(&mut diags, search);
    let _ = loader.load(path);
    let prog = loader.prog;
    let mut out = Vec::new();
    for d in prog.decls.iter() {
        let kind = match d.kind {
            ast::DeclKind::Class => 7,
            ast::DeclKind::Iface => 8,
            ast::DeclKind::Enum => 13,
            ast::DeclKind::Alias => 26,
            ast::DeclKind::Const => 14,
            ast::DeclKind::Fn => 3,
        };
        out.push((d.name.clone(), decl_signature(d), kind));
    }
    out
}

fn find_decl(entry: &str, name: &str) -> Option<(String, u32, u32, usize)> {
    let mut diags = Diag::new();
    diags.collect_mode();
    let search = lsp_search_dirs(Some(entry));
    let mut loader = Loader::new(&mut diags, search);
    let _ = loader.load(entry);
    let prog = loader.prog;
    for id in 0..prog.decls.len() {
        let d = &prog.decls[id];
        if d.name == name {
            let file = prog.modules[d.mod_id].path.clone();
            return Some((file, d.line, d.col, d.name.chars().count()));
        }
    }
    None
}

fn hover_text(entry: &str, name: &str) -> Option<String> {
    let mut diags = Diag::new();
    diags.collect_mode();
    let search = lsp_search_dirs(Some(entry));
    let mut loader = Loader::new(&mut diags, search);
    let _ = loader.load(entry);
    let prog = loader.prog;
    for d in prog.decls.iter() {
        if d.name == name {
            let mut s = decl_signature(d);
            if d.kind == ast::DeclKind::Class && !d.fields.is_empty() {
                s.push('\n');
                for f in d.fields.iter() {
                    s.push_str(&format!("  {} {}\n", f.ty, f.name));
                }
            }
            return Some(s);
        }
    }
    None
}

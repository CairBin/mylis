//! `.mylib`：后端无关的 MIR 库归档（P1）。
//!
//! 单文件文本归档，便于 diff、无第三方压缩依赖：
//! ```text
//! MYLIB 1
//! abi = "mylis-1"
//! name = "mathlib"
//! version = "0.1.0"
//! decl_count = 7
//! checksum = "sha256-..."
//! --- api
//! ...
//! --- mir
//! ; Mylis MIR v0
//! ...
//! --- mir-json
//! {...}
//! ```

use crate::ast::{Decl, DeclKind, Pattern, Program, Res, Type};
use crate::manifest;
use crate::mir::{self, MirClass, MirEnum, MirFunction, MirIfaceMethod, MirInst, MirInstKind, MirInterface, MirLocal, MirOperand, MirProgram, MirTerm, MirVariant};
use crate::mir_json;
use crate::sha256;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct NativeLib {
    pub name: String,
    pub data: Vec<u8>,
}

/// `.mylib` 的传递依赖元数据；薄归档只记录依赖，不把依赖实现写入归档。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MylibDependency {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Mylib {
    pub name: String,
    pub version: String,
    pub entry_stem: String,
    pub decl_count: usize,
    pub api: String,
    /// 子模块 API：模块名 -> 声明源码
    pub apis: Vec<(String, String)>,
    pub mir: MirProgram,
    /// 归档 MIR 中引用到的外部函数：decl_id -> 限定名。
    /// 归档只携带本包 MIR，跨包调用（如 std Vec 方法）的 id 必须按名字重映射。
    pub func_refs: HashMap<usize, String>,
    /// 归档 MIR 中引用到的外部类型：decl_id -> 限定名。
    pub type_refs: HashMap<usize, String>,
    /// 直接依赖：只记录元数据，不包含依赖的 API/MIR。
    pub dependencies: Vec<MylibDependency>,
    /// 原生载荷的目标三元组；仅带 native 载荷时记录。
    pub native_target: Option<String>,
    /// 原生载荷的 ABI（如 msvc / gnu）；仅带 native 载荷时记录。
    pub native_abi: Option<String>,
    /// 归档携带的原生静态库/动态库；有原生载荷时链接阶段优先使用它们，不再合并 MIR。
    pub native_static: Vec<NativeLib>,
    pub native_shared: Vec<NativeLib>,
}

impl Mylib {
    pub fn has_native(&self) -> bool {
        !self.native_static.is_empty() || !self.native_shared.is_empty()
    }
}

/// 从一个 API 段标签解析 (包名, 模块路径)。
/// 根包 API 标签不带包名前缀（`lib`、`vector`、`vector::A`）；
/// 依赖包标签以依赖名为前缀（`dep::lib`、`dep::vector::A`）。
pub fn api_label(root_name: &str, label: &str) -> (String, String) {
    if label == "lib" {
        return (root_name.to_string(), "lib".to_string());
    }
    if let Some(module) = label.strip_prefix(&format!("{}::", root_name)) {
        return (root_name.to_string(), module.to_string());
    }
    match label.split_once("::") {
        Some((pkg, module)) => (pkg.to_string(), module.to_string()),
        None => (root_name.to_string(), label.to_string()),
    }
}

fn module_key(prog: &Program, mod_id: usize) -> String {
    let m = &prog.modules[mod_id];
    if m.module_path.is_empty() {
        "lib".to_string()
    } else {
        m.module_path.join("::")
    }
}

fn type_pkg(prog: &Program, id: usize) -> Option<String> {
    let d = &prog.decls[id];
    prog.modules[d.mod_id].pkg_name.clone()
}

fn fmt_type(prog: &Program, t: &Type, cur_pkg: &str, cur_module: &str) -> String {
    match t {
        Type::Ptr(e) => format!("{}*", fmt_type(prog, e, cur_pkg, cur_module)),
        Type::Array(e) => format!("{}[]", fmt_type(prog, e, cur_pkg, cur_module)),
        Type::Option(e) => format!("Option<{}>", fmt_type(prog, e, cur_pkg, cur_module)),
        Type::Result(a, b) => format!("Result<{}, {}>", fmt_type(prog, a, cur_pkg, cur_module), fmt_type(prog, b, cur_pkg, cur_module)),
        Type::Class(id) | Type::Enum(id) | Type::Iface(id) => {
            // 后端签名里的泛型实例是 Class(Vec__i64) 这样的具体 id；
            // API 必须还原成 Vec<i64>，否则消费端会看到不可解析的 Vec__i64。
            if let Some((base, args, _)) = prog.generic_instances.iter()
                .find(|(_, _, inst)| inst == id)
            {
                let base_ty = match prog.decls[*base].kind {
                    DeclKind::Class => Type::Class(*base),
                    DeclKind::Enum => Type::Enum(*base),
                    DeclKind::Iface => Type::Iface(*base),
                    _ => Type::Class(*base),
                };
                let base_text = fmt_type(prog, &base_ty, cur_pkg, cur_module);
                let mut out = format!("{}<", base_text);
                for (i, a) in args.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    out.push_str(&fmt_type(prog, a, cur_pkg, cur_module));
                }
                out.push('>');
                return out;
            }
            let d = &prog.decls[*id];
            match type_pkg(prog, *id) {
                Some(pkg) if pkg == cur_pkg && module_key(prog, d.mod_id) == cur_module => d.name.clone(),
                Some(pkg) if !pkg.is_empty() => {
                    let m = &prog.modules[d.mod_id];
                    if m.module_leaf() == Some(d.name.as_str()) {
                        m.full_name()
                    } else {
                        format!("{}::{}", m.full_name(), d.name)
                    }
                }
                _ => d.name.clone(),
            }
        }
        _ => t.to_string(),
    }
}

/// Windows canonicalize 会带 `\\?\` 前缀，且大小写不敏感；
/// API 导出过滤不能直接用 Path::starts_with 比较 raw/canonical 路径。
fn norm_path_key(p: &Path) -> String {
    let raw = p.to_string_lossy().to_string();
    let mut s = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();
    if cfg!(windows) {
        s.make_ascii_lowercase();
    }
    s.replace('/', "\\")
}

fn path_starts_with(p: &Path, root: &Path) -> bool {
    let pk = norm_path_key(p);
    let rk = norm_path_key(root);
    if rk.is_empty() {
        return true;
    }
    pk == rk || pk.starts_with(&(rk.clone() + "\\"))
}

fn generic_sig(d: &Decl) -> String {
    if d.type_params.is_empty() { return String::new(); }
    let parts = d.type_params.iter().map(|n| {
        let bounds = d.type_bounds.iter()
            .find(|(p, _)| p == n)
            .map(|(_, bs)| bs.join(" + "))
            .unwrap_or_default();
        if bounds.is_empty() { n.clone() } else { format!("{}: {}", n, bounds) }
    }).collect::<Vec<_>>();
    format!("<{}>", parts.join(", "))
}

fn publ(prog: &Program, root: &Path, only_module: Option<&str>, type_root: &Path, cur_pkg: &str, imports: &[String]) -> String {
    let mut out = String::new();
    let _ = (imports, cur_pkg);
    let scope = only_module.unwrap_or("lib");
    let mut api_imports: Vec<String> = Vec::new();
    for (mid, m) in prog.modules.iter().enumerate() {
        if !path_starts_with(Path::new(&m.path), root) || module_key(prog, mid) != scope {
            continue;
        }
        for (i, (raw, _, _, is_pub, _)) in m.import_raw.iter().enumerate() {
            let names = m.import_selected.get(i).cloned().unwrap_or_default();
            let mut stmt = if names.is_empty() {
                format!("import \"{}\";", raw)
            } else {
                format!("import {{ {} }} from \"{}\";", names.join(", "), raw)
            };
            if *is_pub { stmt = format!("pub {}", stmt); }
            if !api_imports.contains(&stmt) { api_imports.push(stmt); }
        }
    }
    for stmt in &api_imports {
        out.push_str(stmt);
        out.push('\n');
    }
    if !api_imports.is_empty() { out.push('\n'); }
    let in_root = |d: &Decl| {
        let p = Path::new(&prog.modules[d.mod_id].path);
        path_starts_with(p, root) && module_key(prog, d.mod_id) == scope
    };
    let in_type = |d: &Decl| {
        let p = Path::new(&prog.modules[d.mod_id].path);
        path_starts_with(p, type_root) && module_key(prog, d.mod_id) == scope
    };
    let mut types = Vec::new();
    let mut funcs = Vec::new();
    for d in prog.decls.iter().chain(prog.template_api.iter()) {
        if !d.is_pub || d.owner.is_some() || d.generic_base.is_some() {
            continue;
        }
        match d.kind {
            DeclKind::Class | DeclKind::Enum | DeclKind::Iface => {
                if in_type(d) { types.push(d); }
            }
            DeclKind::Alias if d.alias_target != Type::Infer => {
                if in_type(d) { types.push(d); }
            }
            DeclKind::Fn => {
                if in_root(d) { funcs.push(d); }
            }
            _ => {}
        }
    }
    for d in &types {
        if !d.type_params.is_empty() {
            if let Some(src) = &d.source {
                out.push_str(src.trim());
                out.push_str("\n\n");
                continue;
            }
        }
        match d.kind {
            DeclKind::Class => {
                let _ = writeln!(out, "pub class {}{} {{", d.name, generic_sig(d));
                for f in &d.fields {
                    let vis = if f.is_pub || f.is_const { "pub " } else { "" };
                    let st = if f.is_static { "static " } else { "" };
                    let _ = writeln!(out, "  {}{}{} {};", vis, st, fmt_type(prog, &f.ty, cur_pkg, &scope), f.name);
                }
                let class_members: Vec<&Decl> = if d.members.is_empty() && !d.member_asts.is_empty() {
                    d.member_asts.iter().collect()
                } else {
                    d.members.iter().map(|m| &prog.decls[*m]).collect()
                };
                for m in class_members {
                    if m.is_ctor || m.is_pub {
                        let params = m
                            .params
                            .iter()
                            .map(|p| format!("{} {}", fmt_type(prog, &p.ty, cur_pkg, &scope), p.name))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let st = if m.is_static { "static " } else { "" };
                        if m.is_ctor {
                            let _ = writeln!(out, "  {}{}fn {}{}({});", if m.is_pub { "pub " } else { "" }, st, m.name, generic_sig(m), params);
                        } else {
                            let _ = writeln!(out, "  {}{}fn {}{}({}) -> {};", if m.is_pub { "pub " } else { "" }, st, m.name, generic_sig(m), params, fmt_type(prog, &m.ret, cur_pkg, &scope));
                        }
                    }
                }
                let _ = writeln!(out, "}}");
            }
            DeclKind::Enum => {
                let _ = write!(out, "pub enum {}{} {{", d.name, generic_sig(d));
                let mut first = true;
                for v in &d.variants {
                    if !first { let _ = write!(out, ", "); }
                    first = false;
                    let _ = write!(out, "{}", v.name);
                    if !v.fields.is_empty() {
                        let _ = write!(out, "({})", v.fields.iter().map(|t| fmt_type(prog, t, cur_pkg, &scope)).collect::<Vec<_>>().join(", "));
                    }
                }
                let enum_members: Vec<&Decl> = if d.members.is_empty() && !d.member_asts.is_empty() {
                    d.member_asts.iter().collect()
                } else {
                    d.members.iter().map(|m| &prog.decls[*m]).collect()
                };
                for m in enum_members {
                    if m.is_pub {
                        let params = m
                            .params
                            .iter()
                            .map(|p| format!("{} {}", fmt_type(prog, &p.ty, cur_pkg, &scope), p.name))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let _ = writeln!(out, "  pub fn {}{}({}) -> {};", m.name, generic_sig(m), params, fmt_type(prog, &m.ret, cur_pkg, &scope));
                    }
                }
                let _ = writeln!(out, " }}");
            }
            DeclKind::Alias => {
                let _ = writeln!(out, "pub type {} = {};", d.name, fmt_type(prog, &d.alias_target, cur_pkg, &scope));
            }
            DeclKind::Iface => {
                let _ = writeln!(out, "pub interface {}{} {{", d.name, generic_sig(d));
                let iface_members: Vec<&Decl> = if d.members.is_empty() && !d.member_asts.is_empty() {
                    d.member_asts.iter().collect()
                } else {
                    d.members.iter().map(|m| &prog.decls[*m]).collect()
                };
                for m in iface_members {
                    let params = m
                        .params
                        .iter()
                        .map(|p| format!("{} {}", fmt_type(prog, &p.ty, cur_pkg, &scope), p.name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "  fn {}{}({}) -> {};", m.name, generic_sig(m), params, fmt_type(prog, &m.ret, cur_pkg, &scope));
                }
                let _ = writeln!(out, "}}");
            }
            _ => {}
        }
    }
    for d in funcs {
        if !d.type_params.is_empty() {
            if let Some(src) = &d.source {
                out.push_str(src.trim());
                out.push_str("\n\n");
                continue;
            }
        }
        let params = d
            .params
            .iter()
            .map(|p| format!("{} {}", fmt_type(prog, &p.ty, cur_pkg, &scope), p.name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "pub fn {}{}({}) -> {};", d.name, generic_sig(d), params, fmt_type(prog, &d.ret, cur_pkg, &scope));
    }
    out
}

pub fn emit(
    prog: &Program,
    entry: &str,
    out: &Path,
    offline: bool,
    native_static: Option<(&str, &[u8])>,
    native_shared: Option<(&str, &[u8])>,
    native_target: Option<&str>,
    native_abi: Option<&str>,
) -> Result<(), String> {
    let man = manifest::locate_from(Path::new(entry).parent().unwrap_or(Path::new(".")));
    let (name, version) = man
        .as_ref()
        .and_then(|p| manifest::Manifest::load(p).ok())
        .map(|m| (m.name, m.version))
        .unwrap_or_else(|| {
            let stem = Path::new(entry)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "mylib".to_string());
            (stem, "0.1.0".to_string())
        });

    let root = man
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
        .unwrap_or_else(|| {
            std::fs::canonicalize(entry)
                .map(|p| p.parent().unwrap_or(Path::new(".")).to_path_buf())
                .unwrap_or_else(|_| PathBuf::from("."))
        });
    // 薄归档：API/MIR 只保留根包；依赖只写元数据。
    let has_native = native_static.is_some() || native_shared.is_some();
    let native_target = if has_native {
        native_target.filter(|s| !s.is_empty()).map(|s| s.to_string())
    } else {
        None
    };
    let native_abi = if has_native {
        native_abi.filter(|s| !s.is_empty()).map(|s| s.to_string())
    } else {
        None
    };
    let pkg_roots: Vec<(String, PathBuf)> = vec![(name.clone(), root.clone())];

    let man_data = man.as_ref().and_then(|p| manifest::Manifest::load(p).ok());
    let graph = man
        .as_ref()
        .and_then(|p| manifest::build_graph_with_pins_offline(p, None, offline).ok());
    let mut deps: Vec<MylibDependency> = Vec::new();
    if let Some(m) = &man_data {
        for d in &m.dependencies {
            let resolved = graph.as_ref().and_then(|g| g.find(&d.name));
            let version = resolved
                .map(|p| p.version.clone())
                .or_else(|| d.version.clone())
                .unwrap_or_default();
            let source = resolved
                .map(|p| p.source.clone())
                .unwrap_or_else(|| d.source_label());
            let checksum = resolved
                .map(|p| p.checksum.clone())
                .or_else(|| d.checksum.clone());
            deps.push(MylibDependency { name: d.name.clone(), version, source, checksum });
        }
    }
    deps.sort_by(|a, b| a.name.cmp(&b.name));

    let mir_roots: Vec<PathBuf> = pkg_roots.iter().map(|(_, r)| r.clone()).collect();
    // 包模式下入口模块的 module_key 通常带包名前缀（如 `B::B::src`），
    // 归档根 API 必须按入口实际模块生成，不能硬编码 `lib`。
    let entry_canon = std::fs::canonicalize(entry).unwrap_or_else(|_| PathBuf::from(entry));
    let root_scope = prog.modules.iter().enumerate()
        .filter(|(_, m)| path_starts_with(Path::new(&m.path), &root))
        .find(|(_, m)| std::fs::canonicalize(&m.path).map(|p| p == entry_canon).unwrap_or(false))
        .map(|(i, _)| module_key(prog, i))
        .or_else(|| prog.modules.iter().enumerate()
            .filter(|(_, m)| path_starts_with(Path::new(&m.path), &root))
            .map(|(i, _)| module_key(prog, i))
            .next())
        .unwrap_or_else(|| "lib".to_string());
    let api = publ(prog, &root, Some(&root_scope), &root, &name, &[]);
    let mut apis: Vec<(String, String)> = vec![("lib".to_string(), api.clone())];
    let mut seen = std::collections::BTreeSet::new();
    let mut module_names: Vec<String> = prog
        .modules
        .iter()
        .enumerate()
        .filter(|(_, m)| path_starts_with(Path::new(&m.path), &root))
        .filter(|(_, m)| m.pkg_name.as_deref().map_or(true, |p| p.is_empty() || p == name))
        .map(|(i, _)| module_key(prog, i))
        .collect();
    module_names.sort();
    module_names.dedup();
    for module in module_names {
        if module == root_scope { continue; }
        let sub = publ(prog, &root, Some(&module), &root, &name, &[]);
        if sub.trim().is_empty() { continue; }
        let label = format!("{}::{}", name, module);
        if seen.insert(label.clone()) {
            apis.push((label, sub));
        }
    }
    let full = mir::lower(prog);
    let func_refs: HashMap<usize, String> = full.funcs.iter()
        .map(|f| (f.decl_id, f.name.clone()))
        .collect();
    let mut type_refs: HashMap<usize, String> = HashMap::new();
    for c in &full.classes { type_refs.insert(c.decl_id, c.name.clone()); }
    for e in &full.enums { type_refs.insert(e.decl_id, e.name.clone()); }
    for i in &full.ifaces { type_refs.insert(i.decl_id, i.name.clone()); }
    let owned = |id: usize| {
        let d = &prog.decls[id];
        let path = Path::new(&prog.modules[d.mod_id].path);
        mir_roots.iter().any(|r| path_starts_with(path, r))
    };
    // 保留本包声明，并把它们引用到的外部具体实例（如 std 的 Map<string, i64>）
    // 一并闭包进来；不复制整个 std，只带运行所需的具体声明。
    let mut funcs: HashSet<usize> = full.funcs.iter().filter(|f| owned(f.decl_id)).map(|f| f.decl_id).collect();
    let mut classes: HashSet<usize> = full.classes.iter().filter(|c| owned(c.decl_id)).map(|c| c.decl_id).collect();
    let mut enums: HashSet<usize> = full.enums.iter().filter(|e| owned(e.decl_id)).map(|e| e.decl_id).collect();
    let mut ifaces: HashSet<usize> = full.ifaces.iter().filter(|i| owned(i.decl_id)).map(|i| i.decl_id).collect();
    loop {
        let mut need_funcs: HashSet<usize> = HashSet::new();
        let mut need_types: HashSet<usize> = HashSet::new();
        for f in full.funcs.iter().filter(|f| funcs.contains(&f.decl_id)) {
            collect_func_refs(f, &mut need_funcs, &mut need_types);
        }
        for c in full.classes.iter().filter(|c| classes.contains(&c.decl_id)) {
            collect_class_refs(c, &mut need_funcs, &mut need_types);
        }
        for e in full.enums.iter().filter(|e| enums.contains(&e.decl_id)) {
            for v in &e.variants { for t in &v.fields { walk_type_ids(t, &mut need_types); } }
        }
        for i in full.ifaces.iter().filter(|i| ifaces.contains(&i.decl_id)) {
            collect_iface_refs(i, &mut need_funcs, &mut need_types);
        }
        let mut changed = false;
        for id in need_funcs {
            if full.funcs.iter().any(|f| f.decl_id == id) && funcs.insert(id) { changed = true; }
        }
        for id in need_types {
            if full.classes.iter().any(|c| c.decl_id == id) {
                if classes.insert(id) { changed = true; }
            } else if full.enums.iter().any(|e| e.decl_id == id) {
                if enums.insert(id) { changed = true; }
            } else if full.ifaces.iter().any(|i| i.decl_id == id) {
                if ifaces.insert(id) { changed = true; }
            }
        }
        if !changed { break; }
    }
    let mp = MirProgram {
        funcs: full.funcs.into_iter().filter(|f| funcs.contains(&f.decl_id)).collect(),
        classes: full.classes.into_iter().filter(|c| classes.contains(&c.decl_id)).collect(),
        enums: full.enums.into_iter().filter(|e| enums.contains(&e.decl_id)).collect(),
        ifaces: full.ifaces.into_iter().filter(|i| ifaces.contains(&i.decl_id)).collect(),
    };
    let mir_text = mir::dump(&mp);
    let mir_json = mir_json::write(&mp);
    let mut refs_text = String::new();
    let mut frefs: Vec<_> = func_refs.iter().collect();
    frefs.sort_by_key(|(id, _)| **id);
    for (id, name) in frefs {
        if funcs.contains(id) { let _ = writeln!(refs_text, "f {} {}", id, name); }
    }
    let mut trefs: Vec<_> = type_refs.iter().collect();
    trefs.sort_by_key(|(id, _)| **id);
    for (id, name) in trefs {
        if classes.contains(id) || enums.contains(id) || ifaces.contains(id) {
            let _ = writeln!(refs_text, "t {} {}", id, name);
        }
    }
    let mut api_all = String::new();
    for (_, a) in &apis {
        api_all.push_str(a.trim_end());
        api_all.push('\n');
    }
    let native_text = native_sections(native_static, native_shared);
    let native_checksum = native_checksum_text(native_static, native_shared);
    let deps_text = deps.iter()
        .map(|d| format!(
            "dep = \"{}|{}|{}|{}\"",
            d.name,
            d.version,
            d.source,
            d.checksum.as_deref().unwrap_or("")
        ))
        .collect::<Vec<_>>()
        .join("\n");
    let native_meta_text = match (&native_target, &native_abi) {
        (Some(t), Some(a)) => format!("native_target = {}\nnative_abi = {}", t, a),
        (Some(t), None) => format!("native_target = {}", t),
        (None, Some(a)) => format!("native_abi = {}", a),
        (None, None) => String::new(),
    };
    let mut body = format!("{}\n{}\n{}", api_all.trim_end(), mir_text.trim_end(), mir_json.trim_end());
    if !refs_text.is_empty() {
        body.push('\n');
        body.push_str(refs_text.trim_end());
    }
    if !native_checksum.is_empty() {
        body.push('\n');
        body.push_str(native_checksum.trim_end());
    }
    if !native_meta_text.is_empty() {
        body.push('\n');
        body.push_str(&native_meta_text);
    }
    if !deps_text.is_empty() {
        body.push('\n');
        body.push_str(deps_text.trim_end());
    }
    let body = body;
    let checksum = format!("sha256-{}", sha256::hex(body.as_bytes()));
    let entry_stem = Path::new(entry)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "lib".to_string());

    let mut text = String::new();
    let _ = writeln!(text, "MYLIB 1");
    let _ = writeln!(text, "abi = \"mylis-1\"");
    let _ = writeln!(text, "name = \"{}\"", name);
    let _ = writeln!(text, "version = \"{}\"", version);
    let _ = writeln!(text, "entry_stem = \"{}\"", entry_stem);
    let _ = writeln!(text, "decl_count = {}", prog.decls.len());
    let _ = writeln!(text, "checksum = \"{}\"", checksum);
    if let Some(target) = &native_target {
        let _ = writeln!(text, "native_target = \"{}\"", target);
    }
    if let Some(abi) = &native_abi {
        let _ = writeln!(text, "native_abi = \"{}\"", abi);
    }
    if !deps_text.is_empty() {
        let _ = writeln!(text, "--- deps");
        text.push_str(&deps_text);
        text.push('\n');
    }
    for (name, a) in &apis {
        let _ = writeln!(text, "--- api {}", name);
        text.push_str(a);
        if !a.ends_with('\n') {
            text.push('\n');
        }
    }
    let _ = writeln!(text, "--- mir");
    text.push_str(&mir_text);
    let _ = writeln!(text, "--- mir-json");
    text.push_str(&mir_json);
    text.push('\n');
    if !refs_text.is_empty() {
        let _ = writeln!(text, "--- refs");
        text.push_str(&refs_text);
    }
    if !native_text.is_empty() {
        text.push_str(&native_text);
    }

    std::fs::write(out, text).map_err(|e| format!("写入 {} 失败: {}", out.display(), e))
}

fn native_checksum_text(static_lib: Option<(&str, &[u8])>, shared_lib: Option<(&str, &[u8])>) -> String {
    let mut out = String::new();
    if let Some((_, data)) = static_lib { out.push_str(&base64_encode(data)); }
    if let Some((_, data)) = shared_lib { out.push_str(&base64_encode(data)); }
    out
}

fn native_sections(static_lib: Option<(&str, &[u8])>, shared_lib: Option<(&str, &[u8])>) -> String {
    let mut out = String::new();
    if let Some((name, data)) = static_lib {
        let _ = writeln!(out, "--- native-static {}", name);
        out.push_str(&base64_encode(data));
        if !out.ends_with('\n') { out.push('\n'); }
    }
    if let Some((name, data)) = shared_lib {
        let _ = writeln!(out, "--- native-shared {}", name);
        out.push_str(&base64_encode(data));
        if !out.ends_with('\n') { out.push('\n'); }
    }
    out
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 65,
            'a'..='z' => c as u32 - 97 + 26,
            '0'..='9' => c as u32 - 48 + 52,
            '+' => 62,
            '/' => 63,
            '=' => continue,
            c if c.is_whitespace() => continue,
            _ => return Err(format!("非法字符 {}", c)),
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn header_value<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    header.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        if k.trim() == key {
            Some(v.trim().trim_matches('"'))
        } else {
            None
        }
    })
}

pub fn read(path: &Path) -> Result<Mylib, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取 {}: {}", path.display(), e))?;
    let mut header = String::new();
    let mut section = "header";
    let mut api = String::new();
    let mut apis: Vec<(String, String)> = Vec::new();
    let mut deps_raw = String::new();
    let mut dump = String::new();
    let mut json = String::new();
    let mut refs_raw = String::new();
    let mut native_static: Vec<(String, String)> = Vec::new();
    let mut native_shared: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            section = rest.trim();
            if let Some(name) = section.strip_prefix("api") {
                let name = name.trim();
                apis.push((if name.is_empty() { "lib".to_string() } else { name.to_string() }, String::new()));
            } else if let Some(name) = section.strip_prefix("native-static") {
                native_static.push((name.trim().to_string(), String::new()));
            } else if let Some(name) = section.strip_prefix("native-shared") {
                native_shared.push((name.trim().to_string(), String::new()));
            }
            continue;
        }
        match section {
            "header" => {
                header.push_str(line);
                header.push('\n');
            }
            "deps" => {
                deps_raw.push_str(line);
                deps_raw.push('\n');
            }
            _ if section.starts_with("api") => {
                if let Some((_, a)) = apis.last_mut() {
                    a.push_str(line);
                    a.push('\n');
                }
            }
            "mir" => {
                dump.push_str(line);
                dump.push('\n');
            }
            "mir-json" => {
                json.push_str(line);
                json.push('\n');
            }
            "refs" => {
                refs_raw.push_str(line);
                refs_raw.push('\n');
            }
            _ if section.starts_with("native-static") => {
                if let Some((_, p)) = native_static.last_mut() { p.push_str(line); }
            }
            _ if section.starts_with("native-shared") => {
                if let Some((_, p)) = native_shared.last_mut() { p.push_str(line); }
            }
            _ => {}
        }
    }
    if !text.starts_with("MYLIB 1") {
        return Err(format!("{} 不是 MYLIB 1 归档", path.display()));
    }
    let abi = header_value(&header, "abi").ok_or_else(|| "缺少 abi".to_string())?;
    if abi != "mylis-1" {
        return Err(format!("ABI 不匹配：需要 mylis-1，实际 {}", abi));
    }
    let name = header_value(&header, "name").unwrap_or("").to_string();
    let version = header_value(&header, "version").unwrap_or("").to_string();
    let entry_stem = header_value(&header, "entry_stem").unwrap_or("lib").to_string();
    let decl_count = header_value(&header, "decl_count")
        .and_then(|x| x.parse::<usize>().ok())
        .ok_or_else(|| "缺少 decl_count".to_string())?;
    let expected = header_value(&header, "checksum").ok_or_else(|| "缺少 checksum".to_string())?;
    let mut api_all = String::new();
    for (_, a) in &apis {
        api_all.push_str(a.trim_end());
        api_all.push('\n');
    }
    let native_static_text: String = native_static.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>().join("");
    let native_shared_text: String = native_shared.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>().join("");
    let deps_text = deps_raw.trim_end();
    let native_target = header_value(&header, "native_target").filter(|s| !s.is_empty()).map(|s| s.to_string());
    let native_abi = header_value(&header, "native_abi").filter(|s| !s.is_empty()).map(|s| s.to_string());
    let native_meta_text = match (&native_target, &native_abi) {
        (Some(t), Some(a)) => format!("native_target = {}\nnative_abi = {}", t, a),
        (Some(t), None) => format!("native_target = {}", t),
        (None, Some(a)) => format!("native_abi = {}", a),
        (None, None) => String::new(),
    };
    let mut body = format!("{}\n{}\n{}", api_all.trim_end(), dump.trim_end(), json.trim_end());
    let refs_text = refs_raw.trim_end();
    if !refs_text.is_empty() {
        body.push('\n');
        body.push_str(refs_text);
    }
    if !native_static_text.is_empty() {
        body.push('\n');
        body.push_str(native_static_text.trim_end());
    }
    if !native_shared_text.is_empty() {
        body.push('\n');
        body.push_str(native_shared_text.trim_end());
    }
    if !native_meta_text.is_empty() {
        body.push('\n');
        body.push_str(&native_meta_text);
    }
    if !deps_text.is_empty() {
        body.push('\n');
        body.push_str(deps_text);
    }
    let body = body;
    let actual = format!("sha256-{}", sha256::hex(body.as_bytes()));
    if actual != expected {
        return Err(format!("校验和不匹配：期望 {}，实际 {}", expected, actual));
    }
    let mut dependencies: Vec<MylibDependency> = Vec::new();
    for line in deps_raw.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let raw = line.strip_prefix("dep").and_then(|x| x.trim().strip_prefix('=')).map(|x| x.trim()).unwrap_or("");
        let raw = raw.trim_matches('"');
        if raw.is_empty() { continue; }
        let mut parts = raw.splitn(4, '|');
        let name = parts.next().unwrap_or("").to_string();
        let version = parts.next().unwrap_or("").to_string();
        let source = parts.next().unwrap_or("").to_string();
        let checksum = parts.next().map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
        if !name.is_empty() {
            dependencies.push(MylibDependency { name, version, source, checksum });
        }
    }
    dependencies.sort_by(|a, b| a.name.cmp(&b.name));
    let root_api = apis.iter().find(|(n, _)| n == "lib").map(|(_, a)| a.clone()).unwrap_or_default();
    let sub_apis = apis.into_iter().filter(|(n, _)| n != "lib").collect();
    let mir = mir_json::read(json.trim()).ok_or_else(|| "MIR JSON 解析失败".to_string())?;
    let mut func_refs: HashMap<usize, String> = HashMap::new();
    let mut type_refs: HashMap<usize, String> = HashMap::new();
    for line in refs_raw.lines() {
        let mut it = line.trim().splitn(3, ' ');
        let kind = it.next().unwrap_or("");
        let id = it.next().and_then(|x| x.parse::<usize>().ok());
        let name = it.next().unwrap_or("").trim();
        if name.is_empty() { continue; }
        if let Some(id) = id {
            match kind {
                "f" => { func_refs.insert(id, name.to_string()); }
                "t" => { type_refs.insert(id, name.to_string()); }
                _ => {}
            }
        }
    }
    let decode_all = |v: Vec<(String, String)>| -> Result<Vec<NativeLib>, String> {
        v.into_iter()
            .map(|(name, payload)| {
                let data = base64_decode(payload.trim()).map_err(|e| format!("原生库 {} 解码失败: {}", name, e))?;
                Ok(NativeLib { name, data })
            })
            .collect()
    };
    Ok(Mylib {
        name, version, entry_stem, decl_count, api: root_api, apis: sub_apis, mir,
        func_refs, type_refs,
        dependencies,
        native_target,
        native_abi,
        native_static: decode_all(native_static)?,
        native_shared: decode_all(native_shared)?,
    })
}

fn rid(x: usize, base: usize) -> usize {
    x + base
}
fn rty(t: &Type, base: usize, tmap: &HashMap<usize, usize>) -> Type {
    let map = |x: usize| *tmap.get(&x).unwrap_or(&rid(x, base));
    match t {
        Type::Class(x) => Type::Class(map(*x)),
        Type::Enum(x) => Type::Enum(map(*x)),
        Type::Iface(x) => Type::Iface(map(*x)),
        Type::Ptr(x) => Type::Ptr(Box::new(rty(x, base, tmap))),
        Type::Array(x) => Type::Array(Box::new(rty(x, base, tmap))),
        Type::Option(x) => Type::Option(Box::new(rty(x, base, tmap))),
        Type::Result(a, b) => Type::Result(Box::new(rty(a, base, tmap)), Box::new(rty(b, base, tmap))),
        x => x.clone(),
    }
}
fn rop(o: MirOperand) -> MirOperand {
    o
}
fn rres(r: &Res, base: usize, fns: &HashMap<usize, usize>, tmap: &HashMap<usize, usize>) -> Res {
    let tm = |x: usize| *tmap.get(&x).unwrap_or(&rid(x, base));
    match r {
        Res::Fn(x) => Res::Fn(*fns.get(x).unwrap_or(&rid(*x, base))),
        Res::Method(x) => Res::Method(*fns.get(x).unwrap_or(&rid(*x, base))),
        Res::Field(c, f) => Res::Field(tm(*c), *f),
        Res::Ctor(c) => Res::Ctor(tm(*c)),
        Res::IfaceCast(c, i) => Res::IfaceCast(tm(*c), tm(*i)),
        Res::Variant(e, v) => Res::Variant(tm(*e), *v),
        Res::StaticMethod(c, m) => Res::StaticMethod(tm(*c), *fns.get(m).unwrap_or(&rid(*m, base))),
        Res::StaticField(c, f) => Res::StaticField(tm(*c), *f),
        x => x.clone(),
    }
}
fn rpat(p: &Pattern, base: usize, tmap: &HashMap<usize, usize>) -> Pattern {
    match p {
        Pattern::Variant { ename, name, binds, enum_id, vidx } => Pattern::Variant {
            ename: ename.clone(),
            name: name.clone(),
            binds: binds.clone(),
            enum_id: *tmap.get(enum_id).unwrap_or(&rid(*enum_id, base)),
            vidx: *vidx,
        },
        x => x.clone(),
    }
}
fn rinst(i: &mut MirInst, base: usize, fns: &HashMap<usize, usize>, tmap: &HashMap<usize, usize>) {
    i.ty = rty(&i.ty, base, tmap);
    i.res = rres(&i.res, base, fns, tmap);
    if let Some((c, f)) = i.coerce {
        i.coerce = Some((
            *tmap.get(&c).unwrap_or(&rid(c, base)),
            *tmap.get(&f).unwrap_or(&rid(f, base)),
        ));
    }
    match &mut i.kind {
        MirInstKind::Cast { target, .. } => *target = rty(target, base, tmap),
        MirInstKind::Box { from, .. } => *from = rty(from, base, tmap),
        MirInstKind::MatchTest { pattern, .. } => *pattern = rpat(pattern, base, tmap),
        MirInstKind::BindPattern { pattern, .. } => *pattern = rpat(pattern, base, tmap),
        _ => {}
    }
    let _ = rop;
}


fn walk_type_ids(t: &Type, out: &mut HashSet<usize>) {
    match t {
        Type::Class(id) | Type::Enum(id) | Type::Iface(id) => { out.insert(*id); }
        Type::Ptr(e) | Type::Array(e) | Type::Option(e) => walk_type_ids(e, out),
        Type::Result(a, b) => {
            walk_type_ids(a, out);
            walk_type_ids(b, out);
        }
        _ => {}
    }
}

fn walk_pattern_ids(p: &Pattern, out: &mut HashSet<usize>) {
    if let Pattern::Variant { enum_id, .. } = p {
        out.insert(*enum_id);
    }
}

fn walk_res_refs(r: &Res, funcs: &mut HashSet<usize>, types: &mut HashSet<usize>) {
    match r {
        Res::Fn(id) | Res::Method(id) => { funcs.insert(*id); }
        Res::Field(c, _) | Res::Ctor(c) | Res::StaticField(c, _) => { types.insert(*c); }
        Res::IfaceCast(c, i) => { types.insert(*c); types.insert(*i); }
        Res::Variant(e, _) => { types.insert(*e); }
        Res::StaticMethod(c, m) => { types.insert(*c); funcs.insert(*m); }
        _ => {}
    }
}

fn collect_func_refs(f: &MirFunction, funcs: &mut HashSet<usize>, types: &mut HashSet<usize>) {
    walk_type_ids(&f.ret, types);
    for l in &f.params { walk_type_ids(&l.ty, types); }
    for l in &f.locals { walk_type_ids(&l.ty, types); }
    for b in &f.blocks {
        for i in &b.insts {
            walk_type_ids(&i.ty, types);
            if let Some((c, fi)) = i.coerce { types.insert(c); types.insert(fi); }
            walk_res_refs(&i.res, funcs, types);
            match &i.kind {
                MirInstKind::Cast { target, .. } => walk_type_ids(target, types),
                MirInstKind::Box { from, .. } => walk_type_ids(from, types),
                MirInstKind::MatchTest { pattern, .. } | MirInstKind::BindPattern { pattern, .. } => {
                    walk_pattern_ids(pattern, types);
                }
                _ => {}
            }
        }
    }
}

fn collect_class_refs(c: &MirClass, funcs: &mut HashSet<usize>, types: &mut HashSet<usize>) {
    for f in &c.fields { walk_type_ids(&f.ty, types); }
    if let Some(ctor) = c.ctor { funcs.insert(ctor); }
}

fn collect_iface_refs(i: &MirInterface, funcs: &mut HashSet<usize>, types: &mut HashSet<usize>) {
    for m in &i.methods {
        walk_type_ids(&m.ret, types);
        for t in &m.params { walk_type_ids(t, types); }
        funcs.insert(m.decl_id);
    }
}

/// 把库 MIR 的声明 id 整体平移到当前 Program 的尾部，并追加到当前 MIR。
pub fn merge_into(
    current: &mut MirProgram,
    lib: &Mylib,
    base: usize,
    api_ids: &HashMap<String, usize>,
    api_types: &HashMap<String, usize>,
) -> Result<(), String> {
    let mut tmap = HashMap::new();
    for c in &lib.mir.classes {
        // 消费端已经按同名具体实例生成了 class（例如 Box__i64）时，复用它的 id，
        // 避免库里的同名 class 被重新分配 id，导致消费端 MIR 的 Class(id) 与
        // 库 class 对不上。
        let target = current.classes.iter()
            .find(|x| x.name == c.name)
            .map(|x| x.decl_id)
            .or_else(|| api_types.get(&c.name).copied())
            .unwrap_or_else(|| rid(c.decl_id, base));
        tmap.insert(c.decl_id, target);
    }
    for e in &lib.mir.enums {
        let target = api_types
            .get(&e.name)
            .copied()
            .unwrap_or_else(|| rid(e.decl_id, base));
        tmap.insert(e.decl_id, target);
    }
    for i in &lib.mir.ifaces {
        let target = api_types
            .get(&i.name)
            .copied()
            .unwrap_or_else(|| rid(i.decl_id, base));
        tmap.insert(i.decl_id, target);
    }
    let mut fns = HashMap::new();
    for f in &lib.mir.funcs {
        let target = current.funcs.iter()
            .find(|x| x.name == f.name)
            .map(|x| x.decl_id)
            .or_else(|| api_ids.get(&f.name).copied())
            .unwrap_or_else(|| rid(f.decl_id, base));
        fns.insert(f.decl_id, target);
    }
    // 接口方法没有 MirFunction 条目，需要按 `<接口名>::<方法名>` 单独对齐 API 声明。
    for i in &lib.mir.ifaces {
        for m in &i.methods {
            let key = format!("{}::{}", i.name, m.name);
            if let Some(target) = api_ids.get(&key) {
                fns.insert(m.decl_id, *target);
            }
        }
    }
    // 归档只携带本包 MIR；外部依赖（如 std Vec）的类型/函数 id 必须按限定名
    // 对齐到消费端已有声明，不能按 `base + id` 平移，否则会串到别的类型上。
    for (id, name) in &lib.type_refs {
        if tmap.contains_key(id) { continue; }
        let target = api_types.get(name).copied()
            .or_else(|| current.classes.iter().find(|x| x.name == *name).map(|x| x.decl_id))
            .or_else(|| current.enums.iter().find(|x| x.name == *name).map(|x| x.decl_id))
            .or_else(|| current.ifaces.iter().find(|x| x.name == *name).map(|x| x.decl_id))
            .ok_or_else(|| format!("无法解析外部类型引用：{}（#{}）", name, id))?;
        tmap.insert(*id, target);
    }
    for (id, name) in &lib.func_refs {
        if fns.contains_key(id) { continue; }
        let target = api_ids.get(name).copied()
            .or_else(|| current.funcs.iter().find(|x| x.name == *name).map(|x| x.decl_id))
            .ok_or_else(|| format!("无法解析外部函数引用：{}（#{}）", name, id))?;
        fns.insert(*id, target);
    }
    for mut f in lib.mir.funcs.clone() {
        if current.funcs.iter().any(|x| x.name == f.name) {
            continue;
        }
        f.decl_id = *fns.get(&f.decl_id).unwrap_or(&rid(f.decl_id, base));
        f.owner = f.owner.map(|x| *tmap.get(&x).unwrap_or(&rid(x, base)));
        f.ret = rty(&f.ret, base, &tmap);
        for l in &mut f.params {
            l.ty = rty(&l.ty, base, &tmap);
        }
        for l in &mut f.locals {
            l.ty = rty(&l.ty, base, &tmap);
        }
        for b in &mut f.blocks {
            for i in &mut b.insts {
                rinst(i, base, &fns, &tmap);
            }
        }
        current.funcs.push(f);
    }
    for mut c in lib.mir.classes.clone() {
        c.decl_id = *tmap.get(&c.decl_id).unwrap_or(&rid(c.decl_id, base));
        c.ctor = c.ctor.map(|x| *fns.get(&x).unwrap_or(&rid(x, base)));
        for f in &mut c.fields {
            f.ty = rty(&f.ty, base, &tmap);
        }
        if let Some(existing) = current.classes.iter_mut().find(|x| x.name == c.name) {
            // 复用消费端已有 class，只补 ctor（字段由消费端模板实例化提供）。
            existing.ctor = c.ctor;
        } else {
            current.classes.push(c);
        }
    }
    for mut e in lib.mir.enums.clone() {
        e.decl_id = *tmap.get(&e.decl_id).unwrap_or(&rid(e.decl_id, base));
        for v in &mut e.variants {
            for t in &mut v.fields {
                *t = rty(t, base, &tmap);
            }
        }
        current.enums.retain(|x| x.name != e.name);
        current.enums.push(e);
    }
    for mut i in lib.mir.ifaces.clone() {
        i.decl_id = *tmap.get(&i.decl_id).unwrap_or(&rid(i.decl_id, base));
        for m in &mut i.methods {
            m.decl_id = *fns.get(&m.decl_id).unwrap_or(&rid(m.decl_id, base));
            m.ret = rty(&m.ret, base, &tmap);
            for t in &mut m.params {
                *t = rty(t, base, &tmap);
            }
        }
        current.ifaces.retain(|x| x.name != i.name);
        current.ifaces.push(i);
    }
    Ok(())
}

//! 编译驱动：定位工程根/GC、调用前端、调用 C 工具链、产出各类产物。

use crate::cli::{Args, Mode};
use mylc::manifest::{self, Target, TargetKind};
use mylc::lockfile;
use mylc::mylib;
use crate::i18n;
use mylc::backend::{self, Backend, ExportMode};
use mylc::{ast, diag, mir, program, sema, toolchain};

use diag::Diag;
use std::env;
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GcMode {
    Boehm,
    Simple,
    Own,
}



fn validate_native_archive(lib: &mylib::Mylib, archive: &Path, start: &Path) -> Result<(), String> {
    if !lib.has_native() {
        return Ok(());
    }
    let Some(build) = start.ancestors().find_map(|p| toolchain::BuildManifest::load(p)) else {
        return Ok(());
    };
    if let Some(abi) = &lib.native_abi {
        if !abi.is_empty() && !build.abi.eq_ignore_ascii_case(abi) {
            return Err(format!(
                "原生 .mylib 的 ABI 为 `{}`，当前构建清单为 `{}`：{}",
                abi, build.abi, archive.display()
            ));
        }
    }
    if let Some(target) = &lib.native_target {
        if !target.is_empty() && !build.target.eq_ignore_ascii_case(target) {
            return Err(format!(
                "原生 .mylib 的目标为 `{}`，当前构建清单为 `{}`：{}",
                target, build.target, archive.display()
            ));
        }
    }
    Ok(())
}


fn expand_mylib_dependencies(paths: &[String]) -> Result<Vec<String>, String> {
    fn visit(path: &Path, out: &mut Vec<String>, seen: &mut HashSet<String>) -> Result<(), String> {
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let key = canon.to_string_lossy().to_string();
        if !seen.insert(key.clone()) {
            return Ok(());
        }
        let lib = mylib::read(&canon)?;
        for dep in &lib.dependencies {
            let Some(raw) = dep.source.strip_prefix("mylib+") else { continue; };
            let dir = canon.parent().unwrap_or(Path::new("."));
            let name = Path::new(raw).file_name().map(|x| x.to_os_string());
            let candidates = [
                dir.join(raw),
                if raw.starts_with("../") { dir.join(raw.trim_start_matches("../")) } else { PathBuf::new() },
                name.as_ref().map(|n| dir.join(n)).unwrap_or_default(),
            ];
            let mut found = None;
            for c in candidates {
                if !c.as_os_str().is_empty() && c.is_file() {
                    found = Some(c);
                    break;
                }
            }
            let dep_path = found.ok_or_else(|| format!(
                "依赖 {} 的 .mylib 找不到：{}（来源 {}，归档目录 {}）",
                dep.name, raw, dep.source, dir.display()
            ))?;
            visit(&dep_path, out, seen)?;
        }
        out.push(key);
        Ok(())
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for p in paths {
        visit(Path::new(p), &mut out, &mut seen)?;
    }
    Ok(out)
}

fn extract_native_libs(lib: &mylib::Mylib, archive: &Path) -> Result<Vec<(PathBuf, bool)>, String> {
    let mut out = Vec::new();
    let root = env::var("MYLIS_MYLIB_CACHE")
        .map(PathBuf::from)
        .ok()
        .unwrap_or_else(|| {
            let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mylis/mylib")
        })
        .join("native");
    let _ = archive;
    for (shared, list) in [(false, &lib.native_static), (true, &lib.native_shared)] {
        for item in list {
            let key = mylc::sha256::hex(&item.data);
            let dir = root.join(&key);
            std::fs::create_dir_all(&dir).map_err(|e| format!("创建原生库缓存 {} 失败: {}", dir.display(), e))?;
            let name = Path::new(&item.name).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| if shared { "lib.so".into() } else { "lib.a".into() });
            let path = dir.join(name);
            if !path.exists() {
                std::fs::write(&path, &item.data).map_err(|e| format!("写入原生库 {} 失败: {}", path.display(), e))?;
            }
            out.push((path, shared));
        }
    }
    Ok(out)
}

fn gc_mode() -> GcMode {
    match env::var("MYLIS_GC").unwrap_or_default().to_ascii_lowercase().as_str() {
        "boehm" => GcMode::Boehm,
        "simple" => GcMode::Simple,
        _ => GcMode::Own, // 默认自研 GC
    }
}

fn find_root() -> PathBuf {
    if let Ok(r) = env::var("MYLIS_ROOT") {
        let p = PathBuf::from(r);
        if p.join("myrt/mylrt.h").is_file() {
            return p;
        }
    }
    if let Some(r) = option_env!("MYLIS_ROOT_DEFAULT") {
        let p = PathBuf::from(r);
        if p.join("myrt/mylrt.h").is_file() {
            return p;
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            // 自身目录优先：`bin/` 被整体拷贝走时，bin 内的 std/myrt 才是正确的一套
            if dir.join("myrt/mylrt.h").is_file() {
                return dir.to_path_buf();
            }
            if let Some(root) = dir.parent() {
                if root.join("myrt/mylrt.h").is_file() {
                    return root.to_path_buf();
                }
            }
            if dir.join("mylis-rt/myrt/mylrt.h").is_file() {
                return dir.join("mylis-rt");
            }
        }
    }
    PathBuf::from(".")
}

fn gc_root(root: &Path) -> PathBuf {
    if let Ok(r) = env::var("MYLIS_GC_ROOT") {
        return PathBuf::from(r);
    }
    let local = root.join("gc");
    if local.join("lib/libgc.a").is_file() {
        return local;
    }
    if let Ok(home) = env::var("HOME") {
        let p = PathBuf::from(home).join("mylis-toolchain");
        if p.join("lib/libgc.a").is_file() {
            return p;
        }
    }
    local
}

fn default_output(src: &str, mode: Mode, tc: &toolchain::Toolchain) -> String {
    let stem = Path::new(src)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "a".to_string());
    match mode {
        Mode::Exe => stem,
        Mode::Static => format!("{}{}{}", tc.static_prefix, stem, tc.static_suffix),
        Mode::Shared => format!("{}{}{}", tc.shared_prefix, stem, tc.shared_suffix),
        _ => stem,
    }
}

pub fn run(args: Args) -> i32 {
    compile_one(&args, &[])
}

/// 入口源码是否直接出现 `std/...` 导入；用于自动启用 std 平台后端。
fn source_uses_std(src: &str) -> bool {
    match std::fs::read_to_string(src) {
        Ok(text) => text.contains("\"std/") || text.contains("\"std\""),
        Err(_) => false,
    }
}

/// 工程清单是否直接或经路径依赖间接依赖 `std`。
fn manifest_uses_std(src: &str) -> bool {
    let Some(src_dir) = Path::new(src).parent() else { return false; };
    let Some(mpath) = manifest::locate_from(src_dir) else { return false; };
    let Ok(m) = manifest::Manifest::load(&mpath) else { return false; };
    let base = mpath.parent().unwrap_or(Path::new("."));
    let mut visited = std::collections::HashSet::new();
    manifest_depends_std(&m, base, &mut visited, 0)
}

fn manifest_depends_std(
    m: &manifest::Manifest,
    base: &Path,
    visited: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    for dep in &m.dependencies {
        if dep.name == "std" {
            return true;
        }
        let Some(rel) = &dep.path else { continue; };
        let dir = base.join(rel);
        let mpath = dir.join("mylis.toml");
        let Ok(canon) = mpath.canonicalize() else { continue; };
        if !visited.insert(canon) {
            continue;
        }
        let Ok(child) = manifest::Manifest::load(&mpath) else { continue; };
        if manifest_depends_std(&child, &dir, visited, depth + 1) {
            return true;
        }
    }
    false
}

/// 编译单个入口：`extra_includes` 是工程清单里声明的模块搜索根（相对 root）
pub fn compile_one(args: &Args, extra_includes: &[String]) -> i32 {
    // 清单里的 native 链接参数：相对清单目录解析，和命令行参数合并。
    let mut owned = args.clone();
    if let Some(src_dir) = Path::new(&owned.src).parent() {
        if let Some(mpath) = manifest::locate_from(src_dir) {
            if let Ok(m) = manifest::Manifest::load(&mpath) {
                let base = mpath.parent().unwrap_or(Path::new("."));
                for inc in m.native_includes {
                    let p = Path::new(&inc);
                    let path = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
                    if !owned.includes.iter().any(|x| Path::new(x) == path.as_path()) {
                        owned.includes.push(path.to_string_lossy().to_string());
                    }
                }
                for f in m.native_link_files {
                    let p = Path::new(&f);
                    let path = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
                    owned.link_files.push(path.to_string_lossy().to_string());
                }
                for d in m.native_link_dirs {
                    let p = Path::new(&d);
                    let path = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
                    owned.link_dirs.push(path.to_string_lossy().to_string());
                }
                owned.link_libs.extend(m.native_link_libs);
            }
        }
    }
    if !owned.std && (source_uses_std(&owned.src) || manifest_uses_std(&owned.src)) {
        if let Err(e) = crate::cli::apply_std_defaults(&mut owned) {
            eprintln!("mylc: {}", e);
            return 1;
        }
    }
    let args = &owned;
    let root = find_root();
    let project = manifest::locate().and_then(|p| p.parent().map(|x| x.to_path_buf()));
    // 包模式：入口文件所在包有依赖时，首次构建生成 mylis.lock；已有锁文件不静默改写。
    if let Some(src_dir) = Path::new(&args.src).parent() {
        if let Some(mpath) = manifest::locate_from(src_dir) {
            match lockfile::ensure_offline(&mpath, args.update_lock, args.offline) {
                Ok(true) => {
                    if args.verbose {
                        eprintln!("mylc: 已生成 {}", mpath.parent().unwrap_or(Path::new(".")).join("mylis.lock").display());
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    if args.verbose {
                        eprintln!("mylc: 无法生成 mylis.lock：{}", e);
                    }
                }
            }
        }
    }
    // 编译器只内置 core（语言运行时/插值协议/绑定），不再隐式搜索 std。
    // std 通过包依赖、-I 或 MYLIS_PATH 显式提供。
    let mut search: Vec<PathBuf> = vec![root.join("core")];
    for inc in extra_includes {
        let p = PathBuf::from(inc);
        search.push(if p.is_absolute() { p } else { root.join(p) });
    }
    for inc in &args.includes {
        search.push(PathBuf::from(inc));
    }
    if let Ok(extra) = env::var("MYLIS_PATH") {
        for p in extra.split(if cfg!(windows) { ';' } else { ':' }) {
            if !p.is_empty() {
                search.push(PathBuf::from(p));
            }
        }
    }

    let mut diags = Diag::new();

    let expanded_links = match expand_mylib_dependencies(&args.link_mylibs) {
        Ok(v) => v,
        Err(e) => { eprintln!("mylc: {}", e); return 1; }
    };
    let mut linked_mylibs: Vec<String> = Vec::new();
    let prog = {
        let mut loader = program::Loader::new(&mut diags, search);
        if !args.update_lock {
            if let Some(dir) = Path::new(&args.src).parent() {
                if let Some(mpath) = manifest::locate_from(dir) {
                    loader.set_pins(lockfile::read_pins(&mpath));
                    loader.set_offline(args.offline);
                }
            }
        }
        // 先注入 --link-mylib 及其传递依赖的 API，让入口 import "包/模块" 能在加载阶段解析。
        // 同一份 API 源码可能既作为独立归档的根 API，也作为依赖归档的 `依赖::模块`
        // 子 API 出现（例如 Wrap.mylib 与 Multi.mylib 同时链接）。只加载一次，
        // 否则同一个泛型模板会有两份 decl，实例化出两个同名具体类。
        let mut api_sources: HashMap<String, (String, String, PathBuf)> = HashMap::new();
        for path in &expanded_links {
            let lib = match mylib::read(Path::new(path)) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("mylc: {}: {}", path, e);
                    return 1;
                }
            };
            if let Err(e) = validate_native_archive(&lib, Path::new(path), Path::new(&args.src)) {
                eprintln!("mylc: {}: {}", path, e);
                return 1;
            }
            let project_root = Path::new(&args.src)
                .parent()
                .and_then(manifest::locate_from)
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| Path::new(&args.src).parent().unwrap_or(Path::new(".")).to_path_buf());
            let root_base = project_root.join(".mylib");
            let mut modules: Vec<(String, String)> = vec![("lib".to_string(), lib.api.clone())];
            modules.extend(lib.apis.clone());
            for (label, src) in modules {
                if src.trim().is_empty() {
                    continue;
                }
                let (pkg, module) = mylib::api_label(&lib.name, &label);
                let api_path = root_base.join(&pkg).join(format!("{}.myl", module.replace("::", "/")));
                let src_key = src.trim().to_string();
                if let Some((_, _, canon)) = api_sources.get(&src_key) {
                    loader.register_mylib_alias(&pkg, &module, canon.clone());
                    let stem = if module == "lib" { pkg.as_str() } else { module.rsplit("::").next().unwrap_or(&module) };
                    loader.register_api_file_alias(&format!("{}.myl", stem), canon.clone());
                    loader.register_api_file_alias(stem, canon.clone());
                    continue;
                }
                api_sources.insert(src_key, (pkg.clone(), module.clone(), api_path.clone()));
                let stem = if module == "lib" { pkg.as_str() } else { module.rsplit("::").next().unwrap_or(&module) };
                loader.register_api_file_alias(&format!("{}.myl", stem), api_path.clone());
                loader.register_api_file_alias(stem, api_path.clone());
                if let Some(mid) = loader.load_virtual(&api_path, &src, &args.src) {
                    // 必须走 set_pkg_name 重新计算 module_path：直接写 pkg_name 会让
                    // 根 API 虚拟模块保留 ["lib"]，表现为 native_lib::lib::answer，
                    // 而 import "native_lib" 后用户写 native_lib::answer 会失败。
                    loader.set_pkg_name(mid, &pkg);
                    loader.register_mylib_alias(&pkg, &module, api_path.clone());
                }
            }
        }
        let id = match loader.load(&args.src) {
            Some(id) => id,
            None => {
                eprintln!("mylc: {} {}", i18n::t("load.cannot"), args.src);
                return 1;
            }
        };
        linked_mylibs = expanded_links.clone();
        for a in loader.mylib_archives() {
            let s = a.to_string_lossy().to_string();
            if !linked_mylibs.contains(&s) {
                linked_mylibs.push(s);
            }
        }
        // 为每个 .mylib 注入 API 声明模块，让 sema 能解析调用。
        for path in &linked_mylibs {
            let lib = match mylib::read(Path::new(path)) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("mylc: {}: {}", path, e);
                    return 1;
                }
            };
            if let Err(e) = validate_native_archive(&lib, Path::new(path), Path::new(&args.src)) {
                eprintln!("mylc: {}: {}", path, e);
                return 1;
            }
            let project_root = Path::new(&args.src)
                .parent()
                .and_then(manifest::locate_from)
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| Path::new(&args.src).parent().unwrap_or(Path::new(".")).to_path_buf());
            let root_base = project_root.join(".mylib");
            let mut modules: Vec<(String, String)> = vec![("lib".to_string(), lib.api.clone())];
            modules.extend(lib.apis.clone());
            for (label, src) in modules {
                if src.trim().is_empty() {
                    continue;
                }
                let (pkg, module) = mylib::api_label(&lib.name, &label);
                let api_path = root_base.join(&pkg).join(format!("{}.myl", module.replace("::", "/")));
                let src_key = src.trim().to_string();
                if let Some((_, _, canon)) = api_sources.get(&src_key) {
                    loader.register_mylib_alias(&pkg, &module, canon.clone());
                    let stem = if module == "lib" { pkg.as_str() } else { module.rsplit("::").next().unwrap_or(&module) };
                    loader.register_api_file_alias(&format!("{}.myl", stem), canon.clone());
                    loader.register_api_file_alias(stem, canon.clone());
                    continue;
                }
                api_sources.insert(src_key, (pkg.clone(), module.clone(), api_path.clone()));
                let stem = if module == "lib" { pkg.as_str() } else { module.rsplit("::").next().unwrap_or(&module) };
                loader.register_api_file_alias(&format!("{}.myl", stem), api_path.clone());
                loader.register_api_file_alias(stem, api_path.clone());
                match loader.load_virtual(&api_path, &src, &args.src) {
                    Some(mid) => {
                        loader.set_pkg_name(mid, &pkg);
                        loader.register_mylib_alias(&pkg, &module, api_path.clone());
                    }
                    None => {
                        eprintln!("mylc: 无法加载 .mylib API：{}", path);
                        return 1;
                    }
                }
            }
        }
        if args.verbose {
            let list = loader
                .prog
                .modules
                .iter()
                .map(|m| m.stem.clone())
                .collect::<Vec<_>>()
                .join(" -> ");
            eprintln!("mylc: {}", i18n::tf("load.modules", &[("n", &loader.prog.modules.len().to_string()), ("list", &list)]));
            let _ = id;
        }
        loader.dedupe_prefixes();
        let mut prog = loader.prog;
        sema::analyze(&mut prog, &mut diags);
        prog
    };

    if args.mode == Mode::Check {
        if diags.errors > 0 {
            eprintln!("mylc: {}", i18n::tf("sema.failed", &[("n", &diags.errors.to_string())]));
            return 1;
        }
        println!("mylc: {}", i18n::tf("check.ok", &[("m", &prog.modules.len().to_string()), ("d", &prog.decls.len().to_string())]));
        return 0;
    }

    // MIR 库产物：只依赖前端与 MIR，不调用 C/LLVM 工具链。
    if args.mode == Mode::Mylib {
        if diags.errors > 0 {
            eprintln!("mylc: {}", i18n::tf("sema.failed", &[("n", &diags.errors.to_string())]));
            return 1;
        }
        let out = args
            .emit_mylib
            .clone()
            .or_else(|| args.out.clone())
            .unwrap_or_else(|| format!("{}.mylib", Path::new(&args.src).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "lib".to_string())));
        let native_static = match &args.embed_static {
            Some(path) => {
                let data = std::fs::read(path).map_err(|e| format!("读取 {} 失败: {}", path, e));
                match data {
                    Ok(d) => Some((Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "lib.a".to_string()), d)),
                    Err(e) => { eprintln!("mylc: {}", e); return 1; }
                }
            }
            None => None,
        };
        let native_shared = match &args.embed_shared {
            Some(path) => {
                let data = std::fs::read(path).map_err(|e| format!("读取 {} 失败: {}", path, e));
                match data {
                    Ok(d) => Some((Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "lib.so".to_string()), d)),
                    Err(e) => { eprintln!("mylc: {}", e); return 1; }
                }
            }
            None => None,
        };
        let native_static_ref = native_static.as_ref().map(|(n, d)| (n.as_str(), d.as_slice()));
        let native_shared_ref = native_shared.as_ref().map(|(n, d)| (n.as_str(), d.as_slice()));
        let build = toolchain::BuildManifest::load(&root);
        let native_target = if native_static.is_some() || native_shared.is_some() {
            build.as_ref().map(|b| b.target.as_str()).filter(|s| !s.is_empty())
        } else { None };
        let native_abi = if native_static.is_some() || native_shared.is_some() {
            build.as_ref().map(|b| b.abi.as_str()).filter(|s| !s.is_empty())
        } else { None };
        match mylib::emit(&prog, &args.src, Path::new(&out), args.offline, native_static_ref, native_shared_ref, native_target, native_abi) {
            Ok(()) => {
                println!("mylc: 已生成 MIR 库 {}", out);
                return 0;
            }
            Err(e) => {
                eprintln!("mylc: {}", e);
                return 1;
            }
        }
    }

    // MIR 库链接：先 lower 当前程序，再把 .mylib 的 MIR 合并进来。
    // 带原生载荷的归档改为链接原生库，不再合并 MIR，避免同一函数被定义两次。
    let mut native_links: Vec<(PathBuf, bool)> = Vec::new();
    let mut has_mir_archive = false;
    let linked_ir: Option<mir::MirProgram> = if linked_mylibs.is_empty() {
        None
    } else {
        let mut ir = mir::lower(&prog);
        let mut base = prog.decls.len();
        // .mylib 注入的 API 声明位于 `.mylib/<包名>/` 下；按 `模块::函数` 对齐库 MIR 的 decl id。
        let api_ids: std::collections::HashMap<String, usize> = prog
            .decls
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.kind == ast::DeclKind::Fn
                    && prog.modules[d.mod_id].path.contains(".mylib")
            })
            .map(|(id, d)| {
                let m = &prog.modules[d.mod_id];
                let prefix = m.full_name();
                let short = d.name.rsplit("::").next().unwrap_or(&d.name);
                let key = if let Some(owner) = d.owner {
                    let o = &prog.decls[owner];
                    let owner_key = if m.module_leaf() == Some(o.name.as_str()) {
                        prefix.clone()
                    } else {
                        format!("{}::{}", prefix, o.name)
                    };
                    format!("{}::{}", owner_key, short)
                } else {
                    format!("{}::{}", prefix, short)
                };
                (key, id)
            })
            .collect();
        let api_types: std::collections::HashMap<String, usize> = prog
            .decls
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                matches!(d.kind, ast::DeclKind::Class | ast::DeclKind::Enum | ast::DeclKind::Iface)
                    && prog.modules[d.mod_id].path.contains(".mylib")
            })
            .map(|(id, d)| {
                let m = &prog.modules[d.mod_id];
                let prefix = m.full_name();
                let key = if m.module_leaf() == Some(d.name.as_str()) {
                    prefix.clone()
                } else {
                    format!("{}::{}", prefix, d.name)
                };
                (key, id)
            })
            .collect();
        for path in &linked_mylibs {
            let lib = match mylib::read(Path::new(path)) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("mylc: {}: {}", path, e);
                    return 1;
                }
            };
            // 只丢弃该归档 API 导出的 dummy 函数体，由 .mylib 的真实 MIR 替换。
            // 归档闭包里的外部具体实例（如 std Vec::len）不是本归档导出，
            // 不能删消费端已有实例，否则其它函数的 Res::Method id 会悬空。
            for lf in &lib.mir.funcs {
                if api_ids.contains_key(&lf.name) {
                    ir.funcs.retain(|f| f.name != lf.name);
                }
            }
            if lib.has_native() {
                match extract_native_libs(&lib, Path::new(path)) {
                    Ok(mut libs) => native_links.append(&mut libs),
                    Err(e) => { eprintln!("mylc: {}: {}", path, e); return 1; }
                }
                continue;
            }
            has_mir_archive = true;
            if let Err(e) = mylib::merge_into(&mut ir, &lib, base, &api_ids, &api_types) {
                eprintln!("mylc: {}: {}", path, e);
                return 1;
            }
            base += lib.decl_count;
        }
        if (args.mode == Mode::Static || args.mode == Mode::Shared) && has_mir_archive {
            eprintln!("mylc: 静态库/动态库导出只能链接带原生载荷的 .mylib；改用 --embed-static/--embed-shared 生成归档");
            return 1;
        }
        Some(ir)
    };

    // IR 产物不依赖 C 工具链/GC：sema 通过后直接 lower + dump。
    if args.mode == Mode::Ir {
        if diags.errors > 0 {
            eprintln!("mylc: {}", i18n::tf("sema.failed", &[("n", &diags.errors.to_string())]));
            return 1;
        }
        let be = backend::IrBackend;
        let artifact = match &linked_ir {
            Some(ir) => be.emit_exe_mir(&prog, ir),
            None => be.emit_exe(&prog),
        };
        let target = args
            .emit_ir
            .clone()
            .or_else(|| args.out.clone())
            .unwrap_or_else(|| format!("{}.myl.ir", args.src));
        let ir = linked_ir.clone().unwrap_or_else(|| mir::lower(&prog));
        let errors = mir::verify(&ir);
        if args.verbose {
            eprintln!(
                "mylc: MIR functions={} blocks={} verify_errors={}",
                ir.funcs.len(),
                ir.funcs.iter().map(|f| f.blocks.len()).sum::<usize>(),
                errors.len()
            );
            for e in &errors {
                eprintln!("mylc: mir: {}", e);
            }
        }
        if let Err(e) = std::fs::write(&target, artifact.text) {
            eprintln!("mylc: {}: {}", i18n::tf("err.write_c", &[("path", &target)]), e);
            return 1;
        }
        println!("mylc: {}", i18n::tf("emit.ir_ok", &[("path", &target)]));
        return 0;
    }

    if args.mode == Mode::Llvm {
        let has_external_native = !native_links.is_empty()
            || !args.link_files.is_empty()
            || !args.link_libs.is_empty();
        let be = backend::LlvmBackend { native_externs: has_external_native };
        let artifact = match &linked_ir {
            Some(ir) => be.emit_exe_mir(&prog, ir),
            None => be.emit_exe(&prog),
        };
        if let Some(err) = &artifact.error {
            eprintln!("mylc: {}", err);
            return 1;
        }
        let target = args
            .emit_llvm
            .clone()
            .or_else(|| args.out.clone())
            .unwrap_or_else(|| format!("{}.myl.ll", args.src));
        if let Some(parent) = Path::new(&target).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&target, &artifact.text) {
            eprintln!("mylc: {}: {}", i18n::tf("err.write_c", &[("path", &target)]), e);
            return 1;
        }
        println!("mylc: {}", i18n::tf("emit.llvm_ok", &[("path", &target)]));
        return 0;
    }

    let entry_stem = prog.modules[prog.entry].stem.clone();
    let has_main = prog.decls.iter().any(|d| {
        d.kind == ast::DeclKind::Fn && d.name == "main" && d.mod_id == prog.entry
    });
    if args.mode != Mode::Static && args.mode != Mode::Shared && !has_main && diags.errors == 0 {
        eprintln!("mylc: {}", i18n::tf("entry.no_main", &[("stem", &entry_stem)]));
        return 1;
    }
    if diags.errors > 0 {
        eprintln!("mylc: {}", i18n::tf("sema.failed", &[("n", &diags.errors.to_string())]));
        return 1;
    }

    // 工具链与布局（不写死平台/随包路径）
    let want_llvm = match args.backend.as_deref() {
        Some("llvm") => true,
        None => args.mode != Mode::CSource,
        _ => false,
    };
    let mut tc = if want_llvm {
        let mut t = toolchain::Toolchain::builtin("clang")
            .unwrap_or_else(|| toolchain::Toolchain::detect(project.as_deref(), &root));
        // LLVM 14 及更早不支持 opaque pointers；优先选 15+ 的版本化二进制。
        for v in ["clang-20", "clang-19", "clang-18", "clang-17", "clang-16", "clang-15"] {
            if Command::new(v).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
                t.cc = v.to_string();
                let ver = v.strip_prefix("clang-").unwrap_or("");
                for cand in [format!("llvm-ar-{}", ver), "llvm-ar".to_string(), "ar".to_string()] {
                    if Command::new(&cand).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
                        t.ar = cand;
                        break;
                    }
                }
                t.origin = format!("LLVM 自动选择 {}", v);
                break;
            }
        }
        t
    } else {
        toolchain::Toolchain::detect(project.as_deref(), &root)
    };
    if let Some(cc) = &args.cc {
        tc = tc.with_cc(cc);
        tc.origin = "--cc".to_string();
    }
    let gc = gc_root(&root);
    let lay = toolchain::layout(&tc, &root, &gc);
    if args.verbose {
        eprintln!(
            "mylc: {}",
            i18n::tf(
                "toolchain.from",
                &[
                    ("origin", &tc.origin),
                    ("cc", &lay.cc),
                    ("ar", &lay.ar),
                    ("family", &tc.name),
                ]
            )
        );
    }

    let output = args
        .out
        .clone()
        .unwrap_or_else(|| default_output(&args.src, args.mode, &tc));

    // 生成后端产物（当前只有 C 后端；IR 后端将来走同一个 trait）
    let export = if tc.style == toolchain::Style::Msvc { ExportMode::DllExport } else { ExportMode::Default };
    let be: Box<dyn Backend> = match args.backend.as_deref() {
        Some("mir") => Box::new(backend::MirCBackend),
        Some("ir") => Box::new(backend::IrBackend),
        Some("llvm") => Box::new(backend::LlvmBackend { native_externs: !native_links.is_empty() }),
        Some("ast") | Some("c") => Box::new(backend::CBackend { export }),
        // 生成 C 源码时保持 MIR-C；其余默认走原生 LLVM 后端。
        None if args.mode == Mode::CSource => Box::new(backend::MirCBackend),
        None => Box::new(backend::LlvmBackend {
            native_externs: !native_links.is_empty() || !args.link_files.is_empty() || !args.link_libs.is_empty(),
        }),
        Some(_) => Box::new(backend::CBackend { export }),
    };
    let artifact = match args.mode {
        Mode::Static | Mode::Shared => be.emit_lib(&prog, export),
        _ => match &linked_ir {
            Some(ir) => be.emit_exe_mir(&prog, ir),
            None => be.emit_exe(&prog),
        },
    };
    if let Some(err) = &artifact.error {
        eprintln!("mylc: {}", err);
        return 1;
    }
    let mut csrc = artifact.text;
    if !native_links.is_empty() && artifact.kind == backend::ArtifactKind::CSource {
        let proto = native_prototypes(&prog);
        if let Some(pos) = csrc.find("#include \"mylrt_gc.h\"\n") {
            let at = pos + "#include \"mylrt_gc.h\"\n".len();
            csrc.insert_str(at, &proto);
        } else {
            csrc = proto + &csrc;
        }
    }
    if args.mode == Mode::CSource {
        let target = args.emit_c.clone().unwrap_or_else(|| output.clone());
        if let Some(parent) = Path::new(&target).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&target, &csrc) {
            eprintln!("mylc: {}: {}", i18n::tf("err.write_c", &[("path", &target)]), e);
            return 1;
        }
        println!("mylc: {}", i18n::tf("emit.c_ok", &[("path", &target)]));
        return 0;
    }

    let is_llvm = artifact.kind == backend::ArtifactKind::LlvmIr;
    // 构建清单校验：build 由 Makefile/打包脚本生成；缺失时保持旧包兼容。
    // MYLIS_NO_MANIFEST=1 仅用于编译器自身的工具链命令拼装测试（假 CC/AR）。
    let skip_manifest = env::var("MYLIS_NO_MANIFEST")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let build_manifest = toolchain::BuildManifest::load(&root);
    if !skip_manifest {
        if let Some(man) = &build_manifest {
            if let Err(msg) = man.validate(&tc, &lay, &root, is_llvm) {
                eprintln!("mylc: {}", msg);
                return 1;
            }
        }
    }
    let runtime_is_msvc = (is_llvm && cfg!(windows)) || tc.style == toolchain::Style::Msvc;
    let win_system_libs: Vec<String> = if runtime_is_msvc {
        match build_manifest.as_ref().and_then(|m| m.system_libs.as_ref()) {
            Some(libs) => libs.clone(),
            None => default_msvc_system_libs(),
        }
    } else {
        Vec::new()
    };
    // Windows 默认 LLVM 后端仍用 clang 驱动，但运行时是 MSVC 静态库；
    // 不能因为 clang 是 GNU 方言就去找 libmylrt.a。
    let llvm_windows = is_llvm && cfg!(windows);
    let cpath = PathBuf::from(format!("{}.myl.{}", output, if is_llvm { "ll" } else { "c" }));
    if let Err(e) = std::fs::write(&cpath, &csrc) {
        eprintln!("mylc: {}: {}", i18n::tf("err.write_c", &[("path", &cpath.display().to_string())]), e);
        return 1;
    }

    // GC 后端：boehm（默认）/ simple（malloc 不回收）/ own（自研 GC）
    let mut gc = gc_mode();
    let header = be.header(&prog);
    if tc.style == toolchain::Style::Msvc {
        // MSVC 没有随包的 Boehm 静态库：boehm/own 一律落到自研 GC（mylgc.lib），
        // 只有显式 MYLIS_GC=simple 才允许不回收（调试用）。
        if gc != GcMode::Simple {
            if !lay.mylgc_msvc_lib.is_file() {
                eprintln!("mylc: {}", i18n::t("err.msvc_no_gc"));
                return 1;
            }
            if gc == GcMode::Boehm {
                eprintln!("mylc: {}", i18n::t("warn.msvc_boehm_own"));
            }
            gc = GcMode::Own;
        }
        return emit_msvc(&tc, &lay, args, &root, gc, &cpath, &output, &header, &win_system_libs);
    }
    let myrt_path = if llvm_windows { &lay.myrt_msvc_lib } else { &lay.myrt_lib };
    let simple_path = if llvm_windows { &lay.simple_msvc_lib } else { &lay.simple_lib };
    let mylgc_path = if llvm_windows { &lay.mylgc_msvc_lib } else { &lay.mylgc_lib };
    let myrt_ok = myrt_path.is_file();
    let gc_ok = match gc {
        GcMode::Simple => simple_path.is_file(),
        GcMode::Boehm => lay.gc_lib.is_file() && lay.boehm_lib.is_file(),
        GcMode::Own => mylgc_path.is_file(),
    };
    if !myrt_ok {
        eprintln!(
            "mylc: {}",
            i18n::tf("err.no_rt", &[("path", &myrt_path.display().to_string())])
        );
        return 1;
    }
    if !gc_ok {
eprintln!("mylc: {}", i18n::t("err.no_gc"));
        return 1;
    }

    let mut cmd = Command::new(&lay.cc);
    if is_llvm {
        cmd.arg("-O2").arg("-Xclang").arg("-opaque-pointers");
    } else {
        for f in &tc.cflags {
            cmd.arg(f);
        }
    }
    cmd.arg(format!("-I{}", lay.myrt_inc.display()))
        .arg(format!("-I{}", lay.gc_inc.display()));
    // `-I` 首先是 Mylis 模块搜索根，同时也作为 C 头文件搜索路径，
    // 这样 std 的 `@header("mylis_file.h")` 可以随包路径一起被找到。
    for d in &args.includes {
        cmd.arg(format!("-I{}", d));
    }
    for d in &tc.ldflags {
        cmd.arg(d);
    }
    for d in &args.link_dirs {
        cmd.arg(format!("-L{}", d));
    }
    cmd.arg(&cpath);

    match args.mode {
        Mode::Static => {
            let obj = PathBuf::from(format!("{}.o", output));
            cmd.arg("-c").arg("-o").arg(&obj);
            run_cmd(&mut cmd, args.verbose, &lay.cc);
            if native_links.iter().any(|(_, shared)| *shared) {
                eprintln!("mylc: 静态库导出不能嵌入动态库载荷");
                return 1;
            }
            let archive_obj = if native_links.is_empty() {
                obj.clone()
            } else {
                let merged = PathBuf::from(format!("{}.native.o", output));
                let mut ld = Command::new(&lay.cc);
                ld.arg("-r").arg("-o").arg(&merged).arg(&obj);
                for (p, _) in &native_links {
                    ld.arg("-Wl,--whole-archive").arg(p).arg("-Wl,--no-whole-archive");
                }
                run_cmd(&mut ld, args.verbose, &lay.cc);
                merged
            };
            let st = Command::new(&lay.ar)
                .arg("rcs")
                .arg(&output)
                .arg(&archive_obj)
                .status()
                .expect("无法启动 ar");
            if !st.success() {
                eprintln!("mylc: {}", i18n::tf("err.ar_fail", &[("ar", &lay.ar)]));
                return 1;
            }
            let _ = std::fs::remove_file(&obj);
            if archive_obj != obj { let _ = std::fs::remove_file(&archive_obj); }
        }
        Mode::Shared => {
            if !cfg!(windows) {
                cmd.arg("-fPIC");
            }
            cmd.arg("-shared");
            if cfg!(windows) {
                cmd.arg("-Wl,--export-all-symbols").arg("-static-libgcc");
            }
            cmd.arg("-o").arg(&output);
            // 动态库默认不内嵌运行时（Boehm 静态库通常非 PIC）；宿主负责提供 myrt 符号。
            // 需要自包含时加 --embed-runtime，并要求运行库本身以 -fPIC 构建。
            // Windows 的 DLL 不需要 PIC，静态库可直接链入，因此默认内嵌运行时
            // （PE 链接器不允许未定义符号，宿主无法后补 myrt_*）。
            if args.embed_runtime || cfg!(windows) {
                cmd.arg(myrt_path);
                match gc {
                    GcMode::Boehm => { cmd.arg(&lay.boehm_lib); cmd.arg(&lay.gc_lib); }
                    GcMode::Own => { cmd.arg(mylgc_path); }
                    GcMode::Simple => { cmd.arg(simple_path); }
                }
                if llvm_windows {
                    add_windows_system_libs(&mut cmd, &win_system_libs, true);
                }
            }
            add_native_links(&mut cmd, &native_links);
            for l in &args.link_libs {
                cmd.arg(format!("-l{}", l));
            }
            for f in &args.link_files {
                cmd.arg(f);
            }
            run_cmd(&mut cmd, args.verbose, &lay.cc);
        }
        _ => {
            cmd.arg("-o").arg(&output);
            cmd.arg(myrt_path);
            match gc {
                GcMode::Boehm => { cmd.arg(&lay.boehm_lib); cmd.arg(&lay.gc_lib); }
                GcMode::Own => { cmd.arg(mylgc_path); }
                GcMode::Simple => { cmd.arg(simple_path); }
            }
            if llvm_windows {
                add_windows_system_libs(&mut cmd, &win_system_libs, true);
            }
            for l in &args.link_libs {
                cmd.arg(format!("-l{}", l));
            }
            for f in &args.link_files {
                cmd.arg(f);
            }
            add_native_links(&mut cmd, &native_links);
            if llvm_windows {
                // clang 用 MSVC 目标链接 .lib；不要传 GNU -static/-static-libgcc。
                cmd.arg("-Wl,/SUBSYSTEM:CONSOLE");
            } else if cfg!(windows) {
                cmd.arg("-static").arg("-static-libgcc").arg("-Wl,--subsystem,console");
            } else {
                cmd.arg("-lm").arg("-lpthread").arg("-ldl");
            }
            run_cmd(&mut cmd, args.verbose, &lay.cc);
        }
    }

    if args.mode == Mode::Static || args.mode == Mode::Shared {
        let hpath = PathBuf::from(output.clone()).with_extension("h");
        let _ = std::fs::write(&hpath, &header);
        let kind = if args.mode == Mode::Static { i18n::t("kind.static") } else { i18n::t("kind.shared") };
        println!("mylc: {}", i18n::tf("emit.lib", &[("kind", kind), ("path", &output), ("hdr", &hpath.display().to_string())]));
    } else {
        println!("mylc: {}", i18n::tf("emit.exe", &[("path", &output)]));
    }

    if args.run {
        match Command::new(&output).status() {
            Ok(s) => return s.code().unwrap_or(1),
            Err(e) => {
                eprintln!("mylc: {}: {}", i18n::tf("err.run", &[("path", &output)]), e);
                return 1;
            }
        }
    }
    0
}

/// `mylc build [target]` / `mylc run [target]`：按工程清单构建
pub fn build(only: &Option<String>, run_after: bool, update_lock: bool, offline: bool, std: bool) -> i32 {
    let mpath = match manifest::locate() {
        Some(p) => p,
        None => {
            eprintln!("mylc: 当前目录（或其上级）没有 {}，无法按工程构建", manifest::FILE);
            return 1;
        }
    };
    let man = match manifest::Manifest::load(&mpath) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("mylc: {}", e);
            return 1;
        }
    };
    eprintln!("mylc: 工程 {} v{}", man.name, man.version);
    let base = mpath.parent().unwrap_or(Path::new(".")).to_path_buf();
    if std::env::set_current_dir(&base).is_err() {
        eprintln!("mylc: 无法进入工程目录 {}", base.display());
        return 1;
    }
    // 包模式只声明 [lib] 时，合成一个静态库目标；显式 [targets.*] 优先。
    let implicit_lib = man.targets.is_empty() && man.lib_entry.is_some();
    let targets: Vec<Target> = if implicit_lib {
        vec![Target {
            name: man.name.clone(),
            kind: TargetKind::Static,
            entry: man.lib_entry.clone().unwrap_or_default(),
            out: None,
        }]
    } else {
        man.targets.clone()
    };
    if targets.is_empty() {
        eprintln!("mylc: {} 里没有声明任何 [targets.<name>] 或 [lib] entry", manifest::FILE);
        return 1;
    }
    let mut produced: Vec<String> = Vec::new();
    for t in &targets {
        if let Some(only) = only {
            if &t.name != only {
                continue;
            }
        }
        let mode = match t.kind {
            TargetKind::Exe => Mode::Exe,
            TargetKind::Static => Mode::Static,
            TargetKind::Shared => Mode::Shared,
        };
        let out = t
            .out
            .clone()
            .unwrap_or_else(|| format!("build/{}", t.name));
        if let Some(parent) = Path::new(&out).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if t.entry.is_empty() {
            eprintln!("mylc: 目标 `{}` 缺少 entry", t.name);
            return 1;
        }
        let mut args = Args {
            src: t.entry.clone(),
            out: Some(out.clone()),
            emit_c: None,
            emit_ir: None,
            emit_llvm: None,
            emit_mylib: None,
            update_lock,
            offline,
            link_mylibs: Vec::new(),
            backend: man.backend.clone(),
            mode,
            run: false,
            verbose: false,
            includes: Vec::new(),
            link_libs: Vec::new(),
            link_dirs: Vec::new(),
            link_files: Vec::new(),
            cc: None,
            embed_runtime: false,
            embed_static: None,
            embed_shared: None,
            std,
        };
        if args.std {
            if let Err(e) = crate::cli::apply_std_defaults(&mut args) {
                eprintln!("mylc: {}", e);
                return 1;
            }
        }
        let abs_paths: Vec<String> = man
            .paths
            .iter()
            .map(|p| base.join(p).to_string_lossy().to_string())
            .collect();
        let code = compile_one(&args, &abs_paths);
        if code != 0 {
            return code;
        }
        produced.push(out);
    }
    if only.is_some() && produced.is_empty() {
        eprintln!("mylc: 工程里没有目标 {}", only.clone().unwrap_or_default());
        return 1;
    }
    if run_after {
        if let Some(p) = produced.last() {
            return match std::process::Command::new(p).status() {
                Ok(s) => s.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("mylc: 无法运行 {}: {}", p, e);
                    1
                }
            };
        }
    }
    0
}

/// `mylc test`：编译并运行 `tests/**/*.myl`，与同名 `.expected` 比对
pub fn test() -> i32 {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = loop {
        if dir.join("tests").is_dir() {
            break dir.clone();
        }
        if !dir.pop() {
            break PathBuf::from(".");
        }
    };
    // 工程清单存在时，测试也使用其中的搜索根
    let paths = manifest::locate()
        .and_then(|p| manifest::Manifest::load(&p).ok())
        .map(|m| {
            let base = manifest::locate()
                .and_then(|p| p.parent().map(|x| x.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            m.paths
                .iter()
                .map(|p| base.join(p).to_string_lossy().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let out_dir = root.join("build/tests");
    let _ = std::fs::create_dir_all(&out_dir);
    let mut pass = 0;
    let mut fail = 0;
    let mut files: Vec<PathBuf> = Vec::new();
    collect_myl(&root.join("tests"), &mut files);
    for f in files {
        let name = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if name.ends_with("_bind") {
            continue;
        }
        if f.components()
            .any(|c| c.as_os_str() == "negative" || c.as_os_str() == "lib")
        {
            continue;
        }
        let bin = out_dir.join(&name);
        let args = Args {
            src: f.to_string_lossy().to_string(),
            out: Some(bin.to_string_lossy().to_string()),
            emit_c: None,
            emit_ir: None,
            emit_llvm: None,
            emit_mylib: None,
            update_lock: false,
            offline: false,
            link_mylibs: Vec::new(),
            backend: None,
            mode: Mode::Exe,
            run: false,
            verbose: false,
            includes: Vec::new(),
            link_libs: Vec::new(),
            link_dirs: Vec::new(),
            link_files: Vec::new(),
            cc: None,
            embed_runtime: false,
            embed_static: None,
            embed_shared: None,
            std: false,
        };
        if compile_one(&args, &paths) != 0 {
            eprintln!("  FAIL {}（编译失败）", name);
            fail += 1;
            continue;
        }
        let expected = f.with_extension("expected");
        let output = match std::process::Command::new(&bin).output() {
            Ok(o) => o.stdout,
            Err(_) => {
                eprintln!("  FAIL {}（无法运行）", name);
                fail += 1;
                continue;
            }
        };
        if expected.is_file() {
            let want = std::fs::read_to_string(&expected).unwrap_or_default();
            if want.replace("\r\n", "\n") == String::from_utf8_lossy(&output).replace("\r\n", "\n") {
                println!("  ok   {}", name);
                pass += 1;
            } else {
                eprintln!("  FAIL {}（输出不符）", name);
                fail += 1;
            }
        } else {
            println!("  ok   {}（编译通过）", name);
            pass += 1;
        }
    }
    println!("\n通过：{}    失败：{}", pass, fail);
    if fail == 0 { 0 } else { 1 }
}

fn collect_myl(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_myl(&p, out);
        } else if p.extension().map(|x| x == "myl").unwrap_or(false) {
            out.push(p);
        }
    }
}


/// Rust std（windows-msvc）静态库引用的系统库。
/// 不显式列出来时，cl 会在链接 mylrt.lib 时报 __imp_closesocket / NtReadFile 等未解析。
const MSVC_RT_SYSTEM_LIBS: &[&str] = &[
    "ws2_32.lib",
    "ntdll.lib",
    "userenv.lib",
    "advapi32.lib",
    "bcrypt.lib",
    "ole32.lib",
    "oleaut32.lib",
    "shell32.lib",
];

fn add_windows_system_libs(cmd: &mut Command, libs: &[String], dash_l: bool) {
    for lib in libs {
        if dash_l {
            let name = lib.strip_suffix(".lib").unwrap_or(lib);
            cmd.arg(format!("-l{}", name));
        } else {
            cmd.arg(lib);
        }
    }
}

fn default_msvc_system_libs() -> Vec<String> {
    MSVC_RT_SYSTEM_LIBS.iter().map(|s| s.to_string()).collect()
}

/// MSVC 家族（cl.exe + lib.exe）：运行时源码随产物一起编译。
/// GC：默认/boehm/own → 自研 GC（链 mylgc.lib）；simple → 源码内联的 gc_simple。
fn emit_msvc(
    tc: &toolchain::Toolchain,
    lay: &toolchain::Layout,
    args: &Args,
    root: &Path,
    gc: GcMode,
    cpath: &PathBuf,
    output: &str,
    header: &str,
    system_libs: &[String],
) -> i32 {
    let gc_c = lay.myrt_inc.join("gc").join("gc_simple.c");

    let mut cmd = Command::new(&lay.cc);
    for f in &tc.cflags {
        cmd.arg(f);
    }
    cmd.arg("/I").arg(&lay.myrt_inc);
    cmd.arg("/I").arg(&lay.gc_inc);
    for d in &args.includes {
        cmd.arg("/I").arg(d);
    }
    for d in &tc.ldflags {
        cmd.arg(d);
    }
    cmd.arg(cpath);
    cmd.arg(&lay.myrt_msvc_lib);
    match gc {
        GcMode::Simple => { cmd.arg(&gc_c); }
        GcMode::Boehm | GcMode::Own => { cmd.arg(&lay.mylgc_msvc_lib); }
    }

    if args.mode != Mode::Static {
        let dash_l = tc.cc.to_ascii_lowercase().contains("clang")
            && !tc.cc.to_ascii_lowercase().contains("clang-cl");
        add_windows_system_libs(&mut cmd, system_libs, dash_l);
    }

    match args.mode {
        Mode::Static => {
            let obj = format!("{}{}", output, tc.obj_suffix);
            cmd.arg("/c").arg(format!("/Fo{}", obj));
            run_cmd(&mut cmd, args.verbose, &lay.cc);
            let st = Command::new(&lay.ar)
                .arg("/nologo")
                .arg(format!("/OUT:{}", output))
                .arg(&obj)
                .status()
                .expect("无法启动 lib");
            if !st.success() {
                eprintln!("mylc: {}", i18n::tf("err.ar_fail", &[("ar", &lay.ar)]));
                return 1;
            }
            let _ = std::fs::remove_file(&obj);
            let hpath = PathBuf::from(output).with_extension("h");
            let _ = std::fs::write(&hpath, header);
            println!(
                "mylc: {}",
                i18n::tf(
                    "emit.lib",
                    &[
                        ("kind", i18n::t("kind.static")),
                        ("path", output),
                        ("hdr", &hpath.display().to_string()),
                    ]
                )
            );
        }
        Mode::Shared => {
            cmd.arg("/LD").arg(format!("/Fe:{}", output));
            cmd.arg("/link");
            for d in &args.link_dirs {
                cmd.arg(format!("/LIBPATH:{}", d));
            }
            for l in &args.link_libs {
                cmd.arg(tc.link_lib.replace("{name}", l));
            }
            for f in &args.link_files {
                cmd.arg(f);
            }
            run_cmd(&mut cmd, args.verbose, &lay.cc);
            let hpath = PathBuf::from(output).with_extension("h");
            let _ = std::fs::write(&hpath, header);
            println!(
                "mylc: {}",
                i18n::tf(
                    "emit.lib",
                    &[
                        ("kind", i18n::t("kind.shared")),
                        ("path", output),
                        ("hdr", &hpath.display().to_string()),
                    ]
                )
            );
        }
        _ => {
            cmd.arg(format!("/Fe:{}", output));
            cmd.arg("/link").arg("/SUBSYSTEM:CONSOLE");
            for d in &args.link_dirs {
                cmd.arg(format!("/LIBPATH:{}", d));
            }
            for l in &args.link_libs {
                cmd.arg(tc.link_lib.replace("{name}", l));
            }
            for f in &args.link_files {
                cmd.arg(f);
            }
            run_cmd(&mut cmd, args.verbose, &lay.cc);
            println!("mylc: {}", i18n::tf("emit.exe", &[("path", output)]));
        }
    }

    if args.run {
        match Command::new(output).status() {
            Ok(s) => return s.code().unwrap_or(1),
            Err(e) => {
                eprintln!("mylc: {}: {}", i18n::tf("err.run", &[("path", output)]), e);
                return 1;
            }
        }
    }
    let _ = root;
    0
}

fn native_prototypes(prog: &ast::Program) -> String {
    let mut out = String::new();
    for d in &prog.decls {
        if d.kind != ast::DeclKind::Fn || d.owner.is_some() {
            continue;
        }
        let m = &prog.modules[d.mod_id];
        if !m.path.contains(".mylib") {
            continue;
        }
        let name = mylc::codegen::export_name(d)
            .unwrap_or_else(|| d.name.rsplit("::").next().unwrap_or(&d.name).to_string());
        let ret = mylc::codegen::ctype_of(prog, &d.ret);
        let params: Vec<String> = d
            .params
            .iter()
            .map(|p| format!("{} {}", mylc::codegen::ctype_of(prog, &p.ty), p.name))
            .collect();
        out.push_str(&format!("extern {} {}({});\n", ret, name, if params.is_empty() { "void".to_string() } else { params.join(", ") }));
    }
    out
}

fn add_native_links(cmd: &mut Command, links: &[(PathBuf, bool)]) {
    for (p, shared) in links {
        cmd.arg(p);
        if *shared {
            if let Some(dir) = p.parent() {
                cmd.arg(format!("-Wl,-rpath,{}", dir.display()));
            }
        }
    }
}

fn run_cmd(cmd: &mut Command, verbose: bool, cc: &str) {
    if verbose {
        eprintln!("mylc: {}", i18n::tf("exec", &[("cmd", &format!("{:?}", cmd))]));
    }
    let st = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "mylc: {}: {}",
                i18n::tf("err.cc_spawn", &[("cc", cc)]),
                e
            );
            std::process::exit(1);
        }
    };
    if !st.success() {
        eprintln!("mylc: {}", i18n::tf("err.cc_fail", &[("cc", cc)]));
        std::process::exit(1);
    }
}

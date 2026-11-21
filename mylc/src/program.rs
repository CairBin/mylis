//! 模块加载：import 解析、递归收集、循环导入检测（MYL011）、Kahn 拓扑排序。

use crate::ast::*;
use crate::diag::{Diag, E_GENERAL, E_IO};
use crate::manifest;
use crate::mylib;
use crate::parser::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Loader<'a> {
    pub prog: Program,
    diags: &'a mut Diag,
    search: Vec<PathBuf>,
    index: HashMap<String, usize>,
    /// 编辑器里未落盘的缓冲区（路径 → 文本），语言服务器用它做“虚拟文件系统”
    overlay: Option<HashMap<String, String>>,
    /// 入口包解析后的路径依赖图；没有 mylis.toml 时为 None
    pkg_graph: Option<manifest::PackageGraph>,
    /// 入口包根目录与包名
    root_pkg: Option<(PathBuf, String)>,
    /// 锁文件 pin（存在时强制按锁解析）
    pins: Option<manifest::LockPins>,
    /// 离线模式：不允许 git fetch / 下载 .mylib
    offline: bool,
    /// `--link-mylib` 注入的 import 别名：包名/模块路径 → 虚拟模块路径。
    mylib_aliases: HashMap<String, PathBuf>,
}

impl<'a> Loader<'a> {
    pub fn set_pins(&mut self, pins: Option<manifest::LockPins>) {
        self.pins = pins;
    }

    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    pub fn new(diags: &'a mut Diag, search: Vec<PathBuf>) -> Self {
        Loader {
            prog: Program::new(),
            diags,
            search,
            index: HashMap::new(),
            overlay: None,
            pkg_graph: None,
            root_pkg: None,
            pins: None,
            offline: false,
            mylib_aliases: HashMap::new(),
        }
    }

    /// 注入内存中的文档内容（语言服务器用），优先于磁盘
    pub fn set_overlay(&mut self, overlay: HashMap<String, String>) {
        self.overlay = Some(overlay);
    }

    fn sanitize(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                out.push(c);
            } else {
                out.push('_');
            }
        }
        if out.is_empty() {
            out.push('_');
        }
        out
    }


    /// 统一路径键：Windows 的 canonicalize 会带 `\\?\` 前缀，虚拟模块路径不带，
    /// 必须去掉前缀并做大小写归一，否则 import 查不到注入的 `.mylib` 模块。
    fn path_key(path: &Path) -> String {
        let raw = path.to_string_lossy().to_string();
        let mut s = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();
        if cfg!(windows) {
            s.make_ascii_lowercase();
        }
        s
    }

    /// 登记 `--link-mylib` 的 import 别名，避免依赖 Windows 路径前缀/大小写。
    pub fn register_mylib_alias(&mut self, pkg: &str, module: &str, path: PathBuf) {
        if module == "lib" {
            self.mylib_aliases.insert(pkg.to_string(), path);
        } else {
            self.mylib_aliases.insert(format!("{}/{}", pkg, module), path);
        }
    }

    /// 为归档 API 源码中出现的 `import ... from "Foo.myl"` 登记文件别名。
    /// 同一个 API 段可能同时来自独立归档和依赖归档的子 API 段，必须让二者
    /// 解析到同一个虚拟模块，否则同一份 `Wrap<T>` 会被加载两次。
    pub fn register_api_file_alias(&mut self, raw: &str, path: PathBuf) {
        self.mylib_aliases.insert(raw.to_string(), path);
    }

    /// 在 `.mylib` 虚拟模块中查找 import：从当前模块目录向上找最近的 `.mylib`，
    /// 支持根模块 `<pkg>/lib.myl` 与子模块 `<pkg>/<module>.myl`。
    fn resolve_virtual_import(&self, from: &ModuleInfo, raw: &str) -> Option<PathBuf> {
        if raw.contains(".myl") || raw.starts_with("./") || raw.starts_with("../") {
            return None;
        }
        let mut dir = Path::new(&from.dir);
        loop {
            let vbase = dir.join(".mylib");
            let virt = if let Some((pkg, module)) = raw.split_once('/') {
                vbase.join(pkg).join(format!("{}.myl", module.replace('\\', "/")))
            } else {
                vbase.join(raw).join("lib.myl")
            };
            let key = Self::path_key(&virt);
            if self.index.contains_key(&key) {
                return Some(virt);
            }
            dir = dir.parent()?;
        }
    }

    fn resolve_import(&self, from: &ModuleInfo, raw: &str) -> Option<PathBuf> {
        // `--link-mylib` 注入的虚拟模块优先于同名源码文件/包入口。
        if let Some(p) = self.mylib_aliases.get(raw) {
            return Some(p.clone());
        }
        if let Some(v) = self.resolve_virtual_import(from, raw) {
            return Some(v);
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if raw.starts_with("./") || raw.starts_with("../") {
            candidates.push(Path::new(&from.dir).join(raw));
        } else {
            candidates.push(Path::new(&from.dir).join(raw));
            if !raw.ends_with(".myl") {
                candidates.push(Path::new(&from.dir).join(format!("{}.myl", raw)));
            }
            for root in &self.search {
                candidates.push(root.join(raw));
                if !raw.ends_with(".myl") {
                    candidates.push(root.join(format!("{}.myl", raw)));
                }
            }
        }
        for c in candidates {
            if c.is_file() {
                return std::fs::canonicalize(&c).ok().or(Some(c));
            }
        }
        self.resolve_package_import(from, raw)
    }

    /// 包名 import：`import "mathlib"` / `import "mathlib/vector"`
    /// 依赖从最近的 mylis.toml 读取，依赖路径相对 manifest 所在目录。
    fn resolve_package_import(&self, from: &ModuleInfo, raw: &str) -> Option<PathBuf> {
        if raw.starts_with("./") || raw.starts_with("../") || raw.contains(".myl") {
            return None;
        }
        let graph = self.pkg_graph.as_ref()?;
        let first = raw.split('/').next().unwrap_or(raw);
        let rest = raw[first.len()..].trim_start_matches('/');
        let dep = match graph.find(first) {
            Some(d) => d,
            None => {
                // `.mylib` 归档内的依赖包不是图里的独立 package；按 API 段标签回查，
                // 例如 import "dep/Foo" -> api 段 "dep::Foo"。
                if !rest.is_empty() {
                    let label = format!("{}::{}", first, rest.replace('/', "::"));
                    if let Some(path) = graph
                        .packages
                        .iter()
                        .flat_map(|p| p.api_paths.iter())
                        .find(|(n, _)| n == &label)
                        .map(|(_, p)| p.clone())
                    {
                        return Some(path);
                    }
                }
                return None;
            }
        };
        if let Some(root_api) = &dep.api_path {
            if rest.is_empty() {
                return Some(root_api.clone());
            }
            let rest_label = rest.replace('/', "::");
            return dep
                .api_paths
                .iter()
                .find(|(n, _)| {
                    let (p, m) = crate::mylib::api_label(&dep.name, n);
                    p == first && m == rest_label
                })
                .map(|(_, p)| p.clone());
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        if rest.is_empty() {
            let entry = dep
                .manifest
                .lib_entry
                .clone()
                .unwrap_or_else(|| "src/lib.myl".to_string());
            candidates.push(dep.root.join(entry));
        } else {
            candidates.push(dep.root.join("src").join(format!("{}.myl", rest)));
            candidates.push(dep.root.join(format!("{}.myl", rest)));
            candidates.push(dep.root.join(rest));
        }
        for c in candidates {
            if c.is_file() {
                return std::fs::canonicalize(&c).ok().or(Some(c));
            }
        }
        None
    }

    /// 加载入口文件，递归加载全部依赖，返回入口 module id
    pub fn load(&mut self, entry: &str) -> Option<usize> {
        let path = std::fs::canonicalize(entry).unwrap_or_else(|_| PathBuf::from(entry));
        if !path.is_file() {
            self.diags
                .err(E_IO, entry, 1, 1, format!("找不到源文件：{}", entry));
            return None;
        }
        if let Some(dir) = path.parent() {
            if let Some(mpath) = manifest::locate_from(dir) {
                match manifest::build_graph_with_pins_offline(&mpath, self.pins.as_ref(), self.offline) {
                    Ok(g) => {
                        self.pkg_graph = Some(g);
                        if let (Some(base), Ok(man)) = (mpath.parent(), manifest::Manifest::load(&mpath)) {
                            self.root_pkg = Some((base.to_path_buf(), man.name));
                        }
                    }
                    Err(e) => {
                        self.diags.err(E_IO, entry, 1, 1, e);
                        return None;
                    }
                }
            }
        }
        // `.mylib` 依赖：先把归档 API 注入为虚拟模块，import "包名" 才能解析。
        let api_libs: Vec<(PathBuf, PathBuf, String, String)> = self
            .pkg_graph
            .as_ref()
            .map(|g| {
                g.packages
                    .iter()
                    .filter_map(|p| {
                        let a = p.mylib.clone()?;
                        Some(p.api_paths.iter().filter_map(move |(module, path)| {
                            // 根 API 段名就是 "lib"，它不是包名；根模块必须用包名，
                            // 否则 `import "pkg"` 后 `pkg::fn` 无法解析。
                            let pkg = if module == "lib" {
                                p.name.clone()
                            } else {
                                module.split("::").next().unwrap_or(&p.name).to_string()
                            };
                            Some((path.clone(), a.clone(), pkg, module.clone()))
                        }))
                    })
                    .flatten()
                    .collect()
            })
            .unwrap_or_default();
        for (api_path, archive, name, module) in api_libs {
            let lib = match mylib::read(&archive) {
                Ok(l) => l,
                Err(e) => {
                    self.diags.err(E_IO, entry, 1, 1, format!("无法读取 .mylib {}: {}", archive.display(), e));
                    return None;
                }
            };
            let src = if module == "lib" { lib.api.clone() } else {
                lib.apis.iter().find(|(n, _)| n == &module).map(|(_, s)| s.clone()).unwrap_or_default()
            };
            if src.trim().is_empty() {
                continue;
            }
            match self.load_virtual(&api_path, &src, entry) {
                Some(mid) => self.set_pkg_name(mid, &name),
                None => {
                    self.diags.err(E_IO, entry, 1, 1, format!("无法加载 .mylib API：{}", archive.display()));
                    return None;
                }
            }
        }
        let id = self.load_module(&path, entry)?;
        self.prog.entry = id;
        self.attach_prelude();
        Some(id)
    }

    /// 隐式预置 ToString 接口：所有模块都可见，插值用
    fn attach_prelude(&mut self) {
        let mut target: Option<PathBuf> = None;
        for root in &self.search {
            for cand in ["ToString.myl", "core/ToString.myl"] {
                let c = root.join(cand);
                if c.is_file() {
                    target = Some(c);
                    break;
                }
            }
            if target.is_some() {
                break;
            }
        }
        let path = match target {
            Some(p) => p,
            None => return,
        };
        let pid = match self.load_module(&path, "<prelude>") {
            Some(p) => p,
            None => return,
        };
        for m in 0..self.prog.modules.len() {
            if m == pid {
                continue;
            }
            if !self.prog.modules[m].imports.contains(&pid) {
                self.prog.modules[m].imports.push(pid);
            }
        }
    }

    fn package_root_for(&self, path: &Path, pkg: Option<&str>) -> Option<PathBuf> {
        if pkg.is_some() {
            // `.mylib/<pkg>/...` 虚拟模块优先：它们也可能位于项目根之下，
            // 不能先按源码包根匹配，否则模块路径会多出 `.mylib/<pkg>`。
            let comps: Vec<String> = path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            if let Some(i) = comps.iter().position(|c| c == ".mylib") {
                if i + 1 < comps.len() {
                    let root = PathBuf::from(comps[..=i + 1].join("/"));
                    return Some(root);
                }
            }
            if let Some(graph) = &self.pkg_graph {
                for p in &graph.packages {
                    if path.starts_with(&p.root) {
                        return Some(p.root.clone());
                    }
                }
            }
            if let Some((root, _)) = &self.root_pkg {
                if path.starts_with(root) {
                    return Some(root.clone());
                }
            }
        }
        None
    }

    fn module_path_for(&self, path: &Path, pkg: Option<&str>, stem: &str) -> Vec<String> {
        let pkg = match pkg { Some(p) => p, None => return vec![stem.to_string()] };
        let root = match self.package_root_for(path, Some(pkg)) { Some(r) => r, None => return vec![stem.to_string()] };
        let rel = path.strip_prefix(&root).unwrap_or(path);
        let mut comps: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .filter(|s| s != ".")
            .collect();
        if comps.first().map(|s| s == "src").unwrap_or(false) {
            comps.remove(0);
        }
        if let Some(last) = comps.last_mut() {
            if let Some(stem) = Path::new(last).file_stem().and_then(|s| s.to_str()) {
                *last = stem.to_string();
            }
        }
        if comps.last().map(|s| s == "lib" || s == "mod").unwrap_or(false) {
            comps.pop();
        }
        comps
    }

    fn package_name_for(&self, path: &Path) -> Option<String> {
        let graph = self.pkg_graph.as_ref()?;
        for p in &graph.packages {
            if p.api_paths.iter().any(|(_, api)| path == api) {
                return Some(p.name.clone());
            }
            if path.starts_with(&p.root) {
                return Some(p.name.clone());
            }
        }
        if let Some((root, name)) = &self.root_pkg {
            if path.starts_with(root) {
                return Some(name.clone());
            }
        }
        None
    }

    fn load_module(
        &mut self,
        path: &Path,
        site: &str,
    ) -> Option<usize> {
        let key = Self::path_key(path);
        if let Some(id) = self.index.get(&key) {
            return Some(*id);
        }

        let src = match self
            .overlay
            .as_ref()
            .and_then(|ov| ov.get(&key).cloned())
            .map(Ok)
            .unwrap_or_else(|| std::fs::read_to_string(path))
        {
            Ok(s) => s,
            Err(e) => {
                self.diags
                    .err(E_IO, site, 1, 1, format!("读取 {} 失败：{}", key, e));
                return None;
            }
        };

        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let dir = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        let is_bind = stem.ends_with("_bind");

        let mod_id = self.prog.modules.len();
        let disp = key.clone();
        let mut ast = {
            let mut p = Parser::new(&disp, &src, self.diags);
            p.parse_module()
        };

        let pkg_name = self.package_name_for(path);
        let module_path = self.module_path_for(path, pkg_name.as_deref(), &stem);
        self.prog.modules.push(ModuleInfo {
            path: key.clone(),
            stem: stem.clone(),
            module_path,
            pkg_name,
            dir,
            is_bind,
            prefix: Self::sanitize(&stem),
            import_raw: Vec::new(),
            import_selected: Vec::new(),
            imported: Vec::new(),
            reexports: Vec::new(),
            imports: Vec::new(),
            decls: Vec::new(),
            state: 1, // 访问中
            topo: 0,
        });
        self.index.insert(key.clone(), mod_id);

        // 展开声明
        for d in ast.decls.drain(..) {
            let id = self.push_decl(d, mod_id);
            self.prog.modules[mod_id].decls.push(id);
        }
        self.prog.modules[mod_id].import_raw = ast
            .imports
            .iter()
            .map(|i| (i.path.clone(), i.line, i.col, i.is_pub, i.selective))
            .collect();
        self.prog.modules[mod_id].import_selected = ast.imports.iter().map(|i| i.names.clone()).collect();

        // 递归解析依赖
        let imports = ast.imports;
        let mut selected_names: HashMap<String, (u32, u32)> = HashMap::new();
        for imp in imports {
            let (raw, line, col, names, is_pub) = (imp.path, imp.line, imp.col, imp.names, imp.is_pub);
            let target = match self.resolve_import_by_id(mod_id, &raw) {
                Some(p) => p,
                None => {
                    self.diags.err(
                        E_IO,
                        &disp,
                        line,
                        col,
                        format!("找不到模块 \"{}\"（搜索路径：文件目录 + core + -I/包依赖）", raw),
                    );
                    continue;
                }
            };
            let tkey = Self::path_key(&target);
            match self.index.get(&tkey).copied() {
                Some(tid) => {
                    if self.prog.modules[tid].state == 1 {
                        self.diags.err(
                            11,
                            &disp,
                            line,
                            col,
                            format!(
                                "循环导入：{} -> {}",
                                self.prog.modules[mod_id].path, self.prog.modules[tid].path
                            ),
                        );
                        continue;
                    }
                    self.prog.modules[mod_id].imports.push(tid);
                }
                None => {
                    let tid = self.load_module(&target, &disp)?;
                    self.prog.modules[mod_id].imports.push(tid);
                }
            };
            let tid = *self.prog.modules[mod_id].imports.last().unwrap_or(&mod_id);
            for name in names {
                if let Some((l, c)) = selected_names.get(&name) {
                    self.diags.err(
                        11,
                        &disp,
                        line,
                        col,
                        format!("显式导入名 `{}` 重复：已在 {}:{} 导入", name, l, c),
                    );
                    continue;
                }
                if self.prog.modules[mod_id].decls.iter().any(|id| self.prog.decls[*id].name == name) {
                    self.diags.err(11, &disp, line, col, format!("显式导入名 `{}` 与当前模块声明冲突", name));
                    continue;
                }
                selected_names.insert(name.clone(), (line, col));
                self.prog.modules[mod_id].imported.push((name.clone(), tid));
                if is_pub {
                    self.prog.modules[mod_id].reexports.push((name, tid));
                }
            }
        }

        self.prog.modules[mod_id].state = 2;
        let topo = self.prog.order.len();
        self.prog.modules[mod_id].topo = topo;
        self.prog.order.push(mod_id);
        Some(mod_id)
    }

    fn resolve_import_by_id(&self, mod_id: usize, raw: &str) -> Option<PathBuf> {
        let from = &self.prog.modules[mod_id];
        self.resolve_import(from, raw)
    }

    fn push_decl(&mut self, mut d: Decl, mod_id: usize) -> usize {
        d.mod_id = mod_id;
        let module_path = self.prog.modules[mod_id].path.clone();
        crate::derive::expand(&mut d, &module_path, self.diags);
        let mut mids = Vec::new();
        let member_asts: Vec<Decl> = d.member_asts.drain(..).collect();
        for mut m in member_asts {
            m.mod_id = mod_id;
            // 构造器：与类同名的方法（推荐），或历史写法 `init`
            if d.kind == DeclKind::Class && (m.name == d.name || m.name == "init") {
                m.is_ctor = true;
            }
            let id = self.push_decl(m, mod_id);
            mids.push(id);
        }
        d.members = mids.clone();
        let id = self.prog.decls.len();
        self.prog.decls.push(d);
        for m in mids {
            self.prog.decls[m].owner = Some(id);
        }
        id
    }


    /// 让入口模块直接看见一个库 API 模块（`.mylib` 的隐式 import）。
    pub fn expose_to_entry(&mut self, id: usize) {
        if id == self.prog.entry { return; }
        if !self.prog.modules[self.prog.entry].imports.contains(&id) {
            self.prog.modules[self.prog.entry].imports.push(id);
        }
    }

    /// 当前依赖图里的 `.mylib` 归档（用于 driver 链接 MIR）。
    pub fn mylib_archives(&self) -> Vec<PathBuf> {
        self.pkg_graph
            .as_ref()
            .map(|g| g.packages.iter().filter_map(|p| p.mylib.clone()).collect())
            .unwrap_or_default()
    }

    /// 给已经加载的模块补包名，并重新计算相对包根的模块路径。
    pub fn set_pkg_name(&mut self, mid: usize, pkg: &str) {
        let path = PathBuf::from(&self.prog.modules[mid].path);
        let stem = self.prog.modules[mid].stem.clone();
        self.prog.modules[mid].pkg_name = Some(pkg.to_string());
        self.prog.modules[mid].module_path = self.module_path_for(&path, Some(pkg), &stem);
    }

    /// 注入内存中的模块（`.mylib` 的 API 声明）；路径只作为模块键，不要求落盘。
    pub fn load_virtual(&mut self, path: &Path, src: &str, site: &str) -> Option<usize> {
        let key = Self::path_key(path);
        self.overlay
            .get_or_insert_with(HashMap::new)
            .insert(key, src.to_string());
        self.load_module(path, site)
    }

    /// Kahn 拓扑排序校验（DFS 后序已给出顺序，这里再做一次入度校验以自证）
    pub fn check_topology(&mut self) {
        let n = self.prog.modules.len();
        let mut indeg = vec![0usize; n];
        for m in &self.prog.modules {
            for t in &m.imports {
                indeg[*t] += 1;
            }
        }
        let mut queue: Vec<usize> = (0..n).filter(|i| indeg[*i] == 0).collect();
        let mut seen = 0usize;
        while let Some(u) = queue.pop() {
            seen += 1;
            for t in self.prog.modules[u].imports.clone() {
                indeg[t] -= 1;
                if indeg[t] == 0 {
                    queue.push(t);
                }
            }
        }
        if seen != n {
            self.diags.err(
                11,
                "<modules>",
                1,
                1,
                "模块依赖图存在环，无法确定编译顺序",
            );
        }
    }

    /// 前缀去重（同目录下不同路径可能出现同名文件）
    pub fn dedupe_prefixes(&mut self) {
        let mut used: Vec<String> = Vec::new();
        for i in 0..self.prog.modules.len() {
            let base = self.prog.modules[i].prefix.clone();
            let mut p = base.clone();
            let mut k = 1;
            while used.contains(&p) {
                p = format!("{}_{}", base, k);
                k += 1;
            }
            used.push(p.clone());
            self.prog.modules[i].prefix = p;
        }
    }

    pub fn finish(mut self) -> Program {
        self.dedupe_prefixes();
        self.check_topology();
        if self.diags.errors > 0 {
            eprintln!("mylc: 共 {} 个错误", self.diags.errors);
            std::process::exit(1);
        }
        self.prog
    }
}

/// 判断某个文件是否需要主类（非 *_bind.myl）
pub fn needs_main_class(is_bind: bool) -> bool {
    !is_bind
}

pub const _E_GEN: u32 = E_GENERAL;

//! 工程清单 `mylis.toml`：只描述**工程结构**与**构建目标**，不做依赖解析、不联网。
//!
//! ```toml
//! [project]
//! name = "myapp"
//!
//! [build]
//! paths = ["src", "lib"]      # 模块搜索根（相对工程根，按序）
//! backend = "llvm"            # 可选：llvm（默认）/ mir / ast / ir
//! profile = "release"         # 可选：debug / release / relwithdebinfo / minsize
//!
//! [targets.myapp]
//! kind = "exe"                # exe | static | shared
//! profile = "debug"           # 可选：覆盖 [build].profile
//! entry = "src/app.myl"
//! out = "build/myapp"         # 可省略：默认 build/<target 名>
//!
//! [native]
//! include = ["include"]        # C 头文件搜索目录（相对工程根）
//! dirs = ["lib"]               # 链接器库搜索目录
//! libs = ["foo"]               # 链接库名
//! files = ["lib/libfoo.a"]     # 精确链接文件
//! ```

use std::path::{Path, PathBuf};
use crate::semver::{Req, Version};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TargetKind {
    Exe,
    Static,
    Shared,
}

/// 构建 profile：优化级别、是否生成调试信息、是否在最终链接时裁剪调试段。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildProfile {
    Debug,
    Release,
    RelWithDebInfo,
    MinSize,
}

impl Default for BuildProfile {
    fn default() -> Self {
        BuildProfile::Release
    }
}

impl BuildProfile {
    pub fn parse(s: &str) -> Option<BuildProfile> {
        let key: String = s
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_lowercase();
        match key.as_str() {
            "debug" => Some(BuildProfile::Debug),
            "release" => Some(BuildProfile::Release),
            "relwithdebinfo" | "relwithdeb" => Some(BuildProfile::RelWithDebInfo),
            "minsize" | "size" => Some(BuildProfile::MinSize),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
            BuildProfile::RelWithDebInfo => "relwithdebinfo",
            BuildProfile::MinSize => "minsize",
        }
    }

    /// GNU/Clang 风格的优化参数。
    pub fn opt_flag(self) -> &'static str {
        match self {
            BuildProfile::Debug => "-O0",
            BuildProfile::Release | BuildProfile::RelWithDebInfo => "-O2",
            BuildProfile::MinSize => "-Os",
        }
    }

    pub fn wants_debug_info(self) -> bool {
        matches!(self, BuildProfile::Debug | BuildProfile::RelWithDebInfo)
    }

    /// 最终链接的裁剪参数：release 只删调试段，minsize 连符号表一起删。
    pub fn link_strip_flag(self) -> Option<&'static str> {
        match self {
            BuildProfile::Release => Some("-Wl,-S"),
            BuildProfile::MinSize => Some("-Wl,-s"),
            BuildProfile::Debug | BuildProfile::RelWithDebInfo => None,
        }
    }

    /// MSVC/cl.exe 风格的优化参数。
    pub fn msvc_flags(self) -> &'static [&'static str] {
        match self {
            BuildProfile::Debug => &["/Od", "/Zi"],
            BuildProfile::Release => &["/O2"],
            BuildProfile::RelWithDebInfo => &["/O2", "/Zi"],
            BuildProfile::MinSize => &["/O1"],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub kind: TargetKind,
    pub entry: String,
    pub out: Option<String>,
    /// 目标级 profile；缺省继承 `[build].profile`。
    pub profile: Option<BuildProfile>,
}

#[derive(Clone, Debug)]
pub struct Dependency {
    pub name: String,
    /// 依赖根目录，相对当前 manifest 所在目录；registry 依赖为 None
    pub path: Option<String>,
    /// registry 版本约束：`*`、`^1.2.0`、`~1.2.0`、`=1.2.3`
    pub version: Option<String>,
    /// registry 名；缺省 default
    pub registry: Option<String>,
    /// Git 仓库 URL；与 path 互斥
    pub git: Option<String>,
    pub tag: Option<String>,
    pub rev: Option<String>,
    pub branch: Option<String>,
    /// `.mylib` 归档 URL 或本地路径；与 path/git 互斥
    pub mylib: Option<String>,
    /// 可选的 `.mylib` 文件 SHA-256
    pub checksum: Option<String>,
}

impl Dependency {
    pub fn source_label(&self) -> String {
        match &self.path {
            Some(p) => format!("path+{}", p),
            None if self.git.is_some() => format!(
                "git+{}#{}",
                self.git.clone().unwrap_or_default(),
                self.rev.as_deref().or(self.tag.as_deref()).or(self.branch.as_deref()).unwrap_or("HEAD")
            ),
            None if self.mylib.is_some() => format!(
                "mylib+{}",
                self.mylib.clone().unwrap_or_default()
            ),
            None => format!(
                "registry+{}@{}",
                self.registry.clone().unwrap_or_else(|| "default".into()),
                self.version.clone().unwrap_or_else(|| "*".into())
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LockPin {
    pub version: String,
    pub source: String,
    pub checksum: Option<String>,
}

pub type LockPins = std::collections::HashMap<String, LockPin>;

#[derive(Clone, Debug)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub paths: Vec<String>,
    /// `[build] backend`：llvm（默认）/ mir / ast / ir
    pub backend: Option<String>,
    /// `[build] profile`：debug / release / relwithdebinfo / minsize
    pub profile: BuildProfile,
    pub targets: Vec<Target>,
    /// 包模式的库入口，例如 `src/lib.myl`
    pub lib_entry: Option<String>,
    /// 本地路径依赖
    pub dependencies: Vec<Dependency>,
    /// `[native] include`：传给 C 编译器的头文件搜索目录
    pub native_includes: Vec<String>,
    /// `[native] files`：直接传给 `--link` 的原生文件
    pub native_link_files: Vec<String>,
    /// `[native] dirs`：直接传给 `-L` 的原生库目录
    pub native_link_dirs: Vec<String>,
    /// `[native] libs`：直接传给 `-l` 的原生库名
    pub native_link_libs: Vec<String>,
}

pub const FILE: &str = "mylis.toml";

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn parse_profile(path: &Path, value: &str) -> Result<BuildProfile, String> {
    let raw = unquote(value);
    BuildProfile::parse(&raw).ok_or_else(|| {
        format!(
            "{}: 未知构建 profile `{}`；可选：debug / release / relwithdebinfo / minsize",
            path.display(),
            raw
        )
    })
}

fn parse_array(s: &str) -> Vec<String> {
    let v = s.trim().trim_start_matches('[').trim_end_matches(']');
    v.split(',')
        .map(|x| unquote(x))
        .filter(|x| !x.is_empty())
        .collect()
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("无法读取 {}: {}", path.display(), e))?;
        let mut name = String::new();
        let mut version = String::new();
        let mut paths = Vec::new();
        let mut backend: Option<String> = None;
        let mut profile = BuildProfile::Release;
        let mut targets: Vec<Target> = Vec::new();
        let mut lib_entry: Option<String> = None;
        let mut dependencies: Vec<Dependency> = Vec::new();
        let mut native_includes = Vec::new();
        let mut native_link_files = Vec::new();
        let mut native_link_dirs = Vec::new();
        let mut native_link_libs = Vec::new();
        let mut section = String::new();

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                section = line.trim_start_matches('[').trim_end_matches(']').trim().to_string();
                if let Some(t) = section.strip_prefix("targets.") {
                    targets.push(Target {
                        name: t.trim().to_string(),
                        kind: TargetKind::Exe,
                        entry: String::new(),
                        out: None,
                        profile: None,
                    });
                }
                continue;
            }
            let (k, v) = match line.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            match section.as_str() {
                "project" | "package" => match k.trim() {
                    "name" => name = unquote(v),
                    "version" => version = unquote(v),
                    _ => {}
                },
                "build" => match k.trim() {
                    "paths" => paths = parse_array(v),
                    "backend" => backend = Some(unquote(v)),
                    "profile" => profile = parse_profile(path, v)?,
                    _ => {}
                },
                "lib" => match k.trim() {
                    "entry" => lib_entry = Some(unquote(v)),
                    _ => {}
                },
                "native" => match k.trim() {
                    "include" | "includes" => native_includes = parse_array(v),
                    "files" | "link_files" => native_link_files = parse_array(v),
                    "dirs" | "link_dirs" => native_link_dirs = parse_array(v),
                    "libs" | "link_libs" => native_link_libs = parse_array(v),
                    _ => {}
                },
                "dependencies" => {
                    let name = k.trim().to_string();
                    if !name.is_empty() {
                        let raw = v.trim();
                        let mut path = None;
                        let mut version = None;
                        let mut registry = None;
                        let mut git = None;
                        let mut tag = None;
                        let mut rev = None;
                        let mut branch = None;
                        let mut mylib = None;
                        let mut checksum = None;
                        if raw.starts_with('{') {
                            let body = raw.trim_start_matches('{').trim_end_matches('}');
                            for kv in body.split(',') {
                                if let Some((a, b)) = kv.split_once('=') {
                                    let key = a.trim();
                                    let val = unquote(b);
                                    match key {
                                        "path" => path = Some(val),
                                        "version" => version = Some(val),
                                        "registry" => registry = Some(val),
                                        "git" => git = Some(val),
                                        "tag" => tag = Some(val),
                                        "rev" => rev = Some(val),
                                        "branch" => branch = Some(val),
                                        "mylib" => mylib = Some(val),
                                        "checksum" => checksum = Some(val),
                                        _ => {}
                                    }
                                }
                            }
                        } else if !unquote(raw).is_empty() {
                            path = Some(unquote(raw));
                        }
                        if path.is_some() || version.is_some() || git.is_some() || mylib.is_some() {
                            dependencies.push(Dependency { name, path, version, registry, git, tag, rev, branch, mylib, checksum });
                        }
                    }
                }
                s if s.starts_with("targets.") => {
                    let t = targets.last_mut().unwrap();
                    match k.trim() {
                        "kind" => {
                            t.kind = match unquote(v).as_str() {
                                "static" => TargetKind::Static,
                                "shared" => TargetKind::Shared,
                                _ => TargetKind::Exe,
                            }
                        }
                        "entry" => t.entry = unquote(v),
                        "out" => t.out = Some(unquote(v)),
                        "profile" => t.profile = Some(parse_profile(path, v)?),
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if name.is_empty() {
            name = Path::new(path)
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "project".to_string());
        }
        if paths.is_empty() {
            paths = vec!["src".to_string(), "lib".to_string()];
        }
        Ok(Manifest {
            name, version, paths, backend, profile, targets, lib_entry, dependencies,
            native_includes, native_link_files, native_link_dirs, native_link_libs,
        })
    }
}

/// 从当前目录向上查找 `mylis.toml`
pub fn locate() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cand = dir.join(FILE);
        if cand.is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}


/// 从指定目录向上查找 `mylis.toml`
pub fn locate_from(dir: &Path) -> Option<PathBuf> {
    let mut cur = dir.to_path_buf();
    loop {
        let cand = cur.join(FILE);
        if cand.is_file() {
            return Some(cand);
        }
        if !cur.pop() {
            return None;
        }
    }
}


#[derive(Clone, Debug)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    /// 规范化后的包根目录
    pub root: PathBuf,
    /// 首次发现时声明的源（用于锁文件）
    pub source: String,
    pub checksum: String,
    /// `.mylib` 归档路径（MIR 包依赖）
    pub mylib: Option<PathBuf>,
    /// `.mylib` 根 API 虚拟模块路径
    pub api_path: Option<PathBuf>,
    /// `.mylib` 所有 API 模块（模块名 -> 虚拟路径）
    pub api_paths: Vec<(String, PathBuf)>,
    pub manifest: Manifest,
}

#[derive(Clone, Debug, Default)]
pub struct PackageGraph {
    pub packages: Vec<ResolvedPackage>,
}

impl PackageGraph {
    pub fn find(&self, name: &str) -> Option<&ResolvedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn git_cache() -> PathBuf {
    if let Ok(p) = std::env::var("MYLIS_GIT_CACHE") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    PathBuf::from(home).join(".mylis").join("git")
}

fn git_run(args: &[String], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd.output().map_err(|e| format!("无法执行 git：{}", e))?;
    if !out.status.success() {
        return Err(format!("git {} 失败：{}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn mylib_cache() -> PathBuf {
    if let Ok(p) = std::env::var("MYLIS_MYLIB_CACHE") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    PathBuf::from(home).join(".mylis").join("mylib")
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("读取 {} 失败：{}", path.display(), e))?;
    Ok(format!("sha256-{}", crate::sha256::hex(&data)))
}

fn normalize_sha256(s: &str) -> String {
    if s.starts_with("sha256-") { s.to_string() } else { format!("sha256-{}", s) }
}

fn download_mylib(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 .mylib 缓存失败：{}", e))?;
    }
    let out = std::process::Command::new("curl")
        .args(["-fL", "--silent", "--show-error", "-o"])
        .arg(dest)
        .arg(url)
        .output()
        .map_err(|e| format!("无法执行 curl（下载 .mylib 需要 curl）：{}", e))?;
    if !out.status.success() {
        return Err(format!(".mylib 下载失败：{}：{}", url, String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

fn resolve_mylib(dep: &Dependency, base: &Path, offline: bool) -> Result<(PathBuf, Manifest, String, PathBuf, Vec<(String, PathBuf)>), String> {
    let raw = dep.mylib.clone().ok_or_else(|| format!("依赖 `{}` 缺少 mylib URL", dep.name))?;
    let archive = if raw.starts_with("http://") || raw.starts_with("https://") {
        let key = crate::sha256::hex(raw.as_bytes());
        let dest = mylib_cache().join("downloads").join(format!("{}.mylib", &key[..16]));
        if !dest.is_file() {
            if offline {
                return Err(format!("离线模式下依赖 `{}` 没有 .mylib 缓存：{}", dep.name, dest.display()));
            }
            download_mylib(&raw, &dest)?;
        }
        dest
    } else {
        let p = if let Some(rest) = raw.strip_prefix("file://") {
            PathBuf::from(rest)
        } else {
            PathBuf::from(&raw)
        };
        let p = if p.is_absolute() { p } else { base.join(p) };
        if !p.is_file() {
            return Err(format!("依赖 `{}` 的 .mylib 不存在：{}", dep.name, p.display()));
        }
        p
    };
    let checksum = file_sha256(&archive)?;
    if let Some(expected) = &dep.checksum {
        let expected = normalize_sha256(expected);
        if checksum != expected {
            return Err(format!("依赖 `{}` 的 .mylib checksum 不一致\n  expected: {}\n  actual: {}", dep.name, expected, checksum));
        }
    }
    let lib = crate::mylib::read(&archive).map_err(|e| format!("依赖 `{}` 的 .mylib 无效：{}", dep.name, e))?;
    if lib.name != dep.name {
        return Err(format!("依赖名不匹配：清单写的是 `{}`，{} 里的包名是 `{}`", dep.name, archive.display(), lib.name));
    }
    let root = canonical(archive.parent().unwrap_or(Path::new(".")));
    let api_root = canonical(base).join(".mylib");
    let mut api_paths = vec![("lib".to_string(), api_root.join(&lib.name).join("lib.myl"))];
    for (name, _) in &lib.apis {
        let (pkg, module) = crate::mylib::api_label(&lib.name, name);
        let path = api_root.join(&pkg).join(format!("{}.myl", module.replace("::", "/")));
        api_paths.push((name.clone(), path));
    }
    let man = Manifest {
        name: lib.name.clone(),
        version: lib.version.clone(),
        paths: Vec::new(),
        backend: None,
        profile: BuildProfile::Release,
        targets: Vec::new(),
        lib_entry: None,
        dependencies: Vec::new(),
        native_includes: Vec::new(),
        native_link_files: Vec::new(),
        native_link_dirs: Vec::new(),
        native_link_libs: Vec::new(),
    };
    let source = format!("mylib+{}", raw);
    Ok((root, man, source, canonical(&archive), api_paths))
}

fn resolve_git(dep: &Dependency, pin: Option<&LockPin>, offline: bool) -> Result<(PathBuf, Manifest, String), String> {
    let url = dep.git.clone().ok_or_else(|| format!("依赖 `{}` 缺少 git URL", dep.name))?;
    if dep.path.is_some() {
        return Err(format!("依赖 `{}` 不能同时写 path 和 git", dep.name));
    }
    let cache = git_cache();
    let key = crate::sha256::hex(url.as_bytes());
    let key = &key[..16];
    let db = cache.join("db").join(key);
    if !db.is_dir() {
        if offline {
            return Err(format!("离线模式下依赖 `{}` 没有 Git 缓存：{}", dep.name, db.display()));
        }
        if let Some(parent) = db.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建 Git 缓存失败：{}", e))?;
        }
        git_run(&["clone".into(), "--bare".into(), "--quiet".into(), url.clone(), db.to_string_lossy().to_string()], None)?;
    } else if !offline {
        // 已有裸仓库：刷新 tags/分支；离线失败时仍允许使用缓存里的 pin。
        let _ = git_run(&["fetch".into(), "--quiet".into(), "--tags".into(), "--force".into(), "origin".into()], Some(&db));
    }
    let commit = if let Some(pin) = pin {
        pin.source.rsplit_once('#').map(|(_, c)| c.to_string())
            .ok_or_else(|| format!("锁文件 source 缺少 git commit：{}", pin.source))?
    } else {
        let spec = dep.rev.clone().or_else(|| dep.tag.clone()).or_else(|| dep.branch.clone()).unwrap_or_else(|| "HEAD".into());
        let rev = format!("{}^{{commit}}", spec);
        git_run(&["rev-parse".into(), rev], Some(&db))?
    };
    let checkout = cache.join("checkouts").join(key).join(&commit);
    if !checkout.join(FILE).is_file() {
        if let Some(parent) = checkout.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建 Git checkout 目录失败：{}", e))?;
        }
        let path = checkout.to_string_lossy().to_string();
        git_run(&["worktree".into(), "add".into(), "--detach".into(), path, commit.clone()], Some(&db))?;
    }
    let mpath = checkout.join(FILE);
    let man = Manifest::load(&mpath).map_err(|e| format!("依赖 `{}` 无法读取 {}: {}", dep.name, mpath.display(), e))?;
    if man.name != dep.name {
        return Err(format!("依赖名不匹配：清单写的是 `{}`，{} 里的包名是 `{}`", dep.name, mpath.display(), man.name));
    }
    Ok((canonical(&checkout), man, format!("git+{}#{}", url, commit)))
}

fn registry_base(dep: &Dependency) -> Result<PathBuf, String> {
    if let Some(r) = &dep.registry {
        if r.contains('/') || r.contains('\\') || r.starts_with('.') {
            return Ok(PathBuf::from(r));
        }
        let key = format!("MYLIS_REGISTRY_{}", r.to_uppercase().replace('-', "_"));
        if let Ok(p) = std::env::var(&key) {
            return Ok(PathBuf::from(p));
        }
        if r != "default" {
            return Err(format!("未知 registry `{}`；请设置 {}", r, key));
        }
    }
    if let Ok(p) = std::env::var("MYLIS_REGISTRY") {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    Ok(PathBuf::from(home).join(".mylis").join("registry"))
}

fn resolve_registry(dep: &Dependency, pin: Option<&LockPin>) -> Result<(PathBuf, Manifest, String), String> {
    let base = registry_base(dep)?;
    let pkg = base.join(&dep.name);
    let rd = std::fs::read_dir(&pkg)
        .map_err(|e| format!("registry 依赖 `{}` 无法读取 {}: {}", dep.name, pkg.display(), e))?;
    let req = Req::parse(dep.version.as_deref().unwrap_or("*"))
        .ok_or_else(|| format!("依赖 `{}` 的版本约束无法解析：{}", dep.name, dep.version.as_deref().unwrap_or("*")))?;
    let mut best: Option<(Version, PathBuf)> = None;
    if let Some(pin) = pin {
        let v = Version::parse(&pin.version).ok_or_else(|| format!("锁文件版本无法解析：{}", pin.version))?;
        if !req.matches(&v) {
            return Err(format!("锁文件版本 `{}` 不满足依赖 `{}` 的版本约束 `{}`", pin.version, dep.name, dep.version.clone().unwrap_or_else(|| "*".into())));
        }
        return {
            let root = pkg.join(&pin.version);
            let mpath = root.join(FILE);
            let man = Manifest::load(&mpath).map_err(|e| format!("依赖 `{}` 无法读取 {}: {}", dep.name, mpath.display(), e))?;
            if man.name != dep.name {
                return Err(format!("依赖名不匹配：清单写的是 `{}`，{} 里的包名是 `{}`", dep.name, mpath.display(), man.name));
            }
            let reg_name = dep.registry.clone().unwrap_or_else(|| "default".into());
            Ok((canonical(&root), man, format!("registry+{}@{}", reg_name, pin.version)))
        };
    }
    for e in rd.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let Some(v) = Version::parse(&name) else { continue };
        if !req.matches(&v) {
            continue;
        }
        let ok = best.as_ref().map(|(bv, _)| v.cmp(bv) == std::cmp::Ordering::Greater).unwrap_or(true);
        if ok {
            best = Some((v, e.path()));
        }
    }
    let (version, root) = best.ok_or_else(|| format!(
        "registry 中找不到依赖 `{}` 满足版本 `{}`（搜索 {}）",
        dep.name, dep.version.as_deref().unwrap_or("*"), pkg.display()
    ))?;
    let mpath = root.join(FILE);
    let man = Manifest::load(&mpath)
        .map_err(|e| format!("依赖 `{}` 无法读取 {}: {}", dep.name, mpath.display(), e))?;
    if man.name != dep.name {
        return Err(format!("依赖名不匹配：清单写的是 `{}`，{} 里的包名是 `{}`", dep.name, mpath.display(), man.name));
    }
    let reg_name = dep.registry.clone().unwrap_or_else(|| "default".into());
    let source = format!("registry+{}@{}", reg_name, version.major.to_string() + "." + &version.minor.to_string() + "." + &version.patch.to_string());
    Ok((canonical(&root), man, source))
}

fn walk_graph(
    base: &Path,
    deps: &[Dependency],
    chain: &mut Vec<String>,
    graph: &mut PackageGraph,
    stack: &mut Vec<PathBuf>,
    pins: Option<&LockPins>,
    offline: bool,
) -> Result<(), String> {
    for dep in deps {
        let pin = pins.and_then(|p| p.get(&dep.name));
        if pins.is_some() && pin.is_none() {
            return Err(format!("锁文件缺少依赖 `{}`；请运行 `mylc --update-lock` 重新生成", dep.name));
        }
        let (dep_root, man, source, mylib, api_paths) = match &dep.path {
            Some(p) => {
                let root = canonical(&base.join(p));
                let mpath = root.join(FILE);
                let man = Manifest::load(&mpath).map_err(|e| format!("依赖 `{}` 无法读取 {}: {}", dep.name, mpath.display(), e))?;
                (root, man, format!("path+{}", p), None, Vec::new())
            }
            None if dep.git.is_some() => {
                let (root, man, source) = resolve_git(dep, pin, offline)?;
                (root, man, source, None, Vec::new())
            }
            None if dep.mylib.is_some() => {
                let (root, man, source, archive, apis) = resolve_mylib(dep, base, offline)?;
                (root, man, source, Some(archive), apis)
            }
            None => {
                let (root, man, source) = resolve_registry(dep, pin)?;
                (root, man, source, None, Vec::new())
            }
        };
        let checksum = match &mylib {
            Some(p) => file_sha256(p)?,
            None => crate::lockfile::package_checksum(&dep_root),
        };
        if let Some(pin) = pin {
            if pin.version != man.version {
                return Err(format!("锁文件版本不一致：`{}` 锁定 {}，当前解析 {}；请运行 `mylc --update-lock`", dep.name, pin.version, man.version));
            }
            if pin.source != source {
                return Err(format!("锁文件源不一致：`{}` 锁定 {}，当前为 {}；请运行 `mylc --update-lock`", dep.name, pin.source, source));
            }
            if let Some(expected) = &pin.checksum {
                if checksum != *expected {
                    return Err(format!(
                        "依赖 `{}` 内容与 mylis.lock 不一致\n  locked: {}\n  actual: {}\n  如果你确实改了依赖，运行 `mylc --update-lock`",
                        dep.name, expected, checksum
                    ));
                }
            }
        }
        if stack.contains(&dep_root) {
            return Err(format!(
                "依赖循环：{} -> {}",
                chain.join(" -> "),
                dep.name
            ));
        }
        if man.name != dep.name {
            return Err(format!(
                "依赖名不匹配：清单写的是 `{}`，{} 里的包名是 `{}`",
                dep.name,
                dep_root.join(FILE).display(),
                man.name
            ));
        }
        if let Some(old) = graph.find(&dep.name) {
            if old.root != dep_root {
                return Err(format!(
                    "依赖冲突：`{}` 来自两个不同路径\n  本次请求：{} -> {} ({})\n  已解析：{} (source {})",
                    dep.name,
                    chain.join(" -> "),
                    dep.name,
                    dep.path.clone().unwrap_or_else(|| dep.source_label()),
                    old.root.display(),
                    old.source
                ));
            }
            if old.version != man.version {
                return Err(format!(
                    "依赖冲突：`{}` 版本不一致\n  {} -> {} {}\n  已解析为 {} {}",
                    dep.name,
                    chain.join(" -> "),
                    dep.name,
                    man.version,
                    old.name,
                    old.version
                ));
            }
            continue; // 菱形依赖：同源同版本，复用
        }
        graph.packages.push(ResolvedPackage {
            name: man.name.clone(),
            version: man.version.clone(),
            root: dep_root.clone(),
            source,
            checksum: checksum.clone(),
            api_path: api_paths.iter().find(|(n, _)| n == "lib").map(|(_, p)| p.clone()),
            mylib,
            api_paths,
            manifest: man.clone(),
        });
        chain.push(dep.name.clone());
        stack.push(dep_root.clone());
        walk_graph(&dep_root, &man.dependencies, chain, graph, stack, pins, offline)?;
        stack.pop();
        chain.pop();
    }
    Ok(())
}

/// 解析根包的完整路径依赖图；同一包名只允许一个源/版本。
pub fn build_graph(root_manifest: &Path) -> Result<PackageGraph, String> {
    build_graph_with_pins(root_manifest, None)
}

pub fn build_graph_with_pins(root_manifest: &Path, pins: Option<&LockPins>) -> Result<PackageGraph, String> {
    build_graph_with_pins_offline(root_manifest, pins, false)
}

pub fn build_graph_with_pins_offline(root_manifest: &Path, pins: Option<&LockPins>, offline: bool) -> Result<PackageGraph, String> {
    let root_man = Manifest::load(root_manifest)?;
    let base = root_manifest.parent().unwrap_or(Path::new("."));
    let root_root = canonical(base);
    let mut graph = PackageGraph::default();
    let mut chain = vec![root_man.name.clone()];
    let mut stack = vec![root_root];
    walk_graph(base, &root_man.dependencies, &mut chain, &mut graph, &mut stack, pins, offline)?;
    graph.packages.sort_by(|a, b| (a.name.as_str(), a.version.as_str()).cmp(&(b.name.as_str(), b.version.as_str())));
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_parses_aliases_case_insensitively() {
        for s in ["debug", "DEBUG", "release", "relwithdebinfo", "relwith-deb", "relwith_deb", "minsize", "size"] {
            assert!(BuildProfile::parse(s).is_some(), "profile `{}` 应可解析", s);
        }
        assert_eq!(BuildProfile::parse("relwith_deb"), Some(BuildProfile::RelWithDebInfo));
        assert_eq!(BuildProfile::parse("size"), Some(BuildProfile::MinSize));
    }

    #[test]
    fn profile_strip_flags() {
        assert_eq!(BuildProfile::Debug.link_strip_flag(), None);
        assert_eq!(BuildProfile::Release.link_strip_flag(), Some("-Wl,-S"));
        assert_eq!(BuildProfile::RelWithDebInfo.link_strip_flag(), None);
        assert_eq!(BuildProfile::MinSize.link_strip_flag(), Some("-Wl,-s"));
    }

    #[test]
    fn profile_rejects_unknown_value() {
        assert_eq!(BuildProfile::parse("fast"), None);
        assert_eq!(BuildProfile::parse(""), None);
    }

    #[test]
    fn manifest_reads_build_and_target_profile() {
        let path = std::env::temp_dir().join(format!("mylis-profile-test-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            r#"
[project]
name = "demo"

[build]
paths = ["src"]
profile = "debug"

[targets.app]
kind = "exe"
profile = "relwithdebinfo"
entry = "src/main.myl"
"#,
        )
        .unwrap();
        let man = Manifest::load(&path).unwrap();
        assert_eq!(man.profile, BuildProfile::Debug);
        assert_eq!(man.targets[0].profile, Some(BuildProfile::RelWithDebInfo));
        let _ = std::fs::remove_file(&path);
    }
}

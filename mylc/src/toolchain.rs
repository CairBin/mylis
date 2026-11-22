//! C 工具链的“家族”抽象与发现，全部由数据描述，新增编译器**不需要重编 mylc**。
//!
//! # 家族（style）
//! * `gnu`  —— GCC / Clang / MinGW 这类 cc 驱动：`-I -o -c -shared -L -l`
//! * `msvc` —— cl.exe / lib.exe：`/I /Fe: /Fo: /c /link /LIBPATH:`
//!
//! 每个家族的具体可执行文件与参数在 `toolchains/*.toml` 里描述；内置 `gcc`、`clang`(llvm)、
//! `msvc` 三份规格，可被同名文件覆盖。
//!
//! # 规格查找顺序
//! 1. 内置规格（gcc / clang / msvc）
//! 2. 全局：`~/.mylis/toolchains/*.toml`
//! 3. 安装根：`<root>/toolchains/*.toml`
//! 4. 工程：`<project>/.mylis/toolchains/*.toml`（优先级最高）
//! 5. `MYLIS_TOOLCHAINS` 指定的目录（每个目录里的 `*.toml`，最后加载）
//!
//! # 选择哪个规格
//! `MYLIS_FAMILY` / 工程 `mylis.toml` 的 `[toolchain] family` / 随包
//! `mylis-toolchain.json` 的 `family`；都没有时用 `gcc`。
//! 各字段仍可用 `MYLIS_CC`、`MYLIS_AR`、`MYLIS_CFLAGS`、`MYLIS_LDFLAGS` 覆盖。

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Gnu,
    Msvc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Runtime {
    /// 预编译好的 `libmylrt.a` / `libgc.a`
    Archive,
    /// MSVC 没有兼容的预编译运行时：直接把 myrt 源码编进产物
    Sources,
}

#[derive(Clone, Debug)]
pub struct Toolchain {
    pub name: String,
    pub style: Style,
    pub cc: String,
    pub ar: String,
    pub cflags: Vec<String>,
    pub ldflags: Vec<String>,
    /// 含目录参数模板，`{path}` 会被替换
    pub include: String,
    pub libdir: String,
    /// 链接库名模板，`{name}` 会被替换
    pub link_lib: String,
    pub obj_suffix: String,
    pub static_prefix: String,
    pub static_suffix: String,
    pub shared_prefix: String,
    pub shared_suffix: String,
    pub runtime: Runtime,
    /// 来源说明（-v 时打印）
    pub origin: String,
}

fn split_flags(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| x.to_string()).collect()
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    }
}

fn gnu_shared() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("", ".dll")
    } else {
        ("lib", ".so")
    }
}

impl Toolchain {
    fn gcc() -> Toolchain {
        let (sp, ss) = gnu_shared();
        Toolchain {
            name: "gcc".into(),
            style: Style::Gnu,
            cc: exe_name("gcc"),
            ar: exe_name("ar"),
            cflags: split_flags("-std=c11 -O2 -fno-strict-aliasing -Wall -Wextra -Wno-unused-parameter -Wno-unused-function -Wno-unused-variable"),
            ldflags: Vec::new(),
            include: "-I{path}".into(),
            libdir: "-L{path}".into(),
            link_lib: "-l{name}".into(),
            obj_suffix: ".o".into(),
            static_prefix: "lib".into(),
            static_suffix: ".a".into(),
            shared_prefix: sp.into(),
            shared_suffix: ss.into(),
            runtime: Runtime::Archive,
            origin: "内置 gcc 规格".into(),
        }
    }

    fn clang() -> Toolchain {
        let mut tc = Toolchain::gcc();
        tc.name = "clang".into();
        tc.cc = exe_name("clang");
        tc.ar = exe_name("llvm-ar");
        tc.origin = "内置 clang 规格".into();
        tc
    }

    fn msvc() -> Toolchain {
        Toolchain {
            name: "msvc".into(),
            style: Style::Msvc,
            cc: exe_name("cl"),
            ar: exe_name("lib"),
            cflags: vec!["/nologo".into(), "/std:c11".into(), "/O2".into(), "/W3".into(), "/utf-8".into()],
            ldflags: Vec::new(),
            include: "/I{path}".into(),
            libdir: "/LIBPATH:{path}".into(),
            link_lib: "{name}.lib".into(),
            obj_suffix: ".obj".into(),
            static_prefix: "".into(),
            static_suffix: ".lib".into(),
            shared_prefix: "".into(),
            shared_suffix: ".dll".into(),
            runtime: Runtime::Sources,
            origin: "内置 msvc 规格".into(),
        }
    }

    pub fn builtin(name: &str) -> Option<Toolchain> {
        match name.to_ascii_lowercase().as_str() {
            "gcc" | "gnu" | "mingw" => Some(Toolchain::gcc()),
            "clang" | "llvm" => Some(Toolchain::clang()),
            "msvc" | "cl" => Some(Toolchain::msvc()),
            _ => None,
        }
    }

    /// 依编译器文件名粗判家族：cl → msvc，clang → clang，其余沿用
    pub fn guess_builtin(cc: &str) -> Option<&'static str> {
        let stem = Path::new(cc)
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if stem == "cl" || stem == "clang-cl" {
            Some("msvc")
        } else if stem == "clang" || stem == "clang++" {
            Some("clang")
        } else {
            None
        }
    }

    /// 用显式 cc 覆盖，并按文件名推断是否要切换家族
    pub fn with_cc(mut self, cc: &str) -> Toolchain {
        if self.style != Style::Msvc {
            if let Some(name) = Toolchain::guess_builtin(cc) {
                let mut base = Toolchain::builtin(name).unwrap();
                base.origin = self.origin.clone();
                base.cflags = self.cflags.clone();
                base.ldflags = self.ldflags.clone();
                base.cc = cc.to_string();
                return base;
            }
        }
        self.cc = cc.to_string();
        self
    }
}

// ---------------------------------------------------------------------------
// 规格文件解析：只支持 `key = "..."` 与 `key = ["a", "b"]`，足够描述工具链
// ---------------------------------------------------------------------------

fn parse_value(raw: &str) -> Option<Vec<String>> {
    let v = raw.trim();
    if v.starts_with('[') {
        let inner = v.trim_start_matches('[').trim_end_matches(']');
        let mut out = Vec::new();
        for part in inner.split(',') {
            let s = part.trim().trim_matches('"').trim_matches('\'');
            if !s.is_empty() {
                out.push(s.to_string());
            }
        }
        Some(out)
    } else {
        let s = v.trim_matches('"').trim_matches('\'');
        Some(vec![s.to_string()])
    }
}

fn parse_spec(path: &Path) -> Option<Toolchain> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut kv: HashMap<String, Vec<String>> = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let (k, v) = line.split_once('=')?;
        if let Some(val) = parse_value(v) {
            kv.insert(k.trim().to_string(), val);
        }
    }
    let one = |k: &str| kv.get(k).and_then(|v| v.first()).cloned();
    let first_style = one("style")?;
    let mut tc = match first_style.to_ascii_lowercase().as_str() {
        "msvc" => Toolchain::msvc(),
        _ => Toolchain::gcc(),
    };
    tc.name = one("name").unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "custom".into())
    });
    if let Some(v) = one("cc") {
        tc.cc = v;
    }
    if let Some(v) = one("ar") {
        tc.ar = v;
    }
    if let Some(v) = kv.get("cflags") {
        tc.cflags = v.clone();
    }
    if let Some(v) = kv.get("ldflags") {
        tc.ldflags = v.clone();
    }
    for (key, field) in [
        ("include", &mut tc.include),
        ("libdir", &mut tc.libdir),
        ("link_lib", &mut tc.link_lib),
        ("obj_suffix", &mut tc.obj_suffix),
        ("static_prefix", &mut tc.static_prefix),
        ("static_suffix", &mut tc.static_suffix),
        ("shared_prefix", &mut tc.shared_prefix),
        ("shared_suffix", &mut tc.shared_suffix),
    ] {
        if let Some(v) = one(key) {
            *field = v;
        }
    }
    if let Some(v) = one("runtime") {
        tc.runtime = if v.eq_ignore_ascii_case("sources") {
            Runtime::Sources
        } else {
            Runtime::Archive
        };
    }
    tc.origin = format!("规格文件 {}", path.display());
    Some(tc)
}

fn spec_dirs(project: Option<&Path>, root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".mylis/toolchains"));
    }
    dirs.push(root.join("toolchains"));
    if let Some(p) = project {
        dirs.push(p.join(".mylis/toolchains"));
    }
    if let Ok(list) = env::var("MYLIS_TOOLCHAINS") {
        for part in list.split(if cfg!(windows) { ';' } else { ':' }) {
            if !part.trim().is_empty() {
                dirs.push(PathBuf::from(part.trim()));
            }
        }
    }
    dirs
}

fn load_all(project: Option<&Path>, root: &Path) -> HashMap<String, Toolchain> {
    let mut map: HashMap<String, Toolchain> = HashMap::new();
    for name in ["gcc", "clang", "msvc"] {
        if let Some(tc) = Toolchain::builtin(name) {
            map.insert(name.to_string(), tc);
        }
    }
    for dir in spec_dirs(project, root) {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "toml").unwrap_or(false))
            .collect();
        files.sort();
        for f in files {
            if let Some(tc) = parse_spec(&f) {
                map.insert(tc.name.clone(), tc);
            }
        }
    }
    map
}

/// 极简 `[toolchain]` 段解析（工程 `mylis.toml`）
fn read_project_cfg(path: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return out,
    };
    let mut in_section = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_section = line.trim_start_matches('[').trim_end_matches(']').trim() == "toolchain";
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(
                k.trim().to_string(),
                v.trim().trim_matches('"').trim_matches('\'').to_string(),
            );
        }
    }
    out
}

/// 随包描述：`<root>/mylis-toolchain.json`
fn read_bundle(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let p = root.join("mylis-toolchain.json");
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => return out,
    };
    if let Some(v) = crate::json::parse(&text) {
        for key in ["cc", "ar", "family", "runtime"] {
            if let Some(x) = v.get(key).and_then(|x| x.as_str()) {
                out.insert(key.to_string(), x.to_string());
            }
        }
    }
    out
}

impl Toolchain {
    /// `project` 为工程根（有 `mylis.toml` 的目录，可为空）；`root` 为编译器安装根
    pub fn detect(project: Option<&Path>, root: &Path) -> Toolchain {
        let specs = load_all(project, root);
        let project_cfg = project
            .map(|p| read_project_cfg(&p.join("mylis.toml")))
            .unwrap_or_default();
        let bundle = read_bundle(root);

        let requested = env::var("MYLIS_FAMILY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| project_cfg.get("family").cloned())
            .or_else(|| bundle.get("family").cloned())
            .unwrap_or_else(|| {
                if cfg!(all(windows, target_env = "gnu")) {
                    "mingw".to_string()
                } else if cfg!(windows) {
                    "msvc".to_string()
                } else {
                    "gcc".to_string()
                }
            });

        let mut tc = specs.get(&requested).cloned().or_else(|| {
            Toolchain::builtin(&requested).map(|mut b| {
                b.origin = format!("内置规格 {}", b.name);
                b
            })
        }).unwrap_or_else(|| {
            eprintln!(
                "mylc: 未找到工具链规格 `{}`（查找目录：$HOME/.mylis/toolchains、<root>/toolchains、<project>/.mylis/toolchains、$MYLIS_TOOLCHAINS），回退到 gcc",
                requested
            );
            Toolchain::gcc()
        });

        let joined = |p: &str| -> String {
            let pb = PathBuf::from(p);
            if pb.is_absolute() {
                p.to_string()
            } else {
                root.join(pb).to_string_lossy().to_string()
            }
        };

        if let Ok(cc) = env::var("MYLIS_CC") {
            tc = tc.with_cc(&cc);
            tc.origin = "MYLIS_CC".into();
        } else if let Some(cc) = project_cfg.get("cc").or_else(|| bundle.get("cc")) {
            tc.cc = joined(cc);
            tc.origin = if project_cfg.contains_key("cc") {
                "工程 mylis.toml".into()
            } else {
                "mylis-toolchain.json（随包）".into()
            };
        }
        if let Ok(ar) = env::var("MYLIS_AR") {
            tc.ar = ar;
        } else if let Some(ar) = project_cfg.get("ar").or_else(|| bundle.get("ar")) {
            tc.ar = joined(ar);
        }

        if let Ok(s) = env::var("MYLIS_CFLAGS") {
            tc.cflags = split_flags(&s);
        } else if let Some(s) = project_cfg.get("cflags") {
            tc.cflags = split_flags(s);
        }
        if let Ok(s) = env::var("MYLIS_LDFLAGS") {
            tc.ldflags = split_flags(&s);
        } else if let Some(s) = project_cfg.get("ldflags") {
            tc.ldflags = split_flags(s);
        }
        tc
    }
}

/// 构建所需的全部路径（可能因“含空格安装目录”而被搬到缓存目录）
#[derive(Clone, Debug)]
pub struct Layout {
    pub cc: String,
    pub ar: String,
    pub myrt_inc: PathBuf,
    pub gc_inc: PathBuf,
    pub myrt_lib: PathBuf,
    /// Rust 运行时的 MSVC 静态库（mylrt.lib）
    pub myrt_msvc_lib: PathBuf,
    pub gc_lib: PathBuf,
    /// 自研 GC 的静态库（MYLIS_GC=own）
    pub mylgc_lib: PathBuf,
    /// gc_simple 的静态库（MYLIS_GC=simple；Unix）
    pub simple_lib: PathBuf,
    /// gc_simple 的 MSVC 静态库（build/gc_simple.lib）
    pub simple_msvc_lib: PathBuf,
    /// gc_boehm 包装层（myrt_gc_backend_* → GC_*），Boehm 时与 libgc.a 一起链
    pub boehm_lib: PathBuf,
    /// 自研 GC 的 MSVC 版静态库（mylgc.lib），cl.exe 家族用
    pub mylgc_msvc_lib: PathBuf,
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

fn cache_root() -> Option<PathBuf> {
    for var in ["MYLIS_CACHE", "LOCALAPPDATA", "TEMP", "TMP", "USERPROFILE"] {
        if let Ok(v) = env::var(var) {
            if !v.is_empty() && !v.contains(' ') {
                return Some(PathBuf::from(v).join("mylis-cache"));
            }
        }
    }
    if let Ok(drive) = env::var("SystemDrive") {
        if !drive.contains(' ') {
            return Some(PathBuf::from(format!("{}\\mylis-cache", drive)));
        }
    }
    None
}

/// 构建清单：由 Makefile/打包脚本生成，描述当前 build/ 产物的工具链与 ABI。
/// `mylc` 在链接前校验它，避免把 MSVC/ MinGW/ GNU 的运行时库混用。
#[derive(Clone, Debug)]
pub struct BuildManifest {
    pub os: String,
    pub family: String,
    pub style: String,
    pub abi: String,
    pub target: String,
    pub cc: String,
    pub ar: String,
    pub myrt: String,
    pub mylgc: String,
    pub simple: String,
    /// Windows 系统库列表；None 表示旧清单（按默认 std 运行时处理），
    /// Some([]) 表示 freestanding/no_std 运行时，不应额外链入系统库。
    pub system_libs: Option<Vec<String>>,
}

impl BuildManifest {
    /// 依次查找 `<root>/mylis-build.json`、`<root>/build/mylis-build.json`。
    pub fn load(root: &Path) -> Option<BuildManifest> {
        let paths = [root.join("mylis-build.json"), root.join("build/mylis-build.json")];
        for p in paths {
            let text = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let v = crate::json::parse(&text)?;
            let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            return Some(BuildManifest {
                os: get("os"),
                family: get("family"),
                style: get("style"),
                abi: get("abi"),
                target: get("target"),
                cc: get("cc"),
                ar: get("ar"),
                myrt: get("myrt"),
                mylgc: get("mylgc"),
                simple: get("simple"),
                system_libs: v.get("system_libs").and_then(|x| x.as_arr()).map(|a| {
                    a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
                }),
            });
        }
        None
    }

    /// Windows ABI 由 `mylc` 自身的 Rust target 决定：windows-msvc 编出的编译器
    /// 默认链接 MSVC 运行时，windows-gnu 编出的编译器默认链接 GNU 运行时。
    /// LLVM 后端仍使用同一个 host ABI；MinGW 的 clang 以 windows-gnu 为目标，
    /// 因此不能按“Windows + LLVM ⇒ MSVC”推断。
    pub fn validate(&self, tc: &Toolchain, lay: &Layout, root: &Path, _llvm: bool) -> Result<(), String> {
        let current_os = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        if !self.os.is_empty() && !self.os.eq_ignore_ascii_case(current_os) {
            return Err(format!(
                "构建清单 OS 为 `{}`，当前平台是 `{}`；请重新 make",
                self.os, current_os
            ));
        }

        let runtime_msvc = cfg!(windows) && cfg!(target_env = "msvc");
        let manifest_msvc = self.abi.eq_ignore_ascii_case("msvc")
            || self.style.eq_ignore_ascii_case("msvc");
        if manifest_msvc != runtime_msvc {
            return Err(format!(
                "构建清单 ABI 为 `{}`，当前链接路径是 `{}`；编译器/运行时/GC 不一致，请重新 make",
                if manifest_msvc { "msvc" } else { "gnu" },
                if runtime_msvc { "msvc" } else { "gnu" }
            ));
        }

        if !_llvm && !self.cc.is_empty() {
            if let Some(kind) = Toolchain::guess_builtin(&self.cc) {
                let cc_msvc = kind == "msvc";
                if cc_msvc != runtime_msvc {
                    return Err(format!(
                        "构建清单 C 编译器为 `{}`（{}），当前运行时 ABI 为 `{}`；不能用不同 ABI 的编译器链接运行时",
                        self.cc,
                        if cc_msvc { "msvc" } else { "gnu" },
                        if runtime_msvc { "msvc" } else { "gnu" }
                    ));
                }
            }
            // 即使 tc.style 仍是 msvc，显式 --cc/MYLIS_CC 也可能把实际编译器换成 gcc/clang。
            if let Some(kind) = Toolchain::guess_builtin(&tc.cc) {
                let actual_msvc = kind == "msvc";
                if actual_msvc != runtime_msvc {
                    return Err(format!(
                        "实际 C 编译器 `{}` 是 {}，但当前运行时 ABI 是 {}；请使用 cl.exe/clang-cl 或改用对应 ABI 的运行时",
                        tc.cc,
                        if actual_msvc { "msvc" } else { "gnu" },
                        if runtime_msvc { "msvc" } else { "gnu" }
                    ));
                }
            }
        }

        if !self.target.is_empty() {
            let target_msvc = self.target.contains("msvc");
            if target_msvc != runtime_msvc {
                return Err(format!(
                    "构建清单 target 为 `{}`，当前运行时 ABI 为 `{}`；请按对应 target 重建 myrt/mylgc",
                    self.target,
                    if runtime_msvc { "msvc" } else { "gnu" }
                ));
            }
        }

        let expected_myrt = if runtime_msvc { &lay.myrt_msvc_lib } else { &lay.myrt_lib };
        let expected_mylgc = if runtime_msvc { &lay.mylgc_msvc_lib } else { &lay.mylgc_lib };
        if !self.myrt.is_empty() && !same_relative(&self.myrt, expected_myrt, root) {
            return Err(format!(
                "构建清单声明运行时 `{}`，当前工具链需要 `{}`；请重新 make 或修正 mylis-build.json",
                self.myrt,
                relative_path(expected_myrt, root)
            ));
        }
        if !self.mylgc.is_empty() && !same_relative(&self.mylgc, expected_mylgc, root) {
            return Err(format!(
                "构建清单声明 GC `{}`，当前工具链需要 `{}`；请重新 make 或修正 mylis-build.json",
                self.mylgc,
                relative_path(expected_mylgc, root)
            ));
        }
        if !self.simple.is_empty() && runtime_msvc {
            let expected_simple = &lay.simple_msvc_lib;
            if !same_relative(&self.simple, expected_simple, root) {
                return Err(format!(
                    "构建清单声明 simple GC `{}`，当前工具链需要 `{}`",
                    self.simple,
                    relative_path(expected_simple, root)
                ));
            }
        }
        Ok(())
    }
}

fn normalize_rel(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches("./").trim_start_matches('/').to_string()
}

fn relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn same_relative(declared: &str, expected: &Path, root: &Path) -> bool {
    normalize_rel(declared) == normalize_rel(&relative_path(expected, root))
}

/// 解析并（必要时）迁移到无空格目录
pub fn layout(tc: &Toolchain, root: &Path, gc: &Path) -> Layout {
    let myrt_lib = root.join("build/libmylrt.a");
    let gc_lib = gc.join("lib/libgc.a");
    let mut out = Layout {
        cc: tc.cc.clone(),
        ar: tc.ar.clone(),
        myrt_inc: root.join("myrt"),
        gc_inc: gc.join("include"),
        myrt_lib,
        myrt_msvc_lib: root.join("build/mylrt.lib"),
        gc_lib,
        mylgc_lib: root.join("build/libmylgc.a"),
        simple_lib: root.join("build/libgc_simple.a"),
        simple_msvc_lib: root.join("build/gc_simple.lib"),
        boehm_lib: root.join("build/libgc_boehm.a"),
        mylgc_msvc_lib: root.join("build/mylgc.lib"),
    };
    // MSVC 能正确处理含空格的路径，且运行时走源码编译，不需要搬运
    if tc.style == Style::Msvc {
        return out;
    }
    if !tc.cc.contains(' ') && !root.to_string_lossy().contains(' ') {
        return out;
    }
    // 含空格：复制工具链前缀与运行时产物到缓存
    let cache = match cache_root() {
        Some(c) => c,
        None => {
            eprintln!("mylc: 警告：路径含空格且无法使用缓存目录，链接可能失败；建议移到不含空格的路径");
            return out;
        }
    };
    let cc_path = PathBuf::from(&tc.cc);
    let prefix = match cc_path.parent().and_then(|p| p.parent()) {
        Some(p) => p.to_path_buf(),
        None => return out,
    };
    let tc_dir = cache.join("toolchain");
    if !tc_dir.join(".complete").is_file() {
        eprintln!(
            "mylc: 编译器目录含空格，首次运行时复制工具链到 {}（只做一次）",
            tc_dir.display()
        );
        if copy_tree(&prefix, &tc_dir).is_err() {
            return out;
        }
        let _ = std::fs::write(tc_dir.join(".complete"), b"ok");
    }
    if let Some(name) = cc_path.file_name() {
        out.cc = tc_dir.join("bin").join(name).to_string_lossy().to_string();
    }
    if let Some(name) = PathBuf::from(&tc.ar).file_name() {
        let cand = tc_dir.join("bin").join(name);
        if cand.is_file() {
            out.ar = cand.to_string_lossy().to_string();
        }
    }
    // 运行时产物（每次刷新，体积很小）
    let rt = cache.join("rt");
    let _ = std::fs::create_dir_all(&rt);
    for (src, dst) in [
        (root.join("myrt"), rt.join("myrt")),
        (gc.join("include"), rt.join("gc_include")),
    ] {
        if src.is_dir() {
            let _ = copy_tree(&src, &dst);
        }
    }
    let rt_myrt = rt.join("libmylrt.a");
    let rt_gc = rt.join("libgc.a");
    let rt_mylgc = rt.join("libmylgc.a");
    if std::fs::copy(&out.myrt_lib, &rt_myrt).is_ok() {
        out.myrt_lib = rt_myrt;
    }
    if std::fs::copy(&out.gc_lib, &rt_gc).is_ok() {
        out.gc_lib = rt_gc;
    }
    if std::fs::copy(&out.mylgc_lib, &rt_mylgc).is_ok() {
        out.mylgc_lib = rt_mylgc;
    }
    let rt_msvc = cache.join("mylgc.lib");
    if std::fs::copy(&out.mylgc_msvc_lib, &rt_msvc).is_ok() {
        out.mylgc_msvc_lib = rt_msvc;
    }
    out.myrt_inc = rt.join("myrt");
    out.gc_inc = rt.join("gc_include");
    out
}

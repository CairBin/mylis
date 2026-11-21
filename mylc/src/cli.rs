//! 命令行参数解析（与驱动逻辑分离，便于单测与后续扩展子命令）。

use crate::i18n::{self, Lang};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Exe,
    Static,
    Shared,
    CSource,
    Ir,
    Llvm,
    Mylib,
    Check,
}

#[derive(Clone)]
pub struct Args {
    pub src: String,
    pub out: Option<String>,
    pub emit_c: Option<String>,
    pub emit_ir: Option<String>,
    pub emit_llvm: Option<String>,
    pub emit_mylib: Option<String>,
    pub update_lock: bool,
    pub offline: bool,
    pub link_mylibs: Vec<String>,
    pub mode: Mode,
    pub run: bool,
    pub verbose: bool,
    pub includes: Vec<String>,
    pub link_libs: Vec<String>,
    pub link_dirs: Vec<String>,
    pub link_files: Vec<String>,
    pub cc: Option<String>,
    pub embed_runtime: bool,
    /// `.mylib` 内嵌的原生静态库/动态库路径（仅 `--emit-mylib` 生效）。
    pub embed_static: Option<String>,
    pub embed_shared: Option<String>,
    /// 后端选择：None => LLVM（默认），mir => MIR-C，ast/c => AST-C
    pub backend: Option<String>,
    /// 自动加入 std 源码与当前平台 native 产物。
    pub std: bool,
}

pub fn usage() {
    eprintln!(
        "{}\n\n{}\n\n{}\n  -o <path>        {}\n  --static         {}\n  --shared         {}\n  --embed-runtime  {}\n  --emit-c <path>  {}\n  --emit-ir <path> {}\n  --emit-llvm <path> {}\n  --emit-mylib <path> {}\n  --embed-static <path>  把静态库嵌入 .mylib（仅 --emit-mylib）\n  --embed-shared <path>  把动态库嵌入 .mylib（仅 --emit-mylib）\n  --backend <name> {}\n  --check          {}
  --update-lock    {}\n  --offline        {}\n\n{}\n  -l <name>        {}\n  -L <dir>         {}\n  --link <file>    {}\n  --link-mylib <file> {}\n  --std            {}\n\n{}\n  --run            {}\n  -I <dir>         {}\n  --cc <program>   {}\n  --lang zh|en     {}\n  -v               {}",
        i18n::t("usage.head"),
        i18n::t("usage.usage"),
        i18n::t("usage.products"),
        i18n::t("usage.o"),
        i18n::t("usage.static"),
        i18n::t("usage.shared"),
        i18n::t("usage.embed"),
        i18n::t("usage.emitc"),
        i18n::t("usage.emitir"),
        i18n::t("usage.emitllvm"),
        i18n::t("usage.emmylib"),
        i18n::t("usage.backend"),
        i18n::t("usage.check"),
        "重新生成 mylis.lock（registry/path 依赖都会重写）",
        i18n::t("usage.offline"),
        i18n::t("usage.link.head"),
        i18n::t("usage.l"),
        i18n::t("usage.L"),
        i18n::t("usage.linkfile"),
        i18n::t("usage.linkmylib"),
        "自动加入 std 源码与当前平台 native 链接",
        i18n::t("usage.other"),
        i18n::t("usage.run"),
        i18n::t("usage.I"),
        i18n::t("usage.cc"),
        i18n::t("usage.lang"),
        i18n::t("usage.v"),
    );
}

fn need(argv: &[String], i: usize, key: &'static str) -> Result<String, String> {
    match argv.get(i) {
        Some(v) => Ok(v.clone()),
        None => Err(i18n::t(key).to_string()),
    }
}

pub enum Command {
    Compile(Args),
    Build { target: Option<String>, run: bool, update_lock: bool, offline: bool, std: bool },
    Test,
}

pub fn parse() -> Result<Command, String> {
    let argv: Vec<String> = env::args().skip(1).collect();
    i18n::set(i18n::from_env());
    match argv.first().map(|s| s.as_str()) {
        Some("build") | Some("run") => {
            let run = argv.first().map(|s| s == "run").unwrap_or(false);
            let mut target = None;
            let mut update_lock = false;
            let mut offline = false;
            let mut std = false;
            for a in &argv[1..] {
                if a == "--update-lock" {
                    update_lock = true;
                } else if a == "--offline" {
                    offline = true;
                } else if a == "--std" {
                    std = true;
                } else if !a.starts_with('-') {
                    target = Some(a.clone());
                }
            }
            Ok(Command::Build { target, run, update_lock, offline, std })
        }
        Some("test") => Ok(Command::Test),
        _ => parse_args().map(Command::Compile),
    }
}

pub fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = env::args().skip(1).collect();
    i18n::set(i18n::from_env());
    if argv.is_empty() {
        return Err(i18n::t("err.no_input").to_string());
    }
    let mut a = Args {
        src: String::new(),
        out: None,
        emit_c: None,
        emit_ir: None,
        emit_llvm: None,
        emit_mylib: None,
        update_lock: false,
        offline: false,
        link_mylibs: Vec::new(),
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
        backend: None,
        std: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                usage();
                std::process::exit(0);
            }
            "-v" | "--verbose" => a.verbose = true,
            "--check" => a.mode = Mode::Check,
            "--update-lock" => a.update_lock = true,
            "--offline" => a.offline = true,
            "--run" => a.run = true,
            "--lib" | "--static" | "--static-lib" => a.mode = Mode::Static,
            "--shared" | "--dylib" => a.mode = Mode::Shared,
            "--embed-runtime" => a.embed_runtime = true,
            "--embed-static" => {
                i += 1;
                a.embed_static = Some(need(&argv, i, "err.embed_static")?);
            }
            "--embed-shared" => {
                i += 1;
                a.embed_shared = Some(need(&argv, i, "err.embed_shared")?);
            }
            "--lang" => {
                i += 1;
                let v = need(&argv, i, "err.lang_value")?;
                match i18n::parse(&v) {
                    Some(l) => i18n::set(l),
                    None => return Err(i18n::t("err.lang_value").to_string()),
                }
            }
            "--emit-c" => {
                i += 1;
                a.emit_c = Some(need(&argv, i, "err.emitc_path")?);
                a.mode = Mode::CSource;
            }
            "--emit-ir" => {
                i += 1;
                a.emit_ir = Some(need(&argv, i, "err.emitir_path")?);
                a.mode = Mode::Ir;
            }
            "--emit-llvm" => {
                i += 1;
                a.emit_llvm = Some(need(&argv, i, "err.emitllvm_path")?);
                a.mode = Mode::Llvm;
            }
            "--emit-mylib" => {
                i += 1;
                a.emit_mylib = Some(need(&argv, i, "err.emmylib_path")?);
                a.mode = Mode::Mylib;
            }
            "--backend" => {
                i += 1;
                a.backend = Some(need(&argv, i, "err.backend_name")?);
            }
            "-o" | "--out" => {
                i += 1;
                a.out = Some(need(&argv, i, "err.o_path")?);
            }
            "-I" => {
                i += 1;
                a.includes.push(need(&argv, i, "err.I_path")?);
            }
            "-l" => {
                i += 1;
                a.link_libs.push(need(&argv, i, "err.l_name")?);
            }
            "-L" => {
                i += 1;
                a.link_dirs.push(need(&argv, i, "err.L_dir")?);
            }
            "--link" => {
                i += 1;
                a.link_files.push(need(&argv, i, "err.link_file")?);
            }
            "--link-mylib" => {
                i += 1;
                a.link_mylibs.push(need(&argv, i, "err.link_mylib")?);
            }
            "--std" => a.std = true,
            "--cc" => {
                i += 1;
                a.cc = Some(need(&argv, i, "err.cc_prog")?);
            }
            s if s.starts_with('-') => {
                return Err(format!("{} {}", i18n::t("err.unknown_opt"), s));
            }
            s => {
                if !a.src.is_empty() {
                    return Err(i18n::t("err.multi_src").to_string());
                }
                a.src = s.to_string();
            }
        }
        i += 1;
    }
    if a.src.is_empty() {
        return Err(i18n::t("err.no_input").to_string());
    }
    if a.std {
        apply_std_defaults(&mut a)?;
    }
    let _ = Lang::Zh;
    Ok(a)
}

/// 找到工程根：优先 `MYLIS_STD_ROOT`，否则从当前目录向上找 `std/mylis.toml`，
/// 最后尝试编译器可执行文件旁的工程根。
fn find_std_root() -> Option<PathBuf> {
    if let Ok(v) = env::var("MYLIS_STD_ROOT") {
        let p = PathBuf::from(v);
        if p.join("mylis.toml").is_file() {
            return Some(p);
        }
    }
    let cwd = env::current_dir().ok()?;
    for dir in cwd.ancestors() {
        let p = dir.join("std");
        if p.join("mylis.toml").is_file() {
            return Some(p);
        }
    }
    let exe = env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..4 {
        let p = dir.join("std");
        if p.join("mylis.toml").is_file() {
            return Some(p);
        }
        dir = dir.parent()?;
    }
    None
}

pub fn apply_std_defaults(a: &mut Args) -> Result<(), String> {
    let root = find_std_root().ok_or_else(|| {
        "找不到 std 包；请设置 MYLIS_STD_ROOT 或在工程根目录运行".to_string()
    })?;
    let src = root.join("src");
    if src.is_dir() && !a.includes.iter().any(|x| Path::new(x) == src) {
        a.includes.push(src.to_string_lossy().to_string());
    }
    // std 通过 `core/...` 引用 core，因此把工程根也作为模块搜索根。
    if let Some(project) = root.parent() {
        if !a.includes.iter().any(|x| Path::new(x) == project) {
            a.includes.push(project.to_string_lossy().to_string());
        }
    }
    // 平台侧车随编译器目标选择：POSIX 用 std-posix，Windows 用 std-win32。
    let (pkg, prefix, ext) = if cfg!(windows) {
        ("std-win32", "", "lib")
    } else {
        ("std-posix", "lib", "a")
    };
    let native = root.parent().unwrap_or(&root).join(pkg).join("native");
    if native.is_dir() && !a.includes.iter().any(|x| Path::new(x) == native) {
        a.includes.push(native.to_string_lossy().to_string());
    }
    for stem in ["file", "io", "time", "net", "event", "process"] {
        let name = format!("{}mylis_{}.{}", prefix, stem, ext);
        let lib = root.parent().unwrap_or(&root).join("build").join(pkg).join("native").join(&name);
        if lib.is_file() && !a.link_files.iter().any(|x| Path::new(x) == lib) {
            a.link_files.push(lib.to_string_lossy().to_string());
        }
    }
    Ok(())
}

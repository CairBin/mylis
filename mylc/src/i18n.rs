//! mylc 命令行消息的国际化（简中 / 英文）。
//!
//! 选择顺序：`--lang zh|en` → `MYLIS_LANG` → `LC_ALL`/`LANG`（以 `zh` 开头取中文）→ 默认中文。
//! 这里只覆盖 **mylc 自身**的驱动/构建消息；语言语义诊断（sema/parser）文案目前仍为中文，
//! 迁移时把字符串换成 `i18n::t(k)` 即可，机制已经就位。

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq)]
pub enum Lang {
    Zh,
    En,
}

static CURRENT: AtomicU8 = AtomicU8::new(0); // 0 = zh, 1 = en

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Zh
    }
}

pub fn set(l: Lang) {
    CURRENT.store(if l == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn parse(s: &str) -> Option<Lang> {
    let s = s.trim().to_ascii_lowercase();
    if s.starts_with("zh") || s == "cn" || s == "chinese" {
        Some(Lang::Zh)
    } else if s.starts_with("en") || s == "english" {
        Some(Lang::En)
    } else {
        None
    }
}

/// 从环境变量推断语言：`MYLIS_LANG` 优先；系统 locale 只在明确为中文时生效，默认中文
pub fn from_env() -> Lang {
    if let Ok(v) = std::env::var("MYLIS_LANG") {
        if let Some(l) = parse(&v) {
            return l;
        }
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if v.trim().to_ascii_lowercase().starts_with("zh") {
                return Lang::Zh;
            }
        }
    }
    Lang::Zh
}

/// 查表翻译：键为稳定标识，两侧都找不到时回落到中文文案
pub fn t(key: &'static str) -> &'static str {
    let (zh, en) = table(key);
    match current() {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

fn table(key: &'static str) -> (&'static str, &'static str) {
    match key {
        // ---- CLI 用法 ----
        "usage.head" => ("mylc —— Mylis 编译器", "mylc — the Mylis compiler"),
        "usage.usage" => ("用法：mylc <file.myl> [选项]", "Usage: mylc <file.myl> [options]"),
        "usage.products" => ("产物：", "Output:"),
        "usage.o" => ("指定产物路径", "write the artifact to <path>"),
        "usage.static" => ("生成静态库（.a + .h）", "build a static library (.a + .h)"),
        "usage.shared" => ("生成动态库（.dll/.so + .h）", "build a shared library (.dll/.so + .h)"),
        "usage.embed" => ("动态库内嵌运行时（默认：Windows 内嵌，Linux 不内嵌）", "link the runtime into the shared library"),
        "usage.emitc" => ("只生成 C 源码", "emit the generated C source only"),
        "usage.emitir" => ("只生成 MIR 文本（.myl.ir）", "emit MIR text only (.myl.ir)"),
        "usage.emitllvm" => ("只生成 LLVM IR 文本（.myl.ll）", "emit LLVM IR text only (.myl.ll)"),
        "usage.emmylib" => ("生成后端无关的 MIR 库（.mylib）", "emit a backend-neutral MIR library (.mylib)"),
        "err.emmylib_path" => ("--emit-mylib 缺少输出路径", "--emit-mylib requires a path"),
        "usage.linkmylib" => ("链接 .mylib MIR 库（可重复）", "link a .mylib MIR library (repeatable)"),
        "err.link_mylib" => ("--link-mylib 缺少文件路径", "--link-mylib requires a path"),
        "usage.backend" => ("选择后端：llvm（默认）| mir | ast/c（兼容回退）| ir", "backend: llvm (default) | mir | ast/c (fallback) | ir"),
        "usage.debug" => ("使用 debug profile（-O0 -g）", "use the debug profile (-O0 -g)"),
        "usage.release" => ("使用 release profile（-O2，裁剪调试段）", "use the release profile (-O2, strip debug sections)"),
        "usage.profile" => ("构建 profile：debug | release | relwithdebinfo | minsize", "build profile: debug | release | relwithdebinfo | minsize"),
        "usage.offline" => ("离线模式：不拉取 Git、不下载 .mylib，只用缓存", "offline: use cached Git/.mylib only; no fetch/download"),
        "usage.check" => ("只检查，不生成", "type-check only, no output"),
        "usage.link.head" => ("链接外部库：", "Link external libraries:"),
        "usage.l" => ("链接名为 name 的库（可多次）", "link library <name> (repeatable)"),
        "usage.L" => ("追加库搜索目录（可多次）", "add a library search directory (repeatable)"),
        "usage.linkfile" => ("直接链接一个 .a/.o/.dll 文件（可多次）", "link a .a/.o/.dll file directly (repeatable)"),
        "usage.other" => ("其他：", "Other:"),
        "usage.run" => ("编译后立即运行", "run the produced executable"),
        "usage.I" => ("追加模块搜索路径（可多次）", "add a module search path (repeatable)"),
        "usage.cc" => ("覆盖 C 编译器（等同 MYLIS_CC）", "override the C compiler (same as MYLIS_CC)"),
        "usage.lang" => ("输出语言：zh | en", "message language: zh | en"),
        "usage.v" => ("输出详细过程", "verbose driver output"),

        // ---- 参数错误 ----
        "err.no_input" => ("缺少输入文件", "no input file"),
        "err.emitir_path" => ("--emit-ir 需要一个输出路径", "--emit-ir needs an output path"),
        "err.backend_name" => ("--backend 需要一个后端名", "--backend needs a backend name"),
        "err.profile_value" => (
            "--profile 需要 debug / release / relwithdebinfo / minsize",
            "--profile expects debug / release / relwithdebinfo / minsize",
        ),
        "err.emitc_path" => ("--emit-c 需要路径", "--emit-c requires a path"),
        "err.emitllvm_path" => ("--emit-llvm 需要路径", "--emit-llvm requires a path"),
        "err.o_path" => ("-o 需要路径", "-o requires a path"),
        "err.I_path" => ("-I 需要目录", "-I requires a directory"),
        "err.l_name" => ("-l 需要库名", "-l requires a library name"),
        "err.L_dir" => ("-L 需要目录", "-L requires a directory"),
        "err.link_file" => ("--link 需要文件", "--link requires a file"),
        "err.cc_prog" => ("--cc 需要程序名", "--cc requires a program name"),
        "err.lang_value" => ("--lang 需要 zh 或 en", "--lang expects zh or en"),
        "err.unknown_opt" => ("未知选项", "unknown option"),
        "err.multi_src" => ("一次只能编译一个入口文件", "only one entry file may be compiled at a time"),

        // ---- 驱动消息 ----
        "load.modules" => ("已加载 {n} 个模块（拓扑顺序：{list}）", "loaded {n} module(s), topological order: {list}"),
        "load.cannot" => ("无法加载", "cannot load"),
        "sema.failed" => ("编译失败，共 {n} 个错误", "compilation failed with {n} error(s)"),
        "check.ok" => ("检查通过（{m} 模块，{d} 个声明）", "check passed ({m} module(s), {d} declaration(s))"),
        "entry.no_main" => ("入口文件 {stem} 缺少 fn main() -> i32", "entry file {stem} has no fn main() -> i32"),
        "emit.ir_ok" => ("已生成 MIR {path}", "wrote MIR {path}"),
        "emit.llvm_ok" => ("已生成 LLVM IR {path}", "wrote LLVM IR {path}"),
        "emit.c_ok" => ("已生成 C 源码 {path}", "wrote C source {path}"),
        "err.write_c" => ("无法写入 {path}", "cannot write {path}"),
        "emit.exe" => ("已生成 {path}", "wrote {path}"),
        "emit.lib" => ("已生成{kind} {path} 与头文件 {hdr}", "wrote {kind} {path} and header {hdr}"),
        "kind.static" => ("静态库", "static library"),
        "kind.shared" => ("动态库", "shared library"),
        "toolchain.from" => ("工具链来自 {origin}（家族 {family}）：cc={cc} ar={ar}", "toolchain from {origin} (family {family}): cc={cc} ar={ar}"),
        "err.no_rt" => ("找不到运行时静态库 {path}，请先在项目根目录执行 `make`", "runtime library {path} not found; run `make` in the project root first"),
        "err.no_gc" => ("找不到 GC 静态库，请执行 `bash tools/bootstrap_gc.sh`", "GC library not found; run `bash tools/bootstrap_gc.sh`"),
        "err.cc_fail" => ("C 后端编译失败（{cc}）", "C backend failed ({cc})"),
        "err.ar_fail" => ("归档失败（{ar}）", "archiving failed ({ar})"),
        "err.msvc_shared" => ("MSVC 家族暂不支持 --shared（导出符号需 __declspec(dllexport)）", "the msvc family does not support --shared yet (exports need __declspec(dllexport))"),
        "warn.msvc_boehm_own" => ("MSVC 家族没有 Boehm 版静态库，自动改用自研 GC（mylgc.lib）", "no MSVC build of Boehm; using the built-in GC (mylgc.lib) instead"),
        "warn.msvc_own_gc" => ("MSVC 家族使用自研 GC（mylgc.lib）", "msvc family uses the built-in GC (mylgc.lib)"),
        "err.cc_spawn" => ("无法启动编译器 `{cc}`：确认它在 PATH 中。MSVC 需要在 VS 开发者命令提示符里运行，或先执行 vcvars64.bat", "cannot start compiler `{cc}`: make sure it is on PATH. For MSVC, run from a Developer Command Prompt or call vcvars64.bat first"),
        "err.msvc_no_gc" => ("找不到 build/mylgc.lib（MSVC 版自研 GC）。请执行 `make build/mylgc.lib` 或 `cd mylgc && cargo build --release --target x86_64-pc-windows-msvc`", "build/mylgc.lib is missing (MSVC build of the built-in GC). Run `make build/mylgc.lib` or `cd mylgc && cargo build --release --target x86_64-pc-windows-msvc`"),
        "warn.msvc_gc" => ("MSVC 家族使用 gc_simple 运行时（Boehm 无 MSVC 版静态库）；如已设 MYLIS_GC=boehm 将被忽略", "msvc family uses the gc_simple runtime (no MSVC build of Boehm); MYLIS_GC=boehm is ignored"),
        "err.run" => ("无法运行 {path}", "cannot run {path}"),
        "warn.spaces" => (
            "路径含空格且无法使用缓存目录，链接可能失败；建议移到不含空格的路径",
            "paths contain spaces and no cache dir is usable; linking may fail — move the install to a path without spaces",
        ),
        "cache.copy" => ("编译器目录含空格，首次运行时复制工具链到 {dir}（只做一次）", "install path has spaces; copying toolchain to {dir} once"),
        "exec" => ("执行 {cmd}", "exec {cmd}"),
        _ => (key, key),
    }
}

/// 用 `{name}` 占位符格式化消息
pub fn tf(key: &'static str, pairs: &[(&str, &str)]) -> String {
    let mut s = t(key).to_string();
    for (k, v) in pairs {
        s = s.replace(&format!("{{{}}}", k), v);
    }
    s
}

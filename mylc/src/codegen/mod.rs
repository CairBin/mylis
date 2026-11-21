//! C 代码生成：把已通过语义检查的 AST 发射为 C11 源码，
//! 由 gcc 编译并链接 mylrt（Boehm GC）。

use crate::ast::*;
use std::collections::{BTreeSet, HashSet};

pub struct Codegen<'a> {
    prog: &'a Program,
    out: String,
    body: String,
    in_body: bool,
    helper_protos: Vec<String>,
    helper_defs: Vec<String>,
    helpers_done: HashSet<String>,
    emitted_insts: BTreeSet<String>,
    ind: usize,
    scopes: Vec<Vec<(String, String)>>,
    tmp_counter: usize,
    loop_counter: usize,
    continue_stack: Vec<Option<String>>,
    fn_ret: Type,
    is_lib: bool,
    export_dll: bool,
}

/// 读取 `@export("name")` 注解
pub fn export_name(d: &Decl) -> Option<String> {
    for a in &d.annots {
        if a.name == "export" {
            if let Some(AnnotArg::Str(v)) = a.args.first() {
                return Some(v.clone());
            }
        }
    }
    None
}

/// 单个语言类型对应的 C 类型名（与库头文件一致）
pub fn ctype_of(prog: &Program, t: &crate::ast::Type) -> String {
    let cg = Codegen {
        prog,
        out: String::new(),
        body: String::new(),
        in_body: false,
        helper_protos: Vec::new(),
        helper_defs: Vec::new(),
        helpers_done: HashSet::new(),
        emitted_insts: BTreeSet::new(),
        ind: 0,
        scopes: Vec::new(),
        tmp_counter: 0,
        loop_counter: 0,
        continue_stack: Vec::new(),
        fn_ret: crate::ast::Type::Void,
        is_lib: false,
        export_dll: false,
    };
    cg.ctype(t)
}


/// 为库头文件补齐 Option/Result/类/枚举/接口的前置类型声明。
fn declare_header_types(cg: &Codegen, t: &Type, out: &mut String, seen: &mut HashSet<String>) {
    match t {
        Type::Option(inner) => {
            declare_header_types(cg, inner, out, seen);
            let name = cg.ctype(t);
            if seen.insert(name.clone()) {
                out.push_str(&format!(
                    "typedef struct {} {{ myrt_i32 tag; {} value; }} {};\n",
                    name, cg.ctype(inner), name
                ));
            }
        }
        Type::Result(ok, err) => {
            declare_header_types(cg, ok, out, seen);
            declare_header_types(cg, err, out, seen);
            let name = cg.ctype(t);
            if seen.insert(name.clone()) {
                out.push_str(&format!(
                    "typedef struct {} {{ myrt_i32 tag; {} ok; {} err; }} {};\n",
                    name, cg.ctype(ok), cg.ctype(err), name
                ));
            }
        }
        Type::Array(elem) => {
            declare_header_types(cg, elem, out, seen);
            let name = format!("MylA_{}", cg.mangle(t));
            if seen.insert(name.clone()) {
                out.push_str(&format!("typedef struct {} {};\n", name, name));
            }
        }
        Type::Class(id) => {
            let name = cg.class_cname(*id);
            if seen.insert(name.clone()) {
                out.push_str(&format!("typedef struct {} {};\n", name, name));
            }
        }
        Type::Enum(id) => {
            let name = cg.enum_cname(*id);
            if seen.insert(name.clone()) {
                out.push_str(&format!("typedef struct {} {};\n", name, name));
            }
        }
        Type::Iface(id) => {
            let name = cg.iface_cname(*id);
            if seen.insert(name.clone()) {
                out.push_str(&format!("typedef struct {} {{ void* obj; const void* vt; }} {};\n", name, name));
            }
        }
        Type::Ptr(e) => declare_header_types(cg, e, out, seen),
        _ => {}
    }
}

/// 生成库头文件：只列出带 `@export` 的顶层函数
pub fn header_text(prog: &Program) -> String {
    let cg = Codegen {
        prog,
        out: String::new(),
        body: String::new(),
        in_body: false,
        helper_protos: Vec::new(),
        helper_defs: Vec::new(),
        helpers_done: HashSet::new(),
        emitted_insts: BTreeSet::new(),
        ind: 0,
        scopes: Vec::new(),
        tmp_counter: 0,
        loop_counter: 0,
        continue_stack: Vec::new(),
        fn_ret: Type::Void,
        is_lib: false,
        export_dll: false,
    };
    let stem = prog
        .modules
        .get(prog.entry)
        .map(|m| m.stem.clone())
        .unwrap_or_else(|| "mylis".to_string());
    let guard = format!("__MYLIS_{}_H__", stem.to_uppercase());
    let mut type_defs = String::new();
    let mut seen_types: HashSet<String> = HashSet::new();
    for d in prog.decls.iter() {
        if d.kind != DeclKind::Fn || d.owner.is_some() || !d.is_pub || export_name(d).is_none() {
            continue;
        }
        declare_header_types(&cg, &d.ret, &mut type_defs, &mut seen_types);
        for p in &d.params {
            declare_header_types(&cg, &p.ty, &mut type_defs, &mut seen_types);
        }
    }
    let mut out = String::new();
    out.push_str(&format!("/* 由 mylc --lib 生成（{}） */\n", stem));
    out.push_str(&format!("#ifndef {g}\n#define {g}\n\n", g = guard));
    out.push_str("#include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n");
    out.push_str("\ntypedef struct MyStr MyStr;\n\n");
    if !type_defs.is_empty() {
        out.push_str("/* 导出签名使用的聚合类型 */\n");
        out.push_str(&type_defs);
        out.push('\n');
    }
    out.push_str(&format!("/* 初始化运行时与静态成员，使用本库前必须调用一次 */\n"));
    out.push_str(&format!("void {}_init(void);\n\n", stem));
    for (i, d) in prog.decls.iter().enumerate() {
        if d.kind != DeclKind::Fn || d.owner.is_some() || !d.is_pub {
            continue;
        }
        let name = match export_name(d) {
            Some(n) => n,
            None => continue,
        };
        let ret = if d.ret == Type::Void {
            "void".to_string()
        } else {
            cg.ctype(&d.ret)
        };
        let params = if d.params.is_empty() {
            "void".to_string()
        } else {
            d.params
                .iter()
                .map(|p| format!("{} {}", cg.ctype(&p.ty), p.name))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!("/* 来自 {} */\n{} {}({});\n", prog.modules[d.mod_id].stem, ret, name, params));
        let _ = i;
    }
    out.push_str(&format!("\n#endif /* {} */\n", guard));
    // 公共头保持标准 C：不暴露 myrt_* 运行时类型名，调用方不需要 mylrt.h。
    out = out
        .replace("myrt_i8", "int8_t")
        .replace("myrt_i16", "int16_t")
        .replace("myrt_i32", "int32_t")
        .replace("myrt_i64", "int64_t")
        .replace("myrt_u8", "uint8_t")
        .replace("myrt_u16", "uint16_t")
        .replace("myrt_u32", "uint32_t")
        .replace("myrt_u64", "uint64_t")
        .replace("myrt_f32", "float")
        .replace("myrt_f64", "double")
        .replace("myrt_bool", "bool");
    out
}

pub fn generate(prog: &Program) -> String {
    let mut cg = Codegen {
        prog,
        out: String::new(),
        body: String::new(),
        in_body: false,
        helper_protos: Vec::new(),
        helper_defs: Vec::new(),
        helpers_done: HashSet::new(),
        emitted_insts: BTreeSet::new(),
        ind: 0,
        scopes: Vec::new(),
        tmp_counter: 0,
        loop_counter: 0,
        continue_stack: Vec::new(),
        fn_ret: Type::Void,
        is_lib: false,
        export_dll: false,
    };
    cg.run();
    cg.out
}

/// 生成库版 C 源码：不产生 `main`，改为导出 `<stem>_init()` 作为库初始化入口
pub fn generate_lib(prog: &Program) -> String {
    generate_lib_with(prog, false)
}

/// 库版 C 源码，`export_dll` 为真时给导出符号加 `__declspec(dllexport)`
pub fn generate_lib_with(prog: &Program, export_dll: bool) -> String {
    let mut cg = Codegen {
        prog,
        out: String::new(),
        body: String::new(),
        in_body: false,
        helper_protos: Vec::new(),
        helper_defs: Vec::new(),
        helpers_done: HashSet::new(),
        emitted_insts: BTreeSet::new(),
        ind: 0,
        scopes: Vec::new(),
        tmp_counter: 0,
        loop_counter: 0,
        continue_stack: Vec::new(),
        fn_ret: Type::Void,
        is_lib: true,
        export_dll,
    };
    cg.run();
    cg.out
}

mod names;
mod support;
mod emit;
mod stmt;
mod expr;

pub(super) fn block_has_continue(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_continue)
}

pub(super) fn stmt_has_continue(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Continue => true,
        StmtKind::If { then_b, else_b, .. } => {
            block_has_continue(then_b) || else_b.as_ref().map(|e| stmt_has_continue(e)).unwrap_or(false)
        }
        StmtKind::Block(b) | StmtKind::Unsafe(b) => block_has_continue(b),
        _ => false,
    }
}

pub(super) fn c_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

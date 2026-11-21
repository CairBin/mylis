//! MIR → C 实验后端：只覆盖基本类型、局部槽、算术、分支、直接函数调用。
//!
//! 这是“C 后端消费 MIR”的第一段可执行验证，不是完整后端：
//! 字符串/对象/接口/数组/模式绑定仍由 AST 后端负责。遇到不支持的结构会输出
//! `#error`，避免静默生成错误代码。

use super::{Artifact, ArtifactKind, Backend, ExportMode};
use crate::ast::{BinOp, Builtin, Pattern, Program, Res, Type, UnOp};
use crate::mir::{
    self, MirConst, MirFunction, MirInstKind, MirInterpPart, MirOperand, MirProgram, MirTerm,
};
use std::collections::{HashMap, HashSet};

pub struct MirCBackend;

impl Backend for MirCBackend {
    fn name(&self) -> &'static str {
        "mir"
    }

    fn emit_exe(&self, prog: &Program) -> Artifact {
        let ir = mir::lower(prog);
        Artifact { kind: ArtifactKind::CSource, text: emit(&ir, true, prog), error: None }
    }

    fn emit_lib(&self, prog: &Program, mode: ExportMode) -> Artifact {
        let ir = mir::lower(prog);
        let mut c = emit(&ir, false, prog);
        let stem = prog
            .modules
            .get(prog.entry)
            .map(|m| m.stem.clone())
            .unwrap_or_else(|| "mylis".to_string());
        let dll = if mode == ExportMode::DllExport { "__declspec(dllexport) " } else { "" };
        let has_statics = ir.classes.iter().any(|c| c.fields.iter().any(|f| f.is_static));
        c.push_str(&format!(
            "\n{}void {}_init(void) {{\n  myrt_init();\n{}}}\n",
            dll,
            stem,
            if has_statics { "  myl_mir_init_statics();\n" } else { "" }
        ));
        for d in &prog.decls {
            if d.kind != crate::ast::DeclKind::Fn || d.owner.is_some() || !d.is_pub {
                continue;
            }
            let exp = match crate::codegen::export_name(d) {
                Some(n) => n,
                None => continue,
            };
            let f = ir
                .funcs
                .iter()
                .find(|f| f.name == d.name || f.name.ends_with(&format!("::{}", d.name)));
            let f = match f { Some(f) => f, None => continue };
            let ret = crate::codegen::ctype_of(prog, &d.ret);
            let params = d
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{} l{}", crate::codegen::ctype_of(prog, &p.ty), i))
                .collect::<Vec<_>>()
                .join(", ");
            let args = (0..d.params.len())
                .map(|i| format!("l{}", i))
                .collect::<Vec<_>>()
                .join(", ");
            let call = format!("{}({})", cname(&f.name), args);
            let body = if d.ret == Type::Void {
                format!("  {};\n", call)
            } else {
                format!("  return {};\n", call)
            };
            c.push_str(&format!(
                "\n{}{} {}({}) {{\n{}}}\n",
                dll, ret, exp, if params.is_empty() { "void".to_string() } else { params }, body
            ));
        }
        Artifact { kind: ArtifactKind::CSource, text: c, error: None }
    }

    fn header(&self, prog: &Program) -> String {
        crate::codegen::header_text(prog)
    }

    fn emit_exe_mir(&self, prog: &Program, ir: &MirProgram) -> Artifact {
        Artifact { kind: ArtifactKind::CSource, text: emit(ir, true, prog), error: None }
    }
}

/// 接口槽位与具体方法不仅要名字相同，返回值和参数类型也必须一致。
/// 泛型接口的不同实例（如 PairView<i64, string> 与 Pair<string, i64>）
/// 在方法名上可能完全相同，只按名字生成 vtable 会把 C 函数指针写成
/// 不兼容的签名。
fn iface_method_matches(f: &MirFunction, m: &mir::MirIfaceMethod) -> bool {
    // MIR 的函数参数表把非静态方法的接收者放在第 0 位；接口签名里没有它。
    let skip_receiver = usize::from(f.owner.is_some() && !f.is_static);
    let params = &f.params[skip_receiver..];
    f.ret == m.ret
        && params.len() == m.params.len()
        && params.iter().zip(m.params.iter()).all(|(a, b)| a.ty == *b)
}

fn boxable(t: &Type) -> bool {
    matches!(
        t,
        Type::I8 | Type::I16 | Type::I32 | Type::I64
            | Type::U8 | Type::U16 | Type::U32 | Type::U64
            | Type::F32 | Type::F64 | Type::Bool | Type::Str
    )
}

fn box_key(t: &Type) -> &'static str {
    match t {
        Type::I8 => "i8", Type::I16 => "i16", Type::I32 => "i32", Type::I64 => "i64",
        Type::U8 => "u8", Type::U16 => "u16", Type::U32 => "u32", Type::U64 => "u64",
        Type::F32 => "f32", Type::F64 => "f64", Type::Bool => "bool", Type::Str => "str",
        _ => "i32",
    }
}

fn box_id(t: &Type) -> i32 {
    match t {
        Type::I8 => -1, Type::I16 => -2, Type::I32 => -3, Type::I64 => -4,
        Type::U8 => -5, Type::U16 => -6, Type::U32 => -7, Type::U64 => -8,
        Type::F32 => -9, Type::F64 => -10, Type::Bool => -11, Type::Str => -12,
        _ => -3,
    }
}

fn box_ctype(t: &Type) -> &'static str {
    match t {
        Type::I8 => "myrt_i8", Type::I16 => "myrt_i16", Type::I32 => "myrt_i32", Type::I64 => "myrt_i64",
        Type::U8 => "myrt_u8", Type::U16 => "myrt_u16", Type::U32 => "myrt_u32", Type::U64 => "myrt_u64",
        Type::F32 => "float", Type::F64 => "double", Type::Bool => "myrt_bool", Type::Str => "MyStr*",
        _ => "myrt_i32",
    }
}

fn cty(t: &Type) -> Option<String> {
    Some(match t {
        Type::Void => "void".into(),
        Type::I8 => "myrt_i8".into(),
        Type::I16 => "myrt_i16".into(),
        Type::I32 => "myrt_i32".into(),
        Type::I64 => "myrt_i64".into(),
        Type::U8 => "myrt_u8".into(),
        Type::U16 => "myrt_u16".into(),
        Type::U32 => "myrt_u32".into(),
        Type::U64 => "myrt_u64".into(),
        Type::F32 => "float".into(),
        Type::F64 => "double".into(),
        Type::Bool => "myrt_bool".into(),
        Type::Class(_) => "void*".into(),
        Type::Str => "MyStr*".into(),
        Type::Array(_) => "void*".into(),
        Type::Enum(_) => "void*".into(),
        Type::Iface(id) => format!("struct I_{}", id),
        Type::Ptr(_) => "void*".into(),
        Type::Named(n) if n.ends_with('*') => "void*".into(),
        Type::Result(ok, err) => format!("MylR_{}_{}", type_key(ok), type_key(err)),
        Type::Option(ok) => format!("MylO_{}", type_key(ok)),
        _ => return None,
    })
}

fn result_flags(t: &Type) -> (bool, bool) {
    match t {
        Type::Result(ok, err) => (
            matches!(&**ok, Type::Iface(_)),
            matches!(&**err, Type::Iface(_)),
        ),
        _ => (false, false),
    }
}

fn iface_adapt(from: usize, to: usize, expr: String) -> String {
    if from == to { expr } else { format!("A_{}_{}({})", from, to, expr) }
}

fn iface_wrap(from: usize, to: usize, expr: String) -> String {
    format!(
        "((struct I_{}){{ .obj = (void*)({}), .vt = &VT_{}_{} }})",
        to, expr, from, to
    )
}

fn wrap_coerce(i: &crate::mir::MirInst, expr: String) -> String {
    match &i.coerce {
        Some((from, to)) => iface_wrap(*from, *to, expr),
        _ => expr,
    }
}

fn type_key(t: &Type) -> String {
    match t {
        Type::I8 => "i8".into(),
        Type::I16 => "i16".into(),
        Type::I32 => "i32".into(),
        Type::I64 => "i64".into(),
        Type::U8 => "u8".into(),
        Type::U16 => "u16".into(),
        Type::U32 => "u32".into(),
        Type::U64 => "u64".into(),
        Type::F32 => "f32".into(),
        Type::F64 => "f64".into(),
        Type::Bool => "bool".into(),
        Type::Str => "str".into(),
        Type::Class(id) => format!("c{}", id),
        Type::Enum(id) => format!("e{}", id),
        Type::Iface(id) => format!("i{}", id),
        Type::Array(e) => format!("a{}", type_key(e)),
        Type::Ptr(e) => format!("p{}", type_key(e)),
        Type::Option(ok) => format!("o{}", type_key(ok)),
        Type::Result(ok, err) => format!("r{}_{}", type_key(ok), type_key(err)),
        _ => "x".into(),
    }
}

fn collect_special(t: &Type, out: &mut Vec<(String, Type)>) {
    match t {
        Type::Ptr(e) => collect_special(e, out),
        Type::Named(_) => {}
        Type::Result(ok, err) => {
            // 先收集载荷类型，保证被引用的 MylO_*/MylR_* 先于外层声明。
            collect_special(ok, out);
            collect_special(err, out);
            let name = format!("MylR_{}_{}", type_key(ok), type_key(err));
            if !out.iter().any(|(n, _)| *n == name) {
                out.push((name, Type::Result(ok.clone(), err.clone())));
            }
        }
        Type::Option(ok) => {
            collect_special(ok, out);
            let name = format!("MylO_{}", type_key(ok));
            if !out.iter().any(|(n, _)| *n == name) {
                out.push((name, Type::Option(ok.clone())));
            }
        }
        Type::Array(e) | Type::Ptr(e) => collect_special(e, out),
        _ => {}
    }
}

fn static_name(cid: usize, idx: usize) -> String {
    format!("S_{}_{}", cid, idx)
}

fn is_ref(t: &Type) -> bool {
    match t {
        Type::Str | Type::Class(_) | Type::Iface(_) | Type::Array(_) | Type::Enum(_) => true,
        Type::Option(e) => is_ref(e),
        Type::Result(a, b) => is_ref(a) || is_ref(b),
        _ => false,
    }
}

fn static_const(t: &Type, c: &MirConst) -> String {
    match c {
        MirConst::Int(v) => format!("(({}){})", cty(t).unwrap_or_else(|| "myrt_i64".into()), v),
        MirConst::IntU64(v) => format!("(({}){}ULL)", cty(t).unwrap_or_else(|| "myrt_u64".into()), v),
        MirConst::Float(v) => format!("(({}){})", cty(t).unwrap_or_else(|| "double".into()), v),
        MirConst::Str(v) => format!("mylrt_str_lit(\"{}\", {})", c_str(v), v.len()),
        MirConst::Bool(v) => if *v { "1".into() } else { "0".into() },
        MirConst::Null => "(void*)0".into(),
    }
}

fn zero_init(t: &str) -> &'static str {
    if t.starts_with("struct ") || t.starts_with("MylR_") || t.starts_with("MylO_") { "{0}" } else { "0" }
}

fn zero_expr(t: &str) -> String {
    if t.starts_with("struct ") || t.starts_with("MylR_") || t.starts_with("MylO_") {
        format!("({}){{0}}", t)
    } else {
        "0".to_string()
    }
}

fn clabel(label: &str) -> String {
    let mut s = String::from("L_");
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            s.push(ch);
        } else {
            s.push('_');
        }
    }
    s
}

fn cname(name: &str) -> String {
    if name.ends_with("::main") {
        return "main".to_string();
    }
    let mut s = String::from("m_");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            s.push(ch);
        } else {
            s.push('_');
        }
    }
    s
}

fn opt(o: MirOperand) -> String {
    match o {
        MirOperand::Value(v) => format!("v{}", v),
        MirOperand::Local(l) => format!("l{}", l),
    }
}

fn cbin(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::AndAnd => "&&",
        BinOp::OrOr => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
    }
}

fn cun(op: UnOp) -> Option<&'static str> {
    match op {
        UnOp::Neg => Some("-"),
        UnOp::Not => Some("!"),
        UnOp::BitNot => Some("~"),
        _ => None,
    }
}

fn cty_name(name: &str) -> Option<&'static str> {
    match name {
        "i8" => Some("myrt_i8"),
        "i16" => Some("myrt_i16"),
        "i32" => Some("myrt_i32"),
        "i64" => Some("myrt_i64"),
        "u8" => Some("myrt_u8"),
        "u16" => Some("myrt_u16"),
        "u32" => Some("myrt_u32"),
        "u64" => Some("myrt_u64"),
        "f32" => Some("float"),
        "f64" => Some("double"),
        "bool" => Some("myrt_bool"),
        "string" | "str" => Some("MyStr*"),
        n if n.ends_with('*') => Some("void*"),
        _ => None,
    }
}

fn arr_key(elem: &Type) -> String {
    match elem {
        Type::I8 => "i8".into(),
        Type::I16 => "i16".into(),
        Type::I32 => "i32".into(),
        Type::I64 => "i64".into(),
        Type::U8 => "u8".into(),
        Type::U16 => "u16".into(),
        Type::U32 => "u32".into(),
        Type::U64 => "u64".into(),
        Type::F32 => "f32".into(),
        Type::F64 => "f64".into(),
        Type::Bool => "bool".into(),
        Type::Str => "str".into(),
        Type::Class(id) => format!("c{}", id),
        Type::Enum(id) => format!("e{}", id),
        Type::Iface(id) => format!("i{}", id),
        Type::Array(e) => format!("a{}", arr_key(e)),
        Type::Option(e) => format!("o{}", arr_key(e)),
        Type::Result(a, b) => format!("r{}_{}", arr_key(a), arr_key(b)),
        Type::Ptr(_) => "ptr".into(),
        _ => "other".into(),
    }
}

fn arr_elem(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Array(e) => Some(e),
        _ => None,
    }
}

fn c_str(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn operand_ty(
    o: MirOperand,
    f: &MirFunction,
    vtys: &HashMap<u32, Type>,
) -> Type {
    match o {
        MirOperand::Local(l) => f.locals.get(l as usize).map(|x| x.ty.clone()).unwrap_or(Type::Infer),
        MirOperand::Value(v) => vtys.get(&v).cloned().unwrap_or(Type::Infer),
    }
}

fn iface_str_field(id: usize, p: &MirProgram) -> String {
    p.ifaces
        .iter()
        .find(|i| i.decl_id == id)
        .and_then(|i| i.methods.iter().find(|m| matches!(m.name.as_str(), "toString" | "to_string")))
        .map(|m| format!("fn_{}", m.name))
        .unwrap_or_else(|| "fn_to_string".to_string())
}

fn fmt_c_elem(ty: &Type, x: &str, p: &MirProgram) -> Option<String> {
    Some(match ty {
        Type::I8 | Type::I16 | Type::I32 | Type::I64 => format!("myrt_i64_to_string((myrt_i64)({}))", x),
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => format!("myrt_u64_to_string((myrt_u64)({}))", x),
        Type::F32 | Type::F64 => format!("myrt_f64_to_string((double)({}))", x),
        Type::Str => x.to_string(),
        Type::Bool => format!("(({}) ? mylrt_str_lit(\"true\", 4) : mylrt_str_lit(\"false\", 5))", x),
        Type::Iface(id) => {
            let field = iface_str_field(*id, p);
            format!(
                "((MyStr*)((struct I_{}){}).vt->{}(((struct I_{}){}).obj))",
                id, x, field, id, x
            )
        },
        Type::Class(id) | Type::Enum(id) => {
            let m = p.funcs.iter().find(|m| {
                m.owner == Some(*id)
                    && m.name.rsplit("::").next() == Some("to_string")
                    && !m.is_static
            })?;
            format!("{}({})", cname(&m.name), x)
        }
        _ => return None,
    })
}

fn fmt_value(
    o: MirOperand,
    ty: &Type,
    f: &MirFunction,
    vtys: &HashMap<u32, Type>,
    p: &MirProgram,
    errors: &mut Vec<String>,
) -> String {
    match ty {
        Type::I8 | Type::I16 | Type::I32 | Type::I64 => format!("myrt_i64_to_string((myrt_i64)({}))", opt(o)),
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => format!("myrt_u64_to_string((myrt_u64)({}))", opt(o)),
        Type::F32 | Type::F64 => format!("myrt_f64_to_string((double)({}))", opt(o)),
        Type::Str => opt(o),
        Type::Bool => format!("(({}) ? mylrt_str_lit(\"true\", 4) : mylrt_str_lit(\"false\", 5))", opt(o)),
        Type::Iface(id) => {
            let field = iface_str_field(*id, p);
            format!(
                "((MyStr*)((struct I_{}){}).vt->{}(((struct I_{}){}).obj))",
                id, opt(o), field, id, opt(o)
            )
        },
        Type::Class(id) | Type::Enum(id) => {
            let found = p.funcs.iter().find(|x| {
                x.owner == Some(*id)
                    && matches!(x.name.rsplit("::").next(), Some("toString") | Some("to_string"))
                    && !x.is_static
            });
            match found {
                Some(m) => format!("{}({})", cname(&m.name), opt(o)),
                None => {
                    errors.push(format!("{}: no to_string for {}", f.name, ty));
                    "mylrt_str_lit(\"\", 0)".into()
                }
            }
        }
        Type::Array(e) => format!("AS_{}((struct MylA_{}*){})", arr_key(e), arr_key(e), opt(o)),
        _ => {
            let _ = (f, vtys);
            "mylrt_str_lit(\"\", 0)".into()
        }
    }
}

fn emit(p: &MirProgram, want_main: bool, prog: &Program) -> String {
    let call_map: HashMap<&str, &str> = p
        .funcs
        .iter()
        .map(|f| (f.name.rsplit("::").next().unwrap_or(&f.name), f.name.as_str()))
        .collect();
    let mut out = String::new();
    out.push_str("/* generated by mylc --backend mir (MIR v0 subset) */\n");
    out.push_str("#include \"mylrt.h\"\n#include \"mylrt_gc.h\"\n#include <math.h>\n");
    let mut headers: Vec<String> = Vec::new();
    for d in &prog.decls {
        if let Some(h) = &d.c_header {
            if h == "mylrt.h" || h == "mylrt_gc.h" { continue; }
            if !headers.contains(h) {
                headers.push(h.clone());
                out.push_str(&format!("#include <{}>\n", h));
            }
        }
    }
    out.push('\n');

    let known: HashSet<&str> = p.funcs.iter().map(|f| f.name.as_str()).collect();
    let mut errors = Vec::new();

    let mut specials: Vec<(String, Type)> = Vec::new();
    for f in &p.funcs {
        collect_special(&f.ret, &mut specials);
        for l in &f.locals { collect_special(&l.ty, &mut specials); }
        for b in &f.blocks {
            for i in &b.insts { collect_special(&i.ty, &mut specials); }
        }
    }
    for c in &p.classes {
        for f in &c.fields { collect_special(&f.ty, &mut specials); }
    }
    for (name, ty) in &specials {
        match ty {
            Type::Ptr(_) | Type::Named(_) => {}
            Type::Result(ok, err) => {
                let oi = matches!(&**ok, Type::Iface(_));
                let ei = matches!(&**err, Type::Iface(_));
                let ot = cty(ok).unwrap_or_else(|| "myrt_i32".into());
                let et = cty(err).unwrap_or_else(|| "myrt_i32".into());
                match (oi, ei) {
                    (true, true) => out.push_str(&format!(
                        "typedef struct {{ myrt_bool is_ok; void* ok_obj; const void* ok_vt; void* err_obj; const void* err_vt; }} {name};\n"
                    )),
                    (true, false) => out.push_str(&format!(
                        "typedef struct {{ myrt_bool is_ok; void* ok_obj; const void* ok_vt; {et} err; }} {name};\n"
                    )),
                    (false, true) => out.push_str(&format!(
                        "typedef struct {{ myrt_bool is_ok; {ot} ok; void* err_obj; const void* err_vt; }} {name};\n"
                    )),
                    _ => out.push_str(&format!(
                        "typedef struct {{ myrt_bool is_ok; union {{ {ot} ok; {et} err; }} u; }} {name};\n"
                    )),
                }
            }
            Type::Option(ok) => match &**ok {
                Type::Iface(id) => {
                    out.push_str(&format!(
                        "typedef struct {{ myrt_bool is_some; void* obj; const void* vt; }} {name};\n"
                    ));
                    let _ = id;
                }
                _ => {
                    let ot = cty(ok).unwrap_or_else(|| "myrt_i32".into());
                    out.push_str(&format!("typedef struct {{ myrt_bool is_some; union {{ {ot} some; }} u; }} {name};\n"));
                }
            },
            _ => {}
        }
    }
    out.push('\n');

    let mut box_pairs: Vec<(Type, usize)> = Vec::new();
    for f in &p.funcs {
        for b in &f.blocks {
            for i in &b.insts {
                if let MirInstKind::Box { from, .. } = &i.kind {
                    if let Type::Iface(iid) = &i.ty {
                        if boxable(from) && !box_pairs.iter().any(|(t, x)| t == from && *x == *iid) {
                            box_pairs.push((from.clone(), *iid));
                        }
                    }
                }
            }
        }
    }
    for t in [Type::I8, Type::I16, Type::I32, Type::I64, Type::U8, Type::U16,
              Type::U32, Type::U64, Type::F32, Type::F64, Type::Bool, Type::Str] {
        let k = box_key(&t);
        let c = box_ctype(&t);
        out.push_str(&format!("struct MylBox_{} {{ {} v; }};\n", k, c));
        out.push_str(&format!(
            "static void* myl_box_{}({} v) {{ struct MylBox_{}* p = (struct MylBox_{}*)myrt_alloc(sizeof(struct MylBox_{})); p->v = v; return p; }}\n",
            k, c, k, k, k
        ));
        out.push_str(&format!(
            "static {} myl_unbox_{}(void* p) {{ return ((struct MylBox_{}*)p)->v; }}\n",
            c, k, k
        ));
    }
    out.push('\n');

    // 接口值结构可能作为 class 字段被内嵌，必须先于 class 定义。
    for i in &p.ifaces {
        out.push_str(&format!("struct I_{} {{ void* obj; const struct Vt_{}* vt; }};\n", i.decl_id, i.decl_id));
    }

    for c in &p.classes {
        let mut ok = true;
        for f in &c.fields {
            if f.is_static { continue; }
            if cty(&f.ty).is_none() {
                errors.push(format!("class {}: unsupported field {} type {}", c.name, f.name, f.ty));
                ok = false;
            }
        }
        if !ok { continue; }
        out.push_str(&format!("\nstruct C_{} {{\n", c.decl_id));
        let mut any = false;
        for (i, f) in c.fields.iter().enumerate() {
            if f.is_static { continue; }
            any = true;
            out.push_str(&format!("  {} f{}; /* {} */\n", cty(&f.ty).unwrap(), i, f.name));
        }
        if !any { out.push_str("  char _unused;\n"); }
        out.push_str("};\n");
    }
    out.push('\n');

    let has_statics = p.classes.iter().any(|c| c.fields.iter().any(|f| f.is_static));
    let static_root_n: usize = p.classes.iter()
        .flat_map(|c| c.fields.iter())
        .filter(|f| f.is_static && is_ref(&f.ty))
        .count();
    if static_root_n > 0 {
        out.push_str(&format!("static myrt_gc_root_desc myl_static_roots[{}];\n", static_root_n));
    }
    for c in &p.classes {
        for (idx, f) in c.fields.iter().enumerate() {
            if !f.is_static { continue; }
            let t = cty(&f.ty).unwrap_or_else(|| "myrt_i32".into());
            out.push_str(&format!("static {} {n};\n", t, n = static_name(c.decl_id, idx)));
        }
    }
    out.push('\n');

    for i in &p.ifaces {
        out.push_str(&format!("\nstruct Vt_{} {{\n  myrt_i32 type_id;\n", i.decl_id));
        for m in &i.methods {
            let ret = cty(&m.ret).unwrap_or_else(|| "void".into());
            let mut ps = vec!["void*".to_string()];
            for p in &m.params { ps.push(cty(p).unwrap_or_else(|| "myrt_i32".into())); }
            out.push_str(&format!("  {} (*fn_{})({});\n", ret, m.name, ps.join(", ")));
        }

        out.push_str("};\n");
    }
    for (t, iid) in &box_pairs {
        let k = box_key(t);
        out.push_str(&format!(
            "static const struct Vt_{} VT_BOX_{}_{} = {{ {} }};\n",
            iid, k, iid, box_id(t)
        ));
    }
    out.push('\n');

    let mut arrays: Vec<Type> = Vec::new();
    for f in &p.funcs {
        for l in &f.locals {
            if let Type::Array(e) = &l.ty {
                let el = (**e).clone();
                if !arrays.iter().any(|x| arr_key(x) == arr_key(&el)) {
                    arrays.push(el);
                }
            }
        }
        for b in &f.blocks {
            for i in &b.insts {
                if let Type::Array(e) = &i.ty {
                    let el = (**e).clone();
                    if !arrays.iter().any(|x| arr_key(x) == arr_key(&el)) {
                        arrays.push(el);
                    }
                }
            }
        }
    }
    for el in &arrays {
        match cty(el) {
            Some(t) => out.push_str(&format!("\nstruct MylA_{} {{ myrt_u64 len; {} items[]; }};\n", arr_key(el), t)),
            None => errors.push(format!("array element {} unsupported", el)),
        }
    }
    out.push('\n');

    for e in &p.enums {
        out.push_str(&format!("\nstruct E_{} {{ myrt_i32 tag; void* data; }};\n", e.decl_id));
        for (vi, v) in e.variants.iter().enumerate() {
            if !v.fields.is_empty() {
                out.push_str(&format!("struct P_{}_{} {{\n", e.decl_id, vi));
                for (fi, ft) in v.fields.iter().enumerate() {
                    match cty(ft) {
                        Some(t) => out.push_str(&format!("  {} f{};\n", t, fi)),
                        None => errors.push(format!("enum {} variant {} field {} unsupported", e.name, v.name, ft)),
                    }
                }
                out.push_str("};\n");
            }
            let args = v.fields.iter().enumerate().map(|(fi, ft)| format!("{} p{}", cty(ft).unwrap_or_else(|| "myrt_i32".into()), fi)).collect::<Vec<_>>().join(", ");
            out.push_str(&format!("static void* CT_{}_{}({}) {{\n", e.decl_id, vi, if args.is_empty() { "void".into() } else { args }));
            out.push_str(&format!("  struct E_{}* e = (struct E_{}*)myrt_alloc(sizeof(struct E_{}));\n", e.decl_id, e.decl_id, e.decl_id));
            if v.fields.is_empty() {
                out.push_str(&format!("  e->tag = {}; e->data = (void*)0; return (void*)e;\n", vi));
            } else {
                out.push_str(&format!("  struct P_{}_{}* p = (struct P_{}_{}*)myrt_alloc(sizeof(struct P_{}_{}));\n", e.decl_id, vi, e.decl_id, vi, e.decl_id, vi));
                for (fi, _) in v.fields.iter().enumerate() {
                    out.push_str(&format!("  p->f{} = p{};\n", fi, fi));
                }
                out.push_str(&format!("  e->tag = {}; e->data = (void*)p; return (void*)e;\n", vi));
            }
            out.push_str("}\n");
        }
    }
    out.push('\n');

    for f in &p.funcs {
        let ret = match cty(&f.ret) {
            Some(t) => t,
            None => {
                errors.push(format!("{}: unsupported return type {}", f.name, f.ret));
                continue;
            }
        };
        out.push_str(&format!("{} {}(", ret, cname(&f.name)));
        if f.params.is_empty() {
            out.push_str("void");
        } else {
            for (i, l) in f.params.iter().enumerate() {
                match cty(&l.ty) {
                    Some(t) => {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(&format!("{} p{}", t, i));
                    }
                    None => errors.push(format!("{}: unsupported param type {}", f.name, l.ty)),
                }
            }
        }
        out.push_str(");\n");
    }
    out.push('\n');

    for el in &arrays {
        if let Some(fmt) = fmt_c_elem(el, "a->items[i]", p) {
            out.push_str(&format!(
                "static MyStr* AS_{}(struct MylA_{}* a) {{ MyStr* s = mylrt_str_lit(\"[\", 1); for (myrt_u64 i = 0; i < a->len; i++) {{ if (i) s = mylrt_str_concat(s, mylrt_str_lit(\", \", 2)); s = mylrt_str_concat(s, {}); }} return mylrt_str_concat(s, mylrt_str_lit(\"]\", 1)); }}\n",
                arr_key(el), arr_key(el), fmt
            ));
        }
    }
    out.push('\n');

    if has_statics {
        out.push_str("static void myl_mir_init_statics(void);\n\n");
    }
    let mut casts: Vec<(usize, usize)> = Vec::new();
    for f in &p.funcs {
        let mut vty: HashMap<u32, Type> = HashMap::new();
        for b in &f.blocks {
            for inst in &b.insts {
                if let Some(v) = inst.dst {
                    if let MirInstKind::New { .. } = &inst.kind {
                        let cid = match &inst.res {
                            Res::Ctor(c) => *c,
                            Res::IfaceCast(c, _) => *c,
                            _ => 0,
                        };
                        vty.insert(v, Type::Class(cid));
                    } else {
                        vty.insert(v, inst.ty.clone());
                    }
                }
            }
        }
        for b in &f.blocks {
            for inst in &b.insts {
                if let Some((from, to)) = inst.coerce {
                    if !casts.iter().any(|(a, b)| *a == from && *b == to) {
                        casts.push((from, to));
                    }
                }
                if let MirInstKind::Call { args, .. } = &inst.kind {
                    if let Res::Fn(id) | Res::StaticMethod(_, id) = &inst.res {
                        if let Some(tf) = p.funcs.iter().find(|x| x.decl_id == *id) {
                            for (k, a) in args.iter().enumerate() {
                                let src = match a {
                                    MirOperand::Local(l) => f.locals.get(*l as usize).map(|x| x.ty.clone()),
                                    MirOperand::Value(v) => vty.get(v).cloned(),
                                };
                                if let (Some(Type::Class(cid)), Some(Type::Iface(iid))) =
                                    (src, tf.params.get(k).map(|x| x.ty.clone()))
                                {
                                    if !casts.iter().any(|(a, b)| *a == cid && *b == iid) {
                                        casts.push((cid, iid));
                                    }
                                }
                            }
                        }
                    }
                }
                if let MirInstKind::Store { local, value } = &inst.kind {
                    let target = f.locals.get(*local as usize).map(|l| l.ty.clone());
                    let src = match value {
                        MirOperand::Local(l) => f.locals.get(*l as usize).map(|x| x.ty.clone()),
                        MirOperand::Value(v) => vty.get(v).cloned(),
                    };
                    if let (Some(Type::Class(cid)), Some(Type::Iface(iid))) = (src, target) {
                        if !casts.iter().any(|(a, b)| *a == cid && *b == iid) {
                            casts.push((cid, iid));
                        }
                    }
                    if let (Some(Type::Enum(eid)), Some(Type::Iface(iid))) = (
                        match value { MirOperand::Local(l) => f.locals.get(*l as usize).map(|x| x.ty.clone()), MirOperand::Value(v) => vty.get(v).cloned() },
                        f.locals.get(*local as usize).map(|l| l.ty.clone()),
                    ) {
                        if !casts.iter().any(|(a, b)| *a == eid && *b == iid) {
                            casts.push((eid, iid));
                        }
                    }
                }
            }
        }
    }
    let mut vt_pairs: Vec<(usize, usize)> = Vec::new();
    let conc: Vec<usize> = p
        .classes
        .iter()
        .map(|c| c.decl_id)
        .chain(p.enums.iter().map(|e| e.decl_id))
        .collect();
    for iface in &p.ifaces {
        for cid in &conc {
            let ok = iface.methods.iter().all(|m| {
                p.funcs.iter().any(|f| {
                    let leaf = f.name.rsplit("::").next();
                    f.owner == Some(*cid)
                        && (leaf == Some(m.name.as_str())
                            || (m.name == "toString" && leaf == Some("to_string")))
                        && !f.is_static
                        && iface_method_matches(f, m)
                })
            });
            if ok {
                vt_pairs.push((*cid, iface.decl_id));
            }
        }
    }
    for (cid, iid) in &vt_pairs {
        let iface = match p.ifaces.iter().find(|i| i.decl_id == *iid) {
            Some(i) => i,
            None => continue,
        };
        let mut entries = Vec::new();
        for m in &iface.methods {
            let f = p.funcs.iter().find(|f| {
                let leaf = f.name.rsplit("::").next();
                f.owner == Some(*cid)
                    && (leaf == Some(m.name.as_str())
                        || (m.name == "toString" && leaf == Some("to_string")))
                    && !f.is_static
            }).unwrap();
            entries.push(cname(&f.name));
        }
        out.push_str(&format!(
            "\nstatic const struct Vt_{} VT_{}_{} = {{ {}, {} }};\n",
            iid, cid, iid, cid, entries.join(", ")
        ));
    }
    // 接口到接口：按具体实现选择目标 vtable，避免 vt 前缀强转。
    for from_i in &p.ifaces {
        for to_i in &p.ifaces {
            if from_i.decl_id == to_i.decl_id { continue; }
            let possible: Vec<&(usize, usize)> = vt_pairs
                .iter()
                .filter(|(_, iid)| *iid == from_i.decl_id)
                .filter(|(cid, _)| vt_pairs.iter().any(|(c, i)| c == cid && *i == to_i.decl_id))
                .collect();
            if possible.is_empty() { continue; }
            out.push_str(&format!(
                "\nstatic struct I_{} A_{}_{}(struct I_{} v) {{\n",
                to_i.decl_id, from_i.decl_id, to_i.decl_id, from_i.decl_id
            ));
            for (cid, _) in possible {
                out.push_str(&format!(
                    "  if (v.vt == &VT_{}_{}) return (struct I_{}){{ v.obj, &VT_{}_{} }};\n",
                    cid, from_i.decl_id, to_i.decl_id, cid, to_i.decl_id
                ));
            }
            out.push_str(&format!("  return (struct I_{}){{ v.obj, 0 }};\n}}\n", to_i.decl_id));
        }
    }

    for f in &p.funcs {
        if !want_main && f.name.ends_with("::main") {
            continue;
        }
        emit_fn(&mut out, f, p, &known, &call_map, has_statics, &mut errors);
    }

    if has_statics {
        out.push_str("\nstatic void myl_mir_init_statics(void) {\n");
        let mut sri = 0usize;
        for c in &p.classes {
            for (idx, f) in c.fields.iter().enumerate() {
                if !f.is_static { continue; }
                let n = static_name(c.decl_id, idx);
                if is_ref(&f.ty) {
                    out.push_str(&format!("  myl_static_roots[{}].start = (void*)&{};\n", sri, n));
                    out.push_str(&format!("  myl_static_roots[{}].bytes = sizeof({});\n", sri, n));
                    sri += 1;
                }
                if let Some(init) = &f.init {
                    let e = static_const(&f.ty, init);
                    out.push_str(&format!("  {} = {};\n", n, e));
                    if is_ref(&f.ty) {
                        out.push_str(&format!("  myrt_gc_write_barrier_owner((void*)&{}, (void*)({}));\n", n, n));
                    }
                }
            }
        }
        if sri > 0 {
            out.push_str(&format!("  myrt_gc_static_set_roots(myl_static_roots, {});\n", sri));
        }
        out.push_str("}\n");
    }

    if !errors.is_empty() {
        out.push_str("\n/* MIR-C unsupported:\n");
        for e in errors {
            out.push_str(&format!(" * {}\n", e));
        }
        out.push_str("*/\n#error \"MIR-C backend does not support this program yet\"\n");
    }
    out
}

fn emit_fn(
    out: &mut String,
    f: &MirFunction,
    p: &MirProgram,
    known: &HashSet<&str>,
    call_map: &HashMap<&str, &str>,
    has_statics: bool,
    errors: &mut Vec<String>,
) {
    let ret = match cty(&f.ret) {
        Some(t) => t,
        None => return,
    };
    out.push_str(&format!("\n{} {}(", ret, cname(&f.name)));
    if f.params.is_empty() {
        out.push_str("void");
    } else {
        for (i, l) in f.params.iter().enumerate() {
            let t = cty(&l.ty).unwrap_or_else(|| "myrt_i32".into());
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{} p{}", t, i));
        }
    }
    out.push_str(") {\n");
    if f.name.ends_with("::main") {
        out.push_str("  myrt_init();\n");
    }
    if has_statics && f.name.ends_with("::main") {
        out.push_str("  myl_mir_init_statics();\n");
    }

    // 局部槽
    for (i, l) in f.locals.iter().enumerate() {
        match cty(&l.ty) {
            Some(t) => out.push_str(&format!("  {} l{} = {};\n", t, i, zero_init(&t)) ),
            None => {
                errors.push(format!("{}: unsupported local {} type {}", f.name, l.name, l.ty));
                out.push_str(&format!("  myrt_i32 l{} = 0;\n", i));
            }
        }
    }
    for (i, _) in f.params.iter().enumerate() {
        out.push_str(&format!("  l{} = p{};\n", i, i));
    }

    // 临时值
    let mut vtys = HashMap::new();
    let mut vtypes: HashMap<u32, Type> = HashMap::new();
    let mut wrapped: HashSet<u32> = HashSet::new();
    for b in &f.blocks {
        for i in &b.insts {
            if let Some(v) = i.dst {
                if let MirInstKind::New { .. } = &i.kind {
                    let cid = match &i.res {
                        Res::Ctor(c) => *c,
                        Res::IfaceCast(c, _) => *c,
                        _ => 0,
                    };
                    vtys.insert(v, "void*".to_string());
                    vtypes.insert(v, Type::Class(cid));
                    continue;
                }
                let decl_ty = match i.coerce {
                    Some((_, to)) => Type::Iface(to),
                    None => i.ty.clone(),
                };
                let t = cty(&decl_ty).unwrap_or_else(|| "myrt_i32".into());
                vtys.insert(v, t);
                vtypes.insert(v, decl_ty);
                if i.coerce.is_some() { wrapped.insert(v); }
            }
        }
    }
    for (v, t) in &vtys {
        out.push_str(&format!("  {} v{} = {};\n", t, v, zero_init(t)));
    }
    out.push('\n');

    // 帧根区间：只注册 safepoint 根标注的并集（路径敏感活跃性分析结果）。
    let mut live_locals: HashSet<u32> = HashSet::new();
    let mut live_values: HashSet<u32> = HashSet::new();
    for b in &f.blocks {
        for ins in &b.insts {
            if let MirInstKind::Safepoint { roots } = &ins.kind {
                for r in roots {
                    match r {
                        MirOperand::Local(l) => { live_locals.insert(*l); }
                        MirOperand::Value(v) => { live_values.insert(*v); }
                    }
                }
            }
        }
    }
    let mut roots: Vec<String> = Vec::new();
    let mut root_local_idx: HashMap<u32, usize> = HashMap::new();
    let mut root_value_idx: HashMap<u32, usize> = HashMap::new();
    for (i, l) in f.locals.iter().enumerate() {
        if live_locals.contains(&(i as u32)) && is_ref(&l.ty) {
            root_local_idx.insert(i as u32, roots.len());
            roots.push(format!("l{}", i));
        }
    }
    for (v, ty) in &vtypes {
        if live_values.contains(v) && is_ref(ty) {
            root_value_idx.insert(*v, roots.len());
            roots.push(format!("v{}", v));
        }
    }
    let has_frame = !roots.is_empty();
    if has_frame {
        out.push_str(&format!("  myrt_gc_frame_id myl_fid = myrt_gc_frame_enter();\n"));
        out.push_str(&format!("  myrt_gc_root_desc myl_roots[{}];\n", roots.len()));
    }
    let root_cleanup = if has_frame {
        "  myrt_gc_frame_exit(myl_fid);\n".to_string()
    } else { String::new() };

    let mut targets: HashSet<&str> = HashSet::new();
    for b in &f.blocks {
        if let Some(t) = &b.term {
            match t {
                MirTerm::Goto(t) => { targets.insert(t.as_str()); }
                MirTerm::Branch { then_label, else_label, .. } => {
                    targets.insert(then_label.as_str());
                    targets.insert(else_label.as_str());
                }
                _ => {}
            }
        }
    }
    for b in &f.blocks {
        if targets.contains(b.label.as_str()) {
            out.push_str(&format!("{}: ;\n", clabel(&b.label)));
        }
        for i in &b.insts {
            if let MirInstKind::Safepoint { roots } = &i.kind {
                if has_frame {
                    for r in roots {
                        let (idx, slot) = match r {
                            MirOperand::Local(l) => match root_local_idx.get(l) {
                                Some(idx) => (*idx, format!("l{}", l)),
                                None => {
                                    errors.push(format!("safepoint 根 $%{} 未在 MIR-C 根表中", l));
                                    continue;
                                }
                            },
                            MirOperand::Value(v) => match root_value_idx.get(v) {
                                Some(idx) => (*idx, format!("v{}", v)),
                                None => {
                                    errors.push(format!("safepoint 根 %{} 未在 MIR-C 根表中", v));
                                    continue;
                                }
                            },
                        };
                        out.push_str(&format!("  myl_roots[{}].start = (void*)&{};\n", idx, slot));
                        out.push_str(&format!("  myl_roots[{}].bytes = sizeof({});\n", idx, slot));
                    }
                    out.push_str(&format!("  myrt_gc_frame_set_roots(myl_fid, myl_roots, {});\n", roots.len()));
                }
                out.push_str("  myrt_gc_safepoint();\n");
                continue;
            }
            emit_inst(out, f, i, p, known, call_map, &vtypes, &wrapped, errors);
        }
        match &b.term {
            Some(MirTerm::Return(Some(v))) => {
                let src = operand_ty(*v, f, &vtypes);
                let e = match (&f.ret, &src) {
                    (Type::Iface(to), Type::Class(cid)) => iface_wrap(*cid, *to, opt(*v)),
                    (Type::Iface(to), Type::Enum(eid)) => iface_wrap(*eid, *to, opt(*v)),
                    _ => opt(*v),
                };
                out.push_str(&root_cleanup);
                out.push_str(&format!("  return {};\n", e));
            }
            Some(MirTerm::Return(None)) => {
                out.push_str(&root_cleanup);
                out.push_str("  return;\n");
            }
            Some(MirTerm::Goto(l)) => out.push_str(&format!("  goto {};\n", clabel(l))),
            Some(MirTerm::Branch { cond, then_label, else_label }) => {
                out.push_str(&format!(
                    "  if ({}) goto {}; else goto {};\n",
                    opt(*cond),
                    clabel(then_label),
                    clabel(else_label)
                ))
            }
            Some(MirTerm::Unreachable) | None => out.push_str("  /* unreachable */\n"),
        }
    }
    out.push_str("}\n");
}

fn coerce_arg(
    p: &MirProgram,
    f: &MirFunction,
    op: MirOperand,
    param: &Type,
    vtys: &HashMap<u32, Type>,
    wrapped: &HashSet<u32>,
) -> String {
    let src = operand_ty(op, f, vtys);
    let already = match op {
        MirOperand::Value(v) => wrapped.contains(&v),
        _ => false,
    };
    match (param, &src) {
        (Type::Iface(to), Type::Class(cid)) if !already => iface_wrap(*cid, *to, opt(op)),
        (Type::Iface(to), Type::Enum(eid)) if !already => iface_wrap(*eid, *to, opt(op)),
        (Type::Iface(to), Type::Iface(from)) if from != to => {
            iface_adapt(*from, *to, opt(op))
        },
        _ => opt(op),
    }
}

fn emit_inst(
    out: &mut String,
    f: &MirFunction,
    i: &crate::mir::MirInst,
    p: &MirProgram,
    known: &HashSet<&str>,
    call_map: &HashMap<&str, &str>,
    vtys: &HashMap<u32, Type>,
    wrapped: &HashSet<u32>,
    errors: &mut Vec<String>,
) {
    let dst = i.dst.map(|v| format!("v{} = ", v)).unwrap_or_default();
    match &i.kind {
        MirInstKind::Const(MirConst::Int(v)) => out.push_str(&format!("  {}{};\n", dst, v)),
        MirInstKind::Const(MirConst::IntU64(v)) => out.push_str(&format!("  {}{}ULL;\n", dst, v)),
        MirInstKind::Const(MirConst::Bool(v)) => out.push_str(&format!("  {}{};\n", dst, if *v { 1 } else { 0 })),
        MirInstKind::Const(MirConst::Float(v)) => out.push_str(&format!("  {}{};\n", dst, v)),
        MirInstKind::Const(MirConst::Str(v)) => {
            out.push_str(&format!("  {}mylrt_str_lit(\"{}\", {});\n", dst, c_str(v), v.len()));
        }
        MirInstKind::Const(MirConst::Null) => {
            errors.push(format!("{}: unsupported null constant", f.name));
        }
        MirInstKind::Undef => out.push_str(&format!("  {}0;\n", dst)),
        MirInstKind::Copy(v) => out.push_str(&format!("  {}{};\n", dst, opt(*v))),
        MirInstKind::Store { local, value } => {
            let target_ty = f.locals.get(*local as usize).map(|l| l.ty.clone());
            let src_ty = operand_ty(*value, f, vtys);
            let already = match value {
                MirOperand::Value(v) => wrapped.contains(v),
                _ => false,
            };
            let e = match (&src_ty, &target_ty) {
                (Type::Class(cid), Some(Type::Iface(iid))) if !already => {
                    iface_wrap(*cid, *iid, opt(*value))
                }
                (Type::Enum(eid), Some(Type::Iface(iid))) if !already => {
                    iface_wrap(*eid, *iid, opt(*value))
                }
                (Type::Iface(from), Some(Type::Iface(to))) if from != to => {
                    iface_adapt(*from, *to, opt(*value))
                }
                _ => opt(*value),
            };
            out.push_str(&format!("  l{} = {};\n", local, e))
        }
        MirInstKind::Bin(op, a, b) => {
            let ta = operand_ty(*a, f, vtys);
            let tb = operand_ty(*b, f, vtys);
            if matches!(ta, Type::Str) || matches!(tb, Type::Str) {
                match op {
                    BinOp::Eq => out.push_str(&format!("  {}mylrt_str_eq({}, {});\n", dst, opt(*a), opt(*b))),
                    BinOp::Ne => out.push_str(&format!("  {}!mylrt_str_eq({}, {});\n", dst, opt(*a), opt(*b))),
                    BinOp::Add => out.push_str(&format!("  {}mylrt_str_concat({}, {});\n", dst, opt(*a), opt(*b))),
                    _ => errors.push(format!("{}: unsupported string operator {:?}", f.name, op)),
                }
            } else {
                out.push_str(&format!("  {}{} {} {};\n", dst, opt(*a), cbin(*op), opt(*b)))
            }
        }
        MirInstKind::Un(UnOp::Deref, a) => {
            let t = cty(&i.ty).unwrap_or_else(|| "myrt_i64".into());
            out.push_str(&format!("  {}*(({}*)({}));\n", dst, t, opt(*a)));
        }
        MirInstKind::Un(op, a) => match cun(*op) {
            Some(o) => out.push_str(&format!("  {}{}{};\n", dst, o, opt(*a))),
            None => errors.push(format!("{}: unsupported unary {:?}", f.name, op)),
        },
        MirInstKind::Call { callee, args } => {
            let leaf = callee.rsplit("::").next().unwrap_or(callee.as_str());
            if leaf == "Ok" || leaf == "Err" || leaf == "Some" || leaf == "None" {
                let name = cty(&i.ty).unwrap_or_else(|| "void*".into());
                let a = args.first().map(|x| opt(*x)).unwrap_or_default();
                let oiid = match &i.ty {
                    Type::Option(t) => match &**t { Type::Iface(id) => Some(*id), _ => None },
                    _ => None,
                };
                let (res_ok_iface, res_err_iface) = match &i.ty {
                    Type::Result(ok, err) => (
                        match &**ok { Type::Iface(id) => Some(*id), _ => None },
                        match &**err { Type::Iface(id) => Some(*id), _ => None },
                    ),
                    _ => (None, None),
                };
                let arg_ty = args.first().map(|x| operand_ty(*x, f, vtys));
                let iface_arg = |a: String, id: usize| match arg_ty {
                    Some(Type::Class(cid)) => iface_wrap(cid, id, a),
                    Some(Type::Enum(eid)) => iface_wrap(eid, id, a),
                    _ => a,
                };
                let e = match leaf {
                    "Ok" if res_ok_iface.is_some() => {
                        let av = iface_arg(a, res_ok_iface.unwrap());
                        format!("(({}){{ .is_ok = 1, .ok_obj = ({}).obj, .ok_vt = ({}).vt }})", name, av, av)
                    }
                    "Err" if res_err_iface.is_some() => {
                        let av = iface_arg(a, res_err_iface.unwrap());
                        format!("(({}){{ .is_ok = 0, .err_obj = ({}).obj, .err_vt = ({}).vt }})", name, av, av)
                    }
                    "Ok" if res_err_iface.is_some() => format!("(({}){{ .is_ok = 1, .ok = {} }})", name, a),
                    "Err" if res_ok_iface.is_some() => format!("(({}){{ .is_ok = 0, .err = {} }})", name, a),
                    "Ok" => format!("(({}){{ .is_ok = 1, .u = {{ .ok = {} }} }})", name, a),
                    "Err" => format!("(({}){{ .is_ok = 0, .u = {{ .err = {} }} }})", name, a),
                    "Some" if oiid.is_some() => {
                        let id = oiid.unwrap();
                        let av = iface_arg(a, id);
                        format!("(({}){{ .is_some = 1, .obj = ({}).obj, .vt = ({}).vt }})", name, av, av)
                    }
                    "Some" => format!("(({}){{ .is_some = 1, .u = {{ .some = {} }} }})", name, a),
                    _ => format!("(({}){{ .is_some = 0 }})", name),
                };
                out.push_str(&format!("  {}{};\n", dst, e));
                return;
            }
            if let Res::Variant(eid, vi) = &i.res {
                let a = args.iter().map(|x| opt(*x)).collect::<Vec<_>>().join(", ");
                let e = wrap_coerce(i, format!("(void*)CT_{}_{}({})", eid, vi, a));
                out.push_str(&format!("  {}{};\n", dst, e));
                return;
            }
            if let Res::StaticMethod(_, id) | Res::Fn(id) = &i.res {
                if let Some(m) = p.funcs.iter().find(|x| x.decl_id == *id) {
                    let a = args.iter().enumerate().map(|(k, x)| {
                        match m.params.get(k) {
                            Some(pt) => coerce_arg(p, f, *x, &pt.ty, vtys, wrapped),
                            None => opt(*x),
                        }
                    }).collect::<Vec<_>>().join(", ");
                    let call = format!("{}({})", cname(&m.name), a);
                    let e = wrap_coerce(i, call);
                    out.push_str(&format!("  {}{};\n", dst, e));
                    return;
                }
            }
            if let Res::Builtin(b) = &i.res {
                match b {
                    Builtin::StrLen => {
                        out.push_str(&format!("  {}myrt_str_len({});\n", dst, opt(args[0])));
                        return;
                    }
                    Builtin::ArrLen => {
                        out.push_str(&format!("  {}*(myrt_u64*)({});\n", dst, opt(args[0])));
                        return;
                    }
                    _ => {
                        errors.push(format!("{}: unsupported builtin {:?}", f.name, b));
                        return;
                    }
                }
            }
            let target = if known.contains(callee.as_str()) {
                cname(callee)
            } else if let Some(full) = call_map.get(callee.as_str()) {
                cname(full)
            } else if callee.starts_with("myrt_") {
                callee.clone()
            } else {
                callee.rsplit("::").next().unwrap_or(callee.as_str()).to_string()
            };
            let tf = p.funcs.iter().find(|x| {
                cname(&x.name) == target
                    || known.contains(x.name.as_str()) && cname(&x.name) == target
            });
            let a = args.iter().enumerate().map(|(k, x)| {
                match tf.and_then(|f| f.params.get(k)) {
                    Some(pt) => coerce_arg(p, f, *x, &pt.ty, vtys, wrapped),
                    None => opt(*x),
                }
            }).collect::<Vec<_>>().join(", ");
            let e = wrap_coerce(i, format!("{}({})", target, a));
            out.push_str(&format!("  {}{};\n", dst, e));
        }
        MirInstKind::Cast { value, target, checked } => {
            let d = match i.dst {
                Some(v) => format!("v{}", v),
                None => {
                    errors.push(format!("{}: cast without destination", f.name));
                    return;
                }
            };
            let src = operand_ty(*value, f, vtys);
            let ve = opt(*value);
            let panic = "myrt_panic(mylrt_str_lit(\"invalid cast\", 12))";
            match (&src, target) {
                (Type::Iface(_), Type::Class(cid)) => {
                    if *checked {
                        out.push_str(&format!("  {}.is_some = 0;\n", d));
                        out.push_str(&format!(
                            "  if (({}).vt && ({}).vt->type_id == {}) {{ {}.is_some = 1; {}.u.some = (void*)({}).obj; }}\n",
                            ve, ve, cid, d, d, ve
                        ));
                    } else {
                        out.push_str(&format!(
                            "  if (!(({}).vt && ({}).vt->type_id == {})) {};\n",
                            ve, ve, cid, panic
                        ));
                        out.push_str(&format!("  {} = (void*)({}).obj;\n", d, ve));
                    }
                }
                (Type::Class(cid), Type::Iface(to)) => {
                    out.push_str(&format!("  {} = {};\n", d, iface_wrap(*cid, *to, ve)));
                }
                (Type::Enum(eid), Type::Iface(to)) => {
                    out.push_str(&format!("  {} = {};\n", d, iface_wrap(*eid, *to, ve)));
                }
                (Type::Iface(from), Type::Iface(to)) => {
                    if from == to {
                        out.push_str(&format!("  {} = {};\n", d, ve));
                    } else if *checked {
                        out.push_str(&format!("  {}.is_some = 0;\n", d));
                        out.push_str(&format!(
                            "  {{ struct I_{} tmp = A_{}_{}({}); if (tmp.vt) {{ {}.is_some = 1; {}.obj = tmp.obj; {}.vt = tmp.vt; }} }}\n",
                            to, from, to, ve, d, d, d
                        ));
                    } else {
                        out.push_str(&format!(
                            "  {{ struct I_{} tmp = A_{}_{}({}); if (!tmp.vt) {}; {} = tmp; }}\n",
                            to, from, to, ve, panic, d
                        ));
                    }
                }
                (Type::Iface(_), t) if boxable(t) => {
                    let k = box_key(t);
                    if *checked {
                        out.push_str(&format!("  {}.is_some = 0;\n", d));
                        out.push_str(&format!(
                            "  if (({}).vt && ({}).vt->type_id == {}) {{ {}.is_some = 1; {}.u.some = myl_unbox_{}(({}).obj); }}\n",
                            ve, ve, box_id(t), d, d, k, ve
                        ));
                    } else {
                        out.push_str(&format!(
                            "  if (!(({}).vt && ({}).vt->type_id == {})) {};\n",
                            ve, ve, box_id(t), panic
                        ));
                        out.push_str(&format!("  {} = myl_unbox_{}(({}).obj);\n", d, k, ve));
                    }
                }
                (Type::Ptr(_), Type::Ptr(_)) => {
                    let t = cty(target).unwrap_or_else(|| "void*".into());
                    out.push_str(&format!("  {} = ({}){};\n", d, t, ve));
                }
                (a, b) if a.is_numeric() && b.is_numeric() => {
                    let t = cty(target).unwrap_or_else(|| "myrt_i64".into());
                    out.push_str(&format!("  {} = ({}){};\n", d, t, ve));
                }
                _ => errors.push(format!("{}: unsupported cast {} -> {}", f.name, src, target)),
            }
        }
        MirInstKind::Box { value, from } => {
            let iid = match &i.ty {
                Type::Iface(id) => *id,
                _ => {
                    errors.push(format!("{}: box to non-interface {}", f.name, i.ty));
                    return;
                }
            };
            let k = box_key(from);
            out.push_str(&format!(
                "  {}(struct I_{}){{ .obj = myl_box_{}({}), .vt = &VT_BOX_{}_{} }};\n",
                dst, iid, k, opt(*value), k, iid
            ));
        }
        MirInstKind::Field { base, name } => {
            let (cid, idx) = match &i.res {
                Res::Field(c, f) => (*c, *f),
                _ => {
                    errors.push(format!("{}: field {} has no Res::Field", f.name, name));
                    return;
                }
            };
            out.push_str(&format!("  {}((struct C_{}*){})->f{};\n", dst, cid, opt(*base), idx));
        }
        MirInstKind::StoreField { base, name, value } => {
            let (cid, idx) = match &i.res {
                Res::Field(c, f) => (*c, *f),
                _ => {
                    errors.push(format!("{}: field {} has no Res::Field", f.name, name));
                    return;
                }
            };
            out.push_str(&format!("  ((struct C_{}*){})->f{} = {};\n", cid, opt(*base), idx, opt(*value)));
        }
        MirInstKind::New { class, args } => {
            let cid = match &i.res {
                Res::Ctor(c) => {
                    if p.classes.iter().any(|x| x.decl_id == *c) {
                        *c
                    } else if let Some(owner) = p.funcs.iter().find(|f| f.decl_id == *c).and_then(|f| f.owner) {
                        owner
                    } else if let Some(owner) = p.funcs.iter()
                        .find(|f| f.name.contains(&format!("::{}", class)) && f.owner.is_some())
                        .and_then(|f| f.owner) {
                        owner
                    } else {
                        p.classes.iter()
                            .find(|x| x.name.ends_with(&format!("::{}", class)))
                            .map(|x| x.decl_id)
                            .unwrap_or(0)
                    }
                }
                _ => p.classes.iter().find(|c| c.name.ends_with(&format!("::{}", class))).map(|c| c.decl_id).unwrap_or(0),
            };
            let tmp = dst.trim_end_matches(" = ").to_string();
            if tmp.is_empty() {
                errors.push(format!("{}: new without destination", f.name));
                return;
            }
            out.push_str(&format!("  {} = (void*)myrt_alloc(sizeof(struct C_{}));\n", tmp, cid));
            if let Some(c) = p.classes.iter().find(|c| c.decl_id == cid) {
                for (fi, fld) in c.fields.iter().enumerate() {
                    if fld.is_static { continue; }
                    if let Some(init) = &fld.init {
                        let e = static_const(&fld.ty, init);
                        out.push_str(&format!("  ((struct C_{}*){})->f{} = {};\n", cid, tmp, fi, e));
                        if is_ref(&fld.ty) {
                            out.push_str(&format!("  myrt_gc_write_barrier_owner((void*)&((struct C_{}*){})->f{}, (void*)(((struct C_{}*){})->f{}));\n", cid, tmp, fi, cid, tmp, fi));
                        }
                    }
                }
            }
            if let Some(ctor) = p.funcs.iter().find(|x| x.owner == Some(cid) && x.is_ctor) {
                let rest = args
                    .iter()
                    .enumerate()
                    .map(|(k, x)| match ctor.params.get(k + 1) {
                        Some(pt) => coerce_arg(p, f, *x, &pt.ty, vtys, wrapped),
                        None => opt(*x),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let sep = if rest.is_empty() { "" } else { ", " };
                out.push_str(&format!("  {}({}{}{});\n", cname(&ctor.name), tmp, sep, rest));
            }
        }
        MirInstKind::MethodCall { recv, name, args } => {
            if let Res::StaticMethod(_, id) | Res::Fn(id) = &i.res {
                if let Some(m) = p.funcs.iter().find(|x| x.decl_id == *id) {
                    let a = args.iter().enumerate().map(|(k, x)| {
                        match m.params.get(k) {
                            Some(pt) => coerce_arg(p, f, *x, &pt.ty, vtys, wrapped),
                            None => opt(*x),
                        }
                    }).collect::<Vec<_>>().join(", ");
                    out.push_str(&format!("  {}{}({});\n", dst, cname(&m.name), a));
                    return;
                }
            }
            if let Res::Builtin(b) = &i.res {
                match b {
                    Builtin::ArrLen => {
                        out.push_str(&format!("  {}*(myrt_u64*)({});\n", dst, opt(*recv)));
                        return;
                    }
                    Builtin::StrLen => {
                        out.push_str(&format!("  {}myrt_str_len({});\n", dst, opt(*recv)));
                        return;
                    }
                    Builtin::ResIsOk => {
                        out.push_str(&format!("  {}({}).is_ok;\n", dst, opt(*recv)));
                        return;
                    }
                    Builtin::ResIsErr => {
                        out.push_str(&format!("  {}!({}).is_ok;\n", dst, opt(*recv)));
                        return;
                    }
                    Builtin::ResUnwrap => {
                        let r = opt(*recv);
                        let (ok_iface, err_iface) = result_flags(&operand_ty(*recv, f, vtys));
                        if let Type::Iface(id) = &i.ty {
                            out.push_str(&format!(
                                "  {}(({}).is_ok ? ((struct I_{}){{ .obj = ({}).ok_obj, .vt = (const struct Vt_{}*)({}).ok_vt }}) : (mylrt_panic_unwrap(), (struct I_{}){{0}}));\n",
                                dst, r, id, r, id, r, id
                            ));
                        } else {
                            let rt = cty(&i.ty).unwrap_or_else(|| "myrt_i32".into());
                            let field = if err_iface { "ok" } else { "u.ok" };
                            let _ = ok_iface;
                            out.push_str(&format!(
                                "  {}(({}).is_ok ? ({}).{} : (mylrt_panic_unwrap(), {}));\n",
                                dst, r, r, field, zero_expr(&rt)
                            ));
                        }
                        return;
                    }
                    Builtin::ResUnwrapErr => {
                        let r = opt(*recv);
                        let (ok_iface, err_iface) = result_flags(&operand_ty(*recv, f, vtys));
                        if let Type::Iface(id) = &i.ty {
                            out.push_str(&format!(
                                "  {}(({}).is_ok ? ((mylrt_panic_unwrap()), (struct I_{}){{0}}) : ((struct I_{}){{ .obj = ({}).err_obj, .vt = (const struct Vt_{}*)({}).err_vt }}));\n",
                                dst, r, id, id, r, id, r
                            ));
                        } else {
                            let rt = cty(&i.ty).unwrap_or_else(|| "MyStr*".into());
                            let field = if ok_iface { "err" } else { "u.err" };
                            let _ = err_iface;
                            out.push_str(&format!(
                                "  {}(({}).is_ok ? ((mylrt_panic_unwrap()), {}) : ({}).{});\n",
                                dst, r, zero_expr(&rt), r, field
                            ));
                        }
                        return;
                    }
                    Builtin::ResUnwrapOr => {
                        let r = opt(*recv);
                        let (ok_iface, err_iface) = result_flags(&operand_ty(*recv, f, vtys));
                        if let Type::Iface(id) = &i.ty {
                            out.push_str(&format!(
                                "  {}(({}).is_ok ? ((struct I_{}){{ .obj = ({}).ok_obj, .vt = (const struct Vt_{}*)({}).ok_vt }}) : {});\n",
                                dst, r, id, r, id, r, opt(args[0])
                            ));
                        } else {
                            let field = if err_iface { "ok" } else { "u.ok" };
                            let _ = ok_iface;
                            out.push_str(&format!(
                                "  {}(({}).is_ok ? ({}).{} : {});\n",
                                dst, r, r, field, opt(args[0])
                            ));
                        }
                        return;
                    }
                    Builtin::OptIsSome => {
                        out.push_str(&format!("  {}({}).is_some;\n", dst, opt(*recv)));
                        return;
                    }
                    Builtin::OptIsNone => {
                        out.push_str(&format!("  {}!({}).is_some;\n", dst, opt(*recv)));
                        return;
                    }
                    Builtin::OptUnwrap => {
                        let r = opt(*recv);
                        if let Type::Iface(id) = &i.ty {
                            out.push_str(&format!(
                                "  {}(({}).is_some ? ((struct I_{}){{ .obj = ({}).obj, .vt = (const struct Vt_{}*)({}).vt }}) : (mylrt_panic_unwrap(), (struct I_{}){{0}}));\n",
                                dst, r, id, r, id, r, id
                            ));
                        } else {
                            let rt = cty(&i.ty).unwrap_or_else(|| "myrt_i32".into());
                            out.push_str(&format!(
                                "  {}(({}).is_some ? ({}).u.some : (mylrt_panic_unwrap(), {}));\n",
                                dst, r, r, zero_expr(&rt)
                            ));
                        }
                        return;
                    }
                    Builtin::OptUnwrapOr => {
                        let r = opt(*recv);
                        if let Type::Iface(id) = &i.ty {
                            out.push_str(&format!(
                                "  {}(({}).is_some ? ((struct I_{}){{ .obj = ({}).obj, .vt = (const struct Vt_{}*)({}).vt }}) : ({}));\n",
                                dst, r, id, r, id, r, opt(args[0])
                            ));
                        } else {
                            out.push_str(&format!(
                                "  {}(({}).is_some ? ({}).u.some : {});\n",
                                dst, r, r, opt(args[0])
                            ));
                        }
                        return;
                    }
                    Builtin::ValueEq => {
                        let rt = operand_ty(*recv, f, vtys);
                        let r = opt(*recv);
                        let a = opt(args[0]);
                        if rt == Type::Str {
                            out.push_str(&format!("  {}mylrt_str_eq({}, {});\n", dst, r, a));
                        } else {
                            out.push_str(&format!("  {}({} == {});\n", dst, r, a));
                        }
                        return;
                    }
                    Builtin::ValueCmp => {
                        let rt = operand_ty(*recv, f, vtys);
                        let r = opt(*recv);
                        let a = opt(args[0]);
                        if rt == Type::Str {
                            out.push_str(&format!("  {}mylrt_str_cmp({}, {});\n", dst, r, a));
                        } else {
                            out.push_str(&format!(
                                "  {}(({r} < {a}) ? -1 : (({r} > {a}) ? 1 : 0));\n",
                                dst, r = r, a = a
                            ));
                        }
                        return;
                    }
                    Builtin::ValueHash => {
                        let rt = operand_ty(*recv, f, vtys);
                        let r = opt(*recv);
                        if rt == Type::Str {
                            out.push_str(&format!("  {}mylrt_str_hash({});\n", dst, r));
                        } else if rt == Type::F64 {
                            out.push_str(&format!("  {}mylrt_hash_f64({});\n", dst, r));
                        } else if rt == Type::F32 {
                            out.push_str(&format!("  {}mylrt_hash_f64((double)({}));\n", dst, r));
                        } else {
                            out.push_str(&format!("  {}mylrt_hash_u64((myrt_u64)({}));\n", dst, r));
                        }
                        return;
                    }
                    _ => {
                        errors.push(format!("{}: unsupported method builtin {:?}", f.name, b));
                        return;
                    }
                }
            }
            if let Res::Method(id) = &i.res {
                if let Some(im) = p
                    .ifaces
                    .iter()
                    .find(|iface| iface.methods.iter().any(|m| m.decl_id == *id))
                {
                    let iid = im.decl_id;
                    let mname = im
                        .methods
                        .iter()
                        .find(|m| m.decl_id == *id)
                        .map(|m| m.name.as_str())
                        .unwrap_or(name.as_str());
                    let rt = operand_ty(*recv, f, vtys);
                    let r = match rt {
                        Type::Iface(rid) if rid != iid => iface_adapt(rid, iid, opt(*recv)),
                        Type::Class(cid) => iface_wrap(cid, iid, opt(*recv)),
                        Type::Enum(eid) => iface_wrap(eid, iid, opt(*recv)),
                        _ => opt(*recv),
                    };
                    let mut a = vec![format!("((struct I_{}){}).obj", iid, r)];
                    a.extend(args.iter().map(|x| opt(*x)));
                    let call = format!(
                        "((struct I_{}){}).vt->fn_{}({})",
                        iid,
                        r,
                        mname,
                        a.join(", ")
                    );
                    if dst.is_empty() {
                        out.push_str(&format!("  {};\n", call));
                    } else {
                        let rt = cty(&i.ty).unwrap_or_else(|| "void*".into());
                        out.push_str(&format!("  {}({})({});\n", dst, rt, call));
                    }
                    return;
                }
            }
            let recv_cid = match operand_ty(*recv, f, vtys) {
                Type::Class(cid) => Some(cid),
                _ => None,
            };
            let target = match &i.res {
                Res::Method(id) => p.funcs.iter().find(|x| x.decl_id == *id).or_else(|| {
                    // 链接 .mylib 后方法 id 理论上已重映射；若仍找不到，只允许
                    // 在接收者的具体类上按方法名唯一匹配，禁止跨类抓同名方法。
                    let cid = recv_cid?;
                    let mut it = p.funcs.iter().filter(|x| {
                        x.owner == Some(cid) && x.name.rsplit("::").next() == Some(name.as_str())
                    });
                    let first = it.next();
                    if it.next().is_none() { first } else { None }
                }),
                _ => None,
            };
            match target {
                Some(m) => {
                    let mut a = vec![opt(*recv)];
                    a.extend(args.iter().enumerate().map(|(k, x)| {
                        match m.params.get(k) {
                            Some(pt) => coerce_arg(p, f, *x, &pt.ty, vtys, wrapped),
                            None => opt(*x),
                        }
                    }));
                    out.push_str(&format!("  {}{}({});\n", dst, cname(&m.name), a.join(", ")));
                }
                None => errors.push(format!("{}: unresolved method {}", f.name, name)),
            }
        }
        MirInstKind::ArrayNew { len, .. } => {
            let el = arr_elem(&i.ty).cloned();
            let key = el.as_ref().map(arr_key).unwrap_or_else(|| "other".into());
            let size = el.as_ref().and_then(|x| cty(x)).unwrap_or_else(|| "myrt_i32".into());
            out.push_str(&format!(
                "  {}(void*)mylrt_arr_alloc(sizeof({}), (myrt_u64)({}));\n",
                dst, size, opt(*len)
            ));
            let _ = key;
        }
        MirInstKind::ArrayLit { values, .. } => {
            let el = arr_elem(&i.ty).cloned().unwrap_or(Type::Infer);
            let key = arr_key(&el);
            let size = cty(&el).unwrap_or_else(|| "myrt_i32".into());
            let var = dst.trim_end_matches(" = ").to_string();
            out.push_str(&format!(
                "  {} = (void*)mylrt_arr_alloc(sizeof({}), {});\n",
                var, size, values.len()
            ));
            for (idx, v) in values.iter().enumerate() {
                out.push_str(&format!(
                    "  ((struct MylA_{}*){})->items[{}] = {};\n",
                    key, var, idx, opt(*v)
                ));
                if matches!(el, Type::Str | Type::Class(_) | Type::Iface(_) | Type::Array(_)) {
                    out.push_str(&format!(
                        "  myrt_gc_write_barrier_owner((void*)&((struct MylA_{}*){})->items[{}], (void*)(((struct MylA_{}*){})->items[{}]));\n",
                        key, var, idx, key, var, idx
                    ));
                }
            }
        }
        MirInstKind::Index { base, index } => {
            let bt = operand_ty(*base, f, vtys);
            let el = arr_elem(&bt).cloned().unwrap_or(Type::Infer);
            let key = arr_key(&el);
            out.push_str(&format!(
                "  {}((struct MylA_{}*){})->items[mylrt_arr_index(((struct MylA_{}*){})->len, (myrt_i64)({}))];\n",
                dst, key, opt(*base), key, opt(*base), opt(*index)
            ));
        }
        MirInstKind::StoreIndex { base, index, value } => {
            let bt = operand_ty(*base, f, vtys);
            let el = arr_elem(&bt).cloned().unwrap_or(Type::Infer);
            let key = arr_key(&el);
            out.push_str(&format!(
                "  ((struct MylA_{}*){})->items[mylrt_arr_index(((struct MylA_{}*){})->len, (myrt_i64)({}))] = {};\n",
                key, opt(*base), key, opt(*base), opt(*index), opt(*value)
            ));
            if matches!(el, Type::Str | Type::Class(_) | Type::Iface(_) | Type::Array(_)) {
                out.push_str(&format!(
                    "  myrt_gc_write_barrier_owner((void*)&((struct MylA_{}*){})->items[mylrt_arr_index(((struct MylA_{}*){})->len, (myrt_i64)({}))], (void*)(((struct MylA_{}*){})->items[mylrt_arr_index(((struct MylA_{}*){})->len, (myrt_i64)({}))]));\n",
                    key, opt(*base), key, opt(*base), opt(*index), key, opt(*base), key, opt(*base), opt(*index)
                ));
            }
        }
        MirInstKind::Interp { parts } => {
            let var = dst.trim_end_matches(" = ").to_string();
            out.push_str(&format!("  {} = mylrt_str_lit(\"\", 0);\n", var));
            for part in parts {
                match part {
                    MirInterpPart::Lit(t) if t.is_empty() => {}
                    MirInterpPart::Lit(t) => out.push_str(&format!(
                        "  {} = mylrt_str_concat({}, mylrt_str_lit(\"{}\", {}));\n",
                        var, var, c_str(t), t.len()
                    )),
                    MirInterpPart::Value(v) => {
                        let ty = operand_ty(*v, f, vtys);
                        let e = fmt_value(*v, &ty, f, vtys, p, errors);
                        out.push_str(&format!("  {} = mylrt_str_concat({}, {});\n", var, var, e));
                    }
                }
            }
        }
        MirInstKind::MatchTest { subject: value, pattern, .. } => {
            let cond = match pattern {
                Pattern::Variant { enum_id, vidx, .. } => format!(
                    "(((struct E_{}*){})->tag == {})",
                    enum_id, opt(*value), vidx
                ),
                Pattern::Wildcard => "1".to_string(),
                Pattern::Bool(b) => format!("(({}) == {})", opt(*value), if *b { 1 } else { 0 }),
                Pattern::Int(n) => format!("((myrt_i64)({}) == {}LL)", opt(*value), n),
                Pattern::IntU64(n) => format!("((myrt_u64)({}) == {}ULL)", opt(*value), n),
                Pattern::Str(v) => format!("mylrt_str_eq({}, mylrt_str_lit(\"{}\", {}))", opt(*value), c_str(v), v.len()),
                Pattern::Ok(_) => format!("({}).is_ok", opt(*value)),
                Pattern::Err(_) => format!("!({}).is_ok", opt(*value)),
                Pattern::Some(_) => format!("({}).is_some", opt(*value)),
                Pattern::None => format!("!({}).is_some", opt(*value)),
                _ => {
                    errors.push(format!("{}: unsupported match pattern", f.name));
                    "0".to_string()
                }
            };
            out.push_str(&format!("  {}{};\n", dst, cond));
        }
        MirInstKind::BindPattern { subject: value, pattern, locals } => {
            if let Pattern::Variant { enum_id, vidx, binds, .. } = pattern {
                let mut li = 0;
                for (fi, b) in binds.iter().enumerate() {
                    if b.is_some() && li < locals.len() {
                        out.push_str(&format!(
                            "  l{} = ((struct P_{}_{}*)(((struct E_{}*){})->data))->f{};\n",
                            locals[li], enum_id, vidx, enum_id, opt(*value), fi
                        ));
                        li += 1;
                    }
                }
            } else if locals.len() == 1 {
                let li = locals[0] as usize;
                let local_ty = f.locals.get(li).map(|x| x.ty.clone());
                let (ok_iface, err_iface) = result_flags(&operand_ty(*value, f, vtys));
                match pattern {
                    Pattern::Some(_) => {
                        if let Some(Type::Iface(id)) = local_ty {
                            out.push_str(&format!(
                                "  l{} = (struct I_{}){{ .obj = ({}).obj, .vt = (const struct Vt_{}*)({}).vt }};\n",
                                locals[0], id, opt(*value), id, opt(*value)
                            ));
                        } else {
                            out.push_str(&format!("  l{} = ({}).u.some;\n", locals[0], opt(*value)));
                        }
                    }
                    Pattern::Ok(_) => {
                        if let Some(Type::Iface(id)) = local_ty {
                            out.push_str(&format!(
                                "  l{} = (struct I_{}){{ .obj = ({}).ok_obj, .vt = (const struct Vt_{}*)({}).ok_vt }};\n",
                                locals[0], id, opt(*value), id, opt(*value)
                            ));
                        } else {
                            let field = if err_iface { "ok" } else { "u.ok" };
                            let _ = ok_iface;
                            out.push_str(&format!("  l{} = ({}).{};\n", locals[0], opt(*value), field));
                        }
                    }
                    Pattern::Err(_) => {
                        if let Some(Type::Iface(id)) = local_ty {
                            out.push_str(&format!(
                                "  l{} = (struct I_{}){{ .obj = ({}).err_obj, .vt = (const struct Vt_{}*)({}).err_vt }};\n",
                                locals[0], id, opt(*value), id, opt(*value)
                            ));
                        } else {
                            let field = if ok_iface { "err" } else { "u.err" };
                            let _ = err_iface;
                            out.push_str(&format!("  l{} = ({}).{};\n", locals[0], opt(*value), field));
                        }
                    }
                    _ => errors.push(format!("{}: unsupported bind pattern", f.name)),
                }
            } else if !locals.is_empty() {
                errors.push(format!("{}: unsupported bind pattern", f.name));
            }
        }
        MirInstKind::StaticRef(_) => match &i.res {
            Res::Variant(eid, vi) => {
                let e = wrap_coerce(i, format!("(void*)CT_{}_{}()", eid, vi));
                out.push_str(&format!("  {}{};\n", dst, e));
            }
            Res::StaticField(cid, idx) => {
                out.push_str(&format!("  {}{};\n", dst, static_name(*cid, *idx)));
            }
            _ => errors.push(format!("{}: unresolved static ref", f.name)),
        }
        MirInstKind::StorePtr { ptr, value } => {
            let t = operand_ty(*value, f, vtys);
            let c = cty(&t).unwrap_or_else(|| "myrt_i64".into());
            out.push_str(&format!("  *(({}*)({})) = {};\n", c, opt(*ptr), opt(*value)));
        }
        MirInstKind::StoreStatic { value, .. } => {
            if let Res::StaticField(cid, idx) = &i.res {
                let ft = p.classes.iter().find(|c| c.decl_id == *cid)
                    .and_then(|c| c.fields.get(*idx))
                    .map(|f| f.ty.clone());
                out.push_str(&format!("  {} = {};\n", static_name(*cid, *idx), opt(*value)));
                if ft.as_ref().map(is_ref).unwrap_or(false) {
                    out.push_str(&format!("  myrt_gc_write_barrier_owner((void*)&{}, (void*)({}));\n", static_name(*cid, *idx), static_name(*cid, *idx)));
                }
            } else {
                errors.push(format!("{}: static store without field res", f.name));
            }
        }
        MirInstKind::Try { value } => {
            let tmp = format!("t_{}", dst.trim_end_matches(" = ")).replace(' ', "");
            let vty = operand_ty(*value, f, vtys);
            match &f.ret {
                Type::Result(_, _) => {
                    let name = cty(&f.ret).unwrap_or_else(|| "void*".into());
                    let (fok_iface, ferr_iface) = result_flags(&f.ret);
                    let (vok_iface, verr_iface) = result_flags(&vty);
                    out.push_str(&format!("  {} {} = {};\n", name, tmp, opt(*value)));
                    let err_init = if ferr_iface {
                        let _ = verr_iface;
                        format!(".is_ok = 0, .err_obj = {}.err_obj, .err_vt = {}.err_vt", tmp, tmp)
                    } else if fok_iface {
                        format!(".is_ok = 0, .err = {}.err", tmp)
                    } else {
                        let _ = vok_iface;
                        format!(".is_ok = 0, .u = {{ .err = {}.u.err }}", tmp)
                    };
                    out.push_str(&format!("  if (!{}.is_ok) return ({}){{ {} }};\n", tmp, name, err_init));
                    if let Type::Iface(id) = &i.ty {
                        out.push_str(&format!(
                            "  {}(struct I_{}){{ .obj = {}.ok_obj, .vt = (const struct Vt_{}*)({}).ok_vt }};\n",
                            dst, id, tmp, id, tmp
                        ));
                    } else if ferr_iface {
                        out.push_str(&format!("  {}{}.ok;\n", dst, tmp));
                    } else {
                        out.push_str(&format!("  {}{}.u.ok;\n", dst, tmp));
                    }
                }
                Type::Option(_) => {
                    let name = cty(&f.ret).unwrap_or_else(|| "void*".into());
                    let fok_iface = matches!(&f.ret, Type::Option(t) if matches!(&**t, Type::Iface(_)));
                    out.push_str(&format!("  {} {} = {};\n", name, tmp, opt(*value)));
                    out.push_str(&format!("  if (!{}.is_some) return ({}){{ .is_some = 0 }};\n", tmp, name));
                    if let Type::Iface(id) = &i.ty {
                        out.push_str(&format!(
                            "  {}(struct I_{}){{ .obj = {}.obj, .vt = (const struct Vt_{}*)({}).vt }};\n",
                            dst, id, tmp, id, tmp
                        ));
                    } else {
                        let _ = fok_iface;
                        out.push_str(&format!("  {}{}.u.some;\n", dst, tmp));
                    }
                }
                _ => errors.push(format!("{}: ? outside Result/Option", f.name)),
            }
        }
        MirInstKind::Safepoint { .. } => {}
        MirInstKind::Try { .. }
        | MirInstKind::Global(_)
        | MirInstKind::Opaque(_) => {
            errors.push(format!("{}: unsupported MIR instruction {:?}", f.name, i.kind));
        }
    }
}

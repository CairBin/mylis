//! 原生 LLVM IR 后端（第一段）：MIR → 文本 LLVM IR。
//!
//! 覆盖标量、字符串、类/对象、静态字段、数组、Option/Result、enum、接口
//! vtable 与模式匹配，以及可执行/静态库/动态库产物。未 lowering 的结构会
//! 明确返回错误，不回退到 C。

use super::{Artifact, ArtifactKind, Backend, ExportMode};
use crate::ast::{BinOp, Pattern, Program, Res, Type, UnOp};
use crate::mir::{self, MirConst, MirFunction, MirInstKind, MirOperand, MirProgram, MirTerm};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

fn llvm_int_const_text(v: u64, ty: &Type, signed: bool) -> String {
    match ty {
        Type::F32 => format!("{:?}", v as f32),
        Type::F64 => format!("{:?}", v as f64),
        _ => if signed { format!("{}", v as i64) } else { format!("{}", v) },
    }
}

pub struct LlvmBackend {
    /// 消费带原生载荷的 .mylib 时，为 API 顶层函数生成外部声明，而不是生成空定义。
    pub native_externs: bool,
}

impl Backend for LlvmBackend {
    fn name(&self) -> &'static str { "llvm" }

    fn emit_exe(&self, prog: &Program) -> Artifact {
        let ir = mir::lower(prog);
        match emit(&ir, true, self.native_externs, Some(prog)) {
            Ok(text) => Artifact { kind: ArtifactKind::LlvmIr, text, error: None },
            Err(e) => Artifact { kind: ArtifactKind::LlvmIr, text: String::new(), error: Some(e) },
        }
    }

    fn emit_lib(&self, prog: &Program, mode: ExportMode) -> Artifact {
        let ir = mir::lower(prog);
        match emit_lib(prog, &ir, mode, self.native_externs) {
            Ok(text) => Artifact { kind: ArtifactKind::LlvmIr, text, error: None },
            Err(e) => Artifact { kind: ArtifactKind::LlvmIr, text: String::new(), error: Some(e) },
        }
    }

    fn header(&self, prog: &Program) -> String { crate::codegen::header_text(prog) }

    fn emit_exe_mir(&self, prog: &Program, ir: &MirProgram) -> Artifact {
        match emit(ir, true, self.native_externs, Some(prog)) {
            Ok(text) => Artifact { kind: ArtifactKind::LlvmIr, text, error: None },
            Err(e) => Artifact { kind: ArtifactKind::LlvmIr, text: String::new(), error: Some(e) },
        }
    }
}

fn align_up(n: i64, a: i64) -> i64 {
    if a <= 1 { n } else { ((n + a - 1) / a) * a }
}

/// 按 LLVM struct 的自然对齐规则计算字段布局。
fn layout_fields(fields: &[(i64, i64)]) -> (i64, i64) {
    let mut off = 0i64;
    let mut align = 1i64;
    for (size, a) in fields {
        let a = (*a).max(1);
        off = align_up(off, a);
        off += *size;
        align = align.max(a);
    }
    (align_up(off, align), align)
}

fn type_key(t: &Type) -> String {
    match t {
        Type::I8 => "i8".into(), Type::I16 => "i16".into(), Type::I32 => "i32".into(),
        Type::I64 => "i64".into(), Type::U8 => "u8".into(), Type::U16 => "u16".into(),
        Type::U32 => "u32".into(), Type::U64 => "u64".into(),
        Type::F32 => "f32".into(), Type::F64 => "f64".into(),
        Type::Bool => "bool".into(), Type::Str => "str".into(),
        Type::Void => "void".into(),
        Type::Class(id) => format!("c{}", id),
        Type::Enum(id) => format!("e{}", id),
        Type::Iface(id) => format!("i{}", id),
        Type::Array(e) => format!("a{}", type_key(e)),
        Type::Option(e) => format!("o{}", type_key(e)),
        Type::Result(a, b) => format!("r{}_{}", type_key(a), type_key(b)),
        Type::Ptr(_) => "ptr".into(),
        other => format!("x{}", other),
    }
}

// 以单个 GC 指针表示、可直接做写屏障的类型；Iface 是 {obj, vt} 双字 struct，
// Enum 是 { tag, [payload x i64] } 聚合值，二者都不能整体当 ptr 做屏障。
fn is_gc_pointer(t: &Type) -> bool {
    matches!(t, Type::Str | Type::Class(_) | Type::Array(_))
}

// 内联聚合值（Option/Result/Enum）不是单指针；保守地把槽地址同时作为
// owner/value 登记，后续 minor GC 扫 owner 载荷即可覆盖内部引用。
fn is_gc_aggregate(t: &Type) -> bool {
    matches!(t, Type::Option(_) | Type::Result(_, _) | Type::Enum(_))
}

fn llty(t: &Type) -> Result<String, String> {
    Ok(match t {
        Type::I8 | Type::U8 => "i8".into(),
        Type::I16 | Type::U16 => "i16".into(),
        Type::I32 | Type::U32 => "i32".into(),
        Type::I64 | Type::U64 => "i64".into(),
        Type::F32 => "float".into(),
        Type::F64 => "double".into(),
        Type::Bool => "i32".into(),
        Type::Str => "ptr".into(),
        Type::Ptr(_) => "ptr".into(),
        Type::Class(_) | Type::Array(_) => "ptr".into(),
        Type::Enum(id) => format!("%E_{}", id),
        Type::Iface(id) => format!("%I_{}", id),
        Type::Option(_) => format!("%O_{}", type_key(t)),
        Type::Result(_, _) => format!("%R_{}", type_key(t)),
        Type::Void => "void".into(),
        other => return Err(format!("LLVM 后端暂不支持类型 `{}`", other)),
    })
}

fn is_signed(t: &Type) -> bool {
    matches!(t, Type::I8 | Type::I16 | Type::I32 | Type::I64)
}

fn is_float(t: &Type) -> bool { matches!(t, Type::F32 | Type::F64) }

fn is_int(t: &Type) -> bool {
    matches!(t, Type::I8 | Type::I16 | Type::I32 | Type::I64
        | Type::U8 | Type::U16 | Type::U32 | Type::U64)
}

fn is_value(t: &Type) -> bool { llty(t).is_ok() && !matches!(t, Type::Void) }

/// 该类型的槽需要作为 GC 根扫描。
fn gc_ref_type(t: &Type) -> bool {
    match t {
        Type::Str | Type::Class(_) | Type::Array(_) | Type::Iface(_) | Type::Enum(_) => true,
        Type::Option(e) => gc_ref_type(e),
        Type::Result(a, b) => gc_ref_type(a) || gc_ref_type(b),
        _ => false,
    }
}

/// GC 根槽的字节大小；比类型布局更保守：聚合按可能包含引用的字段计算。
fn gc_root_size(t: &Type, ir: &MirProgram) -> i64 {
    match t {
        Type::Str | Type::Class(_) | Type::Array(_) | Type::Ptr(_) => 8,
        Type::Iface(_) => 16,
        Type::Enum(id) => {
            let n = ir.enums.iter().find(|e| e.decl_id == *id)
                .map(|e| e.variants.iter().map(|v| v.fields.len()).max().unwrap_or(0))
                .unwrap_or(0);
            8 + (n as i64) * 8
        }
        Type::Option(e) => layout_fields(&[(4, 4), (gc_root_size(e, ir), 8)]).0,
        Type::Result(a, b) => layout_fields(&[(4, 4), (gc_root_size(a, ir), 8), (gc_root_size(b, ir), 8)]).0,
        _ => 8,
    }
}

fn sym(name: &str) -> String {
    let mut s = String::from("myl_");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

fn label(name: &str) -> String {
    let mut s = String::from("L_");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

fn esc_str(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match *b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7e => out.push(*b as char),
            _ => { let _ = write!(out, "\\{:02X}", b); }
        }
    }
    out
}

/// 运行时 C ABI：名字 → (返回类型, 参数类型)
fn runtime_sig(name: &str) -> Option<(&'static str, Vec<&'static str>)> {
    Some(match name {
        "myrt_init" => ("void", vec![]),
        "myrt_gc_collect" => ("void", vec![]),
        "myrt_print" | "myrt_println" | "myrt_panic" => ("void", vec!["ptr"]),
        "myrt_i64_to_string" | "myrt_u64_to_string" => ("ptr", vec!["i64"]),
        "myrt_f64_to_string" => ("ptr", vec!["double"]),
        "myrt_f64_to_string_prec" => ("ptr", vec!["double", "i32"]),
        "myrt_f64_to_string_fmt" => ("ptr", vec!["double", "i32", "i32", "i32"]),
        "mylrt_str_starts_with" | "mylrt_str_ends_with" | "mylrt_str_contains" => ("i32", vec!["ptr", "ptr"]),
        "mylrt_str_index_of" => ("i64", vec!["ptr", "ptr"]),
        "mylrt_str_substring" => ("ptr", vec!["ptr", "i64", "i64"]),
        "mylrt_str_trim" => ("ptr", vec!["ptr"]),
        "mylrt_str_repeat" => ("ptr", vec!["ptr", "i64"]),
        "mylrt_str_replace" => ("ptr", vec!["ptr", "ptr", "ptr"]),
        "mylrt_str_char_len" => ("i64", vec!["ptr"]),
        "mylrt_str_char_index_of" => ("i64", vec!["ptr", "ptr"]),
        "mylrt_str_char_substring" => ("ptr", vec!["ptr", "i64", "i64"]),
        "mylrt_str_char_at" => ("ptr", vec!["ptr", "i64"]),
        "mylrt_str_char_code_at" => ("i32", vec!["ptr", "i64"]),
        "mylrt_str_to_bytes" => ("ptr", vec!["ptr"]),
        "mylrt_str_from_bytes" => ("ptr", vec!["ptr"]),
        "sqrt" => ("double", vec!["double"]),
        "pow" => ("double", vec!["double", "double"]),
        "fabs" => ("double", vec!["double"]),
        "floor" => ("double", vec!["double"]),
        "ceil" => ("double", vec!["double"]),
        "round" => ("double", vec!["double"]),
        "trunc" => ("double", vec!["double"]),
        "fmod" => ("double", vec!["double", "double"]),
        "myrt_str_cstr" => ("ptr", vec!["ptr"]),
        "myrt_str_ptr" => ("ptr", vec!["ptr"]),
        "myrt_str_len" => ("i64", vec!["ptr"]),
        "myrt_clock" => ("double", vec![]),
        "myrt_alloc" => ("ptr", vec!["i64"]),
        "myrt_register_root" => ("void", vec!["ptr"]),
        "myrt_unregister_root" => ("void", vec!["ptr"]),
        "myrt_gc_write_barrier" => ("void", vec!["ptr"]),
        "myrt_gc_write_barrier_owner" => ("void", vec!["ptr", "ptr"]),
        "myrt_gc_safepoint" => ("void", vec![]),
        "myrt_gc_add_root_range" => ("void", vec!["ptr", "i64"]),
        "myrt_gc_remove_root_range" => ("void", vec!["ptr"]),
        "myrt_gc_frame_enter" => ("i64", vec![]),
        "myrt_gc_frame_set_roots" => ("void", vec!["i64", "ptr", "i64"]),
        "myrt_gc_frame_exit" => ("void", vec!["i64"]),
        "myrt_gc_static_set_roots" => ("void", vec!["ptr", "i64"]),
        "mylrt_arr_alloc" => ("ptr", vec!["i64", "i64"]),
        "mylrt_arr_index" => ("i64", vec!["i64", "i64"]),
        "mylrt_str_lit" => ("ptr", vec!["ptr", "i64"]),
        "mylrt_str_concat" => ("ptr", vec!["ptr", "ptr"]),
        "mylrt_str_eq" => ("i32", vec!["ptr", "ptr"]),
        "mylrt_str_cmp" => ("i32", vec!["ptr", "ptr"]),
        "mylrt_str_hash" => ("i64", vec!["ptr"]),
        "mylrt_hash_u64" => ("i64", vec!["i64"]),
        "mylrt_hash_f64" => ("i64", vec!["double"]),
        "mylrt_panic_unwrap" => ("void", vec![]),
        "mylrt_panic_msg" => ("void", vec!["ptr"]),
        _ => return None,
    })
}

struct FnGen<'a> {
    f: &'a MirFunction,
    out: String,
    pre: String,
    slots: HashMap<u32, (String, Type)>,
    locals: HashMap<u32, (String, Type)>,
    labels: HashMap<String, String>,
    str_lits: &'a mut Vec<(String, String, usize)>,
    str_n: &'a mut usize,
    tmp_n: usize,
    all_funcs: HashMap<&'a str, &'a MirFunction>,
    externs: HashMap<String, ExternalFn>,
    classes: HashMap<usize, &'a crate::mir::MirClass>,
    enums: HashMap<usize, &'a crate::mir::MirEnum>,
    ifaces: HashMap<usize, &'a crate::mir::MirInterface>,
    iface_methods: HashMap<usize, (usize, usize)>,
    frame_id: Option<String>,
    root_frame: Option<String>,
    root_n: usize,
    root_local_idx: HashMap<u32, usize>,
    root_value_idx: HashMap<u32, usize>,
    root_sizes: Vec<i64>,
}

impl<'a> FnGen<'a> {
    fn norm(&self, t: &Type) -> Type {
        match t {
            Type::Class(id) if !self.classes.contains_key(id) && self.enums.contains_key(id) => Type::Enum(*id),
            Type::Array(e) => Type::Array(Box::new(self.norm(e))),
            Type::Option(e) => Type::Option(Box::new(self.norm(e))),
            Type::Result(a, b) => Type::Result(Box::new(self.norm(a)), Box::new(self.norm(b))),
            _ => t.clone(),
        }
    }

    fn ty(&self, t: &Type) -> Result<String, String> { llty(&self.norm(t)) }

    fn slot_of(&self, v: u32) -> Result<(&String, &Type), String> {
        self.slots.get(&v).map(|(s, t)| (s, t))
            .ok_or_else(|| format!("MIR 值 %{} 未定义（LLVM 后端）", v))
    }

    fn local_of(&self, v: u32) -> Result<(&String, &Type), String> {
        self.locals.get(&v).map(|(s, t)| (s, t))
            .ok_or_else(|| format!("MIR 局部槽 ${} 未定义（LLVM 后端）", v))
    }

    /// 把操作数读成 LLVM 值：生成一条 load，返回 (临时值, 类型)
    fn load(&mut self, o: &MirOperand) -> Result<(String, Type), String> {
        let (slot, ty) = match o {
            MirOperand::Value(v) => self.slot_of(*v)?,
            MirOperand::Local(v) => self.local_of(*v)?,
        };
        let (slot, ty) = (slot.clone(), ty.clone());
        let lt = self.ty(&ty)?;
        let n = self.tmp("ld");
        let _ = writeln!(self.out, "  {} = load {}, ptr %{}", n, lt, slot);
        Ok((n, ty))
    }

    fn tmp(&mut self, name: &str) -> String {
        let n = self.tmp_n;
        self.tmp_n += 1;
        format!("%{}_{}", name.replace('.', "_"), n)
    }

    fn elem_ty(ty: &Type) -> Result<Type, String> {
        match ty {
            Type::Array(e) => Ok((**e).clone()),
            _ => Err(format!("期望数组类型，收到 `{}`", ty)),
        }
    }

    fn elem_size(&self, t: &Type) -> Result<i64, String> {
        self.size_align(t).map(|(size, _)| size)
    }

    /// LLVM 聚合布局：(size, align)。数组元素偏移必须与 LLVM 的
    /// `getelementptr <T>` 一致，不能只按“字段大小相加”估算。
    fn size_align(&self, t: &Type) -> Result<(i64, i64), String> {
        Ok(match t {
            Type::I8 | Type::U8 | Type::Bool => (1, 1),
            Type::I16 | Type::U16 => (2, 2),
            Type::I32 | Type::U32 | Type::F32 => (4, 4),
            Type::I64 | Type::U64 | Type::F64 => (8, 8),
            Type::Str | Type::Class(_) | Type::Array(_) | Type::Ptr(_) => (8, 8),
            Type::Iface(_) => (16, 8),
            Type::Enum(id) => {
                let e = self.enums.get(id).ok_or_else(|| format!("LLVM 后端缺少 enum 元数据 {}", id))?;
                let n = e.variants.iter().map(|v| v.fields.len()).max().unwrap_or(0);
                // { i32 tag, [n x i64] payload }：payload 在 offset 8，整体按 8 对齐。
                (8 + (n as i64) * 8, 8)
            }
            Type::Option(e) => {
                let (ps, pa) = self.size_align(e)?;
                layout_fields(&[(4, 4), (ps, pa)])
            }
            Type::Result(a, b) => {
                let (as_, aa) = self.size_align(a)?;
                let (bs, ba) = self.size_align(b)?;
                layout_fields(&[(4, 4), (as_, aa), (bs, ba)])
            }
            _ => return Err(format!("LLVM 后端暂不支持数组元素类型 `{}`", t)),
        })
    }

    fn as_i64(&mut self, x: &str, t: &Type) -> Result<String, String> {
        Ok(match t {
            Type::I64 | Type::U64 => x.to_string(),
            Type::I8 | Type::I16 | Type::I32 => {
                let n = self.tmp("e64"); let _ = writeln!(self.out, "  {} = sext {} {} to i64", n, llty(t)?, x); n
            }
            Type::U8 | Type::U16 | Type::U32 | Type::Bool => {
                let n = self.tmp("e64"); let _ = writeln!(self.out, "  {} = zext {} {} to i64", n, llty(t)?, x); n
            }
            Type::F32 => {
                let b = self.tmp("eb"); let _ = writeln!(self.out, "  {} = bitcast float {} to i32", b, x);
                let n = self.tmp("e64"); let _ = writeln!(self.out, "  {} = zext i32 {} to i64", n, b); n
            }
            Type::F64 => {
                let n = self.tmp("e64"); let _ = writeln!(self.out, "  {} = bitcast double {} to i64", n, x); n
            }
            Type::Str | Type::Class(_) | Type::Array(_) | Type::Ptr(_) | Type::Enum(_) => {
                let n = self.tmp("e64"); let _ = writeln!(self.out, "  {} = ptrtoint ptr {} to i64", n, x); n
            }
            _ => return Err(format!("LLVM 后端暂不支持枚举载荷类型 `{}`", t)),
        })
    }

    fn from_i64(&mut self, x: &str, t: &Type) -> Result<String, String> {
        Ok(match t {
            Type::I8 | Type::I16 | Type::I32 => {
                let n = self.tmp("ed"); let _ = writeln!(self.out, "  {} = trunc i64 {} to {}", n, x, llty(t)?); n
            }
            Type::I64 | Type::U64 => x.to_string(),
            Type::U8 | Type::U16 | Type::U32 | Type::Bool => {
                let n = self.tmp("ed"); let _ = writeln!(self.out, "  {} = trunc i64 {} to {}", n, x, llty(t)?); n
            }
            Type::F64 => {
                let n = self.tmp("ed"); let _ = writeln!(self.out, "  {} = bitcast i64 {} to double", n, x); n
            }
            Type::F32 => {
                let b = self.tmp("ed32"); let _ = writeln!(self.out, "  {} = trunc i64 {} to i32", b, x);
                let n = self.tmp("ed"); let _ = writeln!(self.out, "  {} = bitcast i32 {} to float", n, b); n
            }
            Type::Str | Type::Class(_) | Type::Array(_) | Type::Ptr(_) | Type::Enum(_) => {
                let n = self.tmp("ed"); let _ = writeln!(self.out, "  {} = inttoptr i64 {} to ptr", n, x); n
            }
            _ => return Err(format!("LLVM 后端暂不支持枚举载荷类型 `{}`", t)),
        })
    }

    fn cast_from_iface(&mut self, x: &str, iid: usize, target: &Type, checked: bool, out_ty: &Type) -> Result<String, String> {
        let it = format!("%I_{}", iid);
        let obj = self.tmp("cobj");
        let vt = self.tmp("cvt");
        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, it, x);
        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 1", vt, it, x);
        if let Type::Iface(tid) = target {
            let adapted = self.tmp("cad");
            let _ = writeln!(self.out, "  {} = call %I_{} @IA_{}_{}({} {})", adapted, tid, iid, tid, it, x);
            let avt = self.tmp("cavt");
            let _ = writeln!(self.out, "  {} = extractvalue %I_{} {}, 1", avt, tid, adapted);
            if checked {
                let ost = llty(out_ty)?;
                let o0 = self.tmp("co0");
                let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, i32 0, 0", o0, ost);
                let c = self.tmp("cc");
                let _ = writeln!(self.out, "  {} = icmp ne ptr {}, null", c, avt);
                let v0 = self.tmp("cv0");
                let v1 = self.tmp("cv1");
                let _ = writeln!(self.out, "  {} = insertvalue {} {}, i32 1, 0", v0, ost, o0);
                let _ = writeln!(self.out, "  {} = insertvalue {} {}, %I_{} {}, 1", v1, ost, v0, tid, adapted);
                let sel = self.tmp("csel");
                let _ = writeln!(self.out, "  {} = select i1 {}, {} {}, {} {}", sel, c, ost, v1, ost, o0);
                return Ok(sel);
            }
            let c = self.tmp("cc");
            let _ = writeln!(self.out, "  {} = icmp eq ptr {}, null", c, avt);
            let fail = format!("L_cf_{}", self.tmp_n); self.tmp_n += 1;
            let cont = format!("L_cc_{}", self.tmp_n); self.tmp_n += 1;
            let _ = writeln!(self.out, "  br i1 {}, label %{}, label %{}", c, fail, cont);
            let _ = writeln!(self.out, "{}:", fail);
            let _ = writeln!(self.out, "  call void @mylrt_panic_unwrap()");
            let _ = writeln!(self.out, "  unreachable");
            let _ = writeln!(self.out, "{}:", cont);
            return Ok(adapted);
        }
        if let Type::Iface(tid) = out_ty {
            let _ = tid;
        }
        let payload_ty = if checked { match out_ty { Type::Option(e) => (**e).clone(), _ => target.clone() } } else { target.clone() };
        // 候选 (vtable, 载荷值)
        let mut cands: Vec<(String, String, Type)> = Vec::new();
        match target {
            Type::Class(cid) => {
                cands.push((format!("@VT_{}_{}", cid, iid), obj.clone(), Type::Class(*cid)));
            }
            Type::Enum(eid) => {
                let et = format!("%E_{}", eid);
                let v = self.tmp("cev");
                let _ = writeln!(self.out, "  {} = load {}, ptr {}", v, et, obj);
                cands.push((format!("@VT_{}_{}", eid, iid), v, Type::Enum(*eid)));
            }
            _ => {
                let key = type_key(target);
                let bt = format!("%B_{}", key);
                let p = self.tmp("cbp");
                let v = self.tmp("cbv");
                let _ = writeln!(self.out, "  {} = getelementptr {}, ptr {}, i32 0, i32 0", p, bt, obj);
                let _ = writeln!(self.out, "  {} = load {}, ptr {}", v, llty(target)?, p);
                cands.push((format!("@VT_BOX_{}_{}", key, iid), v, target.clone()));
            }
        }
        if checked {
            let ost = llty(out_ty)?;
            let o0 = self.tmp("co0");
            let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, i32 0, 0", o0, ost);
            let mut acc = o0;
            for (vtg, val, _t) in cands {
                let c = self.tmp("cc");
                let _ = writeln!(self.out, "  {} = icmp eq ptr {}, {}", c, vt, vtg);
                let v0 = self.tmp("cv0");
                let v1 = self.tmp("cv1");
                let _ = writeln!(self.out, "  {} = insertvalue {} {}, i32 1, 0", v0, ost, acc);
                let _ = writeln!(self.out, "  {} = insertvalue {} {}, {} {}, 1", v1, ost, v0, llty(&payload_ty)?, val);
                let sel = self.tmp("csel");
                let _ = writeln!(self.out, "  {} = select i1 {}, {} {}, {} {}", sel, c, ost, v1, ost, acc);
                acc = sel;
            }
            Ok(acc)
        } else {
            let t = llty(&payload_ty)?;
            let slot = self.tmp("cslot");
            let _ = writeln!(self.out, "  {} = alloca {}", slot, t);
            let _ = writeln!(self.out, "  store {} zeroinitializer, ptr {}", t, slot);
            let hit = self.tmp("chit");
            let _ = writeln!(self.out, "  {} = alloca i1", hit);
            let _ = writeln!(self.out, "  store i1 false, ptr {}", hit);
            for (vtg, val, _t) in cands {
                let c = self.tmp("cc");
                let _ = writeln!(self.out, "  {} = icmp eq ptr {}, {}", c, vt, vtg);
                let cur = self.tmp("ccur");
                let _ = writeln!(self.out, "  {} = load {} , ptr {}", cur, t, slot);
                let sel = self.tmp("csel");
                let _ = writeln!(self.out, "  {} = select i1 {}, {} {}, {} {}", sel, c, t, val, t, cur);
                let _ = writeln!(self.out, "  store {} {}, ptr {}", t, sel, slot);
                let old = self.tmp("cold");
                let _ = writeln!(self.out, "  {} = load i1, ptr {}", old, hit);
                let any = self.tmp("cany");
                let _ = writeln!(self.out, "  {} = or i1 {}, {}", any, old, c);
                let _ = writeln!(self.out, "  store i1 {}, ptr {}", any, hit);
            }
            let m = self.tmp("cm");
            let _ = writeln!(self.out, "  {} = load i1, ptr {}", m, hit);
            let fail = format!("L_cf_{}", self.tmp_n); self.tmp_n += 1;
            let cont = format!("L_cc_{}", self.tmp_n); self.tmp_n += 1;
            let _ = writeln!(self.out, "  br i1 {}, label %{}, label %{}", m, cont, fail);
            let _ = writeln!(self.out, "{}:", fail);
            let _ = writeln!(self.out, "  call void @mylrt_panic_unwrap()");
            let _ = writeln!(self.out, "  unreachable");
            let _ = writeln!(self.out, "{}:", cont);
            let r = self.tmp("cr");
            let _ = writeln!(self.out, "  {} = load {}, ptr {}", r, t, slot);
            Ok(r)
        }
    }

    fn wrap_iface(&mut self, x: &str, from: &Type, iid: usize) -> Result<String, String> {
        let obj = match from {
            Type::Class(_) => x.to_string(),
            Type::Enum(eid) => {
                let et = format!("%E_{}", eid);
                let slot = self.tmp("ebox");
                let _ = writeln!(self.out, "  {} = alloca {}", slot, et);
                let _ = writeln!(self.out, "  store {} {}, ptr {}", et, x, slot);
                slot
            }
            Type::Iface(fid) if *fid == iid => return Ok(x.to_string()),
            Type::Iface(fid) => {
                let from = format!("%I_{}", fid);
                let to = format!("%I_{}", iid);
                let fnp = self.tmp("ia");
                let _ = writeln!(self.out, "  {} = call {} @IA_{}_{}({} {})", fnp, to, fid, iid, from, x);
                return Ok(fnp);
            }
            _ => return Err(format!("LLVM 后端暂不支持 `{}` 转接口 {}", from, iid)),
        };
        let it = format!("%I_{}", iid);
        let v0 = self.tmp("iv0");
        let v1 = self.tmp("iv1");
        let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, ptr {}, 0", v0, it, obj);
        let _ = writeln!(self.out, "  {} = insertvalue {} {}, ptr @VT_{}_{}, 1", v1, it, v0, match from {
            Type::Class(cid) => *cid, Type::Enum(eid) => *eid, _ => 0
        }, iid);
        Ok(v1)
    }

    fn iface_dispatch(&mut self, v: &str, iid: usize, slot: usize, ret: &Type, args: &[(String, Type)]) -> Result<String, String> {
        let it = format!("%I_{}", iid);
        let vt = self.tmp("ivt");
        let obj = self.tmp("iobj");
        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 1", vt, it, v);
        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, it, v);
        let fp = self.tmp("ifp");
        let _ = writeln!(self.out, "  {} = getelementptr %Vt_{}, ptr {}, i32 0, i32 {}", fp, iid, vt, slot);
        let f = self.tmp("ifn");
        let _ = writeln!(self.out, "  {} = load ptr, ptr {}", f, fp);
        let mut typed = vec![format!("ptr {}", obj)];
        for (a, t) in args {
            let nt = self.norm(t);
            typed.push(format!("{} {}", llty(&nt)?, a));
        }
        let n = self.tmp("icall");
        if *ret == Type::Void {
            let _ = writeln!(self.out, "  call void {}({})", f, typed.join(", "));
            Ok(n)
        } else {
            let nt = self.norm(ret);
            let _ = writeln!(self.out, "  {} = call {} {}({})", n, llty(&nt)?, f, typed.join(", "));
            Ok(n)
        }
    }

    fn int_const_text(v: u64, ty: &Type, signed: bool) -> String {
        match ty {
            Type::F32 => format!("{:?}", v as f32),
            Type::F64 => format!("{:?}", v as f64),
            _ => {
                if signed { format!("{}", v as i64) } else { format!("{}", v) }
            }
        }
    }

    fn const_value(&mut self, c: &MirConst, ty: &Type) -> Result<String, String> {
        Ok(match c {
            MirConst::Int(v) => Self::int_const_text(*v as u64, ty, true),
            MirConst::IntU64(v) => Self::int_const_text(*v, ty, false),
            MirConst::IntU64(v) => format!("{}", v),
            MirConst::Float(v) => format!("{:?}", v),
            MirConst::Bool(b) => if *b { "1".into() } else { "0".into() },
            MirConst::Null => "null".into(),
            MirConst::Str(s) => {
                let l = self.lit(s);
                let n = self.tmp("fstr");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 {})", n, l, s.as_bytes().len());
                n
            }
        })
    }

    fn call_fn(&mut self, f: &MirFunction, recv: Option<(&str, Type)>, args: &[MirOperand]) -> Result<Option<String>, String> {
        let recv = match recv {
            Some((v, t)) => Some((v.to_string(), t.clone())),
            None => None,
        };
        let mut vals: Vec<(String, Type)> = Vec::new();
        if let Some((v, t)) = recv { vals.push((v, t)); }
        for a in args { vals.push(self.load(a)?); }
        let mut typed = Vec::new();
        for (idx, (v, t)) in vals.into_iter().enumerate() {
            let pt = f.params.get(idx).map(|p| p.ty.clone()).unwrap_or(t.clone());
            let nt = self.norm(&pt);
            let vv = if let Type::Iface(iid) = &nt { self.wrap_iface(&v, &t, *iid)? } else { v };
            typed.push(format!("{} {}", llty(&nt)?, vv));
        }
        let n = self.tmp("call");
        if f.ret == Type::Void {
            let _ = writeln!(self.out, "  call void @{}({})", sym(&f.name), typed.join(", "));
            Ok(None)
        } else {
            let ret = llty(&f.ret)?;
            let _ = writeln!(self.out, "  {} = call {} @{}({})", n, ret, sym(&f.name), typed.join(", "));
            Ok(Some(n))
        }
    }

    fn fmt_value(&mut self, x: &str, ty: &Type) -> Result<String, String> {
        match ty {
            Type::Str => Ok(x.to_string()),
            Type::Bool => {
                let t = self.lit("true");
                let f = self.lit("false");
                let tv = self.tmp("true");
                let fv = self.tmp("false");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 4)", tv, t);
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 5)", fv, f);
                let c = self.tmp("bcond");
                let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", c, x);
                let s = self.tmp("bsel");
                let _ = writeln!(self.out, "  {} = select i1 {}, ptr {}, ptr {}", s, c, tv, fv);
                Ok(s)
            }
            Type::I64 => {
                let s = self.tmp("istr");
                let _ = writeln!(self.out, "  {} = call ptr @myrt_i64_to_string(i64 {})", s, x);
                Ok(s)
            }
            Type::I8 | Type::I16 | Type::I32 => {
                let n = self.tmp("i64");
                let _ = writeln!(self.out, "  {} = sext {} {} to i64", n, llty(ty)?, x);
                let s = self.tmp("istr");
                let _ = writeln!(self.out, "  {} = call ptr @myrt_i64_to_string(i64 {})", s, n);
                Ok(s)
            }
            Type::U8 | Type::U16 | Type::U32 => {
                let n = self.tmp("u64");
                let _ = writeln!(self.out, "  {} = zext {} {} to i64", n, llty(ty)?, x);
                let s = self.tmp("ustr");
                let _ = writeln!(self.out, "  {} = call ptr @myrt_u64_to_string(i64 {})", s, n);
                Ok(s)
            }
            Type::U64 => {
                let s = self.tmp("ustr");
                let _ = writeln!(self.out, "  {} = call ptr @myrt_u64_to_string(i64 {})", s, x);
                Ok(s)
            }
            Type::F32 => {
                let n = self.tmp("f64");
                let _ = writeln!(self.out, "  {} = fpext float {} to double", n, x);
                let s = self.tmp("fstr");
                let _ = writeln!(self.out, "  {} = call ptr @myrt_f64_to_string(double {})", s, n);
                Ok(s)
            }
            Type::F64 => {
                let s = self.tmp("fstr");
                let _ = writeln!(self.out, "  {} = call ptr @myrt_f64_to_string(double {})", s, x);
                Ok(s)
            }
            Type::Iface(iid) => {
                let iface = self.ifaces.get(iid).ok_or_else(|| format!("LLVM 后端缺少接口元数据 {}", iid))?;
                let slot = iface.methods.iter().position(|m| matches!(m.name.as_str(), "toString" | "to_string"))
                    .ok_or_else(|| format!("接口 {} 没有 to_string", iid))?;
                self.iface_dispatch(x, *iid, slot, &Type::Str, &[])
            }
            Type::Enum(eid) => {
                if let Some(m) = self.all_funcs.values().find(|f| f.owner == Some(*eid)
                    && matches!(f.name.rsplit("::").next(), Some("toString") | Some("to_string")) && !f.is_static)
                {
                    let m = *m;
                    let r = self.call_fn(m, Some((x, ty.clone())), &[])?;
                    r.ok_or_else(|| format!("枚举 `{}` 的 to_string 返回 void", ty))
                } else {
                    Err(format!("LLVM 后端暂不支持把 `{}` 格式化进字符串", ty))
                }
            }
            Type::Class(cid) => {
                if let Some(m) = self.all_funcs.values().find(|f| f.owner == Some(*cid)
                    && matches!(f.name.rsplit("::").next(), Some("toString") | Some("to_string")) && !f.is_static)
                {
                    let m = *m;
                    let r = self.call_fn(m, Some((x, ty.clone())), &[])?;
                    r.ok_or_else(|| format!("类 `{}` 的 to_string 返回 void", ty))
                } else {
                    Err(format!("LLVM 后端暂不支持把 `{}` 格式化进字符串", ty))
                }
            }
            Type::Array(e) => {
                let e = (**e).clone();
                let base = x.to_string();
                let acc = self.tmp("acc");
                let _ = writeln!(self.out, "  {} = alloca ptr", acc);
                let empty = self.lit("[");
                let init = self.tmp("ainit");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 1)", init, empty);
                let _ = writeln!(self.out, "  store ptr {}, ptr {}", init, acc);
                let i = self.tmp("ai");
                let _ = writeln!(self.out, "  {} = alloca i64", i);
                let _ = writeln!(self.out, "  store i64 0, ptr {}", i);
                let lstart = format!("L_arr_{}", self.tmp_n);
                self.tmp_n += 1;
                let lbody = format!("L_arrb_{}", self.tmp_n);
                self.tmp_n += 1;
                let lend = format!("L_arre_{}", self.tmp_n);
                self.tmp_n += 1;
                let _ = writeln!(self.out, "  br label %{}", lstart);
                let _ = writeln!(self.out, "{}:", lstart);
                let iv = self.tmp("aiv");
                let _ = writeln!(self.out, "  {} = load i64, ptr {}", iv, i);
                let len = self.tmp("alen");
                let _ = writeln!(self.out, "  {} = load i64, ptr {}", len, base);
                let cmp = self.tmp("acmp");
                let _ = writeln!(self.out, "  {} = icmp slt i64 {}, {}", cmp, iv, len);
                let _ = writeln!(self.out, "  br i1 {}, label %{}, label %{}", cmp, lbody, lend);
                let _ = writeln!(self.out, "{}:", lbody);
                let nz = self.tmp("anz");
                let _ = writeln!(self.out, "  {} = icmp ne i64 {}, 0", nz, iv);
                let sep = self.lit(", ");
                let sepp = self.tmp("sep");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 2)", sepp, sep);
                let cur = self.tmp("acur");
                let _ = writeln!(self.out, "  {} = load ptr, ptr {}", cur, acc);
                let withsep = self.tmp("asep");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_concat(ptr {}, ptr {})", withsep, cur, sepp);
                let sel = self.tmp("asel");
                let _ = writeln!(self.out, "  {} = select i1 {}, ptr {}, ptr {}", sel, nz, withsep, cur);
                let _ = writeln!(self.out, "  store ptr {}, ptr {}", sel, acc);
                let p = self.tmp("ap");
                let _ = writeln!(self.out, "  {} = getelementptr i8, ptr {}, i64 8", p, base);
                let q = self.tmp("ae");
                let _ = writeln!(self.out, "  {} = getelementptr {}, ptr {}, i64 {}", q, llty(&e)?, p, iv);
                let v = self.tmp("av");
                let _ = writeln!(self.out, "  {} = load {}, ptr {}", v, llty(&e)?, q);
                let piece = self.fmt_value(&v, &e)?;
                let cur2 = self.tmp("acur2");
                let _ = writeln!(self.out, "  {} = load ptr, ptr {}", cur2, acc);
                let cat = self.tmp("acat2");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_concat(ptr {}, ptr {})", cat, cur2, piece);
                let _ = writeln!(self.out, "  store ptr {}, ptr {}", cat, acc);
                let nxt = self.tmp("anext");
                let _ = writeln!(self.out, "  {} = add i64 {}, 1", nxt, iv);
                let _ = writeln!(self.out, "  store i64 {}, ptr {}", nxt, i);
                let _ = writeln!(self.out, "  br label %{}", lstart);
                let _ = writeln!(self.out, "{}:", lend);
                let close = self.lit("]");
                let cpp = self.tmp("aclose");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 1)", cpp, close);
                let cur3 = self.tmp("acur3");
                let _ = writeln!(self.out, "  {} = load ptr, ptr {}", cur3, acc);
                let fin = self.tmp("afin");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_concat(ptr {}, ptr {})", fin, cur3, cpp);
                Ok(fin)
            }
            _ => Err(format!("LLVM 后端暂不支持把 `{}` 格式化进字符串", ty)),
        }
    }

    fn builtin(
        &mut self,
        b: crate::ast::Builtin,
        recv: &MirOperand,
        args: &[MirOperand],
        ins: &crate::mir::MirInst,
    ) -> Result<(), String> {
        use crate::ast::Builtin;
        let dst = ins.dst.ok_or_else(|| "builtin 缺少 dst".to_string())?;
        // 标量/字符串的哈希内建：接收者是具体值，不是 Option/Result 聚合。
        if matches!(b, Builtin::ValueHash) {
            let (x, xt) = self.load(recv)?;
            let res = if xt == Type::Str {
                let n = self.tmp("strhash");
                let _ = writeln!(self.out, "  {} = call i64 @mylrt_str_hash(ptr {})", n, x);
                n
            } else if xt == Type::F64 {
                let n = self.tmp("f64hash");
                let _ = writeln!(self.out, "  {} = call i64 @mylrt_hash_f64(double {})", n, x);
                n
            } else if xt == Type::F32 {
                let ext = self.tmp("f32ext");
                let _ = writeln!(self.out, "  {} = fpext float {} to double", ext, x);
                let n = self.tmp("f32hash");
                let _ = writeln!(self.out, "  {} = call i64 @mylrt_hash_f64(double {})", n, ext);
                n
            } else {
                let xtl = self.ty(&xt)?;
                let ext = self.tmp("hext");
                let cast = if xtl == "i1" || !is_signed(&xt) { "zext" } else { "sext" };
                let _ = writeln!(self.out, "  {} = {} {} {} to i64", ext, cast, xtl, x);
                let n = self.tmp("hash");
                let _ = writeln!(self.out, "  {} = call i64 @mylrt_hash_u64(i64 {})", n, ext);
                n
            };
            self.store_slot(dst, &res)?;
            return Ok(());
        }
        // 标量/字符串的 Eq/Ord 内建：接收者是具体值，不是 Option/Result 聚合。
        if matches!(b, Builtin::ValueEq | Builtin::ValueCmp) {
            let (x, xt) = self.load(recv)?;
            let (y, _yt) = self.load(&args[0])?;
            let res = if xt == Type::Str {
                let n = self.tmp("strcmp");
                if matches!(b, Builtin::ValueEq) {
                    let _ = writeln!(self.out, "  {} = call i32 @mylrt_str_eq(ptr {}, ptr {})", n, x, y);
                    let z = self.tmp("bool");
                    let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", z, n);
                    let bz = self.tmp("bool");
                    let _ = writeln!(self.out, "  {} = zext i1 {} to i32", bz, z);
                    bz
                } else {
                    let _ = writeln!(self.out, "  {} = call i32 @mylrt_str_cmp(ptr {}, ptr {})", n, x, y);
                    n
                }
            } else if matches!(b, Builtin::ValueEq) {
                let xtl = self.ty(&xt)?;
                let eq = self.tmp("eq");
                if is_float(&xt) {
                    let _ = writeln!(self.out, "  {} = fcmp oeq {} {}, {}", eq, xtl, x, y);
                } else {
                    let _ = writeln!(self.out, "  {} = icmp eq {} {}, {}", eq, xtl, x, y);
                }
                let n = self.tmp("bool");
                let _ = writeln!(self.out, "  {} = zext i1 {} to i32", n, eq);
                n
            } else {
                let xtl = self.ty(&xt)?;
                let lt = self.tmp("cmp");
                let gt = self.tmp("cmp");
                if is_float(&xt) {
                    let _ = writeln!(self.out, "  {} = fcmp olt {} {}, {}", lt, xtl, x, y);
                    let _ = writeln!(self.out, "  {} = fcmp ogt {} {}, {}", gt, xtl, x, y);
                } else {
                    let lp = if is_signed(&xt) { "slt" } else { "ult" };
                    let gp = if is_signed(&xt) { "sgt" } else { "ugt" };
                    let _ = writeln!(self.out, "  {} = icmp {} {} {}, {}", lt, lp, xtl, x, y);
                    let _ = writeln!(self.out, "  {} = icmp {} {} {}, {}", gt, gp, xtl, x, y);
                }
                let z = self.tmp("cmp0");
                let _ = writeln!(self.out, "  {} = select i1 {}, i32 -1, i32 0", z, lt);
                let n = self.tmp("cmp1");
                let _ = writeln!(self.out, "  {} = select i1 {}, i32 1, i32 {}", n, gt, z);
                n
            };
            self.store_slot(dst, &res)?;
            return Ok(());
        }
        let (v, st) = self.load(recv)?;
        let sty = llty(&st)?;
        let out_ty = ins.ty.clone();
        let flag = self.tmp("bflag");
        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", flag, sty, v);
        match b {
            Builtin::ResIsOk | Builtin::OptIsSome => self.store_slot(dst, &flag),
            Builtin::ResIsErr | Builtin::OptIsNone => {
                let n = self.tmp("bnot");
                let _ = writeln!(self.out, "  {} = xor i32 {}, 1", n, flag);
                self.store_slot(dst, &n)
            }
            Builtin::ResUnwrap => {
                let c = self.tmp("bcond");
                let cont = format!("L_bok_{}", self.tmp_n); self.tmp_n += 1;
                let fail = format!("L_bpan_{}", self.tmp_n); self.tmp_n += 1;
                let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", c, flag);
                let _ = writeln!(self.out, "  br i1 {}, label %{}, label %{}", c, cont, fail);
                let _ = writeln!(self.out, "{}:", fail);
                let _ = writeln!(self.out, "  call void @mylrt_panic_unwrap()");
                let _ = writeln!(self.out, "  unreachable");
                let _ = writeln!(self.out, "{}:", cont);
                let pv = self.tmp("bval");
                let _ = writeln!(self.out, "  {} = extractvalue {} {}, 1", pv, sty, v);
                self.store_slot(dst, &pv)
            }
            Builtin::ResUnwrapErr => {
                let c = self.tmp("bcond");
                let cont = format!("L_berr_{}", self.tmp_n); self.tmp_n += 1;
                let fail = format!("L_bpan_{}", self.tmp_n); self.tmp_n += 1;
                let _ = writeln!(self.out, "  {} = icmp eq i32 {}, 0", c, flag);
                let _ = writeln!(self.out, "  br i1 {}, label %{}, label %{}", c, cont, fail);
                let _ = writeln!(self.out, "{}:", fail);
                let _ = writeln!(self.out, "  call void @mylrt_panic_unwrap()");
                let _ = writeln!(self.out, "  unreachable");
                let _ = writeln!(self.out, "{}:", cont);
                let pv = self.tmp("berr");
                let _ = writeln!(self.out, "  {} = extractvalue {} {}, 2", pv, sty, v);
                self.store_slot(dst, &pv)
            }
            Builtin::OptUnwrap => {
                let c = self.tmp("bcond");
                let cont = format!("L_bok_{}", self.tmp_n); self.tmp_n += 1;
                let fail = format!("L_bpan_{}", self.tmp_n); self.tmp_n += 1;
                let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", c, flag);
                let _ = writeln!(self.out, "  br i1 {}, label %{}, label %{}", c, cont, fail);
                let _ = writeln!(self.out, "{}:", fail);
                let _ = writeln!(self.out, "  call void @mylrt_panic_unwrap()");
                let _ = writeln!(self.out, "  unreachable");
                let _ = writeln!(self.out, "{}:", cont);
                let pv = self.tmp("bval");
                let _ = writeln!(self.out, "  {} = extractvalue {} {}, 1", pv, sty, v);
                self.store_slot(dst, &pv)
            }
            Builtin::ResUnwrapOr | Builtin::OptUnwrapOr => {
                let (fb, _) = self.load(&args[0])?;
                let pv = self.tmp("bval");
                let _ = writeln!(self.out, "  {} = extractvalue {} {}, 1", pv, sty, v);
                let c = self.tmp("bcond");
                let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", c, flag);
                let n = self.tmp("bsel");
                let _ = writeln!(self.out, "  {} = select i1 {}, {} {}, {} {}", n, c, llty(&out_ty)?, pv, llty(&out_ty)?, fb);
                self.store_slot(dst, &n)
            }
            _ => Err(format!("LLVM 后端暂不支持内建方法 {:?}", b)),
        }
    }

    fn store_slot(&mut self, dst: u32, value: &str) -> Result<(), String> {
        let ty = self.slots.get(&dst).ok_or_else(|| format!("MIR dst %{} 无槽", dst))?.1.clone();
        let lt = self.ty(&ty)?;
        let slot = self.slots.get(&dst).unwrap().0.clone();
        let _ = writeln!(self.out, "  store {} {}, ptr %{}", lt, value, slot);
        Ok(())
    }

    fn lit(&mut self, s: &str) -> String {
        let idx = *self.str_n;
        *self.str_n += 1;
        let name = format!("@.myl_str_{}", idx);
        self.str_lits.push((name.clone(), s.to_string(), s.as_bytes().len()));
        name
    }

    fn inst(&mut self, ins: &crate::mir::MirInst) -> Result<(), String> {
        match &ins.kind {
            MirInstKind::Const(c) => {
                let dst = ins.dst.ok_or_else(|| "const 缺少 dst".to_string())?;
                let val = match c {
                    MirConst::Int(v) => Self::int_const_text(*v as u64, &ins.ty, true),
                    MirConst::IntU64(v) => Self::int_const_text(*v, &ins.ty, false),
                    MirConst::Float(v) => format!("{:?}", v),
                    MirConst::Bool(b) => if *b { "1".into() } else { "0".into() },
                    MirConst::Str(s) => {
                        let lit = self.lit(s);
                        let len = s.as_bytes().len();
                        let t = self.tmp("strlit");
                        let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 {})", t, lit, len);
                        t
                    }
                    MirConst::Null => "null".into(),
                };
                self.store_slot(dst, &val)
            }
            MirInstKind::Undef => {
                let dst = ins.dst.ok_or_else(|| "undef 缺少 dst".to_string())?;
                let lt = self.ty(&ins.ty)?;
                self.store_slot(dst, &format!("zeroinitializer"))?;
                let _ = lt;
                Ok(())
            }
            MirInstKind::Copy(v) => {
                let dst = ins.dst.ok_or_else(|| "copy 缺少 dst".to_string())?;
                let (x, _) = self.load(v)?;
                self.store_slot(dst, &x)
            }
            MirInstKind::Store { local, value } => {
                let (x, src) = self.load(value)?;
                let (slot, ty) = {
                    let (s, t) = self.local_of(*local)?;
                    (s.clone(), self.norm(t))
                };
                let x = if let Type::Iface(iid) = &ty {
                    self.wrap_iface(&x, &src, *iid)?
                } else { x };
                let lt = self.ty(&ty)?;
                let _ = writeln!(self.out, "  store {} {}, ptr %{}", lt, x, slot);
                Ok(())
            }
            MirInstKind::Bin(op, a, b) => {
                let dst = ins.dst.ok_or_else(|| "bin 缺少 dst".to_string())?;
                let (x, xt) = self.load(a)?;
                let (y, yt) = self.load(b)?;
                let xtl = self.ty(&xt)?;
                if matches!(xt, Type::Str) && matches!(op, BinOp::Add) {
                    let n = self.tmp("scat");
                    let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_concat(ptr {}, ptr {})", n, x, y);
                    self.store_slot(dst, &n)?;
                    return Ok(());
                }
                if matches!(xt, Type::Str) && matches!(op, BinOp::Eq | BinOp::Ne) {
                    let n = self.tmp("seq");
                    let _ = writeln!(self.out, "  {} = call i32 @mylrt_str_eq(ptr {}, ptr {})", n, x, y);
                    let n2 = if matches!(op, BinOp::Ne) {
                        let t = self.tmp("sne");
                        let _ = writeln!(self.out, "  {} = icmp eq i32 {}, 0", t, n);
                        let z = self.tmp("bool");
                        let _ = writeln!(self.out, "  {} = zext i1 {} to i32", z, t);
                        z
                    } else { n };
                    self.store_slot(dst, &n2)?;
                    return Ok(());
                }
                let res = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
                    | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        if is_float(&xt) {
                            let instr = match op {
                                BinOp::Add => "fadd", BinOp::Sub => "fsub", BinOp::Mul => "fmul",
                                BinOp::Div => "fdiv", BinOp::Rem => "frem",
                                _ => return Err(format!("LLVM 后端暂不支持浮点运算 {:?}", op)),
                            };
                            let n = self.tmp("fbin");
                            let _ = writeln!(self.out, "  {} = {} {} {}, {}", n, instr, xtl, x, y);
                            n
                        } else {
                            let instr = match op {
                                BinOp::Add => "add", BinOp::Sub => "sub", BinOp::Mul => "mul",
                                BinOp::Div => if is_signed(&xt) { "sdiv" } else { "udiv" },
                                BinOp::Rem => if is_signed(&xt) { "srem" } else { "urem" },
                                BinOp::BitAnd => "and", BinOp::BitOr => "or", BinOp::BitXor => "xor",
                                BinOp::Shl => "shl",
                                BinOp::Shr => if is_signed(&xt) { "ashr" } else { "lshr" },
                                _ => unreachable!(),
                            };
                            let n = self.tmp("ibin");
                            let _ = writeln!(self.out, "  {} = {} {} {}, {}", n, instr, xtl, x, y);
                            n
                        }
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let cmp = if is_float(&xt) {
                            let p = match op {
                                BinOp::Eq => "oeq", BinOp::Ne => "une", BinOp::Lt => "olt",
                                BinOp::Le => "ole", BinOp::Gt => "ogt", BinOp::Ge => "oge",
                                _ => unreachable!(),
                            };
                            let n = self.tmp("fcmp");
                            let _ = writeln!(self.out, "  {} = fcmp {} {} {}, {}", n, p, xtl, x, y);
                            n
                        } else {
                            let p = match op {
                                BinOp::Eq => "eq", BinOp::Ne => "ne", BinOp::Lt => "slt",
                                BinOp::Le => "sle", BinOp::Gt => "sgt", BinOp::Ge => "sge",
                                _ => unreachable!(),
                            };
                            let p = if is_signed(&xt) { p } else {
                                match op { BinOp::Lt => "ult", BinOp::Le => "ule", BinOp::Gt => "ugt", BinOp::Ge => "uge", _ => p }
                            };
                            let n = self.tmp("icmp");
                            let _ = writeln!(self.out, "  {} = icmp {} {} {}, {}", n, p, xtl, x, y);
                            n
                        };
                        let n = self.tmp("bool");
                        let _ = writeln!(self.out, "  {} = zext i1 {} to i32", n, cmp);
                        n
                    }
                    BinOp::AndAnd | BinOp::OrOr => {
                        let n1 = self.tmp("b1");
                        let n2 = self.tmp("b2");
                        let _ = writeln!(self.out, "  {} = icmp ne {} {}, 0", n1, xtl, x);
                        let _ = writeln!(self.out, "  {} = icmp ne {} {}, 0", n2, xtl, y);
                        let opn = if matches!(op, BinOp::AndAnd) { "and" } else { "or" };
                        let n3 = self.tmp("logic");
                        let _ = writeln!(self.out, "  {} = {} i1 {}, {}", n3, opn, n1, n2);
                        let n4 = self.tmp("bool");
                        let _ = writeln!(self.out, "  {} = zext i1 {} to i32", n4, n3);
                        n4
                    }
                };
                self.store_slot(dst, &res)
            }
            MirInstKind::Un(op, a) => {
                let dst = ins.dst.ok_or_else(|| "un 缺少 dst".to_string())?;
                let (x, xt) = self.load(a)?;
                let xtl = self.ty(&xt)?;
                let res = match op {
                    UnOp::Neg if is_float(&xt) => {
                        let n = self.tmp("neg");
                        let _ = writeln!(self.out, "  {} = fneg {} {}", n, xtl, x);
                        n
                    }
                    UnOp::Neg => {
                        let n = self.tmp("neg");
                        let _ = writeln!(self.out, "  {} = sub {} 0, {}", n, xtl, x);
                        n
                    }
                    UnOp::Not => {
                        let n = self.tmp("not");
                        let _ = writeln!(self.out, "  {} = icmp eq {} {}, 0", n, xtl, x);
                        let n2 = self.tmp("bool");
                        let _ = writeln!(self.out, "  {} = zext i1 {} to i32", n2, n);
                        n2
                    }
                    UnOp::BitNot => {
                        let n = self.tmp("bnot");
                        let _ = writeln!(self.out, "  {} = xor {} {}, -1", n, xtl, x);
                        n
                    }
                    UnOp::Deref => {
                        let lt = self.ty(&ins.ty)?;
                        let n = self.tmp("deref");
                        let _ = writeln!(self.out, "  {} = load {}, ptr {}", n, lt, x);
                        n
                    }
                    UnOp::AddrOf => {
                        // 操作数已经是 MIR 槽；取址返回该槽的 alloca 指针。
                        let slot = match a {
                            MirOperand::Value(v) => self.slot_of(*v)?.0.clone(),
                            MirOperand::Local(v) => self.local_of(*v)?.0.clone(),
                        };
                        format!("%{}", slot)
                    }
                };
                self.store_slot(dst, &res)
            }
            MirInstKind::Cast { value, target, checked } => {
                let dst = ins.dst.ok_or_else(|| "cast 缺少 dst".to_string())?;
                let (x, src) = self.load(value)?;
                if let Type::Iface(iid) = &src {
                    let out = self.cast_from_iface(&x, *iid, target, *checked, &ins.ty)?;
                    self.store_slot(dst, &out)?;
                    return Ok(());
                }
                if let Type::Iface(iid) = target {
                    let w = self.wrap_iface(&x, &src, *iid)?;
                    self.store_slot(dst, &w)?;
                    return Ok(());
                }
                let st = self.ty(&src)?;
                let dt = self.ty(target)?;
                let n = if src == *target {
                    x.clone()
                } else if is_int(&src) && is_int(target) && st == dt {
                    // 不同整型名义类型可能落到相同的 LLVM 宽度（如 i64 与 u64），无需 cast。
                    x.clone()
                } else if is_int(&src) && is_int(target) {
                    let sb = int_bits(&src)?;
                    let db = int_bits(target)?;
                    let op = if db < sb { "trunc" } else if is_signed(&src) { "sext" } else { "zext" };
                    let n = self.tmp("cast");
                    let _ = writeln!(self.out, "  {} = {} {} {} to {}", n, op, st, x, dt);
                    n
                } else if is_int(&src) && is_float(target) {
                    let op = if is_signed(&src) { "sitofp" } else { "uitofp" };
                    let n = self.tmp("cast");
                    let _ = writeln!(self.out, "  {} = {} {} {} to {}", n, op, st, x, dt);
                    n
                } else if is_float(&src) && is_int(target) {
                    let op = if is_signed(target) { "fptosi" } else { "fptoui" };
                    let n = self.tmp("cast");
                    let _ = writeln!(self.out, "  {} = {} {} {} to {}", n, op, st, x, dt);
                    n
                } else if is_float(&src) && is_float(target) {
                    let sb = if matches!(src, Type::F32) { 32 } else { 64 };
                    let db = if matches!(target, Type::F32) { 32 } else { 64 };
                    let op = if db > sb { "fpext" } else { "fptrunc" };
                    let n = self.tmp("cast");
                    let _ = writeln!(self.out, "  {} = {} {} {} to {}", n, op, st, x, dt);
                    n
                } else if matches!(src, Type::Bool) && is_int(target) {
                    let n = self.tmp("cast");
                    let _ = writeln!(self.out, "  {} = zext i32 {} to {}", n, x, dt);
                    n
                } else if is_int(&src) && matches!(target, Type::Bool) {
                    let n = self.tmp("cast");
                    let _ = writeln!(self.out, "  {} = icmp ne {} {}, 0", n, st, x);
                    let n2 = self.tmp("cast");
                    let _ = writeln!(self.out, "  {} = zext i1 {} to i32", n2, n);
                    n2
                } else {
                    return Err(format!("LLVM 后端暂不支持从 `{}` 转换到 `{}`", src, target));
                };
                self.store_slot(dst, &n)
            }
            MirInstKind::Call { callee, args } => {
                if let Res::Variant(eid, vidx) = &ins.res {
                    let dst = ins.dst.ok_or_else(|| "枚举构造缺少 dst".to_string())?;
                    let ty = ins.ty.clone();
                    let ety = Type::Enum(*eid);
                    let st = llty(&ety)?;
                    let e = self.enums.get(eid).ok_or_else(|| format!("LLVM 后端缺少 enum 元数据 {}", eid))?;
                    let n = e.variants.iter().map(|v| v.fields.len()).max().unwrap_or(0);
                    let arr_ty = format!("[{} x i64]", n);
                    let a0 = self.tmp("einit");
                    let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, i32 {}, 0", a0, st, vidx);
                    let mut agg = a0;
                    let mut arr = format!("zeroinitializer");
                    for (i, a) in args.iter().enumerate() {
                        let (v, vt) = self.load(a)?;
                        let v64 = self.as_i64(&v, &vt)?;
                        let na = self.tmp("earr");
                        let _ = writeln!(self.out, "  {} = insertvalue {} {}, i64 {}, {}", na, arr_ty, arr, v64, i);
                        arr = na.clone();
                        let ng = self.tmp("eagg");
                        let _ = writeln!(self.out, "  {} = insertvalue {} {}, {} {}, 1", ng, st, agg, arr_ty, na);
                        agg = ng;
                    }
                    if let Type::Iface(iid) = &ty {
                        let w = self.wrap_iface(&agg, &Type::Enum(*eid), *iid)?;
                        self.store_slot(dst, &w)?;
                    } else {
                        self.store_slot(dst, &agg)?;
                    }
                    return Ok(());
                }
                if matches!(callee.as_str(), "Ok" | "Err" | "Some" | "None") {
                    let dst = ins.dst.ok_or_else(|| "构造器缺少 dst".to_string())?;
                    let ty = ins.ty.clone();
                    let st = llty(&ty)?;
                    let a0 = self.tmp("agg");
                    let flag = if callee == "Err" { 0 } else { 1 };
                    let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, i32 {}, 0", a0, st, flag);
                    let mut agg = a0;
                    if callee != "None" {
                        let (mut v, vt) = self.load(&args[0])?;
                        let pt = match &ty {
                            Type::Result(a, _) if callee == "Ok" => (**a).clone(),
                            Type::Result(_, b) if callee == "Err" => (**b).clone(),
                            Type::Option(a) if callee == "Some" => (**a).clone(),
                            _ => return Err(format!("LLVM 构造器 `{}` 类型不匹配 `{}`", callee, ty)),
                        };
                        if let Type::Iface(iid) = &pt {
                            v = self.wrap_iface(&v, &vt, *iid)?;
                        }
                        let payload = llty(&pt)?;
                        let n = self.tmp("agg");
                        let _ = writeln!(self.out, "  {} = insertvalue {} {}, {} {}, {}", n, st, agg, payload, v, if callee == "Err" { 2 } else { 1 });
                        agg = n;
                    }
                    if callee == "None" {
                        let n = self.tmp("none");
                        let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, i32 0, 0", n, st);
                        agg = n;
                    }
                    self.store_slot(dst, &agg)?;
                    return Ok(());
                }
                let mut av: Vec<(String, Type)> = Vec::new();
                for a in args { av.push(self.load(a)?); }
                // 泛型静态方法的 callee 是源码模板名（如 `Wrap::Make`），
                // 实际函数名已实例化为 `Wrap__i64::Make`。必须优先按 MIR
                // 的 Res 声明 id 找函数，不能只按 callee 字符串查。
                let resolved = match &ins.res {
                    Res::Fn(mid) | Res::StaticMethod(_, mid) => self.all_funcs.values()
                        .find(|f| f.decl_id == *mid)
                        .cloned(),
                    _ => resolve_fn(callee, &self.all_funcs),
                };
                if let Some(cf) = &resolved {
                    for (i, (v, t)) in av.iter_mut().enumerate() {
                        if let Some(p) = cf.params.get(i) {
                            let nt = self.norm(&p.ty);
                            if let Type::Iface(iid) = nt {
                                *v = self.wrap_iface(v, t, iid)?;
                                *t = Type::Iface(iid);
                            } else {
                                *t = nt;
                            }
                        }
                    }
                }
                let d = if let Some(dst) = ins.dst { dst } else { 0 };
                let ret = if ins.ty == Type::Void { None } else { Some(self.ty(&ins.ty)?) };
                let target = if runtime_sig(callee).is_some() {
                    callee.to_string()
                } else if let Some(f) = &resolved {
                    sym(&f.name)
                } else if let Some(ext) = self.externs.get(callee) {
                    ext.symbol.clone()
                } else {
                    return Err(format!("LLVM 后端找不到函数 `{}`", callee));
                };
                let nl = self.tmp("call");
                let rets = ret.clone().unwrap_or_else(|| "void".into());
                let argtxt = av.iter()
                    .map(|(v, t)| Ok(format!("{} {}", llty(t)?, v)))
                    .collect::<Result<Vec<_>, String>>()?
                    .join(", ");
                if ret.is_some() {
                    let _ = writeln!(self.out, "  {} = call {} @{}({})", nl, rets, target, argtxt);
                    self.store_slot(d, &nl)?;
                } else {
                    let _ = writeln!(self.out, "  call void @{}({})", target, argtxt);
                }
                Ok(())
            }
            MirInstKind::Interp { parts } => {
                let dst = ins.dst.ok_or_else(|| "interp 缺少 dst".to_string())?;
                let empty = self.lit("");
                let n0 = self.tmp("interp");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 0)", n0, empty);
                let mut acc = n0;
                for part in parts {
                    let piece = match part {
                        crate::mir::MirInterpPart::Lit(t) if t.is_empty() => continue,
                        crate::mir::MirInterpPart::Lit(t) => {
                            let l = self.lit(t);
                            let n = self.tmp("ilit");
                            let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 {})", n, l, t.as_bytes().len());
                            n
                        }
                        crate::mir::MirInterpPart::Value(v) => {
                            let (x, ty) = self.load(v)?;
                            self.fmt_value(&x, &ty)?
                        }
                    };
                    let n = self.tmp("cat");
                    let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_concat(ptr {}, ptr {})", n, acc, piece);
                    acc = n;
                }
                self.store_slot(dst, &acc)
            }
            MirInstKind::New { class, args } => {
                let dst = ins.dst.ok_or_else(|| "new 缺少 dst".to_string())?;
                let cid = match &ins.res {
                    Res::Ctor(c) if self.classes.contains_key(c) => Some(*c),
                    _ => None,
                }.or_else(|| {
                    self.classes.values().find(|c| {
                        let short = c.name.rsplit("::").next().unwrap_or("");
                        short == class || short.starts_with(&format!("{}__", class))
                    }).map(|c| c.decl_id)
                }).or_else(|| {
                    self.classes.values().find(|c| c.name.ends_with(&format!("::{}", class))).map(|c| c.decl_id)
                }).ok_or_else(|| format!("LLVM 后端找不到类 `{}`", class))?;
                let c = self.classes.get(&cid).ok_or_else(|| format!("LLVM 后端缺少类元数据 `{}`", cid))?;
                let c = *c;
                let size = self.tmp("size");
                let gep = self.tmp("size_gep");
                let _ = writeln!(self.out, "  {} = getelementptr %C_{}, ptr null, i32 1", gep, cid);
                let _ = writeln!(self.out, "  {} = ptrtoint ptr {} to i64", size, gep);
                let obj = self.tmp("obj");
                let _ = writeln!(self.out, "  {} = call ptr @myrt_alloc(i64 {})", obj, size);
                let _ = writeln!(self.out, "  call void @llvm.memset.p0.i64(ptr {}, i8 0, i64 {}, i1 false)", obj, size);
                for (fi, fld) in c.fields.iter().enumerate() {
                    if fld.is_static { continue; }
                    if let Some(init) = &fld.init {
                        let v = self.const_value(init, &fld.ty)?;
                        let p = self.tmp("fp");
                        let _ = writeln!(self.out, "  {} = getelementptr inbounds %C_{}, ptr {}, i32 0, i32 {}", p, cid, obj, fi);
                        let _ = writeln!(self.out, "  store {} {}, ptr {}", llty(&fld.ty)?, v, p);
                        if is_gc_pointer(&fld.ty) {
                            let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, v);
                        } else if matches!(fld.ty, Type::Iface(_)) {
                            let obj = self.tmp("ifobj");
                            let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, llty(&fld.ty)?, v);
                            let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, obj);
                        } else if is_gc_aggregate(&fld.ty) {
                            let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, p);
                        }
                    }
                }
                if let Some(ctor) = self.all_funcs.values().find(|f| f.owner == Some(cid) && f.is_ctor) {
                    let ctor = ctor.clone();
                    let _ = self.call_fn(&ctor, Some((&obj, Type::Class(cid))), args)?;
                }
                if let Type::Iface(iid) = &ins.ty {
                    let w = self.wrap_iface(&obj, &Type::Class(cid), *iid)?;
                    self.store_slot(dst, &w)
                } else {
                    self.store_slot(dst, &obj)
                }
            }
            MirInstKind::Field { base, .. } => {
                let dst = ins.dst.ok_or_else(|| "field 缺少 dst".to_string())?;
                let (cid, idx) = match &ins.res {
                    Res::Field(cid, idx) => (*cid, *idx),
                    _ => return Err("LLVM 后端 field 缺少解析结果".into()),
                };
                let (b, _) = self.load(base)?;
                let p = self.tmp("fld");
                let _ = writeln!(self.out, "  {} = getelementptr inbounds %C_{}, ptr {}, i32 0, i32 {}", p, cid, b, idx);
                let n = self.tmp("fldv");
                let _ = writeln!(self.out, "  {} = load {}, ptr {}", n, llty(&ins.ty)?, p);
                self.store_slot(dst, &n)
            }
            MirInstKind::StoreField { base, value, .. } => {
                let (cid, idx) = match &ins.res {
                    Res::Field(cid, idx) => (*cid, *idx),
                    _ => return Err("LLVM 后端 store.field 缺少解析结果".into()),
                };
                let (b, _) = self.load(base)?;
                let (v, _) = self.load(value)?;
                let ft = self.classes.get(&cid)
                    .and_then(|c| c.fields.get(idx)).map(|f| f.ty.clone())
                    .ok_or_else(|| format!("LLVM 后端缺少字段元数据 {}.{}", cid, idx))?;
                let p = self.tmp("sfld");
                let _ = writeln!(self.out, "  {} = getelementptr inbounds %C_{}, ptr {}, i32 0, i32 {}", p, cid, b, idx);
                let _ = writeln!(self.out, "  store {} {}, ptr {}", llty(&ft)?, v, p);
                if is_gc_pointer(&ft) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, v);
                } else if matches!(ft, Type::Iface(_)) {
                    let obj = self.tmp("ifobj");
                    let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, llty(&ft)?, v);
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, obj);
                } else if is_gc_aggregate(&ft) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, p);
                }
                Ok(())
            }
            MirInstKind::MethodCall { recv, name, args } => {
                // string / array 的 len() 是内建；class 的 len() 必须走普通方法调用。
                if name == "len" && args.is_empty() {
                    let (x, ty) = self.load(recv)?;
                    let n = match ty {
                        Type::Str => {
                            let n = self.tmp("strlen");
                            let _ = writeln!(self.out, "  {} = call i64 @myrt_str_len(ptr {})", n, x);
                            n
                        }
                        Type::Array(_) => {
                            let n = self.tmp("arrlen");
                            let _ = writeln!(self.out, "  {} = load i64, ptr {}", n, x);
                            n
                        }
                        _ => String::new(),
                    };
                    if !n.is_empty() {
                        let dst = ins.dst.ok_or_else(|| "methodcall 缺少 dst".to_string())?;
                        let n2 = if matches!(ins.ty, Type::I32 | Type::U32 | Type::I16 | Type::U16 | Type::I8 | Type::U8) {
                            let t = self.tmp("len");
                            let _ = writeln!(self.out, "  {} = trunc i64 {} to {}", t, n, llty(&ins.ty)?);
                            t
                        } else { n };
                        self.store_slot(dst, &n2)?;
                        return Ok(());
                    }
                }
                if let Res::Builtin(b) = &ins.res {
                    if !matches!(b, crate::ast::Builtin::StrLen | crate::ast::Builtin::ArrLen) {
                        return self.builtin(b.clone(), recv, args, ins);
                    }
                }
                let (rv, rt) = self.load(recv)?;
                if let Type::Iface(iid) = &rt {
                    let mid = match &ins.res { Res::Method(m) => *m, _ => return Err("LLVM 接口方法缺少声明 id".into()) };
                    let (diid, slot) = *self.iface_methods.get(&mid)
                        .ok_or_else(|| format!("LLVM 后端找不到接口方法 {}", mid))?;
                    let mut av = Vec::new();
                    for a in args { av.push(self.load(a)?); }
                    let out = self.iface_dispatch(&rv, diid, slot, &ins.ty, &av)?;
                    if ins.ty != Type::Void {
                        let dst = ins.dst.ok_or_else(|| "方法调用缺少 dst".to_string())?;
                        self.store_slot(dst, &out)?;
                    }
                    return Ok(());
                }
                let recv_cid = match &rt {
                    Type::Class(cid) => Some(*cid),
                    _ => None,
                };
                let f = match &ins.res {
                    Res::Method(mid) => self.all_funcs.values().find(|f| f.decl_id == *mid).cloned()
                        .or_else(|| {
                            let cid = recv_cid?;
                            let mut it = self.all_funcs.values().filter(|f| {
                                f.owner == Some(cid) && f.name.rsplit("::").next() == Some(name.as_str())
                            });
                            let first = it.next().cloned();
                            if it.next().is_none() { first } else { None }
                        }),
                    Res::StaticMethod(_, mid) | Res::Fn(mid) => self.all_funcs.values().find(|f| f.decl_id == *mid).cloned(),
                    Res::Builtin(crate::ast::Builtin::StrLen) => None,
                    _ => None,
                };
                let f = match f {
                    Some(f) => f,
                    None => return Err(format!("LLVM 后端暂不支持方法调用 {:?}", ins.kind)),
                };
                let ret = self.call_fn(&f, Some((&rv, rt)), args)?;
                if let Some(v) = ret {
                    let d = ins.dst.ok_or_else(|| "methodcall 缺少 dst".to_string())?;
                    self.store_slot(d, &v)?;
                }
                Ok(())
            }
            MirInstKind::StaticRef(_) => {
                let dst = ins.dst.ok_or_else(|| "static 缺少 dst".to_string())?;
                if let Res::Variant(eid, vidx) = &ins.res {
                    // 无载荷 enum 变体先构造具体 enum；如果插值/接口转换把它
                    // 改成了 interface 类型，再包成 { obj, vtable }。
                    let et = Type::Enum(*eid);
                    let st = llty(&et)?;
                    let n = self.tmp("eunit");
                    let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, i32 {}, 0", n, st, vidx);
                    if let Type::Iface(iid) = ins.ty {
                        let w = self.wrap_iface(&n, &et, iid)?;
                        self.store_slot(dst, &w)?;
                    } else {
                        if !matches!(ins.ty, Type::Enum(_)) {
                            return Err("LLVM 枚举静态构造类型不匹配".into());
                        }
                        self.store_slot(dst, &n)?;
                    }
                    return Ok(());
                }
                let (cid, idx) = match &ins.res {
                    Res::StaticField(cid, idx) => (*cid, *idx),
                    _ => return Err("LLVM 后端 static 缺少解析结果".into()),
                };
                let n = self.tmp("static");
                let _ = writeln!(self.out, "  {} = load {}, ptr @S_{}_{}", n, llty(&ins.ty)?, cid, idx);
                self.store_slot(dst, &n)
            }
            MirInstKind::StoreStatic { value, .. } => {
                let (cid, idx) = match &ins.res {
                    Res::StaticField(cid, idx) => (*cid, *idx),
                    _ => return Err("LLVM 后端 static.store 缺少解析结果".into()),
                };
                let (v, _) = self.load(value)?;
                let ft = self.classes.get(&cid)
                    .and_then(|c| c.fields.get(idx)).map(|f| f.ty.clone())
                    .ok_or_else(|| format!("LLVM 后端缺少静态字段元数据 {}.{}", cid, idx))?;
                let _ = writeln!(self.out, "  store {} {}, ptr @S_{}_{}", llty(&ft)?, v, cid, idx);
                if is_gc_pointer(&ft) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr @S_{}_{}, ptr {})", cid, idx, v);
                } else if matches!(ft, Type::Iface(_)) {
                    let obj = self.tmp("ifobj");
                    let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, llty(&ft)?, v);
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr @S_{}_{}, ptr {})", cid, idx, obj);
                } else if is_gc_aggregate(&ft) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr @S_{}_{}, ptr @S_{}_{})", cid, idx, cid, idx);
                }
                Ok(())
            }
            MirInstKind::ArrayNew { len, .. } => {
                let dst = ins.dst.ok_or_else(|| "array.new 缺少 dst".to_string())?;
                let elem = match &ins.ty { Type::Array(e) => (**e).clone(), _ => return Err("array.new 结果不是数组".into()) };
                let esz = self.elem_size(&elem)?;
                let (l, lt) = self.load(len)?;
                let l64 = if matches!(lt, Type::I64 | Type::U64) { l.clone() } else {
                    let n = self.tmp("alen");
                    let _ = writeln!(self.out, "  {} = sext {} {} to i64", n, llty(&lt)?, l);
                    n
                };
                let a = self.tmp("arr");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_arr_alloc(i64 {}, i64 {})", a, esz, l64);
                self.store_slot(dst, &a)
            }
            MirInstKind::ArrayLit { values, .. } => {
                let dst = ins.dst.ok_or_else(|| "array.lit 缺少 dst".to_string())?;
                let elem = match &ins.ty { Type::Array(e) => (**e).clone(), _ => return Err("array.lit 结果不是数组".into()) };
                let esz = self.elem_size(&elem)?;
                let n = values.len();
                let a = self.tmp("arr");
                let _ = writeln!(self.out, "  {} = call ptr @mylrt_arr_alloc(i64 {}, i64 {})", a, esz, n);
                for (i, v) in values.iter().enumerate() {
                    let (x, _) = self.load(v)?;
                    let off = 8 + (i as i64) * esz;
                    let p = self.tmp("ae");
                    let _ = writeln!(self.out, "  {} = getelementptr i8, ptr {}, i64 {}", p, a, off);
                    let _ = writeln!(self.out, "  store {} {}, ptr {}", llty(&elem)?, x, p);
                    if is_gc_pointer(&elem) {
                        let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, x);
                    } else if matches!(elem, Type::Iface(_)) {
                        let obj = self.tmp("aeobj");
                        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, llty(&elem)?, x);
                        let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, obj);
                    } else if is_gc_aggregate(&elem) {
                        let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, p);
                    }
                }
                self.store_slot(dst, &a)
            }
            MirInstKind::Index { base, index } => {
                let dst = ins.dst.ok_or_else(|| "index 缺少 dst".to_string())?;
                let (b, bt) = self.load(base)?;
                let elem = Self::elem_ty(&bt)?;
                let (i, it) = self.load(index)?;
                let i64v = if matches!(it, Type::I64 | Type::U64) { i.clone() } else {
                    let n = self.tmp("i64");
                    let _ = writeln!(self.out, "  {} = sext {} {} to i64", n, llty(&it)?, i);
                    n
                };
                let len = self.tmp("len");
                let _ = writeln!(self.out, "  {} = load i64, ptr {}", len, b);
                let chk = self.tmp("chk");
                let _ = writeln!(self.out, "  {} = call i64 @mylrt_arr_index(i64 {}, i64 {})", chk, len, i64v);
                let esz = self.elem_size(&elem)?;
                let p = self.tmp("ap");
                let _ = writeln!(self.out, "  {} = getelementptr i8, ptr {}, i64 8", p, b);
                let q = self.tmp("ae");
                let _ = writeln!(self.out, "  {} = getelementptr {}, ptr {}, i64 {}", q, llty(&elem)?, p, chk);
                let v = self.tmp("av");
                let _ = writeln!(self.out, "  {} = load {}, ptr {}", v, llty(&elem)?, q);
                let _ = esz;
                self.store_slot(dst, &v)
            }
            MirInstKind::StoreIndex { base, index, value } => {
                let (b, bt) = self.load(base)?;
                let elem = Self::elem_ty(&bt)?;
                let (i, it) = self.load(index)?;
                let (v, _) = self.load(value)?;
                let i64v = if matches!(it, Type::I64 | Type::U64) { i.clone() } else {
                    let n = self.tmp("i64");
                    let _ = writeln!(self.out, "  {} = sext {} {} to i64", n, llty(&it)?, i);
                    n
                };
                let len = self.tmp("len");
                let _ = writeln!(self.out, "  {} = load i64, ptr {}", len, b);
                let chk = self.tmp("chk");
                let _ = writeln!(self.out, "  {} = call i64 @mylrt_arr_index(i64 {}, i64 {})", chk, len, i64v);
                let p = self.tmp("ap");
                let _ = writeln!(self.out, "  {} = getelementptr i8, ptr {}, i64 8", p, b);
                let q = self.tmp("ae");
                let _ = writeln!(self.out, "  {} = getelementptr {}, ptr {}, i64 {}", q, llty(&elem)?, p, chk);
                let _ = writeln!(self.out, "  store {} {}, ptr {}", llty(&elem)?, v, q);
                if is_gc_pointer(&elem) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", q, v);
                } else if matches!(elem, Type::Iface(_)) {
                    let obj = self.tmp("aeobj");
                    let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, llty(&elem)?, v);
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", q, obj);
                } else if is_gc_aggregate(&elem) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", q, q);
                }
                Ok(())
            }
            MirInstKind::Safepoint { roots } => {
                if let (Some(fid), Some(rf)) = (self.frame_id.clone(), self.root_frame.clone()) {
                    let n = self.root_n;
                    // 根描述表必须按当前 safepoint 的根连续填充；全局并集下标只用于
                    // 查槽和尺寸，不能直接当描述表下标，否则 count 与下标会错位。
                    let mut ri = 0usize;
                    for r in roots {
                        let (slot, size) = match r {
                            MirOperand::Local(l) => {
                                let idx = *self.root_local_idx.get(l).ok_or_else(|| format!("safepoint 根 $%{} 未在根表中", l))?;
                                (format!("l_{}", l), self.root_sizes[idx])
                            }
                            MirOperand::Value(v) => {
                                let idx = *self.root_value_idx.get(v).ok_or_else(|| format!("safepoint 根 %{} 未在根表中", v))?;
                                (format!("v_{}", v), self.root_sizes[idx])
                            }
                        };
                        let d = self.tmp("rd");
                        let _ = writeln!(self.out, "  {} = getelementptr [{} x %MylRootDesc], ptr %{}, i64 0, i64 {}", d, n, rf, ri);
                        let f0 = self.tmp("rd0");
                        let _ = writeln!(self.out, "  {} = getelementptr %MylRootDesc, ptr {}, i32 0, i32 0", f0, d);
                        let _ = writeln!(self.out, "  store ptr %{}, ptr {}", slot, f0);
                        let f1 = self.tmp("rd1");
                        let _ = writeln!(self.out, "  {} = getelementptr %MylRootDesc, ptr {}, i32 0, i32 1", f1, d);
                        let _ = writeln!(self.out, "  store i64 {}, ptr {}", size, f1);
                        ri += 1;
                    }
                    let _ = writeln!(self.out, "  call void @myrt_gc_frame_set_roots(i64 {}, ptr %{}, i64 {})", fid, rf, ri);
                }
                let _ = writeln!(self.out, "  call void @myrt_gc_safepoint()");
                Ok(())
            }
            MirInstKind::StorePtr { ptr, value } => {
                let (p, _) = self.load(ptr)?;
                let (v, vt) = self.load(value)?;
                let lt = self.ty(&vt)?;
                let _ = writeln!(self.out, "  store {} {}, ptr {}", lt, v, p);
                if is_gc_pointer(&vt) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, v);
                } else if is_gc_aggregate(&vt) {
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, p);
                } else if let Type::Iface(_) = &vt {
                    let it = self.ty(&vt)?;
                    let obj = self.tmp("ifobj");
                    let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", obj, it, v);
                    let _ = writeln!(self.out, "  call void @myrt_gc_write_barrier_owner(ptr {}, ptr {})", p, obj);
                }
                Ok(())
            }
            MirInstKind::Box { value, from } => {
                let dst = ins.dst.ok_or_else(|| "box 缺少 dst".to_string())?;
                let iid = match &ins.ty { Type::Iface(i) => *i, _ => return Err("LLVM Box 目标不是接口".into()) };
                let (x, xt) = self.load(value)?;
                let key = type_key(from);
                let bt = format!("%B_{}", key);
                let slot = self.tmp("box");
                let _ = writeln!(self.out, "  {} = alloca {}", slot, bt);
                let _ = writeln!(self.out, "  store {} {}, ptr {}", llty(from)?, x, slot);
                let it = format!("%I_{}", iid);
                let v0 = self.tmp("bv0");
                let v1 = self.tmp("bv1");
                let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, ptr {}, 0", v0, it, slot);
                let _ = writeln!(self.out, "  {} = insertvalue {} {}, ptr @VT_BOX_{}_{}, 1", v1, it, v0, key, iid);
                let _ = xt;
                self.store_slot(dst, &v1)
            }
            MirInstKind::Try { value } => {
                let dst = ins.dst.ok_or_else(|| "try 缺少 dst".to_string())?;
                let (v, st) = self.load(value)?;
                let sty = llty(&st)?;
                let okf = self.tmp("tryok");
                let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", okf, sty, v);
                let c = self.tmp("tryc");
                let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", c, okf);
                let fail = format!("L_tryf_{}", self.tmp_n); self.tmp_n += 1;
                let cont = format!("L_tryc_{}", self.tmp_n); self.tmp_n += 1;
                let _ = writeln!(self.out, "  br i1 {}, label %{}, label %{}", c, cont, fail);
                let _ = writeln!(self.out, "{}:", fail);
                let ret_ty = self.f.ret.clone();
                match (&st, &ret_ty) {
                    (Type::Result(a, b), Type::Result(_, rb)) => {
                        let err = self.tmp("trye");
                        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 2", err, sty, v);
                        let r0 = self.tmp("r0");
                        let r1 = self.tmp("r1");
                        let _ = writeln!(self.out, "  {} = insertvalue {} zeroinitializer, i32 0, 0", r0, llty(&ret_ty)?);
                        let _ = writeln!(self.out, "  {} = insertvalue {} {}, {} {}, 2", r1, llty(&ret_ty)?, r0, llty(rb)?, err);
                        let _ = writeln!(self.out, "  ret {} {}", llty(&ret_ty)?, r1);
                        let _ = a;
                    }
                    (Type::Option(_), Type::Option(_)) => {
                        let _ = writeln!(self.out, "  ret {} zeroinitializer", llty(&ret_ty)?);
                    }
                    _ => return Err("LLVM 后端 ? 的源/目标协议不一致".into()),
                }
                let _ = writeln!(self.out, "{}:", cont);
                let pv = self.tmp("tryv");
                let _ = writeln!(self.out, "  {} = extractvalue {} {}, 1", pv, sty, v);
                self.store_slot(dst, &pv)
            }
            MirInstKind::MatchTest { subject, pattern } => {
                let dst = ins.dst.ok_or_else(|| "match.test 缺少 dst".to_string())?;
                let (v, st) = self.load(subject)?;
                let cond: String = match pattern {
                    Pattern::Wildcard => "1".into(),
                    Pattern::Int(n) if is_int(&st) => {
                        let c = self.tmp("mcmp");
                        let _ = writeln!(self.out, "  {} = icmp eq {} {}, {}", c, llty(&st)?, v, n);
                        c
                    }
                    Pattern::IntU64(n) if is_int(&st) => {
                        let c = self.tmp("mcmp");
                        let _ = writeln!(self.out, "  {} = icmp eq {} {}, {}", c, llty(&st)?, v, n);
                        c
                    }
                    Pattern::Float(n) if is_float(&st) => {
                        let c = self.tmp("mcmp");
                        let _ = writeln!(self.out, "  {} = fcmp oeq {} {}, {}", c, llty(&st)?, v, n);
                        c
                    }
                    Pattern::Bool(b) => {
                        let c = self.tmp("mcmp");
                        let _ = writeln!(self.out, "  {} = icmp eq i32 {}, {}", c, v, if *b { 1 } else { 0 });
                        c
                    }
                    Pattern::Str(sv) => {
                        let p = self.lit(sv);
                        let q = self.tmp("mstr");
                        let _ = writeln!(self.out, "  {} = call ptr @mylrt_str_lit(ptr {}, i64 {})", q, p, sv.len());
                        let e = self.tmp("meq");
                        let _ = writeln!(self.out, "  {} = call i32 @mylrt_str_eq(ptr {}, ptr {})", e, v, q);
                        let c = self.tmp("mcmp");
                        let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", c, e);
                        c
                    }
                    Pattern::Null => {
                        let c = self.tmp("mcmp");
                        let _ = writeln!(self.out, "  {} = icmp eq ptr {}, null", c, v);
                        c
                    }
                    Pattern::Ok(_) | Pattern::Some(_) => {
                        let sty = llty(&st)?;
                        let f = self.tmp("mflag");
                        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", f, sty, v);
                        let c = self.tmp("mok");
                        let _ = writeln!(self.out, "  {} = icmp ne i32 {}, 0", c, f);
                        c
                    }
                    Pattern::Err(_) | Pattern::None => {
                        let sty = llty(&st)?;
                        let f = self.tmp("mflag");
                        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", f, sty, v);
                        let c = self.tmp("merr");
                        let _ = writeln!(self.out, "  {} = icmp eq i32 {}, 0", c, f);
                        c
                    }
                    Pattern::Variant { enum_id, vidx, .. } => {
                        let ety = format!("%E_{}", enum_id);
                        let tag = self.tmp("etag");
                        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 0", tag, ety, v);
                        let c = self.tmp("ecmp");
                        let _ = writeln!(self.out, "  {} = icmp eq i32 {}, {}", c, tag, vidx);
                        c
                    }
                    _ => return Err(format!("LLVM 后端暂不支持模式 {:?}", pattern)),
                };
                let n = self.tmp("mbool");
                if cond == "1" {
                    let _ = writeln!(self.out, "  {} = zext i1 true to i32", n);
                } else {
                    let _ = writeln!(self.out, "  {} = zext i1 {} to i32", n, cond);
                }
                self.store_slot(dst, &n)
            }
            MirInstKind::BindPattern { subject, pattern, locals } => {
                let (v, st) = self.load(subject)?;
                match pattern {
                    Pattern::Variant { enum_id, .. } => {
                        let ety = format!("%E_{}", enum_id);
                        let e = self.enums.get(enum_id).ok_or_else(|| format!("LLVM 后端缺少 enum 元数据 {}", enum_id))?;
                        let n = e.variants.iter().map(|x| x.fields.len()).max().unwrap_or(0);
                        let arr_ty = format!("[{} x i64]", n);
                        let arr = self.tmp("ebind");
                        let _ = writeln!(self.out, "  {} = extractvalue {} {}, 1", arr, ety, v);
                        for (i, l) in locals.iter().enumerate() {
                            let (slot, ty) = self.local_of(*l)?;
                            let slot = slot.clone();
                            let ty = ty.clone();
                            let q = self.tmp("ebv");
                            let _ = writeln!(self.out, "  {} = extractvalue {} {}, {}", q, arr_ty, arr, i);
                            let cv = self.from_i64(&q, &ty)?;
                            let _ = writeln!(self.out, "  store {} {}, ptr %{}", llty(&ty)?, cv, slot);
                        }
                        let _ = st;
                        Ok(())
                    }
                    Pattern::Ok(_) | Pattern::Some(_) | Pattern::Err(_) => {
                        let sty = llty(&st)?;
                        let field = match pattern {
                            Pattern::Ok(_) | Pattern::Some(_) => 1,
                            Pattern::Err(_) => 2,
                            _ => unreachable!(),
                        };
                        if locals.is_empty() { return Ok(()); }
                        let pv = self.tmp("bind");
                        let _ = writeln!(self.out, "  {} = extractvalue {} {}, {}", pv, sty, v, field);
                        let (slot, ty) = self.local_of(locals[0])?;
                        let slot = slot.clone();
                        let lt = llty(&ty)?;
                        let _ = writeln!(self.out, "  store {} {}, ptr %{}", lt, pv, slot);
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
            other => Err(format!("LLVM 后端暂不支持 MIR 指令 {:?}", other)),
        }
    }
}

fn int_bits(t: &Type) -> Result<u32, String> {
    Ok(match t {
        Type::I8 | Type::U8 => 8, Type::I16 | Type::U16 => 16,
        Type::I32 | Type::U32 => 32, Type::I64 | Type::U64 => 64,
        _ => return Err(format!("`{}` 不是整数类型", t)),
    })
}

fn collect_agg(t: &Type, out: &mut Vec<Type>, seen: &mut HashSet<String>) {
    match t {
        Type::Option(e) => {
            collect_agg(e, out, seen);
            let k = type_key(t);
            if seen.insert(k) { out.push(t.clone()); }
        }
        Type::Result(a, b) => {
            collect_agg(a, out, seen);
            collect_agg(b, out, seen);
            let k = type_key(t);
            if seen.insert(k) { out.push(t.clone()); }
        }
        Type::Array(e) => collect_agg(e, out, seen),
        _ => {}
    }
}

fn resolve_fn<'a>(callee: &str, map: &HashMap<&str, &'a MirFunction>) -> Option<&'a MirFunction> {
    if let Some(f) = map.get(callee) { return Some(f); }
    // MIR 里跨模块调用可能只保留短名（如 `fmt_i64`），按唯一后缀解析。
    let suffix = format!("::{}", callee);
    let mut found = None;
    for (name, f) in map {
        if name.ends_with(&suffix) {
            if found.is_some() { return None; }
            found = Some(*f);
        }
    }
    found
}

fn visit_fn<'a>(
    f: &'a MirFunction,
    map: &HashMap<&str, &'a MirFunction>,
    seen: &mut HashMap<String, u8>,
    order: &mut Vec<String>,
) {
    match seen.get(&f.name) {
        Some(2) => return,
        Some(1) => return,
        _ => {}
    }
    seen.insert(f.name.clone(), 1);
    for b in &f.blocks {
        for i in &b.insts {
            if let MirInstKind::Call { callee, .. } = &i.kind {
                if let Some(g) = resolve_fn(callee, map) {
                    visit_fn(g, map, seen, order);
                }
            }
        }
    }
    seen.insert(f.name.clone(), 2);
    order.push(f.name.clone());
}

fn lib_ffi_ok(t: &Type) -> bool {
    match t {
        Type::Void | Type::I8 | Type::I16 | Type::I32 | Type::I64
        | Type::U8 | Type::U16 | Type::U32 | Type::U64 | Type::F32 | Type::F64
        | Type::Bool | Type::Str | Type::Ptr(_) | Type::Class(_) | Type::Iface(_) | Type::Array(_) => true,
        _ => false,
    }
}

fn abi_size_align(t: &Type, ir: &MirProgram) -> Option<(i64, i64)> {
    let norm = |t: &Type| -> Type {
        if let Type::Class(id) = t {
            if ir.enums.iter().any(|e| e.decl_id == *id) { return Type::Enum(*id); }
        }
        t.clone()
    };
    match norm(t) {
        Type::I8 | Type::U8 | Type::Bool => Some((1, 1)),
        Type::I16 | Type::U16 => Some((2, 2)),
        Type::I32 | Type::U32 | Type::F32 => Some((4, 4)),
        Type::I64 | Type::U64 | Type::F64 => Some((8, 8)),
        Type::Str | Type::Class(_) | Type::Array(_) | Type::Ptr(_) => Some((8, 8)),
        Type::Iface(_) => Some((16, 8)),
        Type::Enum(id) => {
            let n = ir.enums.iter().find(|e| e.decl_id == id)
                .map(|e| e.variants.iter().map(|v| v.fields.len()).max().unwrap_or(0))
                .unwrap_or(0);
            Some((8 + (n as i64) * 8, 8))
        }
        Type::Option(e) => {
            let (ps, pa) = abi_size_align(&e, ir)?;
            Some(layout_fields(&[(4, 4), (ps, pa)]))
        }
        Type::Result(a, b) => {
            let (as_, aa) = abi_size_align(&a, ir)?;
            let (bs, ba) = abi_size_align(&b, ir)?;
            Some(layout_fields(&[(4, 4), (as_, aa), (bs, ba)]))
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibRetAbi { Value, PackedI64, PackedI64Pair }

/// LLVM 库导出返回 ABI 与 C 头文件的匹配策略：
/// - 普通类型：直接返回 LLVM 类型；
/// - 8 字节 Option：LLVM 聚合返回走 RAX/RDX，C 侧把 8 字节聚合打包进 RAX，
///   因此导出包装改为 i64 打包；
/// - 16 字节 Option/Result：C 侧使用隐藏 sret 指针，导出包装改成 void + sret。
fn lib_ret_abi(t: &Type, ir: &MirProgram) -> Option<LibRetAbi> {
    if lib_ffi_ok(t) { return Some(LibRetAbi::Value); }
    let (size, _) = abi_size_align(t, ir)?;
    match t {
        Type::Option(_) if size == 8 => Some(LibRetAbi::PackedI64),
        Type::Option(_) if size == 16 => Some(LibRetAbi::Value),
        Type::Result(_, _) if size == 16 => Some(LibRetAbi::PackedI64Pair),
        _ => None,
    }
}

fn lib_ty(t: &Type, ir: &MirProgram) -> Result<String, String> {
    if let Type::Class(id) = t {
        if ir.enums.iter().any(|e| e.decl_id == *id) {
            return llty(&Type::Enum(*id));
        }
    }
    llty(t)
}

fn emit_lib(prog: &Program, ir: &MirProgram, mode: ExportMode, native_externs: bool) -> Result<String, String> {
    let mut text = emit(ir, false, native_externs, Some(prog))?;
    let stem = prog.modules.get(prog.entry).map(|m| m.stem.clone()).unwrap_or_else(|| "mylis".into());
    let dll = if mode == ExportMode::DllExport { "dllexport " } else { "" };
    let has_statics = ir.classes.iter().any(|c| c.fields.iter().any(|f| f.is_static));
    let _ = writeln!(text, "\ndefine {}void @{}_init() {{", dll, stem);
    let _ = writeln!(text, "  call void @myrt_init()");
    if has_statics { let _ = writeln!(text, "  call void @myl_init_statics()"); }
    let _ = writeln!(text, "  ret void");
    let _ = writeln!(text, "}}\n");
    for d in &prog.decls {
        if d.kind != crate::ast::DeclKind::Fn || d.owner.is_some() || !d.is_pub { continue; }
        let exp = match crate::codegen::export_name(d) { Some(n) => n, None => continue };
        let f = match ir.funcs.iter().find(|f| f.name == d.name || f.name.ends_with(&format!("::{}", d.name))) {
            Some(f) => f, None => continue,
        };
        let ret_abi = match lib_ret_abi(&d.ret, ir) {
            Some(a) => a,
            None => return Err(format!("LLVM 库模式暂不支持导出函数 `{}` 的返回类型 `{}`", exp, d.ret)),
        };
        for p in &d.params {
            let t = lib_ty(&p.ty, ir)?;
            let _ = t;
            if !lib_ffi_ok(&p.ty) { return Err(format!("LLVM 库模式暂不支持导出函数 `{}` 的参数类型 `{}`", exp, p.ty)); }
        }
        let ret_ll = lib_ty(&d.ret, ir)?;
        let mut ps: Vec<String> = d.params.iter().enumerate()
            .map(|(i, p)| lib_ty(&p.ty, ir).map(|t| format!("{} %a{}", t, i)))
            .collect::<Result<_,_>>()?;
        let args: Vec<String> = d.params.iter().enumerate()
            .map(|(i, p)| lib_ty(&p.ty, ir).map(|t| format!("{} %a{}", t, i)))
            .collect::<Result<_,_>>()?;
        let wire_ret = match ret_abi {
            LibRetAbi::Value => ret_ll.clone(),
            LibRetAbi::PackedI64 => "i64".to_string(),
            LibRetAbi::PackedI64Pair => "{ i64, ptr }".to_string(),
        };
        let _ = writeln!(text, "define {} {} @{}({}) {{", dll, wire_ret, exp, ps.join(", "));
        let call = format!("{} @{}({})", ret_ll, sym(&f.name), args.join(", "));
        match ret_abi {
            LibRetAbi::Value if d.ret == Type::Void => {
                let _ = writeln!(text, "  call {}", call);
                let _ = writeln!(text, "  ret void");
            }
            LibRetAbi::Value => {
                let _ = writeln!(text, "  %r = call {}", call);
                let _ = writeln!(text, "  ret {} %r", ret_ll);
            }
            LibRetAbi::PackedI64Pair => {
                let _ = writeln!(text, "  %r = call {}", call);
                let _ = writeln!(text, "  %tag = extractvalue {} %r, 0", ret_ll);
                let _ = writeln!(text, "  %ok = extractvalue {} %r, 1", ret_ll);
                let _ = writeln!(text, "  %err = extractvalue {} %r, 2", ret_ll);
                let _ = writeln!(text, "  %tag64 = zext i32 %tag to i64");
                let _ = writeln!(text, "  %ok64 = zext i32 %ok to i64");
                let _ = writeln!(text, "  %okhi = shl i64 %ok64, 32");
                let _ = writeln!(text, "  %first = or i64 %tag64, %okhi");
                let _ = writeln!(text, "  %w0 = insertvalue {{ i64, ptr }} zeroinitializer, i64 %first, 0");
                let _ = writeln!(text, "  %w1 = insertvalue {{ i64, ptr }} %w0, ptr %err, 1");
                let _ = writeln!(text, "  ret {{ i64, ptr }} %w1");
            }
            LibRetAbi::PackedI64 => {
                let _ = writeln!(text, "  %r = call {}", call);
                let inner = match &d.ret { Type::Option(e) => (**e).clone(), _ => Type::I32 };
                let _ = writeln!(text, "  %tag = extractvalue {} %r, 0", ret_ll);
                let _ = writeln!(text, "  %val = extractvalue {} %r, 1", ret_ll);
                let _ = writeln!(text, "  %tag64 = zext i32 %tag to i64");
                let value_bits = match &inner {
                    Type::F32 => { let _ = writeln!(text, "  %valbits = bitcast float %val to i32"); "%valbits".to_string() }
                    Type::I8 | Type::U8 | Type::Bool => { let _ = writeln!(text, "  %valbits = zext {} %val to i32", llty(&inner).unwrap()); "%valbits".to_string() }
                    Type::I16 | Type::U16 => { let _ = writeln!(text, "  %valbits = zext {} %val to i32", llty(&inner).unwrap()); "%valbits".to_string() }
                    _ => "%val".to_string(),
                };
                let _ = writeln!(text, "  %val32 = zext i32 {} to i64", value_bits);
                let _ = writeln!(text, "  %shl = shl i64 %val32, 32");
                let _ = writeln!(text, "  %packed = or i64 %tag64, %shl");
                let _ = writeln!(text, "  ret i64 %packed");
            }
        }
        let _ = writeln!(text, "}}\n");
    }
    Ok(text)
}

#[derive(Clone, Debug)]
struct ExternalFn {
    symbol: String,
    ret: Type,
    params: Vec<Type>,
}

fn collect_externs(prog: &Program, ir: &MirProgram) -> HashMap<String, ExternalFn> {
    let mut out = HashMap::new();
    for d in &prog.decls {
        if d.kind != crate::ast::DeclKind::Fn || d.owner.is_some() {
            continue;
        }
        let m = &prog.modules[d.mod_id];
        let is_native_ffi = d.body.is_none() && d.c_header.is_some();
        if !m.path.contains(".mylib") && !is_native_ffi {
            continue;
        }
        let leaf = d.name.rsplit("::").next().unwrap_or(&d.name);
        let full = format!("{}::{}", m.full_name(), leaf);
        // 混合链接原生归档和 MIR 归档时，MIR 归档里的函数已经在 ir.funcs 中定义，
        // 不能重复声明成外部符号。
        if ir.funcs.iter().any(|f| f.name == full) {
            continue;
        }
        let symbol = crate::codegen::export_name(d).unwrap_or_else(|| leaf.to_string());
        let params = d.params.iter().map(|p| p.ty.clone()).collect();
        let ext = ExternalFn { symbol, ret: d.ret.clone(), params };
        out.insert(full, ext.clone());
        out.entry(leaf.to_string()).or_insert(ext);
    }
    out
}

fn emit(ir: &MirProgram, executable: bool, native_externs: bool, prog: Option<&Program>) -> Result<String, String> {
    // 预检函数签名与局部类型，避免生成一半再失败。
    let mut funcs: HashMap<String, (Type, Vec<Type>)> = HashMap::new();
    for f in &ir.funcs {
        if !is_value(&f.ret) && f.ret != Type::Void {
            return Err(format!("函数 `{}` 的返回类型 `{}` 暂不支持", f.name, f.ret));
        }
        let mut ps = Vec::new();
        for p in &f.params {
            if !is_value(&p.ty) {
                return Err(format!("函数 `{}` 的参数类型 `{}` 暂不支持", f.name, p.ty));
            }
            ps.push(p.ty.clone());
        }
        funcs.insert(f.name.clone(), (f.ret.clone(), ps));
    }
    for f in &ir.funcs {
        for l in &f.locals {
            if !is_value(&l.ty) {
                return Err(format!("函数 `{}` 的局部 `{}` 类型 `{}` 暂不支持", f.name, l.name, l.ty));
            }
        }
        for b in &f.blocks {
            for i in &b.insts {
                if let Some(d) = i.dst {
                    let _ = d;
                }
                if !matches!(i.kind,
                    MirInstKind::Const(_) | MirInstKind::Undef | MirInstKind::Copy(_)
                    | MirInstKind::Store { .. } | MirInstKind::Bin(..) | MirInstKind::Un(..)
                    | MirInstKind::Cast { .. } | MirInstKind::Call { .. }
                    | MirInstKind::Interp { .. } | MirInstKind::New { .. }
                    | MirInstKind::Field { .. } | MirInstKind::StoreField { .. }
                    | MirInstKind::StaticRef(_) | MirInstKind::StoreStatic { .. }
                    | MirInstKind::ArrayNew { .. } | MirInstKind::ArrayLit { .. }
                    | MirInstKind::Index { .. } | MirInstKind::StoreIndex { .. }
                    | MirInstKind::StorePtr { .. }
                    | MirInstKind::Safepoint { .. }
                    | MirInstKind::Try { .. } | MirInstKind::MatchTest { .. } | MirInstKind::Box { .. }
                    | MirInstKind::BindPattern { .. })
                    && !matches!(&i.kind, MirInstKind::MethodCall { .. })
                {
                    return Err(format!("LLVM 后端暂不支持 MIR 指令 {:?}", i.kind));
                }
                if !matches!(i.ty, Type::Void) && !is_value(&i.ty) {
                    return Err(format!("LLVM 后端暂不支持结果类型 `{}`", i.ty));
                }
            }
        }
    }

    let mut out = String::new();
    out.push_str("; Mylis LLVM IR v0（原生 MIR 后端）\n");
    out.push_str("target triple = \"x86_64-pc-linux-gnu\"\n");
    out.push_str("%MylRootDesc = type { ptr, i64 }\n\n");
    let static_root_n: usize = ir.classes.iter()
        .flat_map(|c| c.fields.iter())
        .filter(|f| f.is_static && gc_ref_type(&f.ty))
        .count();
    if static_root_n > 0 {
        let _ = writeln!(out, "@myl_static_roots = global [{} x %MylRootDesc] zeroinitializer", static_root_n);
    }
    let mut class_ids: HashMap<usize, &crate::mir::MirClass> = HashMap::new();
    for e in &ir.enums {
        let n = e.variants.iter().map(|v| v.fields.len()).max().unwrap_or(0);
        let _ = writeln!(out, "%E_{} = type {{ i32, [{} x i64] }}", e.decl_id, n);
    }
    for c in &ir.classes {
        class_ids.insert(c.decl_id, c);
        // 保留字段的原始下标：Res::Field 使用声明顺序（含 static 字段），
        // 这里用 i8 占位，避免实例字段的下标整体前移。
        let fields: Vec<String> = c.fields.iter()
            .map(|f| if f.is_static { Ok("i8".to_string()) } else { llty(&f.ty) })
            .collect::<Result<_, _>>()?;
        let _ = writeln!(out, "%C_{} = type {{ {} }}", c.decl_id, fields.join(", "));
    }
    for c in &ir.classes {
        for (idx, f) in c.fields.iter().enumerate() {
            if f.is_static {
                let _ = writeln!(out, "@S_{}_{} = global {} zeroinitializer", c.decl_id, idx, llty(&f.ty)?);
            }
        }
    }
    // 接口值 { obj, vtable } 与每个接口的 vtable 类型
    let enum_ids: HashSet<usize> = ir.enums.iter().map(|e| e.decl_id).collect();
    for i in &ir.ifaces {
        let _ = writeln!(out, "%I_{} = type {{ ptr, ptr }}", i.decl_id);
        let slots = if i.methods.is_empty() { "i8".to_string() } else { vec!["ptr"; i.methods.len()].join(", ") };
        let _ = writeln!(out, "%Vt_{} = type {{ {} }}", i.decl_id, slots);
    }
    let mut vt_globals = String::new();
    let mut vt_wrappers = String::new();
    let conc: Vec<usize> = ir.classes.iter().map(|c| c.decl_id)
        .chain(ir.enums.iter().map(|e| e.decl_id)).collect();
    let mut vt_pairs: HashSet<(usize, usize)> = HashSet::new();
    for iface in &ir.ifaces {
        for tid in &conc {
            let mut fns = Vec::new();
            let mut ok = true;
            for m in &iface.methods {
                match ir.funcs.iter().find(|f| {
                    let leaf = f.name.rsplit("::").next();
                    f.owner == Some(*tid)
                        && (leaf == Some(m.name.as_str())
                            || (m.name == "toString" && leaf == Some("to_string")))
                        && !f.is_static
                }) {
                    Some(f) => fns.push(f.clone()),
                    None => { ok = false; break; }
                }
            }
            if !ok { continue; }
            let is_enum = enum_ids.contains(tid);
            let mut entries = Vec::new();
            for (k, f) in fns.iter().enumerate() {
                if is_enum {
                    let wname = format!("VTW_{}_{}_{}", tid, iface.decl_id, k);
                    let ety = format!("%E_{}", tid);
                    let ps: Vec<String> = f.params.iter().skip(1)
                        .map(|p| {
                            let t = match &p.ty { Type::Class(id) if enum_ids.contains(id) => Type::Enum(*id), x => x.clone() };
                            llty(&t).map(|s| format!("{} %a{}", s, f.params.iter().position(|q| std::ptr::eq(q, p)).unwrap()))
                        }).collect::<Result<_,_>>()?;
                    let args: Vec<String> = f.params.iter().skip(1).enumerate()
                        .map(|(j, p)| {
                            let t = match &p.ty { Type::Class(id) if enum_ids.contains(id) => Type::Enum(*id), x => x.clone() };
                            llty(&t).map(|s| format!("{} %a{}", s, j + 1))
                        }).collect::<Result<_,_>>()?;
                    let ret = match &f.ret { Type::Class(id) if enum_ids.contains(id) => Type::Enum(*id), x => x.clone() };
                    let rt = llty(&ret)?;
                    let _ = writeln!(vt_wrappers, "define {} @{}(ptr %a0{}) {{", rt, wname, if ps.is_empty() { "".into() } else { format!(", {}", ps.join(", ")) });
                    let _ = writeln!(vt_wrappers, "  %thisv = load {}, ptr %a0", ety);
                    let cargs = if args.is_empty() { format!("{} %thisv", ety) } else { format!("{} %thisv, {}", ety, args.join(", ")) };
                    if f.ret == Type::Void {
                        let _ = writeln!(vt_wrappers, "  call void @{}({})", sym(&f.name), cargs);
                        let _ = writeln!(vt_wrappers, "  ret void");
                    } else {
                        let _ = writeln!(vt_wrappers, "  %r = call {} @{}({})", rt, sym(&f.name), cargs);
                        let _ = writeln!(vt_wrappers, "  ret {} %r", rt);
                    }
                    let _ = writeln!(vt_wrappers, "}}\n");
                    entries.push(format!("ptr @{}", wname));
                } else {
                    entries.push(format!("ptr @{}", sym(&f.name)));
                }
            }
            vt_pairs.insert((*tid, iface.decl_id));
            let entries = if entries.is_empty() { "i8 0".to_string() } else { entries.join(", ") };
            let _ = writeln!(vt_globals, "@VT_{}_{} = global %Vt_{} {{ {} }}", tid, iface.decl_id, iface.decl_id, entries);
        }
    }
    let mut adapters = String::new();
    for fi in &ir.ifaces {
        for ti in &ir.ifaces {
            if fi.decl_id == ti.decl_id { continue; }
            let ft = format!("%I_{}", fi.decl_id);
            let tt = format!("%I_{}", ti.decl_id);
            let _ = writeln!(adapters, "define {} @IA_{}_{}({} %v) {{", tt, fi.decl_id, ti.decl_id, ft);
            let _ = writeln!(adapters, "  %obj = extractvalue {} %v, 0", ft);
            let _ = writeln!(adapters, "  %vt = extractvalue {} %v, 1", ft);
            let _ = writeln!(adapters, "  %acc = alloca ptr");
            let _ = writeln!(adapters, "  store ptr null, ptr %acc");
            for tid in &conc {
                if !vt_pairs.contains(&(*tid, fi.decl_id)) || !vt_pairs.contains(&(*tid, ti.decl_id)) { continue; }
                let _ = writeln!(adapters, "  %c{} = icmp eq ptr %vt, @VT_{}_{}", tid, tid, fi.decl_id);
                let _ = writeln!(adapters, "  %cur{} = load ptr, ptr %acc", tid);
                let _ = writeln!(adapters, "  %s{} = select i1 %c{}, ptr @VT_{}_{}, ptr %cur{}", tid, tid, tid, ti.decl_id, tid);
                let _ = writeln!(adapters, "  store ptr %s{}, ptr %acc", tid);
            }
            let _ = writeln!(adapters, "  %pvt = load ptr, ptr %acc");
            let _ = writeln!(adapters, "  %o0 = insertvalue {} zeroinitializer, ptr %obj, 0", tt);
            let _ = writeln!(adapters, "  %o1 = insertvalue {} %o0, ptr %pvt, 1", tt);
            let _ = writeln!(adapters, "  ret {} %o1", tt);
            let _ = writeln!(adapters, "}}\n");
        }
    }
    // 标量/字符串装箱类型与 box vtable
    let mut boxes: Vec<(Type, usize)> = Vec::new();
    for f in &ir.funcs { for b in &f.blocks { for ins in &b.insts {
        if let MirInstKind::Box { from, .. } = &ins.kind {
            if let Type::Iface(iid) = &ins.ty {
                if !boxes.iter().any(|(t, x)| t == from && *x == *iid) { boxes.push((from.clone(), *iid)); }
            }
        }
    }}}
    let mut box_defs = String::new();
    let mut box_vts = String::new();
    for (t, iid) in &boxes {
        let key = type_key(t);
        let bt = llty(t)?;
        let _ = writeln!(box_defs, "%B_{} = type {{ {} }}", key, bt);
        let iface = ir.ifaces.iter().find(|i| i.decl_id == *iid).ok_or_else(|| format!("缺少接口 {}", iid))?;
        if !iface.methods.is_empty() { return Err(format!("LLVM 后端暂不支持向非空接口 {} 装箱标量", iid)); }
        let _ = writeln!(box_vts, "@VT_BOX_{}_{} = global %Vt_{} {{ i8 0 }}", key, iid, iid);
    }
    out.push_str(&vt_wrappers);
    out.push_str(&vt_globals);
    out.push_str(&adapters);
    out.push_str(&box_defs);
    out.push_str(&box_vts);
    out.push('\n');
    // Option / Result 聚合类型（LLVM 后端自有布局：显式字段，不用 union）
    let mut agg: Vec<Type> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for f in &ir.funcs {
        collect_agg(&f.ret, &mut agg, &mut seen);
        for p in &f.params { collect_agg(&p.ty, &mut agg, &mut seen); }
        for l in &f.locals { collect_agg(&l.ty, &mut agg, &mut seen); }
        for b in &f.blocks { for i in &b.insts { collect_agg(&i.ty, &mut agg, &mut seen); } }
    }
    for c in &ir.classes { for f in &c.fields { collect_agg(&f.ty, &mut agg, &mut seen); } }
    for t in &agg {
        match t {
            Type::Option(e) => {
                let _ = writeln!(out, "%O_{} = type {{ i32, {} }}", type_key(t), llty(e)?);
            }
            Type::Result(a, b) => {
                let _ = writeln!(out, "%R_{} = type {{ i32, {}, {} }}", type_key(t), llty(a)?, llty(b)?);
            }
            _ => {}
        }
    }
    out.push('\n');
    // 运行时声明
    let mut decls = String::new();
    let mut declared = HashSet::new();
    for (name, (r, ps)) in [
        ("myrt_init", ("void", vec![])),
        ("myrt_gc_collect", ("void", vec![])),
        ("myrt_print", ("void", vec!["ptr"])),
        ("myrt_println", ("void", vec!["ptr"])),
        ("myrt_panic", ("void", vec!["ptr"])),
        ("myrt_i64_to_string", ("ptr", vec!["i64"])),
        ("myrt_u64_to_string", ("ptr", vec!["i64"])),
        ("myrt_f64_to_string", ("ptr", vec!["double"])),
        ("myrt_f64_to_string_prec", ("ptr", vec!["double", "i32"])),
        ("myrt_f64_to_string_fmt", ("ptr", vec!["double", "i32", "i32", "i32"])),
        ("mylrt_str_starts_with", ("i32", vec!["ptr", "ptr"])),
        ("mylrt_str_ends_with", ("i32", vec!["ptr", "ptr"])),
        ("mylrt_str_contains", ("i32", vec!["ptr", "ptr"])),
        ("mylrt_str_index_of", ("i64", vec!["ptr", "ptr"])),
        ("mylrt_str_substring", ("ptr", vec!["ptr", "i64", "i64"])),
        ("mylrt_str_trim", ("ptr", vec!["ptr"])),
        ("mylrt_str_repeat", ("ptr", vec!["ptr", "i64"])),
        ("mylrt_str_replace", ("ptr", vec!["ptr", "ptr", "ptr"])),
        ("mylrt_str_char_len", ("i64", vec!["ptr"])),
        ("mylrt_str_char_index_of", ("i64", vec!["ptr", "ptr"])),
        ("mylrt_str_char_substring", ("ptr", vec!["ptr", "i64", "i64"])),
        ("mylrt_str_char_at", ("ptr", vec!["ptr", "i64"])),
        ("mylrt_str_char_code_at", ("i32", vec!["ptr", "i64"])),
        ("mylrt_str_to_bytes", ("ptr", vec!["ptr"])),
        ("mylrt_str_from_bytes", ("ptr", vec!["ptr"])),
        ("sqrt", ("double", vec!["double"])),
        ("pow", ("double", vec!["double", "double"])),
        ("fabs", ("double", vec!["double"])),
        ("floor", ("double", vec!["double"])),
        ("ceil", ("double", vec!["double"])),
        ("round", ("double", vec!["double"])),
        ("trunc", ("double", vec!["double"])),
        ("fmod", ("double", vec!["double", "double"])),
        ("myrt_str_cstr", ("ptr", vec!["ptr"])),
        ("myrt_str_ptr", ("ptr", vec!["ptr"])),
        ("myrt_str_len", ("i64", vec!["ptr"])),
        ("myrt_clock", ("double", vec![])),
        ("myrt_alloc", ("ptr", vec!["i64"])),
        ("myrt_register_root", ("void", vec!["ptr"])),
        ("myrt_unregister_root", ("void", vec!["ptr"])),
        ("myrt_gc_write_barrier", ("void", vec!["ptr"])),
        ("myrt_gc_write_barrier_owner", ("void", vec!["ptr", "ptr"])),
        ("myrt_gc_safepoint", ("void", vec![])),
        ("myrt_gc_add_root_range", ("void", vec!["ptr", "i64"])),
        ("myrt_gc_remove_root_range", ("void", vec!["ptr"])),
        ("myrt_gc_frame_enter", ("i64", vec![])),
        ("myrt_gc_frame_set_roots", ("void", vec!["i64", "ptr", "i64"])),
        ("myrt_gc_frame_exit", ("void", vec!["i64"])),
        ("myrt_gc_static_set_roots", ("void", vec!["ptr", "i64"])),
        ("mylrt_arr_alloc", ("ptr", vec!["i64", "i64"])),
        ("mylrt_arr_index", ("i64", vec!["i64", "i64"])),
        ("mylrt_str_lit", ("ptr", vec!["ptr", "i64"])),
        ("mylrt_str_concat", ("ptr", vec!["ptr", "ptr"])),
        ("mylrt_str_eq", ("i32", vec!["ptr", "ptr"])),
        ("mylrt_str_cmp", ("i32", vec!["ptr", "ptr"])),
        ("mylrt_str_hash", ("i64", vec!["ptr"])),
        ("mylrt_hash_u64", ("i64", vec!["i64"])),
        ("mylrt_hash_f64", ("i64", vec!["double"])),
        ("mylrt_panic_unwrap", ("void", vec![])),
        ("mylrt_panic_msg", ("void", vec!["ptr"])),
    ] {
        let _ = writeln!(decls, "declare {} @{}({})", r, name, ps.join(", "));
        declared.insert(name.to_string());
    }
    out.push_str(&decls);
    // 源码里的 `@header` unsafe 声明本身就是外部符号，不依赖 --link/--cc；
    // `native_externs` 只控制 .mylib 原生载荷是否参与声明。
    let has_source_ffi = prog.map(|p| p.decls.iter().any(|d| {
        d.kind == crate::ast::DeclKind::Fn && d.body.is_none() && d.c_header.is_some()
    })).unwrap_or(false);
    let externs = if native_externs || has_source_ffi {
        prog.map(|p| collect_externs(p, ir)).unwrap_or_default()
    } else {
        HashMap::new()
    };
    for ext in externs.values() {
        if !declared.insert(ext.symbol.clone()) { continue; }
        let r = llty(&ext.ret)?;
        let ps = ext.params.iter().map(llty).collect::<Result<Vec<_>, _>>()?;
        let _ = writeln!(out, "declare {} @{}({})", r, ext.symbol, ps.join(", "));
    }
    out.push_str("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)\n");
    out.push('\n');

    // LLVM 不允许 declare 后 define 同名，按“被调用者先定义”做拓扑排序。
    let mut fmap: HashMap<&str, &MirFunction> = HashMap::new();
    for f in &ir.funcs { fmap.insert(f.name.as_str(), f); }
    let mut seen: HashMap<String, u8> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for f in &ir.funcs { visit_fn(f, &fmap, &mut seen, &mut order); }

    let mut str_lits: Vec<(String, String, usize)> = Vec::new();
    let mut str_n = 0usize;
    let mut bodies = String::new();
    for fname in &order {
        let f = fmap[fname.as_str()];
        let mut g = FnGen {
            f,
            out: String::new(),
            pre: String::new(),
            slots: HashMap::new(),
            locals: HashMap::new(),
            labels: HashMap::new(),
            str_lits: &mut str_lits,
            str_n: &mut str_n,
            tmp_n: 0,
            all_funcs: fmap.clone(),
            externs: externs.clone(),
            classes: class_ids.clone(),
            enums: ir.enums.iter().map(|e| (e.decl_id, e)).collect(),
            ifaces: ir.ifaces.iter().map(|i| (i.decl_id, i)).collect(),
            iface_methods: ir.ifaces.iter().flat_map(|i| {
                i.methods.iter().enumerate().map(move |(k, m)| (m.decl_id, (i.decl_id, k)))
            }).collect(),
            frame_id: None,
            root_frame: None,
            root_n: 0,
            root_local_idx: HashMap::new(),
            root_value_idx: HashMap::new(),
            root_sizes: Vec::new(),
        };
        // 局部槽
        for (i, l) in f.locals.iter().enumerate() {
            let name = format!("l_{}", i);
            let nt = g.norm(&l.ty);
            let lt = llty(&nt)?;
            let _ = writeln!(g.pre, "  %{} = alloca {}", name, lt);
            g.locals.insert(i as u32, (name, nt));
        }
        // SSA 结果槽
        for b in &f.blocks {
            for ins in &b.insts {
                if let Some(d) = ins.dst {
                    if g.slots.contains_key(&d) { continue; }
                    if ins.ty == Type::Void { continue; }
                    let name = format!("v_{}", d);
                    let nt = g.norm(&ins.ty);
                    let lt = llty(&nt)?;
                    let _ = writeln!(g.pre, "  %{} = alloca {}", name, lt);
                    g.slots.insert(d, (name, nt));
                }
            }
        }
        // 当前帧中需要作为 GC 根扫描的槽：只取 safepoint 根标注的并集，
        // 这是路径敏感活跃性分析后的结果；从未在任何安全点活跃的引用槽不注册。
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
        let mut root_slots: Vec<(String, i64)> = Vec::new();
        let mut root_local_idx: HashMap<u32, usize> = HashMap::new();
        let mut root_value_idx: HashMap<u32, usize> = HashMap::new();
        for (i, l) in f.locals.iter().enumerate() {
            if live_locals.contains(&(i as u32)) {
                let nt = g.norm(&l.ty);
                if gc_ref_type(&nt) {
                    let size = g.size_align(&nt)?.0;
                    root_local_idx.insert(i as u32, root_slots.len());
                    root_slots.push((format!("l_{}", i), size));
                }
            }
        }
        for (v, (name, ty)) in g.slots.iter() {
            if live_values.contains(v) && gc_ref_type(ty) {
                let size = g.size_align(ty)?.0;
                root_value_idx.insert(*v, root_slots.len());
                root_slots.push((name.clone(), size));
            }
        }
        g.root_n = root_slots.len();
        g.root_local_idx = root_local_idx;
        g.root_value_idx = root_value_idx;
        g.root_sizes = root_slots.iter().map(|(_, s)| *s).collect();
        let mut root_epilogue = String::new();
        if g.root_n > 0 {
            // 每个函数一张根描述表；safepoint 只更新这张表的快照。
            let _ = writeln!(g.pre, "  %myl_roots = alloca [{} x %MylRootDesc]", g.root_n);
            g.frame_id = Some("%myl_fid".into());
            g.root_frame = Some("myl_roots".into());
            let _ = writeln!(root_epilogue, "  call void @myrt_gc_frame_exit(i64 %myl_fid)");
        }

        // MIR 块标签
        for (i, b) in f.blocks.iter().enumerate() {
            g.labels.insert(b.label.clone(), format!("{}_{}", label(&b.label), i));
        }

        let mut body = String::new();
        let nret = g.norm(&f.ret);
        let _ = writeln!(body, "define {} @{}({}) {{", llty(&nret)?, sym(&f.name),
            f.params.iter().enumerate().map(|(i, p)| {
                let nt = g.norm(&p.ty);
                format!("{} %a{}", llty(&nt).unwrap(), i)
            }).collect::<Vec<_>>().join(", "));
        let _ = writeln!(body, "entry:");
        body.push_str(&g.pre);
        if g.frame_id.is_some() {
            let _ = writeln!(body, "  %myl_fid = call i64 @myrt_gc_frame_enter()");
        }
        // 参数 → 局部槽 0..n
        for (i, p) in f.params.iter().enumerate() {
            if let Some((slot, _)) = g.locals.get(&(i as u32)) {
                let nt = g.norm(&p.ty);
                let _ = writeln!(body, "  store {} %a{}, ptr %{}", llty(&nt)?, i, slot);
            }
        }
        if let Some(first) = f.blocks.first() {
            let l = g.labels.get(&first.label).cloned().unwrap();
            let _ = writeln!(body, "  br label %{}", l);
        } else {
            let _ = writeln!(body, "  unreachable");
        }
        for b in &f.blocks {
            let l = g.labels.get(&b.label).cloned().unwrap();
            let _ = writeln!(body, "{}:", l);
            g.out = String::new();
            for ins in &b.insts {
                g.inst(ins)?;
            }
            body.push_str(&g.out);
            match &b.term {
                Some(MirTerm::Return(None)) => {
                    body.push_str(&root_epilogue);
                    let _ = writeln!(body, "  ret void");
                }
                Some(MirTerm::Return(Some(o))) => {
                    g.out.clear();
                    let (x, t) = g.load(o)?;
                    let ret_ty = llty(&t)?;
                    body.push_str(&g.out);
                    body.push_str(&root_epilogue);
                    let _ = writeln!(body, "  ret {} {}", ret_ty, x);
                }
                Some(MirTerm::Goto(t)) => {
                    let tl = g.labels.get(t).ok_or_else(|| format!("未知 MIR 块 `{}`", t))?;
                    let _ = writeln!(body, "  br label %{}", tl);
                }
                Some(MirTerm::Branch { cond, then_label, else_label }) => {
                    g.out.clear();
                    let (x, t) = g.load(cond)?;
                    body.push_str(&g.out);
                    let lt = llty(&t)?;
                    let c = g.tmp("cond");
                    let _ = writeln!(body, "  {} = icmp ne {} {}, 0", c, lt, x);
                    let tl = g.labels.get(then_label).ok_or_else(|| format!("未知 MIR 块 `{}`", then_label))?;
                    let el = g.labels.get(else_label).ok_or_else(|| format!("未知 MIR 块 `{}`", else_label))?;
                    let _ = writeln!(body, "  br i1 {}, label %{}, label %{}", c, tl, el);
                }
                Some(MirTerm::Unreachable) => { let _ = writeln!(body, "  unreachable"); }
                None => { let _ = writeln!(body, "  unreachable"); }
            }
        }
        let _ = writeln!(body, "}}");
        bodies.push_str(&body);
        bodies.push('\n');
    }

    let mut init = String::new();
    let _ = writeln!(init, "define void @myl_init_statics() {{");
    let _ = writeln!(init, "entry:");
    // 静态字段根表：直接写入全局描述符数组，之后交给运行时长期持有。
    let mut sr_idx = 0usize;
    for c in &ir.classes {
        for (idx, f) in c.fields.iter().enumerate() {
            if !f.is_static || !gc_ref_type(&f.ty) { continue; }
            let size = gc_root_size(&f.ty, ir);
            let d = format!("%srd_{}", sr_idx);
            let f0 = format!("%srdf0_{}", sr_idx);
            let f1 = format!("%srdf1_{}", sr_idx);
            let _ = writeln!(init, "  {} = getelementptr [{} x %MylRootDesc], ptr @myl_static_roots, i64 0, i64 {}", d, static_root_n, sr_idx);
            let _ = writeln!(init, "  {} = getelementptr %MylRootDesc, ptr {}, i32 0, i32 0", f0, d);
            let _ = writeln!(init, "  store ptr @S_{}_{}, ptr {}", c.decl_id, idx, f0);
            let _ = writeln!(init, "  {} = getelementptr %MylRootDesc, ptr {}, i32 0, i32 1", f1, d);
            let _ = writeln!(init, "  store i64 {}, ptr {}", size, f1);
            sr_idx += 1;
        }
    }
    if sr_idx > 0 {
        let _ = writeln!(init, "  call void @myrt_gc_static_set_roots(ptr @myl_static_roots, i64 {})", sr_idx);
    }
    for c in &ir.classes {
        for (idx, f) in c.fields.iter().enumerate() {
            if !f.is_static { continue; }
            if let Some(v) = &f.init {
                match v {
                    MirConst::Str(t) => {
                        let l = format!("@.myl_str_static_{}_{}", c.decl_id, idx);
                        let _ = writeln!(init, "  %s = call ptr @mylrt_str_lit(ptr {}, i64 {})", l, t.as_bytes().len());
                        let _ = writeln!(init, "  store ptr %s, ptr @S_{}_{}", c.decl_id, idx);
                        // 这里生成的字符串字面量稍后补进 globals
                        str_lits.push((l, t.clone(), t.as_bytes().len()));
                    }
                    MirConst::Int(n) => { let v = llvm_int_const_text(*n as u64, &f.ty, true); let _ = writeln!(init, "  store {} {}, ptr @S_{}_{}", llty(&f.ty)?, v, c.decl_id, idx); }
                    MirConst::IntU64(n) => { let v = llvm_int_const_text(*n, &f.ty, false); let _ = writeln!(init, "  store {} {}, ptr @S_{}_{}", llty(&f.ty)?, v, c.decl_id, idx); }
                    MirConst::Float(x) => { let _ = writeln!(init, "  store {} {:?}, ptr @S_{}_{}", llty(&f.ty)?, x, c.decl_id, idx); }
                    MirConst::Bool(b) => { let _ = writeln!(init, "  store {} {}, ptr @S_{}_{}", llty(&f.ty)?, if *b {1} else {0}, c.decl_id, idx); }
                    MirConst::Null => { let _ = writeln!(init, "  store ptr null, ptr @S_{}_{}", c.decl_id, idx); }
                }
            }
        }
    }
    let _ = writeln!(init, "  ret void");
    let _ = writeln!(init, "}}");
    out.push_str(&init);
    out.push('\n');

    let mut globals = String::new();
    for (name, s, len) in &str_lits {
        let _ = writeln!(globals, "{} = private unnamed_addr constant [{} x i8] c\"{}\"", name, len, esc_str(s));
    }
    out.push_str(&globals);
    out.push('\n');
    out.push_str(&bodies);

    if executable {
        // 找入口模块的 main（名字以 ::main 结尾）
        let mainf = ir.funcs.iter().find(|f| f.name.ends_with("::main"))
            .ok_or_else(|| "找不到 main 函数".to_string())?;
        if mainf.ret != Type::I32 {
            return Err("LLVM 后端暂只支持 i32 main".into());
        }
        if !mainf.params.is_empty() {
            return Err("LLVM 后端暂不支持带参数的 main".into());
        }
        out.push_str(&format!(
            "\ndefine i32 @main() {{\nentry:\n  call void @myrt_init()\n  call void @myl_init_statics()\n  %r = call i32 @{}()\n  ret i32 %r\n}}\n",
            sym(&mainf.name)
        ));
    }
    Ok(out)
}

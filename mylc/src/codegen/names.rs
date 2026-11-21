use super::*;

impl<'a> Codegen<'a> {
    pub(super) fn prefix(&self, decl_id: usize) -> String {
        self.prog.modules[self.prog.decls[decl_id].mod_id]
            .prefix
            .clone()
    }

    pub(super) fn class_cname(&self, id: usize) -> String {
        format!("C_{}_{}", self.prefix(id), self.prog.decls[id].name)
    }

    pub(super) fn enum_cname(&self, id: usize) -> String {
        format!("E_{}_{}", self.prefix(id), self.prog.decls[id].name)
    }

    pub(super) fn payload_cname(&self, eid: usize, vidx: usize) -> String {
        format!(
            "P_{}_{}_{}",
            self.prefix(eid),
            self.prog.decls[eid].name,
            self.prog.decls[eid].variants[vidx].name
        )
    }

    pub(super) fn decl_cname(&self, id: usize) -> String {
        match self.prog.decls[id].kind {
            DeclKind::Enum => self.enum_cname(id),
            _ => self.class_cname(id),
        }
    }

    pub(super) fn iface_cname(&self, id: usize) -> String {
        format!("I_{}_{}", self.prefix(id), self.prog.decls[id].name)
    }

    pub(super) fn vt_cname(&self, id: usize) -> String {
        format!("Vt_{}_{}", self.prefix(id), self.prog.decls[id].name)
    }

    pub(super) fn adapter_name(&self, from: usize, to: usize) -> String {
        format!(
            "AD_{}_{}_{}_{}",
            self.prefix(from), self.prog.decls[from].name,
            self.prefix(to), self.prog.decls[to].name
        )
    }

    pub(super) fn mangle(&self, t: &Type) -> String {
        match t {
            Type::Void => "void".to_string(),
            Type::I8 => "i8".to_string(),
            Type::I16 => "i16".to_string(),
            Type::I32 => "i32".to_string(),
            Type::I64 => "i64".to_string(),
            Type::U8 => "u8".to_string(),
            Type::U16 => "u16".to_string(),
            Type::U32 => "u32".to_string(),
            Type::U64 => "u64".to_string(),
            Type::F32 => "f32".to_string(),
            Type::F64 => "f64".to_string(),
            Type::Bool => "myrt_bool".to_string(),
            Type::Str => "str".to_string(),
            Type::Ptr(e) => format!("p{}", self.mangle(e)),
            Type::Array(e) => format!("a{}", self.mangle(e)),
            Type::Result(a, b) => format!("r{}_{}", self.mangle(a), self.mangle(b)),
            Type::Option(a) => format!("o{}", self.mangle(a)),
            Type::Class(id) => format!("c{}", id),
            Type::Iface(id) => format!("i{}", id),
            Type::Enum(id) => format!("e{}", id),
            Type::Named(n) => format!("n{}", n),
            Type::Generic(n, _) => format!("n{}", n),
            // late mono 之后不应再有未实例化的名义类型或类型参数进入后端。
            // 如果这里被触发，说明 monomorphize 漏掉了某个表达式/声明类型。
            Type::Adt { .. } => unreachable!("late monomorphization left Type::Adt in codegen mangle"),
            Type::Param(_) => unreachable!("late monomorphization left Type::Param in codegen mangle"),
            Type::Infer => "infer".to_string(),
        }
    }

    pub(super) fn unsigned_ctype(&self, t: &Type) -> String {
        match t {
            Type::I8 => "myrt_u8".to_string(),
            Type::I16 => "myrt_u16".to_string(),
            Type::I32 => "myrt_u32".to_string(),
            Type::I64 => "myrt_u64".to_string(),
            Type::U8 => "myrt_u8".to_string(),
            Type::U16 => "myrt_u16".to_string(),
            Type::U32 => "myrt_u32".to_string(),
            Type::U64 => "myrt_u64".to_string(),
            _ => self.ctype(t),
        }
    }

    /// GC 管理的引用类型（写入这些位置需要写屏障）
    pub(super) fn is_gc_ref(&self, t: &Type) -> bool {
        matches!(t, Type::Str | Type::Class(_) | Type::Array(_) | Type::Enum(_) | Type::Adt { .. })
    }

    pub(super) fn barrier(&mut self, t: &Type, value: &str) {
        if self.is_gc_ref(t) {
            self.line(&format!("myrt_gc_write_barrier((void*)({}));", value));
        } else if matches!(t, Type::Iface(_)) {
            // 接口是 { obj, vt } 值结构：屏障它的对象指针
            self.line(&format!("myrt_gc_write_barrier((void*)(({}).obj));", value));
        }
    }

    /// 带 owner 槽地址的屏障：owner 指向堆对象内部时，GC 可记录老→年轻引用。
    pub(super) fn barrier_slot(&mut self, t: &Type, slot: &str, value: &str) {
        if self.is_gc_ref(t) {
            self.line(&format!("myrt_gc_write_barrier_owner((void*)({}), (void*)({}));", slot, value));
        } else if matches!(t, Type::Iface(_)) {
            self.line(&format!("myrt_gc_write_barrier_owner((void*)({}), (void*)(({}).obj));", slot, value));
        } else if matches!(t, Type::Option(_) | Type::Result(_, _)) {
            // 内联聚合值：不能整体强转成指针，登记 owner 槽让 minor 扫描聚合载荷。
            self.line(&format!("myrt_gc_write_barrier_owner((void*)({}), (void*)({}));", slot, slot));
        }
    }

    pub(super) fn ctype(&self, t: &Type) -> String {
        match t {
            Type::Void => "void".to_string(),
            Type::I8 => "myrt_i8".to_string(),
            Type::I16 => "myrt_i16".to_string(),
            Type::I32 => "myrt_i32".to_string(),
            Type::I64 => "myrt_i64".to_string(),
            Type::U8 => "myrt_u8".to_string(),
            Type::U16 => "myrt_u16".to_string(),
            Type::U32 => "myrt_u32".to_string(),
            Type::U64 => "myrt_u64".to_string(),
            Type::F32 => "float".to_string(),
            Type::F64 => "double".to_string(),
            Type::Bool => "myrt_bool".to_string(),
            Type::Str => "MyStr*".to_string(),
            Type::Ptr(e) => match **e {
                // i8 / u8 指针在 C 侧对应 char / unsigned char（FFI 与 myrt 一致）
                Type::I8 => "char*".to_string(),
                Type::U8 => "unsigned char*".to_string(),
                _ => format!("{}*", self.ctype(e)),
            },
            Type::Result(_, _) => format!("MylR_{}", self.mangle(t)),
            Type::Option(_) => format!("MylO_{}", self.mangle(t)),
            Type::Array(_) => format!("MylA_{}*", self.mangle(t)),
            Type::Class(id) => format!("{}*", self.class_cname(*id)),
            Type::Iface(id) => self.iface_cname(*id),
            Type::Enum(id) => format!("{}*", self.enum_cname(*id)),
            Type::Named(n) => n.clone(),
            Type::Generic(n, _) => n.clone(),
            Type::Adt { .. } => unreachable!("late monomorphization left Type::Adt in codegen ctype"),
            Type::Param(_) => unreachable!("late monomorphization left Type::Param in codegen ctype"),
            Type::Infer => "void*".to_string(),
        }
    }
}

pub(super) fn box_key(t: &Type) -> &'static str {
    match t {
        Type::I8 => "i8", Type::I16 => "i16", Type::I32 => "i32", Type::I64 => "i64",
        Type::U8 => "u8", Type::U16 => "u16", Type::U32 => "u32", Type::U64 => "u64",
        Type::F32 => "f32", Type::F64 => "f64", Type::Bool => "bool", Type::Str => "str",
        _ => "i32",
    }
}

pub(super) fn box_id(t: &Type) -> i32 {
    match t {
        Type::I8 => -1, Type::I16 => -2, Type::I32 => -3, Type::I64 => -4,
        Type::U8 => -5, Type::U16 => -6, Type::U32 => -7, Type::U64 => -8,
        Type::F32 => -9, Type::F64 => -10, Type::Bool => -11, Type::Str => -12,
        _ => -3,
    }
}

pub(super) fn boxable(t: &Type) -> bool {
    matches!(t, Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::U8 | Type::U16
        | Type::U32 | Type::U64 | Type::F32 | Type::F64 | Type::Bool | Type::Str)
}

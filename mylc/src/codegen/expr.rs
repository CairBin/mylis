use super::*;
use super::names::{box_id, box_key, boxable};

impl<'a> Codegen<'a> {
    pub(super) fn gen_box(&mut self, raw: String, from: &Type, iid: usize) -> String {
        let k = box_key(from);
        format!(
            "(({}){{ .obj = myl_box_{}({}), .vt = &VT_BOX_{}_{}_{} }})",
            self.iface_cname(iid), k, raw, k, self.prefix(iid), self.prog.decls[iid].name
        )
    }

    pub(super) fn gen_expr(&mut self, e: &Expr) -> String {
        if let Some((from, iid)) = &e.box_to {
            let raw = match &e.kind {
                ExprKind::Cast(inner, _, _) => self.gen_expr(inner),
                _ => self.gen_expr_raw(e),
            };
            return self.gen_box(raw, from, *iid);
        }
        if let Some((tid, iid)) = e.coerce {
            let raw = self.gen_expr_raw(e);
            return format!(
                "(({}){{ .obj = (void*)({}), .vt = &VT_{}_{} }})",
                self.iface_cname(iid),
                raw,
                self.ctor_name(tid),
                self.prog.decls[iid].name
            );
        }
        if let Res::IfaceCast(cid, iid) = e.res {
            if !matches!(e.kind, ExprKind::New(_, _, _) | ExprKind::Cast(_, _, _)) {
                let raw = self.gen_expr_raw(e);
                return format!(
                    "(({}){{ .obj = (void*)({}), .vt = &VT_{}_{} }})",
                    self.iface_cname(iid),
                    raw,
                    self.ctor_name(cid),
                    self.prog.decls[iid].name
                );
            }
        }
        self.gen_expr_raw(e)
    }

    pub(super) fn gen_expr_raw(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => {
                let ty = e.box_to.as_ref().map(|(t, _)| t).unwrap_or(&e.ty);
                match ty {
                    Type::U64 => format!("({}ULL)", *v as u64),
                    Type::I64 => format!("({}LL)", v),
                    _ => format!("(({}){})", self.ctype(ty), v),
                }
            }
            ExprKind::IntU64(v) => {
                let ty = e.box_to.as_ref().map(|(t, _)| t).unwrap_or(&e.ty);
                match ty {
                    Type::U64 => format!("({}ULL)", v),
                    Type::I64 => format!("({}LL)", *v as i64),
                    _ => format!("(({}){})", self.ctype(ty), v),
                }
            }
            ExprKind::Float(v) => {
                let ty = e.box_to.as_ref().map(|(t, _)| t).unwrap_or(&e.ty);
                match ty {
                    Type::F32 => format!("((float){:?})", v),
                    _ => format!("((double){:?})", v),
                }
            }
            ExprKind::Str(s) => {
                let esc = c_escape(s);
                format!("mylrt_str_lit(\"{}\", {})", esc, s.len())
            }
            ExprKind::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
            ExprKind::Null => "0".to_string(),
            ExprKind::This => "self".to_string(),
            ExprKind::Ident(name) => self
                .lookup_local(name)
                .unwrap_or_else(|| format!("/*未解析 {}*/", name)),
            ExprKind::Bin(op, l, r) => self.gen_bin(*op, l, r, &e.ty),
            ExprKind::Un(op, x) => {
                let v = self.gen_expr(x);
                match op {
                    UnOp::Neg => {
                        if e.ty.is_signed() {
                            let u = self.unsigned_ctype(&e.ty);
                            let st = self.ctype(&e.ty);
                            format!("(({})(0u - ({})({})))", st, u, v)
                        } else {
                            format!("(-{})", v)
                        }
                    }
                    UnOp::Not => format!("(!{})", v),
                    UnOp::BitNot => format!("(~{})", v),
                    UnOp::Deref => format!("(*{})", v),
                    UnOp::AddrOf => {
                        let p = self.gen_place(x);
                        format!("(&{})", p)
                    }
                }
            }
            ExprKind::Call(name, _targs, args) => {
                let callee = match e.res {
                    Res::Fn(id) => {
                        if self.prog.decls[id].body.is_none() {
                            if self.ffi_has_header(id) {
                                self.ffi_name(id)
                              } else {
                                self.prog.decls[id].name.clone()
                              }
                        } else {
                            self.fn_cname(id)
                        }
                    }
                    _ => name.clone(),
                };
                let mut a: Vec<String> = Vec::new();
                for x in args {
                    let v = self.gen_expr(x);
                    let t = self.gen_order(x, v);
                    a.push(t);
                }
                format!("{}({})", callee, a.join(", "))
            }
            ExprKind::Method(recv, mname, _targs, args) => self.gen_method(e, recv, mname, args),
            ExprKind::Field(recv, fname) => {
                    let base = self.gen_addrish(recv);
                    format!("{}->{}", base, fname)
            }
            ExprKind::New(cname, _targs, args) => {
                let (cid, iface) = match e.res {
                    Res::Ctor(id) => (id, None),
                    Res::IfaceCast(id, iid) => (id, Some(iid)),
                    _ => {
                        return format!("/*未解析 new {}*/", cname);
                    }
                };
                let mut a: Vec<String> = Vec::new();
                for x in args {
                    let v = self.gen_expr(x);
                    let t = self.gen_order(x, v);
                    a.push(t);
                }
                let call = format!("NEW_{}({})", self.ctor_name(cid), a.join(", "));
                match iface {
                    Some(iid) => format!(
                        "(({}){{ .obj = (void*)({}), .vt = &VT_{}_{} }})",
                        self.iface_cname(iid),
                        call,
                        self.ctor_name(cid),
                        self.prog.decls[iid].name
                    ),
                    None => call,
                }
            }
            ExprKind::Ok(inner) => {
                let v = self.gen_expr(inner);
                let (oi, ei) = match &e.ty {
                    Type::Result(a, b) => (
                        matches!(&**a, Type::Iface(_)),
                        matches!(&**b, Type::Iface(_)),
                    ),
                    _ => (false, false),
                };
                if oi {
                    let v = self.gen_order(inner, v);
                    format!("(({}){{ .is_ok = 1, .ok_obj = ({}).obj, .ok_vt = ({}).vt }})",
                        self.ctype(&e.ty), v, v)
                } else if ei {
                    format!("(({}){{ .is_ok = 1, .ok = {} }})", self.ctype(&e.ty), v)
                } else {
                    format!("(({}){{ .is_ok = 1, .u = {{ .ok = {} }} }})", self.ctype(&e.ty), v)
                }
            }
            ExprKind::Err(inner) => {
                let v = self.gen_expr(inner);
                let (oi, ei) = match &e.ty {
                    Type::Result(a, b) => (
                        matches!(&**a, Type::Iface(_)),
                        matches!(&**b, Type::Iface(_)),
                    ),
                    _ => (false, false),
                };
                if ei {
                    let v = self.gen_order(inner, v);
                    format!("(({}){{ .is_ok = 0, .err_obj = ({}).obj, .err_vt = ({}).vt }})",
                        self.ctype(&e.ty), v, v)
                } else if oi {
                    format!("(({}){{ .is_ok = 0, .err = {} }})", self.ctype(&e.ty), v)
                } else {
                    format!("(({}){{ .is_ok = 0, .u = {{ .err = {} }} }})", self.ctype(&e.ty), v)
                }
            }
            ExprKind::Some_(inner) => {
                let v = self.gen_expr(inner);
                let oi = matches!(&e.ty, Type::Option(t) if matches!(&**t, Type::Iface(_)));
                if oi {
                    let v = self.gen_order(inner, v);
                    format!("(({}){{ .is_some = 1, .obj = ({}).obj, .vt = ({}).vt }})",
                        self.ctype(&e.ty), v, v)
                } else {
                    format!("(({}){{ .is_some = 1, .u = {{ .some = {} }} }})", self.ctype(&e.ty), v)
                }
            }
            ExprKind::None_ => format!("(({}){{0}})", self.ctype(&e.ty)),
            ExprKind::Cast(inner, target, checked) => {
                let v = self.gen_expr(inner);
                if let Res::IfaceCast(cid, iid) = e.res {
                    return format!(
                        "(({}){{ .obj = (void*)({}), .vt = &VT_{}_{} }})",
                        self.iface_cname(iid),
                        v,
                        self.ctor_name(cid),
                        self.prog.decls[iid].name
                    );
                }
                let src = inner.ty.clone();
                let panic = "myrt_panic(mylrt_str_lit(\"invalid cast\", 12))";
                if *checked {
                    let opt_ty = self.ctype(&e.ty);
                    let d = self.tmp("cast");
                    self.line(&format!("{} {} = {{0}};", opt_ty, d));
                    match (&src, target) {
                        (Type::Iface(_), Type::Class(cid)) => {
                            let s = self.tmp("ifc");
                            self.line(&format!("{} {} = {};", self.ctype(&src), s, v));
                            self.line(&format!(
                                "if (({}).vt && ({}).vt->type_id == {}) {{", s, s, cid
                            ));
                            self.ind += 1;
                            self.line(&format!("{}.is_some = 1; {}.u.some = (void*)({}).obj;", d, d, s));
                            self.ind -= 1;
                            self.line("}");
                        }
                        (Type::Iface(from), Type::Iface(to)) => {
                            let s = self.tmp("ifc");
                            self.line(&format!("{} {} = {};", self.ctype(&src), s, v));
                            let r = self.tmp("ad");
                            self.line(&format!("{} {} = {}({});", self.iface_cname(*to), r, self.adapter_name(*from, *to), s));
                            self.line(&format!("if ({} .vt) {{", r));
                            self.ind += 1;
                            self.line(&format!("{}.is_some = 1; {}.obj = {}.obj; {}.vt = {}.vt;", d, d, r, d, r));
                            self.ind -= 1;
                            self.line("}");
                        }
                        (Type::Iface(_), t) if boxable(t) => {
                            let s = self.tmp("ifc");
                            self.line(&format!("{} {} = {};", self.ctype(&src), s, v));
                            self.line(&format!(
                                "if (({}).vt && ({}).vt->type_id == {}) {{", s, s, box_id(t)
                            ));
                            self.ind += 1;
                            self.line(&format!(
                                "{}.is_some = 1; {}.u.some = myl_unbox_{}(({}).obj);",
                                d, d, box_key(t), s
                            ));
                            self.ind -= 1;
                            self.line("}");
                        }
                        _ => {}
                    }
                    return d;
                }
                match (&src, target) {
                    (Type::Iface(_), Type::Class(cid)) => {
                        let s = self.tmp("ifc");
                        self.line(&format!("{} {} = {};", self.ctype(&src), s, v));
                        self.line(&format!(
                            "if (!(({}).vt && ({}).vt->type_id == {})) {};", s, s, cid, panic
                        ));
                        format!("({}*)({}).obj", self.class_cname(*cid), s)
                    }
                    (Type::Iface(from), Type::Iface(to)) => {
                        let s = self.tmp("ifc");
                        self.line(&format!("{} {} = {};", self.ctype(&src), s, v));
                        let r = self.tmp("ad");
                        self.line(&format!("{} {} = {}({});", self.iface_cname(*to), r, self.adapter_name(*from, *to), s));
                        self.line(&format!("if (!{} .vt) {};", r, panic));
                        r
                    }
                    (Type::Iface(_), t) if boxable(t) => {
                        let s = self.tmp("ifc");
                        self.line(&format!("{} {} = {};", self.ctype(&src), s, v));
                        self.line(&format!(
                            "if (!(({}).vt && ({}).vt->type_id == {})) {};", s, s, box_id(t), panic
                        ));
                        format!("myl_unbox_{}(({}).obj)", box_key(t), s)
                    }
                    _ => format!("(({}){})", self.ctype(&e.ty), v),
                }
            }
            ExprKind::Index(recv, idx) => {
                let base = self.gen_expr(recv);
                let i = self.gen_expr(idx);
                let bv = self.tmp("arr");
                self.line(&format!("{} {} = {};", self.ctype(&recv.ty), bv, base));
                format!(
                    "{}->items[mylrt_arr_index({}->len, (myrt_i64)({}))]",
                    bv, bv, i
                )
            }
            ExprKind::ArrayLit(items) => {
                let elem = match &e.ty {
                    Type::Array(el) => (**el).clone(),
                    _ => Type::Infer,
                };
                let arr = self.tmp("arr");
                self.line(&format!(
                    "{} {} = ({} )mylrt_arr_alloc(sizeof({}), {});",
                    self.ctype(&e.ty),
                    arr,
                    self.ctype(&e.ty),
                    self.ctype(&elem),
                    items.len()
                ));
                for (i, it) in items.iter().enumerate() {
                    let v = self.gen_expr(it);
                    self.line(&format!("{}->items[{}] = {};", arr, i, v));
                    let et = elem.clone();
                    self.barrier_slot(&et, &format!("&{}->items[{}]", arr, i), &format!("{}->items[{}]", arr, i));
                }
                arr
            }
            ExprKind::ArrayNew(et, len) => {
                let ll = self.gen_expr(len);
                let arr = self.tmp("arr");
                self.line(&format!(
                    "{} {} = ({} )mylrt_arr_alloc(sizeof({}), (myrt_u64)({}));",
                    self.ctype(&e.ty),
                    arr,
                    self.ctype(&e.ty),
                    self.ctype(et),
                    ll
                ));
                arr
            }
            ExprKind::Interp(parts) => self.gen_interp(parts),
            ExprKind::StaticPath { args, call, .. } => {
                if *call {
                    match e.res {
                        Res::Variant(eid, vi) => {
                            let mut a: Vec<String> = Vec::new();
                            for x in args {
                                let v = self.gen_expr(x);
                                a.push(self.gen_order(x, v));
                            }
                            format!("{}({})", self.variant_ctor_cname(eid, vi), a.join(", "))
                        }
                        Res::StaticMethod(cid, mid) => {
                            let f = self.static_fn_cname(cid, mid);
                            let mut a: Vec<String> = Vec::new();
                            for x in args {
                                let v = self.gen_expr(x);
                                a.push(self.gen_order(x, v));
                            }
                            format!("{}({})", f, a.join(", "))
                        }
                        Res::Fn(fid) => {
                            let f = self.fn_cname(fid);
                            let mut a: Vec<String> = Vec::new();
                            for x in args {
                                let v = self.gen_expr(x);
                                a.push(self.gen_order(x, v));
                            }
                            format!("{}({})", f, a.join(", "))
                        }
                        _ => "/*未解析静态方法*/0".to_string(),
                    }
                } else {
                    match e.res {
                        Res::Variant(eid, vi) => {
                            format!("{}(/*unit*/)", self.variant_ctor_cname(eid, vi))
                        }
                        Res::StaticField(cid, idx) => self.static_field_cname(cid, idx),
                        _ => "/*未解析静态字段*/0".to_string(),
                    }
                }
            }
            ExprKind::Try(inner) => {
                let v = self.gen_expr(inner);
                let src_ty = inner.ty.clone();
                let t = self.tmp("try");
                self.line(&format!("{} {} = {};", self.ctype(&src_ty), t, v));
                let ret_ty = self.fn_ret.clone();
                match &ret_ty {
                    Type::Result(_, _) => {
                        let (fok, ferr) = match &ret_ty {
                            Type::Result(a, b) => (
                                matches!(&**a, Type::Iface(_)),
                                matches!(&**b, Type::Iface(_)),
                            ),
                            _ => (false, false),
                        };
                        let (vok, verr) = match &src_ty {
                            Type::Result(a, b) => (
                                matches!(&**a, Type::Iface(_)),
                                matches!(&**b, Type::Iface(_)),
                            ),
                            _ => (false, false),
                        };
                        self.line(&format!("if (!{}.is_ok) {{", t));
                        self.ind += 1;
                        let err = if ferr {
                            let _ = verr;
                            format!("({}){{ .is_ok = 0, .err_obj = {}.err_obj, .err_vt = {}.err_vt }}", self.ctype(&ret_ty), t, t)
                        } else if fok {
                            format!("({}){{ .is_ok = 0, .err = {}.err }}", self.ctype(&ret_ty), t)
                        } else {
                            let _ = vok;
                            format!("({}){{ .is_ok = 0, .u = {{ .err = {}.u.err }} }}", self.ctype(&ret_ty), t)
                        };
                        self.line(&format!("return {};", err));
                        self.ind -= 1;
                        self.line("}");
                        if let Type::Iface(id) = &e.ty {
                            format!("({}){{ .obj = {}.ok_obj, .vt = (const {}*){}.ok_vt }}",
                                self.iface_cname(*id), t, self.vt_cname(*id), t)
                        } else if ferr {
                            format!("{}.ok", t)
                        } else {
                            format!("{}.u.ok", t)
                        }
                    }
                    Type::Option(_) => {
                        self.line(&format!("if (!{}.is_some) {{", t));
                        self.ind += 1;
                        self.line(&format!("return ({}){{ .is_some = 0 }};", self.ctype(&ret_ty)));
                        self.ind -= 1;
                        self.line("}");
                        if let Type::Iface(id) = &e.ty {
                            format!("({}){{ .obj = {}.obj, .vt = (const {}*){}.vt }}",
                                self.iface_cname(*id), t, self.vt_cname(*id), t)
                        } else {
                            format!("{}.u.some", t)
                        }
                    }
                    _ => "/*? 不在 Result/Option 函数内*/0".to_string(),
                }
            }
        }
    }

    pub(super) fn gen_interp(&mut self, parts: &[InterpPart]) -> String {
        let t = self.tmp("interp");
        self.line(&format!("MyStr* {} = mylrt_str_lit(\"\", 0);", t));
        for part in parts {
            match part {
                InterpPart::Lit(sv) if sv.is_empty() => {}
                InterpPart::Lit(sv) => {
                    let esc = c_escape(sv);
                    self.line(&format!(
                        "{} = mylrt_str_concat({}, mylrt_str_lit(\"{}\", {}));",
                        t,
                        t,
                        esc,
                        sv.len()
                    ));
                }
                InterpPart::Expr(x) => {
                    let f = self.fmt_expr(x);
                    self.line(&format!("{} = mylrt_str_concat({}, {});", t, t, f));
                }
            }
        }
        t
    }

    pub(super) fn fmt_expr(&mut self, e: &Expr) -> String {
        let t = e.ty.clone();
        match &t {
            Type::I8 | Type::I16 | Type::I32 | Type::I64 => {
                let v = self.gen_expr(e);
                format!("myrt_i64_to_string((myrt_i64)({}))", v)
            }
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => {
                let v = self.gen_expr(e);
                format!("myrt_u64_to_string((myrt_u64)({}))", v)
            }
            Type::F32 | Type::F64 => {
                let v = self.gen_expr(e);
                format!("myrt_f64_to_string((double)({}))", v)
            }
            Type::Str => self.gen_expr(e),
            Type::Bool => {
                let v = self.gen_expr(e);
                format!(
                    "(({}) ? mylrt_str_lit(\"true\", 4) : mylrt_str_lit(\"false\", 5))",
                    v
                )
            }
            Type::Class(cid) => {
                let h = self.need_cls_str(*cid);
                let v = self.gen_expr(e);
                format!("{}({})", h, v)
            }
            Type::Enum(eid) => {
                let v = self.gen_expr(e);
                self.fmt_c(&v, &Type::Enum(*eid))
            }
            Type::Iface(id) => {
                let v = self.gen_expr(e);
                let w = self.tmp("ifc");
                self.line(&format!("{} {} = {};", self.ctype(&t), w, v));
                let field = self.iface_str_field(*id);
                format!("{}.vt->{}({}.obj)", w, field, w)
            }
            Type::Array(_) => {
                let h = self.need_arr_str(&t);
                let v = self.gen_expr(e);
                format!("{}({})", h, v)
            }
            _ => "mylrt_str_lit(\"\", 0)".to_string(),
        }
    }

    pub(super) fn fmt_c(&mut self, x: &str, t: &Type) -> String {
        match t {
            Type::I8 | Type::I16 | Type::I32 | Type::I64 => {
                format!("myrt_i64_to_string((myrt_i64)({}))", x)
            }
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => {
                format!("myrt_u64_to_string((myrt_u64)({}))", x)
            }
            Type::F32 | Type::F64 => format!("myrt_f64_to_string((double)({}))", x),
            Type::Str => x.to_string(),
            Type::Bool => format!(
                "(({}) ? mylrt_str_lit(\"true\", 4) : mylrt_str_lit(\"false\", 5))",
                x
            ),
            Type::Class(cid) => {
                let h = self.need_cls_str(*cid);
                format!("{}({})", h, x)
            }
            Type::Iface(id) => {
                let field = self.iface_str_field(*id);
                format!("(({}).vt->{}(({}).obj))", x, field, x)
            }
            Type::Enum(eid) => {
                let found = self.prog.decls[*eid]
                    .members
                    .iter()
                    .copied()
                    .find(|m| matches!(self.prog.decls[*m].name.as_str(), "toString" | "to_string"));
                match found {
                    Some(mid) => format!("{}({})", self.fn_cname(mid), x),
                    None => "mylrt_str_lit(\"<enum>\", 6)".to_string(),
                }
            }
            Type::Array(_) => {
                let h = self.need_arr_str(t);
                format!("{}({})", h, x)
            }
            _ => "mylrt_str_lit(\"\", 0)".to_string(),
        }
    }

    pub(super) fn need_cls_str(&mut self, cid: usize) -> String {
        let name = format!("h_cls_str_{}", cid);
        if self.helpers_done.insert(name.clone()) {
            if let Some(iid) = self.prog.tostring_iface {
                let cname = self.decl_cname(cid);
                let iname = self.iface_cname(iid);
                let vt = format!("VT_{}_{}", self.ctor_name(cid), self.prog.decls[iid].name);
                self.helper_protos
                    .push(format!("static MyStr* {}({}* o);", name, cname));
                let field = self.iface_str_field(iid);
                let body = format!(
                    "static MyStr* {name}({cname}* o) {{ {iname} w = {{ .obj = (void*)o, .vt = &{vt} }}; return w.vt->{field}(w.obj); }}"
                );
                self.helper_defs.push(body);
            }
        }
        name
    }

    fn iface_str_field(&self, iid: usize) -> String {
        self.prog.decls[iid]
            .members
            .iter()
            .copied()
            .find(|m| matches!(self.prog.decls[*m].name.as_str(), "toString" | "to_string"))
            .map(|m| format!("fn_{}", self.prog.decls[m].name))
            .unwrap_or_else(|| "fn_to_string".to_string())
    }

    pub(super) fn need_arr_str(&mut self, arr: &Type) -> String {
        let name = format!("h_arr_str_{}", self.mangle(arr));
        if self.helpers_done.insert(name.clone()) {
            let elem = match arr {
                Type::Array(e) => (**e).clone(),
                _ => Type::Infer,
            };
            let fmt = self.fmt_c("a->items[i]", &elem);
            let ct = self.ctype(arr);
            if arr.has_raw_ptr() {
                // 指针元素数组不做字符串化（sema 已拦截）
            }
            self.helper_protos
                .push(format!("static MyStr* {}({} a);", name, ct));
            let body = format!(
                "static MyStr* {}({} a) {{ MyStr* s = mylrt_str_lit(\"[\", 1); for (myrt_u64 i = 0; i < a->len; i++) {{ if (i) s = mylrt_str_concat(s, mylrt_str_lit(\", \", 2)); s = mylrt_str_concat(s, {}); }} return mylrt_str_concat(s, mylrt_str_lit(\"]\", 1)); }}",
                name, ct, fmt
            );
            self.helper_defs.push(body);
        }
        name
    }

    /// 把已经生成的表达式物化到临时变量，保证求值顺序和只求值一次。
    fn gen_order(&mut self, e: &Expr, value: String) -> String {
        let ty = if let Some((_, iid)) = e.box_to {
            Type::Iface(iid)
        } else if let Some((_, iid)) = e.coerce {
            Type::Iface(iid)
        } else if let Res::IfaceCast(_, iid) = e.res {
            Type::Iface(iid)
        } else {
            e.ty.clone()
        };
        let t = self.tmp("ord");
        self.line(&format!("{} {} = {};", self.ctype(&ty), t, value));
        t
    }

    pub(super) fn gen_bin(&mut self, op: BinOp, l: &Expr, r: &Expr, ty: &Type) -> String {
        use BinOp::*;
        // 逻辑短路必须在代码生成阶段处理：右侧可能包含需要提前落行的
        // 边界检查、接收者临时变量或调用；不能先生成右侧再交给 C 的 &&/||。
        if matches!(op, AndAnd | OrOr) {
            let a = self.gen_expr(l);
            let t = self.tmp("logic");
            self.line(&format!("{} {} = {};", self.ctype(&Type::Bool), t, a));
            let open = if op == AndAnd {
                format!("if ({}) {{", t)
            } else {
                format!("if (!{}) {{", t)
            };
            self.line(&open);
            self.ind += 1;
            let b = self.gen_expr(r);
            self.line(&format!("{} = {};", t, b));
            self.ind -= 1;
            self.line("}");
            return t;
        }
        let lt = l.ty.clone();
        // 字符串
        if lt == Type::Str {
            match op {
                Add => {
                    let a = self.gen_expr(l);
                    let a = self.gen_order(l, a);
                    let b = self.gen_expr(r);
                    let b = self.gen_order(r, b);
                    return format!("mylrt_str_concat({}, {})", a, b);
                }
                Eq | Ne => {
                    let a = self.gen_expr(l);
                    let a = self.gen_order(l, a);
                    let b = self.gen_expr(r);
                    let b = self.gen_order(r, b);
                    let base = format!("mylrt_str_eq({}, {})", a, b);
                    return if op == Eq { base } else { format!("(!{})", base) };
                }
                Lt | Le | Gt | Ge => {
                    let a = self.gen_expr(l);
                    let a = self.gen_order(l, a);
                    let b = self.gen_expr(r);
                    let b = self.gen_order(r, b);
                    let cmp = format!("mylrt_str_cmp({}, {})", a, b);
                    let o = match op {
                        Lt => "< 0",
                        Le => "<= 0",
                        Gt => "> 0",
                        _ => ">= 0",
                    };
                    return format!("({} {})", cmp, o);
                }
                _ => {}
            }
        }
        // 除零保护
        if matches!(op, Div | Rem) && ty.is_integer() {
            let a = self.gen_expr(l);
            let b = self.gen_expr(r);
            let ta = self.tmp("div");
            let tb = self.tmp("div");
            self.line(&format!("{} {} = {};", self.ctype(&lt), ta, a));
            self.line(&format!("{} {} = {};", self.ctype(&lt), tb, b));
            let zero = "0";
            return format!(
                "(({} == {}) ? (mylrt_panic_msg(\"division by zero\"), (({})0)) : (({} {} {})))",
                tb,
                zero,
                self.ctype(&lt),
                ta,
                if op == Div { "/" } else { "%" },
                tb
            );
        }
        let a = self.gen_expr(l);
        let a = if matches!(op, AndAnd | OrOr) { a } else { self.gen_order(l, a) };
        let b = self.gen_expr(r);
        let b = if matches!(op, AndAnd | OrOr) { b } else { self.gen_order(r, b) };
        // 有符号 + - * 走无符号算术再回转，避免 C 的有符号溢出 UB（Mylis 规定回绕）
        if ty.is_signed() && matches!(op, Add | Sub | Mul) {
            let u = self.unsigned_ctype(ty);
            let sct = self.ctype(ty);
            let o = match op {
                Add => "+",
                Sub => "-",
                _ => "*",
            };
            return format!(
                "(({s})(({u})({a}) {o} ({u})({b})))",
                s = sct,
                u = u,
                a = a,
                b = b,
                o = o
            );
        }
        let o = match op {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            AndAnd => "&&",
            OrOr => "||",
            BitAnd => "&",
            BitOr => "|",
            BitXor => "^",
            Shl => "<<",
            Shr => ">>",
        };
        format!("({} {} {})", a, o, b)
    }

    pub(super) fn gen_method(&mut self, e: &Expr, recv: &Expr, mname: &str, args: &[Expr]) -> String {
        match e.res {
            Res::Builtin(b) => {
                let rt = recv.ty.clone();
                let r = self.gen_expr(recv);
                let t = self.tmp("bi");
                self.line(&format!("{} {} = {};", self.ctype(&rt), t, r));
                match b {
                    Builtin::StrLen => format!("myrt_str_len({})", t),
                    Builtin::ArrLen => format!("((myrt_i32)({}->len))", t),
                    Builtin::ResIsOk => format!("({}.is_ok)", t),
                    Builtin::ResIsErr => format!("(!{}.is_ok)", t),
                    Builtin::ResUnwrap => {
                        let h = self.need_res_unwrap(&rt);
                        format!("{}({})", h, t)
                    }
                    Builtin::ResUnwrapErr => {
                        let h = self.need_res_unwrap_err(&rt);
                        format!("{}({})", h, t)
                    }
                    Builtin::ResUnwrapOr => {
                        let h = self.need_res_unwrap_or(&rt);
                        let d = self.gen_expr(&args[0]);
                        format!("{}({}, {})", h, t, d)
                    }
                    Builtin::OptIsSome => format!("({}.is_some)", t),
                    Builtin::OptIsNone => format!("(!{}.is_some)", t),
                    Builtin::OptUnwrap => {
                        let h = self.need_opt_unwrap(&rt);
                        format!("{}({})", h, t)
                    }
                    Builtin::OptUnwrapOr => {
                        let h = self.need_opt_unwrap_or(&rt);
                        let d = self.gen_expr(&args[0]);
                        format!("{}({}, {})", h, t, d)
                    }
                    Builtin::ValueEq => {
                        let v = self.gen_expr(&args[0]);
                        let a = self.gen_order(&args[0], v);
                        if rt == Type::Str {
                            format!("mylrt_str_eq({}, {})", t, a)
                        } else {
                            format!("({} == {})", t, a)
                        }
                    }
                    Builtin::ValueCmp => {
                        let v = self.gen_expr(&args[0]);
                        let a = self.gen_order(&args[0], v);
                        if rt == Type::Str {
                            format!("mylrt_str_cmp({}, {})", t, a)
                        } else {
                            format!(
                                "(({t} < {a}) ? -1 : (({t} > {a}) ? 1 : 0))",
                                t = t,
                                a = a
                            )
                        }
                    }
                    Builtin::ValueHash => {
                        if rt == Type::Str {
                            format!("mylrt_str_hash({})", t)
                        } else if rt == Type::F64 {
                            format!("mylrt_hash_f64({})", t)
                        } else if rt == Type::F32 {
                            format!("mylrt_hash_f64((double)({}))", t)
                        } else {
                            format!("mylrt_hash_u64((myrt_u64)({}))", t)
                        }
                    }
                }
            }
            Res::Method(mid) => {
                let owner = self.prog.decls[mid].owner;
                let is_iface = owner
                    .map(|o| self.prog.decls[o].kind == DeclKind::Iface)
                    .unwrap_or(false);
                if is_iface {
                    let rt = recv.ty.clone();
                    let r = self.gen_expr(recv);
                    let t = self.tmp("ifc");
                    self.line(&format!("{} {} = {};", self.ctype(&rt), t, r));
                    let mut a: Vec<String> = vec![format!("{}.obj", t)];
                    for x in args {
                        let v = self.gen_expr(x);
                        let t = self.gen_order(x, v);
                        a.push(t);
                    }
                    let call = format!("{}.vt->fn_{}({})", t, mname, a.join(", "));
                    if self.prog.decls[mid].ret == Type::Void {
                        self.line(&format!("{};", call));
                        "/*void*/".to_string()
                    } else {
                        call
                    }
                } else {
                    let base = self.gen_addrish(recv);
                    let rt = self.ctype(&recv.ty);
                    let rtok = self.tmp("recv");
                    self.line(&format!("{} {} = {};", rt, rtok, base));
                    let mut a: Vec<String> = vec![rtok];
                    for x in args {
                        let v = self.gen_expr(x);
                        let t = self.gen_order(x, v);
                        a.push(t);
                    }
                    format!("{}({})", self.fn_cname(mid), a.join(", "))
                }
            }
            _ => "/*未解析方法*/0".to_string(),
        }
    }

    pub(super) fn need_res_unwrap(&mut self, t: &Type) -> String {
        let (ok, err) = match t {
            Type::Result(a, b) => ((**a).clone(), (**b).clone()),
            _ => (Type::Void, Type::Void),
        };
        let name = format!("h_res_unwrap_{}", self.mangle(t));
        if self.helpers_done.insert(name.clone()) {
            self.helper_protos.push(format!("static {} {}({} v);", self.ctype(&ok), name, self.ctype(t)));
            let body = if let Type::Iface(id) = &ok {
                format!(
                    "if (!v.is_ok) mylrt_panic_unwrap(); return ({}){{ .obj = v.ok_obj, .vt = (const {}*)v.ok_vt }};",
                    self.iface_cname(*id), self.vt_cname(*id)
                )
            } else if matches!(err, Type::Iface(_)) {
                "if (!v.is_ok) mylrt_panic_unwrap(); return v.ok;".to_string()
            } else {
                "if (!v.is_ok) mylrt_panic_unwrap(); return v.u.ok;".to_string()
            };
            self.helper_defs.push(format!(
                "static {} {}({} v) {{ {} }}",
                self.ctype(&ok), name, self.ctype(t), body
            ));
        }
        name
    }

    pub(super) fn need_res_unwrap_err(&mut self, t: &Type) -> String {
        let err = match t {
            Type::Result(_, b) => (**b).clone(),
            _ => Type::Void,
        };
        let name = format!("h_res_unwrap_err_{}", self.mangle(t));
        if self.helpers_done.insert(name.clone()) {
            self.helper_protos
                .push(format!("static {} {}({} v);", self.ctype(&err), name, self.ctype(t)));
            let ok_iface = matches!(t, Type::Result(a, _) if matches!(&**a, Type::Iface(_)));
            let body = if let Type::Iface(id) = &err {
                format!(
                    "if (v.is_ok) mylrt_panic_unwrap(); return ({}){{ .obj = v.err_obj, .vt = (const {}*)v.err_vt }};",
                    self.iface_cname(*id), self.vt_cname(*id)
                )
            } else if ok_iface {
                "if (v.is_ok) mylrt_panic_unwrap(); return v.err;".to_string()
            } else {
                "if (v.is_ok) mylrt_panic_unwrap(); return v.u.err;".to_string()
            };
            self.helper_defs.push(format!(
                "static {} {}({} v) {{ {} }}",
                self.ctype(&err), name, self.ctype(t), body
            ));
        }
        name
    }

    pub(super) fn need_res_unwrap_or(&mut self, t: &Type) -> String {
        let ok = match t {
            Type::Result(a, _) => (**a).clone(),
            _ => Type::Void,
        };
        let name = format!("h_res_unwrap_or_{}", self.mangle(t));
        if self.helpers_done.insert(name.clone()) {
            self.helper_protos.push(format!(
                "static {} {}({} v, {} d);",
                self.ctype(&ok),
                name,
                self.ctype(t),
                self.ctype(&ok)
            ));
            let eif = matches!(t, Type::Result(_, b) if matches!(&**b, Type::Iface(_)));
            let val = if let Type::Iface(id) = &ok {
                format!("({}){{ .obj = v.ok_obj, .vt = (const {}*)v.ok_vt }}", self.iface_cname(*id), self.vt_cname(*id))
            } else if eif {
                "v.ok".to_string()
            } else {
                "v.u.ok".to_string()
            };
            self.helper_defs.push(format!(
                "static {} {}({} v, {} d) {{ return v.is_ok ? {} : d; }}",
                self.ctype(&ok),
                name,
                self.ctype(t),
                self.ctype(&ok),
                val
            ));
        }
        name
    }

    pub(super) fn need_opt_unwrap(&mut self, t: &Type) -> String {
        let inner = match t {
            Type::Option(a) => (**a).clone(),
            _ => Type::Void,
        };
        let name = format!("h_opt_unwrap_{}", self.mangle(t));
        if self.helpers_done.insert(name.clone()) {
            self.helper_protos.push(format!(
                "static {} {}({} v);",
                self.ctype(&inner),
                name,
                self.ctype(t)
            ));
            let body = if let Type::Iface(id) = &inner {
                format!(
                    "if (!v.is_some) mylrt_panic_unwrap(); return ({}){{ .obj = v.obj, .vt = (const {}*)v.vt }};",
                    self.iface_cname(*id), self.vt_cname(*id)
                )
            } else {
                "if (!v.is_some) mylrt_panic_unwrap(); return v.u.some;".to_string()
            };
            self.helper_defs.push(format!(
                "static {} {}({} v) {{ {} }}",
                self.ctype(&inner), name, self.ctype(t), body
            ));
        }
        name
    }

    pub(super) fn need_opt_unwrap_or(&mut self, t: &Type) -> String {
        let inner = match t {
            Type::Option(a) => (**a).clone(),
            _ => Type::Void,
        };
        let name = format!("h_opt_unwrap_or_{}", self.mangle(t));
        if self.helpers_done.insert(name.clone()) {
            self.helper_protos.push(format!(
                "static {} {}({} v, {} d);",
                self.ctype(&inner),
                name,
                self.ctype(t),
                self.ctype(&inner)
            ));
            let val = if let Type::Iface(id) = &inner {
                format!("({}){{ .obj = v.obj, .vt = (const {}*)v.vt }}", self.iface_cname(*id), self.vt_cname(*id))
            } else {
                "v.u.some".to_string()
            };
            self.helper_defs.push(format!(
                "static {} {}({} v, {} d) {{ return v.is_some ? {} : d; }}",
                self.ctype(&inner),
                name,
                self.ctype(t),
                self.ctype(&inner),
                val
            ));
        }
        name
    }
}

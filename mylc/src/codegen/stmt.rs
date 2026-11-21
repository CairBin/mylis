use super::*;

impl<'a> Codegen<'a> {
    pub(super) fn gen_block(&mut self, b: &Block) {
        self.line("{");
        self.ind += 1;
        self.push_scope();
        for s in &b.stmts {
            self.gen_stmt(s);
        }
        self.pop_scope();
        self.ind -= 1;
        self.line("}");
    }

    pub(super) fn gen_stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Var { ty, name, init, is_const } => {
                let ct = self.ctype(ty);
                let cname = format!("v_{}_{}", name, self.tmp_counter);
                self.tmp_counter += 1;
                let value = match init {
                    Some(e) => self.gen_expr(e),
                    None => self.default_value(ty),
                };
                let q = if *is_const { "const " } else { "" };
                self.line(&format!("{}{} {} = {};", q, ct, cname, value));
                self.barrier_slot(ty, &format!("&{}", cname), &cname);
                self.declare_local(name, &cname);
            }
            StmtKind::Assign { target, value } => {
                // 先物化右值：写屏障会再次引用右值，不能让它执行两次。
                let ty = target.ty.clone();
                let v = self.gen_expr(value);
                let vt = self.tmp("val");
                self.line(&format!("{} {} = {};", self.ctype(&ty), vt, v));
                let lhs = self.gen_place(target);
                self.line(&format!("{} = {};", lhs, vt));
                self.barrier_slot(&ty, &format!("&({})", lhs), &vt);
            }
            StmtKind::ExprStmt(e) => {
                let v = self.gen_expr(e);
                match &e.ty {
                    Type::Void => self.line(&format!("{};", v)),
                    _ => self.line(&format!("(void)({});", v)),
                }
            }
            StmtKind::If {
                cond,
                then_b,
                else_b,
            } => {
                let c = self.gen_cond(cond);
                self.line(&format!("if ({}) {{", c));
                self.ind += 1;
                self.push_scope();
                for st in &then_b.stmts {
                    self.gen_stmt(st);
                }
                self.pop_scope();
                self.ind -= 1;
                match else_b {
                    None => self.line("}"),
                    Some(eb) => {
                        self.line("} else {");
                        self.ind += 1;
                        self.gen_stmt(eb);
                        self.ind -= 1;
                        self.line("}");
                    }
                }
            }
            StmtKind::While { cond, body } => {
                self.continue_stack.push(None);
                self.line("while (1) {");
                self.ind += 1;
                self.push_scope();
                let c = self.gen_cond(cond);
                self.line(&format!("if (!({})) break;", c));
                for st in &body.stmts {
                    self.gen_stmt(st);
                }
                self.pop_scope();
                self.ind -= 1;
                self.line("}");
                self.continue_stack.pop();
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                let need_label = block_has_continue(body);
                let label = if need_label {
                    self.loop_counter += 1;
                    Some(format!("L_continue_{}", self.loop_counter))
                } else {
                    None
                };
                self.line("{");
                self.ind += 1;
                self.push_scope();
                self.gen_stmt(init);
                self.line("while (1) {");
                self.ind += 1;
                self.push_scope();
                if let Some(c) = cond {
                    let cs = self.gen_cond(c);
                    self.line(&format!("if (!({})) break;", cs));
                }
                self.continue_stack.push(label.clone());
                for st in &body.stmts {
                    self.gen_stmt(st);
                }
                self.continue_stack.pop();
                if let Some(l) = &label {
                    self.line(&format!("{}: ;", l));
                }
                if let Some(stx) = step {
                    self.gen_stmt(stx);
                }
                self.pop_scope();
                self.ind -= 1;
                self.line("}");
                self.pop_scope();
                self.ind -= 1;
                self.line("}");
            }
            StmtKind::Return(e) => match e {
                None => self.line("return;"),
                Some(e) => {
                    let v = self.gen_expr(e);
                    self.line(&format!("return {};", v));
                }
            },
            StmtKind::Break => self.line("break;"),
            StmtKind::Continue => match self.continue_stack.last().cloned().flatten() {
                Some(label) => self.line(&format!("goto {};", label)),
                None => self.line("continue;"),
            },
            StmtKind::Unsafe(b) => {
                self.line("/* unsafe */");
                self.push_scope();
                for st in &b.stmts {
                    self.gen_stmt(st);
                }
                self.pop_scope();
            }
            StmtKind::Block(b) => self.gen_block(b),
            StmtKind::Match { subject, arms } => self.gen_match(subject, arms),
        }
    }

    pub(super) fn gen_match(&mut self, subject: &Expr, arms: &[MatchArm]) {
        let st = subject.ty.clone();
        let v = self.gen_expr(subject);
        let t = self.tmp("m");
        self.line(&format!("{} {} = {};", self.ctype(&st), t, v));
        let mut first = true;
        let mut seen_wild = false;
        for arm in arms {
            let cond = self.pattern_cond(&arm.pat, &t, &st);
            match cond {
                Some(c) => {
                    if first {
                        self.line(&format!("if ({}) {{", c));
                        first = false;
                    } else {
                        self.line(&format!("}} else if ({}) {{", c));
                    }
                }
                None => {
                    seen_wild = true;
                    if first {
                        self.line("{");
                        first = false;
                    } else {
                        self.line("} else {");
                    }
                }
            }
            self.ind += 1;
            self.push_scope();
            self.bind_pattern(&arm.pat, &t, &st);
            for s2 in &arm.body.stmts {
                self.gen_stmt(s2);
            }
            self.pop_scope();
            self.ind -= 1;
        }
        if !seen_wild {
            self.line("} else {");
            self.ind += 1;
            self.line("mylrt_panic_msg(\"non-exhaustive match\");");
            self.ind -= 1;
        }
        self.line("}");
    }

    pub(super) fn bind_pattern(&mut self, pat: &Pattern, t: &str, st: &Type) {
        let (ok_iface, err_iface) = match st {
            Type::Result(a, b) => (
                matches!(&**a, Type::Iface(_)),
                matches!(&**b, Type::Iface(_)),
            ),
            _ => (false, false),
        };
        match pat {
            Pattern::Ok(Some(b)) => {
                let e = if let Type::Iface(id) = match st {
                    Type::Result(a, _) => &**a,
                    _ => &Type::Void,
                } {
                    format!("({}){{ .obj = {}.ok_obj, .vt = (const {}*){}.ok_vt }}",
                        self.iface_cname(*id), t, self.vt_cname(*id), t)
                } else if err_iface {
                    format!("{}.ok", t)
                } else {
                    format!("{}.u.ok", t)
                };
                self.declare_local(b, &e);
            }
            Pattern::Err(Some(b)) => {
                let e = if let Type::Iface(id) = match st {
                    Type::Result(_, b) => &**b,
                    _ => &Type::Void,
                } {
                    format!("({}){{ .obj = {}.err_obj, .vt = (const {}*){}.err_vt }}",
                        self.iface_cname(*id), t, self.vt_cname(*id), t)
                } else if ok_iface {
                    format!("{}.err", t)
                } else {
                    format!("{}.u.err", t)
                };
                self.declare_local(b, &e);
            }
            Pattern::Some(Some(b)) => {
                let e = if let Type::Iface(id) = match st {
                    Type::Option(a) => &**a,
                    _ => &Type::Void,
                } {
                    format!("({}){{ .obj = {}.obj, .vt = (const {}*){}.vt }}",
                        self.iface_cname(*id), t, self.vt_cname(*id), t)
                } else {
                    format!("{}.u.some", t)
                };
                self.declare_local(b, &e);
            }
            Pattern::Variant { enum_id, vidx, binds, .. } => {
                let pname = self.payload_cname(*enum_id, *vidx);
                for (i, b) in binds.iter().enumerate() {
                    if let Some(name) = b {
                        let expr = format!("(({}*)({}->data))->f{}", pname, t, i);
                        self.declare_local(name, &expr);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn pattern_cond(&mut self, pat: &Pattern, t: &str, st: &Type) -> Option<String> {
        let ct = self.ctype(st);
        Some(match pat {
            Pattern::Wildcard => return None,
            Pattern::Ok(_) => format!("{}.is_ok", t),
            Pattern::Err(_) => format!("(!{}.is_ok)", t),
            Pattern::Some(_) => format!("{}.is_some", t),
            Pattern::None => format!("(!{}.is_some)", t),
            Pattern::Int(v) => format!("({} == ({})({}))", t, ct, v),
            Pattern::IntU64(v) => format!("({} == ({})({}ULL))", t, ct, v),
            Pattern::Float(v) => format!("({} == ({})({:?}))", t, ct, v),
            Pattern::Str(sv) => {
                let esc = c_escape(sv);
                format!(
                    "mylrt_str_eq({}, mylrt_str_lit(\"{}\", {}))",
                    t,
                    esc,
                    sv.len()
                )
            }
            Pattern::Bool(b) => {
                if *b {
                    format!("({})", t)
                } else {
                    format!("(!({}))", t)
                }
            }
            Pattern::Null => format!("({} == 0)", t),
            Pattern::Variant { enum_id: _, vidx, .. } => format!(
                "({}->tag == (myrt_i32){})",
                t, vidx
            ),
        })
    }

    pub(super) fn gen_cond(&mut self, e: &Expr) -> String {
        self.gen_expr(e)
    }

    pub(super) fn default_value(&self, t: &Type) -> String {
        match t {
            Type::F32 | Type::F64 => "0.0".to_string(),
            Type::Str => "0".to_string(),
            Type::Ptr(_) | Type::Class(_) | Type::Enum(_) | Type::Array(_) => "0".to_string(),
            Type::Iface(id) => format!("(({}){{ .obj = 0, .vt = 0 }})", self.iface_cname(*id)),
            Type::Result(_, _) | Type::Option(_) => {
                format!("(({}){{0}})", self.ctype(t))
            }
            _ => "0".to_string(),
        }
    }

    pub(super) fn gen_place(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Ident(name) => self
                .lookup_local(name)
                .unwrap_or_else(|| format!("/*未解析变量 {}*/", name)),
            ExprKind::Field(recv, fname) => {
                let base = self.gen_addrish(recv);
                format!("{}->{}", base, fname)
            }
            ExprKind::Index(recv, idx) => {
                let base = self.gen_expr(recv);
                let i = self.gen_expr(idx);
                let b = self.tmp("arr");
                let it = self.tmp("idx");
                self.line(&format!("{} {} = {};", self.ctype(&recv.ty), b, base));
                self.line(&format!("myrt_i64 {} = (myrt_i64)({});", it, i));
                format!(
                    "{}->items[mylrt_arr_index({}->len, {})]",
                    b, b, it
                )
            }
            ExprKind::StaticPath { .. } => match e.res {
                Res::StaticField(cid, idx) => self.static_field_cname(cid, idx),
                _ => "/*未解析静态字段*/0".to_string(),
            },
            ExprKind::Un(UnOp::Deref, inner) => {
                let v = self.gen_expr(inner);
                let t = self.tmp("ptr");
                self.line(&format!("{} {} = {};", self.ctype(&inner.ty), t, v));
                format!("(*{})", t)
            }
            _ => "/*非法左值*/".to_string(),
        }
    }

    pub(super) fn gen_addrish(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Ident(_) | ExprKind::Field(_, _) => self.gen_expr(e),
            _ => {
                let v = self.gen_expr(e);
                let ty = e.ty.clone();
                let t = self.tmp("recv");
                self.line(&format!("{} {} = {};", self.ctype(&ty), t, v));
                t
            }
        }
    }
}

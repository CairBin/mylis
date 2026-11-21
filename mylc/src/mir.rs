//! MIR（Mylis IR）v0：函数级 CFG + SSA 值 + 显式局部槽。
//!
//! 这一层只依赖已通过 sema 的 `ast::Program`，不依赖 C codegen。
//! 当前目标是把前端与具体后端解耦：
//!
//! ```text
//! AST --lower--> MIR --dump--> .myl.ir
//!                  |
//!                  +--(下一步)--> C backend / LLVM backend
//! ```
//!
//! v0 已覆盖：常量、局部变量、算术/逻辑、调用/方法调用/字段/下标、
//! `new`/数组、cast、try、字符串插值、if/while/for/break/continue/return，
//! 以及 `match` 的模式测试 + 绑定槽 lowering。

use crate::ast::{
    BinOp, Block, Decl, DeclKind, Expr, ExprKind, InterpPart, MatchArm, Pattern, Program, Res,
    Stmt, StmtKind, Type, UnOp,
};
use std::collections::HashMap;
use std::fmt::{self, Write};

mod pass;

// ============================================================== 数据结构

#[derive(Clone, Debug)]
pub struct MirProgram {
    pub funcs: Vec<MirFunction>,
    pub classes: Vec<MirClass>,
    pub enums: Vec<MirEnum>,
    pub ifaces: Vec<MirInterface>,
}

#[derive(Clone, Debug)]
pub struct MirInterface {
    pub name: String,
    pub decl_id: usize,
    pub methods: Vec<MirIfaceMethod>,
}

#[derive(Clone, Debug)]
pub struct MirIfaceMethod {
    pub name: String,
    pub decl_id: usize,
    pub ret: Type,
    pub params: Vec<Type>,
}

#[derive(Clone, Debug)]
pub struct MirEnum {
    pub name: String,
    pub decl_id: usize,
    pub variants: Vec<MirVariant>,
}

#[derive(Clone, Debug)]
pub struct MirVariant {
    pub name: String,
    pub fields: Vec<Type>,
}

#[derive(Clone, Debug)]
pub struct MirClass {
    pub name: String,
    /// class 声明 id（`Res::Field` 的第一个参数）
    pub decl_id: usize,
    pub fields: Vec<MirField>,
    /// 构造器声明 id（`init` / 与类同名）
    pub ctor: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct MirField {
    pub name: String,
    pub ty: Type,
    pub is_static: bool,
    pub is_pub: bool,
    pub init: Option<MirConst>,
}

#[derive(Clone, Debug)]
pub struct MirFunction {
    pub name: String,
    /// 声明 id、所属类型、静态/构造器标记
    pub decl_id: usize,
    pub owner: Option<usize>,
    pub is_static: bool,
    pub is_ctor: bool,
    pub params: Vec<MirLocal>,
    pub locals: Vec<MirLocal>,
    pub ret: Type,
    pub blocks: Vec<MirBlock>,
}

#[derive(Clone, Debug)]
pub struct MirLocal {
    pub name: String,
    pub ty: Type,
    pub mutable: bool,
}

#[derive(Clone, Debug)]
pub struct MirBlock {
    pub label: String,
    pub insts: Vec<MirInst>,
    pub term: Option<MirTerm>,
}

#[derive(Clone, Debug)]
pub struct MirInst {
    /// 产生值时非空；void 指令为 None。
    pub dst: Option<u32>,
    /// 结果类型（void 指令为 `Type::Void`）。
    pub ty: Type,
    /// sema 的解析结果（字段下标、方法/构造器 id、内置函数等）。
    pub res: Res,
    /// class/enum → interface 的自动转换
    pub coerce: Option<(usize, usize)>,
    pub kind: MirInstKind,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub enum MirInstKind {
    Const(MirConst),
    Undef,
    Copy(MirOperand),
    Store { local: u32, value: MirOperand },
    Bin(BinOp, MirOperand, MirOperand),
    Un(UnOp, MirOperand),
    Call { callee: String, args: Vec<MirOperand> },
    MethodCall { recv: MirOperand, name: String, args: Vec<MirOperand> },
    Field { base: MirOperand, name: String },
    StoreField { base: MirOperand, name: String, value: MirOperand },
    Index { base: MirOperand, index: MirOperand },
    StoreIndex { base: MirOperand, index: MirOperand, value: MirOperand },
    New { class: String, args: Vec<MirOperand> },
    ArrayNew { elem: String, len: MirOperand },
    ArrayLit { elem: String, values: Vec<MirOperand> },
    Cast { value: MirOperand, target: Type, checked: bool },
    Box { value: MirOperand, from: Type },
    Try { value: MirOperand },
    Interp { parts: Vec<MirInterpPart> },
    Global(String),
    /// `Class::name`（值语义的静态字段/无调用路径）
    StaticRef(String),
    StoreStatic { name: String, value: MirOperand },
    StorePtr { ptr: MirOperand, value: MirOperand },
    /// 模式测试：对 subject 判断是否匹配 pattern，结果是 bool。
    MatchTest { subject: MirOperand, pattern: Pattern },
    /// 把 pattern 的载荷绑定到一组局部槽（按绑定顺序）。
    BindPattern { subject: MirOperand, pattern: Pattern, locals: Vec<u32> },
    /// GC 安全点：调用/分配前的活跃引用槽标注。
    /// roots 由反向活跃性分析得到；LLVM/MIR-C 后端按 roots 并集注册帧根，
    /// 精确模式（MYLIS_GC_PRECISE=1）只扫描这些显式根区间。
    Safepoint { roots: Vec<MirOperand> },
    /// 明确标记“尚未 lowering”的表达式/语句，便于统计覆盖率。
    Opaque(String),
}

#[derive(Clone, Debug)]
pub enum MirInterpPart {
    Lit(String),
    Value(MirOperand),
}

#[derive(Clone, Debug)]
pub enum MirConst {
    Int(i64),
    IntU64(u64),
    Float(f64),
    Bool(bool),
    Str(String),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirOperand {
    /// SSA 值 `%n`
    Value(u32),
    /// 局部槽 `$n`（读取即 load）
    Local(u32),
}

#[derive(Clone, Debug)]
pub enum MirTerm {
    Return(Option<MirOperand>),
    Goto(String),
    Branch { cond: MirOperand, then_label: String, else_label: String },
    Unreachable,
}

// ============================================================== 入口

pub fn lower(prog: &Program) -> MirProgram {
    let mut p = lower_raw(prog);
    pass::simplify(&mut p);
    p
}

fn lower_raw(prog: &Program) -> MirProgram {
    let mut funcs = Vec::new();
    for (decl_id, d) in prog.decls.iter().enumerate() {
        if d.kind == DeclKind::Fn && d.body.is_some() {
            funcs.push(lower_function(prog, d, decl_id));
        }
    }
    let mut classes = Vec::new();
    for (decl_id, d) in prog.decls.iter().enumerate() {
        if d.kind == DeclKind::Class {
            classes.push(MirClass {
                name: qualified_type_name(prog, d),
                decl_id,
                ctor: d
                    .members
                    .iter()
                    .copied()
                    .find(|m| prog.decls[*m].is_ctor),
                fields: d
                    .fields
                    .iter()
                    .map(|f| MirField {
                        name: f.name.clone(),
                        ty: f.ty.clone(),
                        is_static: f.is_static,
                        is_pub: f.is_pub,
                        init: f.init.as_ref().and_then(|e| match &e.kind {
                            ExprKind::Int(v) => Some(MirConst::Int(*v)),
                            ExprKind::IntU64(v) => Some(MirConst::IntU64(*v)),
                            ExprKind::Float(v) => Some(MirConst::Float(*v)),
                            ExprKind::Str(v) => Some(MirConst::Str(v.clone())),
                            ExprKind::Bool(v) => Some(MirConst::Bool(*v)),
                            ExprKind::Null => Some(MirConst::Null),
                            _ => None,
                        }),
                    })
                    .collect(),
            });
        }
    }
    let mut enums = Vec::new();
    for (decl_id, d) in prog.decls.iter().enumerate() {
        if d.kind == DeclKind::Enum {
            enums.push(MirEnum {
                name: qualified_type_name(prog, d),
                decl_id,
                variants: d
                    .variants
                    .iter()
                    .map(|v| MirVariant { name: v.name.clone(), fields: v.fields.clone() })
                    .collect(),
            });
        }
    }
    let mut ifaces = Vec::new();
    for (decl_id, d) in prog.decls.iter().enumerate() {
        if d.kind == DeclKind::Iface {
            // 含 `Self` 的接口只作为 bound 协议使用（如 Eq/Ord），
            // 不生成运行时 interface 值/vtable，避免把未实例化的 Self 带进后端。
            let bound_only = d.members.iter().any(|m| {
                let md = &prog.decls[*m];
                md.params.iter().any(|p| has_self_type(&p.ty)) || has_self_type(&md.ret)
            });
            if bound_only {
                continue;
            }
            ifaces.push(MirInterface {
                name: qualified_type_name(prog, d),
                decl_id,
                methods: d
                    .members
                    .iter()
                    .filter_map(|m| {
                        let md = &prog.decls[*m];
                        Some(MirIfaceMethod {
                            name: md.name.clone(),
                            decl_id: *m,
                            ret: md.ret.clone(),
                            params: md.params.iter().map(|p| p.ty.clone()).collect(),
                        })
                    })
                    .collect(),
            });
        }
    }
    MirProgram { funcs, classes, enums, ifaces }
}

fn module_prefix(prog: &Program, mod_id: usize) -> String {
    prog.modules
        .get(mod_id)
        .map(|m| m.full_name())
        .unwrap_or_else(|| "?".to_string())
}

fn qualified_type_name(prog: &Program, d: &Decl) -> String {
    if let Some(m) = prog.modules.get(d.mod_id) {
        // 类型与所在模块同名时（如模块 A 里的 class A），完整名就是模块名，不再重复末段。
        if m.module_leaf() == Some(d.name.as_str()) {
            return m.full_name();
        }
    }
    format!("{}::{}", module_prefix(prog, d.mod_id), d.name)
}

fn function_name(prog: &Program, d: &Decl) -> String {
    let short = d.name.rsplit("::").next().unwrap_or(&d.name);
    if let Some(owner) = d.owner {
        let owner = qualified_type_name(prog, &prog.decls[owner]);
        return format!("{}::{}", owner, short);
    }
    format!("{}::{}", module_prefix(prog, d.mod_id), short)
}

fn lower_function(prog: &Program, d: &Decl, decl_id: usize) -> MirFunction {
    let mut f = FuncLower {
        name: function_name(prog, d),
        ret: d.ret.clone(),
        locals: Vec::new(),
        names: HashMap::new(),
        blocks: Vec::new(),
        cur: 0,
        next_value: 1,
        next_label: 1,
        loops: Vec::new(),
        variant_fields: prog
            .decls
            .iter()
            .map(|d| d.variants.iter().map(|v| v.fields.clone()).collect())
            .collect(),
    };
    if d.owner.is_some() && !d.is_static {
        let ty = d
            .owner
            .map(|id| Type::Class(id))
            .unwrap_or(Type::Infer);
        f.add_local("this", ty, false);
    }
    for p in &d.params {
        f.add_local(&p.name, p.ty.clone(), false);
    }
    f.new_block("bb0");
    if let Some(body) = &d.body {
        f.block(body);
    }
    f.finish();
    MirFunction {
        name: f.name,
        decl_id,
        owner: d.owner,
        is_static: d.is_static,
        is_ctor: d.is_ctor,
        params: f.locals.iter().take(d.params.len() + usize::from(d.owner.is_some() && !d.is_static)).cloned().collect(),
        locals: f.locals,
        ret: f.ret,
        blocks: f.blocks,
    }
}

// ============================================================== lowering

struct FuncLower {
    name: String,
    ret: Type,
    locals: Vec<MirLocal>,
    names: HashMap<String, u32>,
    blocks: Vec<MirBlock>,
    cur: usize,
    next_value: u32,
    next_label: u32,
    /// (break 标签, continue 标签)
    loops: Vec<(String, String)>,
    variant_fields: Vec<Vec<Vec<Type>>>,
}

impl FuncLower {
    fn add_local(&mut self, name: &str, ty: Type, mutable: bool) -> u32 {
        let id = self.locals.len() as u32;
        self.locals.push(MirLocal {
            name: name.to_string(),
            ty,
            mutable,
        });
        self.names.insert(name.to_string(), id);
        id
    }

    fn new_block(&mut self, label: &str) -> usize {
        let idx = self.blocks.len();
        self.blocks.push(MirBlock {
            label: label.to_string(),
            insts: Vec::new(),
            term: None,
        });
        idx
    }

    fn fresh_label(&mut self, prefix: &str) -> String {
        let l = format!("{}{}", prefix, self.next_label);
        self.next_label += 1;
        l
    }

    fn ended(&self) -> bool {
        self.blocks[self.cur].term.is_some()
    }

    /// 只给“可到达且缺少终结指令”的块补 return；不可达块统一 `unreachable`。
    fn finish(&mut self) {
        let reachable = reachable_blocks(&self.blocks);
        for idx in &reachable {
            if self.blocks[*idx].term.is_none() {
                let term = if self.ret == Type::Void {
                    MirTerm::Return(None)
                } else {
                    let line = 0;
                    let v = self.undef_in(*idx, line);
                    MirTerm::Return(Some(v))
                };
                self.blocks[*idx].term = Some(term);
            }
        }
        for (idx, b) in self.blocks.iter_mut().enumerate() {
            if b.term.is_none() {
                let _ = idx;
                b.term = Some(MirTerm::Unreachable);
            }
        }
    }

    /// 在指定块末尾追加 undef（用于非法/不可达路径的占位返回值）。
    fn undef_in(&mut self, block: usize, line: u32) -> MirOperand {
        let v = self.next_value;
        self.next_value += 1;
        self.blocks[block].insts.push(MirInst {
            dst: Some(v),
            ty: Type::Infer,
            res: Res::None,
            coerce: None,
            kind: MirInstKind::Undef,
            line,
        });
        MirOperand::Value(v)
    }

    fn ensure_open(&mut self) {
        if self.ended() {
            let l = self.fresh_label("dead");
            self.cur = self.new_block(&l);
        }
    }

    fn terminate(&mut self, term: MirTerm) {
        if self.blocks[self.cur].term.is_none() {
            self.blocks[self.cur].term = Some(term);
        }
    }

    fn goto(&mut self, label: &str) {
        self.terminate(MirTerm::Goto(label.to_string()));
    }

    fn inst_full(
        &mut self,
        kind: MirInstKind,
        line: u32,
        ty: Type,
        res: Res,
        coerce: Option<(usize, usize)>,
    ) -> Option<u32> {
        self.ensure_open();
        let dst = if is_void_kind(&kind) || ty == Type::Void {
            None
        } else {
            let v = self.next_value;
            self.next_value += 1;
            Some(v)
        };
        self.blocks[self.cur].insts.push(MirInst { dst, ty, res, coerce, kind, line });
        dst
    }

    fn inst(&mut self, kind: MirInstKind, line: u32) -> Option<u32> {
        self.inst_full(kind, line, Type::Void, Res::None, None)
    }

    fn inst_ty(&mut self, kind: MirInstKind, line: u32, ty: &Type) -> MirOperand {
        let dst = self.inst_full(kind, line, ty.clone(), Res::None, None);
        match dst {
            Some(v) => MirOperand::Value(v),
            // void 表达式当成一个显式 undef 值，保证调用方总有操作数
            None => {
                if *ty == Type::Void {
                    MirOperand::Value(0)
                } else {
                    self.undef(line)
                }
            }
        }
    }

    fn inst_expr(&mut self, kind: MirInstKind, e: &Expr) -> MirOperand {
        let ty = match &e.box_to {
            Some((from, _)) => from.clone(),
            None => e.ty.clone(),
        };
        let res = e.res.clone();
        let coerce = e.coerce;
        let dst = self.inst_full(kind, e.line, ty.clone(), res, coerce);
        let value = match dst {
            Some(v) => MirOperand::Value(v),
            None => {
                if ty == Type::Void { MirOperand::Value(0) } else { self.undef(e.line) }
            }
        };
        match &e.box_to {
            Some((from, iid)) => self.inst_ty(
                MirInstKind::Box { value, from: from.clone() },
                e.line,
                &Type::Iface(*iid),
            ),
            None => value,
        }
    }

    fn undef(&mut self, line: u32) -> MirOperand {
        match self.inst(MirInstKind::Undef, line) {
            Some(v) => MirOperand::Value(v),
            None => MirOperand::Value(0),
        }
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        self.ensure_open();
        match &s.kind {
            StmtKind::Var { ty, name, init, is_const } => {
                let local = self.add_local(name, ty.clone(), !is_const);
                let (value, coerce) = match init {
                    Some(e) => {
                        let c = e.coerce;
                        (self.expr(e), c)
                    }
                    None => (self.undef(s.line), None),
                };
                self.inst_full(
                    MirInstKind::Store { local, value },
                    s.line,
                    Type::Void,
                    Res::None,
                    coerce,
                );
            }
            StmtKind::Assign { target, value } => {
                let c = value.coerce;
                let v = self.expr(value);
                self.assign(target, v, c, s.line);
            }
            StmtKind::ExprStmt(e) => {
                self.expr(e);
            }
            StmtKind::If { cond, then_b, else_b } => self.if_stmt(cond, then_b, else_b.as_deref(), s.line),
            StmtKind::While { cond, body } => self.while_stmt(cond, body, s.line),
            StmtKind::For { init, cond, step, body } => {
                self.stmt(init);
                let head = self.fresh_label("for.cond");
                let body_l = self.fresh_label("for.body");
                let step_l = self.fresh_label("for.step");
                let exit_l = self.fresh_label("for.end");
                self.goto(&head);
                self.cur = self.new_block(&head);
                let c = match cond {
                    Some(e) => self.expr(e),
                    None => self.inst_ty(MirInstKind::Const(MirConst::Bool(true)), s.line, &Type::Bool),
                };
                self.terminate(MirTerm::Branch {
                    cond: c,
                    then_label: body_l.clone(),
                    else_label: exit_l.clone(),
                });
                self.cur = self.new_block(&body_l);
                self.loops.push((exit_l.clone(), step_l.clone()));
                self.block(body);
                self.loops.pop();
                if !self.ended() {
                    self.goto(&step_l);
                }
                self.cur = self.new_block(&step_l);
                if let Some(st) = step {
                    self.stmt(st);
                }
                if !self.ended() {
                    self.goto(&head);
                }
                self.cur = self.new_block(&exit_l);
            }
            StmtKind::Return(v) => {
                let val = v.as_ref().map(|e| self.expr(e));
                self.terminate(MirTerm::Return(val));
            }
            StmtKind::Break => {
                if let Some((b, _)) = self.loops.last() {
                    let b = b.clone();
                    self.goto(&b);
                } else {
                    self.inst(MirInstKind::Opaque("break outside loop".into()), s.line);
                }
            }
            StmtKind::Continue => {
                if let Some((_, c)) = self.loops.last() {
                    let c = c.clone();
                    self.goto(&c);
                } else {
                    self.inst(MirInstKind::Opaque("continue outside loop".into()), s.line);
                }
            }
            StmtKind::Unsafe(b) | StmtKind::Block(b) => self.block(b),
            StmtKind::Match { subject, arms } => {
                let subj = self.expr(subject);
                let sty = subject.ty.clone();
                self.match_stmt(subj, arms, s.line, &sty);
            }
        }
    }

    fn if_stmt(&mut self, cond: &Expr, then_b: &Block, else_b: Option<&Stmt>, line: u32) {
        let c = self.expr(cond);
        let then_l = self.fresh_label("if.then");
        let else_l = self.fresh_label("if.else");
        let end_l = self.fresh_label("if.end");
        self.terminate(MirTerm::Branch {
            cond: c,
            then_label: then_l.clone(),
            else_label: else_l.clone(),
        });
        self.cur = self.new_block(&then_l);
        self.block(then_b);
        if !self.ended() {
            self.goto(&end_l);
        }
        self.cur = self.new_block(&else_l);
        if let Some(e) = else_b {
            self.stmt(e);
        }
        if !self.ended() {
            self.goto(&end_l);
        }
        self.cur = self.new_block(&end_l);
        let _ = line;
    }

    fn while_stmt(&mut self, cond: &Expr, body: &Block, line: u32) {
        let head = self.fresh_label("while.cond");
        let body_l = self.fresh_label("while.body");
        let exit_l = self.fresh_label("while.end");
        self.goto(&head);
        self.cur = self.new_block(&head);
        let c = self.expr(cond);
        self.terminate(MirTerm::Branch {
            cond: c,
            then_label: body_l.clone(),
            else_label: exit_l.clone(),
        });
        self.cur = self.new_block(&body_l);
        self.loops.push((exit_l.clone(), head.clone()));
        self.block(body);
        self.loops.pop();
        if !self.ended() {
            self.goto(&head);
        }
        self.cur = self.new_block(&exit_l);
        let _ = line;
    }

    fn match_stmt(&mut self, subject: MirOperand, arms: &[MatchArm], line: u32, sty: &Type) {
        let end = self.fresh_label("match.end");
        for arm in arms {
            let body = self.fresh_label("match.arm");
            let next = self.fresh_label("match.next");
            let cond = self.inst_ty(
                MirInstKind::MatchTest { subject, pattern: arm.pat.clone() },
                arm.line,
                &Type::Bool,
            );
            self.terminate(MirTerm::Branch {
                cond,
                then_label: body.clone(),
                else_label: next.clone(),
            });
            self.cur = self.new_block(&body);
            let locals = self.bind_locals(&arm.pat, sty);
            if !locals.is_empty() {
                self.inst(
                    MirInstKind::BindPattern {
                        subject,
                        pattern: arm.pat.clone(),
                        locals,
                    },
                    arm.line,
                );
            }
            self.block(&arm.body);
            if !self.ended() {
                self.goto(&end);
            }
            self.cur = self.new_block(&next);
        }
        // 所有臂都不匹配：sema 已保证穷尽；这里保留显式不可达。
        self.terminate(MirTerm::Unreachable);
        self.cur = self.new_block(&end);
        let _ = line;
    }

    fn bind_locals(&mut self, p: &Pattern, sty: &Type) -> Vec<u32> {
        let mut items: Vec<(String, Type)> = Vec::new();
        match p {
            Pattern::Ok(Some(n)) => {
                let t = match sty { Type::Result(ok, _) => (**ok).clone(), _ => Type::Infer };
                items.push((n.clone(), t));
            }
            Pattern::Err(Some(n)) => {
                let t = match sty { Type::Result(_, err) => (**err).clone(), _ => Type::Infer };
                items.push((n.clone(), t));
            }
            Pattern::Some(Some(n)) => {
                let t = match sty { Type::Option(ok) => (**ok).clone(), _ => Type::Infer };
                items.push((n.clone(), t));
            }
            Pattern::Variant { enum_id, vidx, binds, .. } => {
                let fields = self
                    .variant_fields
                    .get(*enum_id)
                    .and_then(|v| v.get(*vidx))
                    .cloned()
                    .unwrap_or_default();
                for (i, b) in binds.iter().enumerate() {
                    if let Some(n) = b {
                        items.push((n.clone(), fields.get(i).cloned().unwrap_or(Type::Infer)));
                    }
                }
            }
            _ => {}
        }
        items
            .iter()
            .map(|(n, t)| self.add_local(n, t.clone(), false))
            .collect()
    }

    fn assign(&mut self, target: &Expr, value: MirOperand, coerce: Option<(usize, usize)>, line: u32) {
        match &target.kind {
            ExprKind::Ident(name) => {
                if let Some(&local) = self.names.get(name) {
                    self.inst_full(
                        MirInstKind::Store { local, value },
                        line,
                        Type::Void,
                        target.res.clone(),
                        coerce.or(target.coerce),
                    );
                } else {
                    self.inst(
                        MirInstKind::Opaque(format!("assign to unknown `{}`", name)),
                        line,
                    );
                }
            }
            ExprKind::Field(base, name) => {
                let b = self.expr(base);
                self.inst_full(
                    MirInstKind::StoreField { base: b, name: name.clone(), value },
                    line,
                    Type::Void,
                    target.res.clone(),
                    coerce.or(target.coerce),
                );
            }
            ExprKind::Un(UnOp::Deref, ptr) => {
                let p = self.expr(ptr);
                self.inst(
                    MirInstKind::StorePtr { ptr: p, value },
                    line,
                );
            }
            ExprKind::StaticPath { class, name, .. } => {
                self.inst_full(
                    MirInstKind::StoreStatic { name: format!("{}::{}", class, name), value },
                    line,
                    Type::Void,
                    target.res.clone(),
                    coerce.or(target.coerce),
                );
            }
            ExprKind::Index(base, idx) => {
                let b = self.expr(base);
                let i = self.expr(idx);
                self.inst_full(
                    MirInstKind::StoreIndex { base: b, index: i, value },
                    line,
                    Type::Void,
                    target.res.clone(),
                    target.coerce,
                );
            }
            _ => {
                self.inst(MirInstKind::Opaque("unsupported assignment target".into()), line);
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> MirOperand {
        self.ensure_open();
        let line = e.line;
        match &e.kind {
            ExprKind::Int(v) => self.inst_expr(MirInstKind::Const(MirConst::Int(*v)), e),
            ExprKind::IntU64(v) => self.inst_expr(MirInstKind::Const(MirConst::IntU64(*v)), e),
            ExprKind::Float(v) => self.inst_expr(MirInstKind::Const(MirConst::Float(*v)), e),
            ExprKind::Str(v) => self.inst_expr(MirInstKind::Const(MirConst::Str(v.clone())), e),
            ExprKind::Bool(v) => self.inst_expr(MirInstKind::Const(MirConst::Bool(*v)), e),
            ExprKind::Null => self.inst_expr(MirInstKind::Const(MirConst::Null), e),
            ExprKind::This => self.load_name("this", line),
            ExprKind::Ident(name) => self.load_name(name, line),
            ExprKind::Bin(op, a, b) if matches!(op, BinOp::AndAnd | BinOp::OrOr) => {
                self.logical_expr(*op, a, b, e)
            }
            ExprKind::Bin(op, a, b) => {
                let x = self.expr(a);
                let y = self.expr(b);
                self.inst_expr(MirInstKind::Bin(*op, x, y), e)
            }
            ExprKind::Un(op, a) => {
                let x = self.expr(a);
                self.inst_expr(MirInstKind::Un(*op, x), e)
            }
            ExprKind::Call(name, _targs, args) => {
                let av = self.args(args);
                self.inst_expr(MirInstKind::Call { callee: name.clone(), args: av }, e)
            }
            ExprKind::Method(recv, name, _targs, args) => {
                let r = self.expr(recv);
                let av = self.args(args);
                self.inst_expr(
                    MirInstKind::MethodCall { recv: r, name: name.clone(), args: av },
                    e,
                )
            }
            ExprKind::Field(base, name) => {
                let b = self.expr(base);
                self.inst_expr(MirInstKind::Field { base: b, name: name.clone() }, e)
            }
            ExprKind::New(class, _targs, args) => {
                let av = self.args(args);
                self.inst_expr(MirInstKind::New { class: class.clone(), args: av }, e)
            }
            ExprKind::Ok(v) | ExprKind::Err(v) | ExprKind::Some_(v) => {
                let name = match &e.kind {
                    ExprKind::Ok(_) => "Ok",
                    ExprKind::Err(_) => "Err",
                    _ => "Some",
                };
                let x = self.expr(v);
                self.inst_expr(MirInstKind::Call { callee: name.to_string(), args: vec![x] }, e)
            }
            ExprKind::None_ => self.inst_expr(
                MirInstKind::Call { callee: "None".into(), args: Vec::new() },
                e,
            ),
            ExprKind::Cast(v, ty, checked) => {
                if let Some((from, iid)) = &e.box_to {
                    let x = self.expr(v);
                    return self.inst_ty(
                        MirInstKind::Box { value: x, from: from.clone() },
                        e.line,
                        &Type::Iface(*iid),
                    );
                }
                let x = self.expr(v);
                self.inst_expr(
                    MirInstKind::Cast { value: x, target: ty.clone(), checked: *checked },
                    e,
                )
            }
            ExprKind::Try(v) => {
                let x = self.expr(v);
                self.inst_expr(MirInstKind::Try { value: x }, e)
            }
            ExprKind::Index(base, idx) => {
                let b = self.expr(base);
                let i = self.expr(idx);
                self.inst_expr(MirInstKind::Index { base: b, index: i }, e)
            }
            ExprKind::ArrayLit(values) => {
                let vals = self.args(values);
                let elem = match &e.ty {
                    Type::Array(t) => t.to_string(),
                    _ => "?".into(),
                };
                self.inst_expr(MirInstKind::ArrayLit { elem, values: vals }, e)
            }
            ExprKind::ArrayNew(ty, len) => {
                let n = self.expr(len);
                self.inst_expr(MirInstKind::ArrayNew { elem: ty.to_string(), len: n }, e)
            }
            ExprKind::Interp(parts) => {
                let mut ir = Vec::new();
                for p in parts {
                    match p {
                        InterpPart::Lit(s) => ir.push(MirInterpPart::Lit(s.clone())),
                        InterpPart::Expr(x) => {
                            let v = self.expr(x);
                            ir.push(MirInterpPart::Value(v));
                        }
                    }
                }
                self.inst_expr(MirInstKind::Interp { parts: ir }, e)
            }
            ExprKind::StaticPath { class, name, args, call, .. } => {
                if *call {

                    let av = self.args(args);
                    self.inst_expr(
                        MirInstKind::Call {
                            callee: format!("{}::{}", class, name),
                            args: av,
                        },
                        e,
                    )
                } else {
                    self.inst_expr(
                        MirInstKind::StaticRef(format!("{}::{}", class, name)),
                        e,
                    )
                }
            }
        }
    }

    fn args(&mut self, args: &[Expr]) -> Vec<MirOperand> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            let v = self.expr(a);
            out.push(v);
        }
        out
    }

    /// 短路求值：`&&` 左侧为假、`||` 左侧为真时不再求右侧。
    fn logical_expr(&mut self, op: BinOp, a: &Expr, b: &Expr, e: &Expr) -> MirOperand {
        let line = e.line;
        let left = self.expr(a);
        let init = matches!(op, BinOp::OrOr);
        let tmp_name = format!("__short{}", self.next_value);
        let tmp = self.add_local(&tmp_name, Type::Bool, true);
        let init_v = self.inst_ty(
            MirInstKind::Const(MirConst::Bool(init)),
            line,
            &Type::Bool,
        );
        self.inst(MirInstKind::Store { local: tmp, value: init_v }, line);
        let rhs_l = self.fresh_label("logic.rhs");
        let done_l = self.fresh_label("logic.end");
        if init {
            self.terminate(MirTerm::Branch {
                cond: left,
                then_label: done_l.clone(),
                else_label: rhs_l.clone(),
            });
        } else {
            self.terminate(MirTerm::Branch {
                cond: left,
                then_label: rhs_l.clone(),
                else_label: done_l.clone(),
            });
        }
        self.cur = self.new_block(&rhs_l);
        let right = self.expr(b);
        self.inst(MirInstKind::Store { local: tmp, value: right }, line);
        if !self.ended() {
            self.goto(&done_l);
        }
        self.cur = self.new_block(&done_l);
        MirOperand::Local(tmp)
    }

    fn load_name(&mut self, name: &str, line: u32) -> MirOperand {
        if let Some(&local) = self.names.get(name) {
            MirOperand::Local(local)
        } else {
            self.inst_ty(MirInstKind::Global(name.to_string()), line, &Type::Infer)
        }
    }
}

fn reachable_blocks(blocks: &[MirBlock]) -> std::collections::HashSet<usize> {
    use std::collections::{HashMap, HashSet};
    let mut labels = HashMap::new();
    for (i, b) in blocks.iter().enumerate() {
        labels.insert(b.label.as_str(), i);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        if !seen.insert(i) {
            continue;
        }
        match blocks.get(i).and_then(|b| b.term.as_ref()) {
            Some(MirTerm::Goto(l)) => {
                if let Some(&n) = labels.get(l.as_str()) {
                    stack.push(n);
                }
            }
            Some(MirTerm::Branch { then_label, else_label, .. }) => {
                if let Some(&n) = labels.get(then_label.as_str()) {
                    stack.push(n);
                }
                if let Some(&n) = labels.get(else_label.as_str()) {
                    stack.push(n);
                }
            }
            _ => {}
        }
    }
    seen
}

fn is_void_kind(k: &MirInstKind) -> bool {
    matches!(
        k,
        MirInstKind::Store { .. }
            | MirInstKind::StoreField { .. }
            | MirInstKind::StoreIndex { .. }
            | MirInstKind::BindPattern { .. }
            | MirInstKind::Safepoint { .. }
            | MirInstKind::Opaque(_)
    )
}

// ============================================================== 文本格式

pub fn dump(p: &MirProgram) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "; Mylis MIR v0");
    let _ = writeln!(out, "; 由 mylc 生成；match 已展开为模式测试 + 绑定槽。");
    for c in &p.classes {
        let _ = writeln!(out, "\nclass @{} {{", c.name);
        for (i, f) in c.fields.iter().enumerate() {
            let _ = writeln!(
                out,
                "  {}field {} {}: {}{}",
                if f.is_static { "static " } else { "" },
                i,
                f.name,
                f.ty,
                if f.is_pub { " (pub)" } else { "" }
            );
        }
        let _ = writeln!(out, "}}");
    }
    for e in &p.enums {
        let _ = writeln!(out, "\nenum @{} {{", e.name);
        for (i, v) in e.variants.iter().enumerate() {
            let fields = v.fields.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ");
            let _ = writeln!(out, "  variant {} {}({})", i, v.name, fields);
        }
        let _ = writeln!(out, "}}");
    }
    for i in &p.ifaces {
        let _ = writeln!(out, "\niface @{} {{", i.name);
        for m in &i.methods {
            let _ = writeln!(out, "  fn {}", m.name);
        }
        let _ = writeln!(out, "}}");
    }
    for f in &p.funcs {
        let _ = writeln!(out, "\nfn @{}(", f.name);
        for (i, l) in f.params.iter().enumerate() {
            let _ = writeln!(
                out,
                "  param ${} {}: {}{}",
                i,
                l.name,
                l.ty,
                if i + 1 == f.params.len() { "" } else { "," }
            );
        }
        let _ = writeln!(out, ") -> {} {{", f.ret);
        let _ = writeln!(out, "  locals {{");
        for (i, l) in f.locals.iter().enumerate() {
            let _ = writeln!(
                out,
                "    ${} {}: {}{}",
                i,
                l.name,
                l.ty,
                if l.mutable { "" } else { " (const)" }
            );
        }
        let _ = writeln!(out, "  }}");
        for b in &f.blocks {
            let _ = writeln!(out, "  {}:", b.label);
            for i in &b.insts {
                let _ = writeln!(out, "    {}", fmt_inst(i));
            }
            if let Some(t) = &b.term {
                let _ = writeln!(out, "    {}", fmt_term(t));
            }
        }
        let _ = writeln!(out, "}}");
    }
    out
}

fn fmt_inst(i: &MirInst) -> String {
    let lhs = match i.dst {
        Some(v) => format!("%{} = ", v),
        None => String::new(),
    };
    let mut meta = String::new();
    if !matches!(i.res, Res::None) {
        meta.push_str(&format!(" res={}", fmt_res(&i.res)));
    }
    if let Some((c, f)) = i.coerce {
        meta.push_str(&format!(" coerce=#{}->#{}", c, f));
    }
    format!("{}; line {}{}", lhs + &fmt_kind(&i.kind), i.line, meta)
}

fn fmt_kind(k: &MirInstKind) -> String {
    match k {
        MirInstKind::Const(c) => format!("const {}", fmt_const(c)),
        MirInstKind::Undef => "undef".into(),
        MirInstKind::Copy(v) => format!("copy {}", fmt_op(*v)),
        MirInstKind::Store { local, value } => format!("store ${}, {}", local, fmt_op(*value)),
        MirInstKind::Bin(op, a, b) => {
            format!("{} {}, {}", bin_name(*op), fmt_op(*a), fmt_op(*b))
        }
        MirInstKind::Un(op, a) => format!("{} {}", un_name(*op), fmt_op(*a)),
        MirInstKind::Call { callee, args } => {
            format!("call @{}({})", callee, fmt_args(args))
        }
        MirInstKind::MethodCall { recv, name, args } => {
            format!("call {}.{}({})", fmt_op(*recv), name, fmt_args(args))
        }
        MirInstKind::Field { base, name } => format!("field {}, {}", fmt_op(*base), name),
        MirInstKind::StoreField { base, name, value } => {
            format!("store.field {}, {}, {}", fmt_op(*base), name, fmt_op(*value))
        }
        MirInstKind::Index { base, index } => {
            format!("index {}, {}", fmt_op(*base), fmt_op(*index))
        }
        MirInstKind::StoreIndex { base, index, value } => {
            format!(
                "store.index {}, {}, {}",
                fmt_op(*base),
                fmt_op(*index),
                fmt_op(*value)
            )
        }
        MirInstKind::New { class, args } => format!("new @{}({})", class, fmt_args(args)),
        MirInstKind::ArrayNew { elem, len } => format!("array.new {}[{}]", elem, fmt_op(*len)),
        MirInstKind::ArrayLit { elem, values } => {
            format!("array.lit {}[{}]", elem, fmt_args(values))
        }
        MirInstKind::Cast { value, target, checked } => {
            format!("{}cast {}, {}", if *checked { "try-" } else { "" }, fmt_op(*value), target)
        }
        MirInstKind::Box { value, from } => format!("box {} as {}", fmt_op(*value), from),
        MirInstKind::Try { value } => format!("try {}", fmt_op(*value)),
        MirInstKind::Interp { parts } => {
            let mut s = String::new();
            for p in parts {
                match p {
                    MirInterpPart::Lit(t) => s.push_str(&format!("{:?}", t)),
                    MirInterpPart::Value(v) => s.push_str(&fmt_op(*v)),
                }
            }
            format!("interp {}", s)
        }
        MirInstKind::Global(n) => format!("global @{}", n),
        MirInstKind::StaticRef(n) => format!("static @{}", n),
        MirInstKind::StoreStatic { name, value } => format!("static.store @{} = {}", name, fmt_op(*value)),
        MirInstKind::StorePtr { ptr, value } => format!("store.ptr {}, {}", fmt_op(*ptr), fmt_op(*value)),
        MirInstKind::MatchTest { subject, pattern } => {
            format!("match.test {}, {}", fmt_op(*subject), fmt_pattern(pattern))
        }
        MirInstKind::BindPattern { subject, pattern, locals } => {
            let l = locals.iter().map(|v| format!("${}", v)).collect::<Vec<_>>().join(", ");
            format!("bind {}, {}, [{}]", fmt_op(*subject), fmt_pattern(pattern), l)
        }
        MirInstKind::Safepoint { roots } => {
            let r = roots.iter().map(|v| fmt_op(*v)).collect::<Vec<_>>().join(", ");
            format!("safepoint roots=[{}]", r)
        }
        MirInstKind::Opaque(t) => format!("opaque {:?}", t),
    }
}

fn fmt_term(t: &MirTerm) -> String {
    match t {
        MirTerm::Return(Some(v)) => format!("ret {}", fmt_op(*v)),
        MirTerm::Return(None) => "ret void".into(),
        MirTerm::Goto(l) => format!("goto {}", l),
        MirTerm::Branch { cond, then_label, else_label } => {
            format!("br {}, {}, {}", fmt_op(*cond), then_label, else_label)
        }
        MirTerm::Unreachable => "unreachable".into(),
    }
}

fn fmt_op(o: MirOperand) -> String {
    match o {
        MirOperand::Value(v) => format!("%{}", v),
        MirOperand::Local(l) => format!("${}", l),
    }
}

fn fmt_args(args: &[MirOperand]) -> String {
    args.iter().map(|a| fmt_op(*a)).collect::<Vec<_>>().join(", ")
}

fn fmt_const(c: &MirConst) -> String {
    match c {
        MirConst::Int(v) => format!("{}", v),
        MirConst::IntU64(v) => format!("{}u64", v),
        MirConst::Float(v) => format!("{}", v),
        MirConst::Bool(v) => format!("{}", v),
        MirConst::Str(s) => format!("{:?}", s),
        MirConst::Null => "null".into(),
    }
}

fn bin_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::Div => "div",
        BinOp::Rem => "rem",
        BinOp::Eq => "eq",
        BinOp::Ne => "ne",
        BinOp::Lt => "lt",
        BinOp::Le => "le",
        BinOp::Gt => "gt",
        BinOp::Ge => "ge",
        BinOp::AndAnd => "and",
        BinOp::OrOr => "or",
        BinOp::BitAnd => "bitand",
        BinOp::BitOr => "bitor",
        BinOp::BitXor => "bitxor",
        BinOp::Shl => "shl",
        BinOp::Shr => "shr",
    }
}

fn un_name(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "neg",
        UnOp::Not => "not",
        UnOp::BitNot => "bitnot",
        UnOp::Deref => "load",
        UnOp::AddrOf => "addr",
    }
}

fn fmt_res(r: &Res) -> String {
    match r {
        Res::None => "none".into(),
        Res::Fn(id) => format!("fn#{}", id),
        Res::Method(id) => format!("method#{}", id),
        Res::Field(c, f) => format!("field#{}:{}", c, f),
        Res::Var => "var".into(),
        Res::Ctor(c) => format!("ctor#{}", c),
        Res::IfaceCast(c, i) => format!("cast#{}:#{}", c, i),
        Res::Variant(e, v) => format!("variant#{}:{}", e, v),
        Res::StaticMethod(c, m) => format!("static-method#{}:{}", c, m),
        Res::StaticField(c, f) => format!("static-field#{}:{}", c, f),
        Res::Builtin(b) => format!("builtin {:?}", b),
        Res::AdtCtor { base, .. } => format!("adt-ctor#{}", base),
        Res::AdtFn { base, .. } => format!("adt-fn#{}", base),
        Res::AdtMethod { method, .. } => format!("adt-method#{}", method),
        Res::AdtStaticMethod { method, .. } => format!("adt-static#{}", method),
        Res::AdtField { base, field, .. } => format!("adt-inst-field#{}:{}", base, field),
        Res::AdtStaticField { field, .. } => format!("adt-field#{}", field),
        Res::AdtVariant { base, variant, .. } => format!("adt-variant#{}:{}", base, variant),
    }
}

fn fmt_pattern(p: &Pattern) -> String {
    match p {
        Pattern::Wildcard => "_".into(),
        Pattern::Int(v) => format!("{}", v),
        Pattern::IntU64(v) => format!("{}u64", v),
        Pattern::Float(v) => format!("{}", v),
        Pattern::Str(s) => format!("{:?}", s),
        Pattern::Bool(v) => format!("{}", v),
        Pattern::Null => "null".into(),
        Pattern::Ok(b) => format!("Ok({})", bind_name(b)),
        Pattern::Err(b) => format!("Err({})", bind_name(b)),
        Pattern::Some(b) => format!("Some({})", bind_name(b)),
        Pattern::None => "None".into(),
        Pattern::Variant { ename, name, binds, .. } => {
            let b = binds.iter().map(bind_name).collect::<Vec<_>>().join(", ");
            if b.is_empty() {
                format!("{}::{}", ename, name)
            } else {
                format!("{}::{}({})", ename, name, b)
            }
        }
    }
}

fn bind_name(b: &Option<String>) -> &str {
    b.as_deref().unwrap_or("_")
}

// ============================================================== 校验

/// 基本结构校验：标签存在、每个块有终结指令、操作数引用合法。
/// 返回空表示通过；当前用于 `--emit-ir` 的 `--verbose` 输出与测试。
pub fn verify(p: &MirProgram) -> Vec<String> {
    let mut errs = Vec::new();
    for f in &p.funcs {
        let mut labels = HashMap::new();
        for (i, b) in f.blocks.iter().enumerate() {
            if labels.insert(b.label.clone(), i).is_some() {
                errs.push(format!("{}: duplicate block {}", f.name, b.label));
            }
        }
        let mut values = HashMap::new();
        for b in &f.blocks {
            for i in &b.insts {
                if let Some(v) = i.dst {
                    values.insert(v, ());
                }
                for op in inst_operands(&i.kind) {
                    if let MirOperand::Value(v) = op {
                        if !values.contains_key(&v) && v != 0 {
                            errs.push(format!("{}: use of undefined %{}", f.name, v));
                        }
                    }
                    if let MirOperand::Local(l) = op {
                        if l as usize >= f.locals.len() {
                            errs.push(format!("{}: use of unknown local ${}", f.name, l));
                        }
                    }
                }
            }
            match &b.term {
                None => errs.push(format!("{}: block {} has no terminator", f.name, b.label)),
                Some(MirTerm::Goto(l)) => {
                    if !labels.contains_key(l) {
                        errs.push(format!("{}: goto unknown {}", f.name, l));
                    }
                }
                Some(MirTerm::Return(_)) => {}
                Some(MirTerm::Branch { cond, then_label, else_label }) => {
                    if let MirOperand::Value(v) = cond {
                        if !values.contains_key(v) && *v != 0 {
                            errs.push(format!("{}: branch on undefined %{}", f.name, v));
                        }
                    }
                    for l in [then_label, else_label] {
                        if !labels.contains_key(l) {
                            errs.push(format!("{}: branch to unknown {}", f.name, l));
                        }
                    }
                }
                Some(MirTerm::Unreachable) => {}
            }
        }
    }
    errs
}

fn inst_operands(k: &MirInstKind) -> Vec<MirOperand> {
    match k {
        MirInstKind::Const(_) | MirInstKind::Undef | MirInstKind::Global(_) | MirInstKind::StaticRef(_) => vec![],
        MirInstKind::Copy(v) => vec![*v],
        MirInstKind::Store { value, .. } => vec![*value],
        MirInstKind::Bin(_, a, b) => vec![*a, *b],
        MirInstKind::Un(_, a) => vec![*a],
        MirInstKind::Call { args, .. } => args.clone(),
        MirInstKind::MethodCall { recv, args, .. } => {
            let mut v = vec![*recv];
            v.extend(args.iter().copied());
            v
        }
        MirInstKind::Field { base, .. } => vec![*base],
        MirInstKind::StoreField { base, value, .. } => vec![*base, *value],
        MirInstKind::Index { base, index } => vec![*base, *index],
        MirInstKind::StoreIndex { base, index, value } => vec![*base, *index, *value],
        MirInstKind::New { args, .. } => args.clone(),
        MirInstKind::ArrayNew { len, .. } => vec![*len],
        MirInstKind::ArrayLit { values, .. } => values.clone(),
        MirInstKind::Cast { value, .. } | MirInstKind::Try { value } => vec![*value],
        MirInstKind::Box { value, .. } => vec![*value],
        MirInstKind::Interp { parts } => parts
            .iter()
            .filter_map(|p| match p {
                MirInterpPart::Value(v) => Some(*v),
                MirInterpPart::Lit(_) => None,
            })
            .collect(),
        MirInstKind::MatchTest { subject, .. } => vec![*subject],
        MirInstKind::BindPattern { subject, .. } => vec![*subject],
        MirInstKind::StoreStatic { value, .. } => vec![*value],
        MirInstKind::StorePtr { ptr, value } => vec![*ptr, *value],
        MirInstKind::Safepoint { roots } => roots.clone(),
        MirInstKind::Opaque(_) => vec![],
    }
}

impl fmt::Display for MirProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", dump(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_accepts_empty() {
        let p = MirProgram { funcs: vec![], classes: vec![], enums: vec![], ifaces: vec![] };
        assert!(verify(&p).is_empty());
    }
}

fn has_self_type(t: &Type) -> bool {
    match t {
        Type::Named(n) => n == "Self",
        Type::Generic(_, args) => args.iter().any(has_self_type),
        Type::Ptr(e) | Type::Array(e) | Type::Option(e) => has_self_type(e),
        Type::Result(a, b) => has_self_type(a) || has_self_type(b),
        _ => false,
    }
}

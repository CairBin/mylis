//! 泛型实例化的 AST 替换工具。
//!
//! 第一版只做编译期单态化：模板里的类型参数在实例化时全部替换成具体类型，
//! 不保留运行时泛型信息。

use crate::ast::*;
use std::collections::HashMap;

pub fn subst_type(t: &Type, map: &HashMap<String, Type>) -> Type {
    match t {
        Type::Named(n) | Type::Param(n) => map.get(n).cloned().unwrap_or_else(|| t.clone()),
        Type::Generic(n, args) => {
            let base = map.get(n).cloned();
            if let Some(base) = base {
                return base;
            }
            Type::Generic(n.clone(), args.iter().map(|a| subst_type(a, map)).collect())
        }
        Type::Adt { base, args } => Type::Adt {
            base: *base,
            args: args.iter().map(|a| subst_type(a, map)).collect(),
        },
        Type::Ptr(e) => Type::Ptr(Box::new(subst_type(e, map))),
        Type::Array(e) => Type::Array(Box::new(subst_type(e, map))),
        Type::Result(a, b) => Type::Result(Box::new(subst_type(a, map)), Box::new(subst_type(b, map))),
        Type::Option(a) => Type::Option(Box::new(subst_type(a, map))),
        _ => t.clone(),
    }
}

/// 接口签名中的 `Self` 占位替换为实际接收者类型。
/// 目前只用于 Eq/Ord 这类仅作 bound 的接口，不进入运行时 vtable。
pub fn subst_self_type(t: &Type, actual: &Type) -> Type {
    match t {
        Type::Named(n) if n == "Self" => actual.clone(),
        Type::Generic(n, args) => Type::Generic(
            n.clone(),
            args.iter().map(|a| subst_self_type(a, actual)).collect(),
        ),
        Type::Adt { base, args } => Type::Adt {
            base: *base,
            args: args.iter().map(|a| subst_self_type(a, actual)).collect(),
        },
        Type::Ptr(e) => Type::Ptr(Box::new(subst_self_type(e, actual))),
        Type::Array(e) => Type::Array(Box::new(subst_self_type(e, actual))),
        Type::Result(a, b) => Type::Result(
            Box::new(subst_self_type(a, actual)),
            Box::new(subst_self_type(b, actual)),
        ),
        Type::Option(a) => Type::Option(Box::new(subst_self_type(a, actual))),
        _ => t.clone(),
    }
}

pub fn subst_expr(e: &mut Expr, map: &HashMap<String, Type>) {
    e.ty = subst_type(&e.ty, map);
    e.box_to = e.box_to.take().map(|(t, id)| (subst_type(&t, map), id));
    e.res = subst_res(&e.res, map);
    match &mut e.kind {
        ExprKind::Bin(_, a, b) => {
            subst_expr(a, map);
            subst_expr(b, map);
        }
        ExprKind::Un(_, a) | ExprKind::Field(a, _)
        | ExprKind::Try(a) | ExprKind::Ok(a) | ExprKind::Err(a) | ExprKind::Some_(a) => {
            subst_expr(a, map);
        }
        ExprKind::Call(_, targs, args) => {
            for t in targs.iter_mut() { *t = subst_type(t, map); }
            for a in args.iter_mut() { subst_expr(a, map); }
        }
        ExprKind::Method(recv, _, targs, args) => {
            subst_expr(recv, map);
            for t in targs.iter_mut() { *t = subst_type(t, map); }
            for a in args.iter_mut() { subst_expr(a, map); }
        }
        ExprKind::New(_, targs, args) => {
            for t in targs.iter_mut() { *t = subst_type(t, map); }
            for a in args.iter_mut() { subst_expr(a, map); }
        }
        ExprKind::Cast(a, t, _) => {
            subst_expr(a, map);
            *t = subst_type(t, map);
        }
        ExprKind::Index(a, b) => { subst_expr(a, map); subst_expr(b, map); }
        ExprKind::ArrayLit(items) => for a in items.iter_mut() { subst_expr(a, map); },
        ExprKind::ArrayNew(t, n) => {
            *t = subst_type(t, map);
            subst_expr(n, map);
        }
        ExprKind::Interp(parts) => for p in parts.iter_mut() {
            if let InterpPart::Expr(a) = p { subst_expr(a, map); }
        },
        ExprKind::StaticPath { args, .. } => for a in args.iter_mut() { subst_expr(a, map); },
        _ => {}
    }
}

fn subst_res(r: &Res, map: &HashMap<String, Type>) -> Res {
    match r {
        Res::AdtCtor { base, args } => Res::AdtCtor {
            base: *base,
            args: args.iter().map(|a| subst_type(a, map)).collect(),
        },
        Res::AdtFn { base, args } => Res::AdtFn {
            base: *base,
            args: args.iter().map(|a| subst_type(a, map)).collect(),
        },
        Res::AdtMethod { base, method, class_args, method_args } => Res::AdtMethod {
            base: *base,
            method: *method,
            class_args: class_args.iter().map(|a| subst_type(a, map)).collect(),
            method_args: method_args.iter().map(|a| subst_type(a, map)).collect(),
        },
        Res::AdtStaticMethod { base, method, class_args, method_args } => Res::AdtStaticMethod {
            base: *base,
            method: *method,
            class_args: class_args.iter().map(|a| subst_type(a, map)).collect(),
            method_args: method_args.iter().map(|a| subst_type(a, map)).collect(),
        },
        Res::AdtField { base, field, class_args } => Res::AdtField {
            base: *base,
            field: *field,
            class_args: class_args.iter().map(|a| subst_type(a, map)).collect(),
        },
        Res::AdtStaticField { base, field, class_args } => Res::AdtStaticField {
            base: *base,
            field: *field,
            class_args: class_args.iter().map(|a| subst_type(a, map)).collect(),
        },
        Res::AdtVariant { base, variant, args } => Res::AdtVariant {
            base: *base,
            variant: *variant,
            args: args.iter().map(|a| subst_type(a, map)).collect(),
        },
        other => other.clone(),
    }
}

pub fn subst_block(b: &mut Block, map: &HashMap<String, Type>) {
    for s in b.stmts.iter_mut() { subst_stmt(s, map); }
}

pub fn subst_stmt(s: &mut Stmt, map: &HashMap<String, Type>) {
    match &mut s.kind {
        StmtKind::Var { ty, init, .. } => {
            *ty = subst_type(ty, map);
            if let Some(e) = init { subst_expr(e, map); }
        }
        StmtKind::Assign { target, value } => { subst_expr(target, map); subst_expr(value, map); }
        StmtKind::ExprStmt(e) | StmtKind::Return(Some(e)) => subst_expr(e, map),
        StmtKind::If { cond, then_b, else_b } => {
            subst_expr(cond, map); subst_block(then_b, map);
            if let Some(s) = else_b { subst_stmt(s, map); }
        }
        StmtKind::While { cond, body } => { subst_expr(cond, map); subst_block(body, map); }
        StmtKind::For { init, cond, step, body } => {
            subst_stmt(init, map);
            if let Some(c) = cond { subst_expr(c, map); }
            if let Some(st) = step { subst_stmt(st, map); }
            subst_block(body, map);
        }
        StmtKind::Unsafe(b) | StmtKind::Block(b) => subst_block(b, map),
        StmtKind::Match { subject, arms } => {
            subst_expr(subject, map);
            for a in arms.iter_mut() { subst_block(&mut a.body, map); }
        }
        _ => {}
    }
}

pub fn subst_decl(d: &mut Decl, map: &HashMap<String, Type>) {
    d.ret = subst_type(&d.ret, map);
    for p in d.params.iter_mut() { p.ty = subst_type(&p.ty, map); }
    for f in d.fields.iter_mut() {
        f.ty = subst_type(&f.ty, map);
        if let Some(e) = &mut f.init { subst_expr(e, map); }
    }
    for v in d.variants.iter_mut() {
        for t in v.fields.iter_mut() { *t = subst_type(t, map); }
    }
    if let Some(b) = &mut d.body { subst_block(b, map); }
}

/// 泛型实例的稳定后缀：只依赖类型实参，不依赖声明顺序。
/// 类/枚举/接口类型必须用声明名，不能用全局名称表的快照；实例是在
/// 名称表建立之后才创建的，直接 `Display` 会退化成 `#id`。
pub fn generic_suffix(prog: &Program, args: &[Type]) -> String {
    let mut out = String::new();
    for a in args {
        out.push_str("__");
        let raw = suffix_type_name(prog, a);
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                out.push(ch);
            } else {
                out.push('_');
            }
        }
    }
    out
}

fn suffix_type_name(prog: &Program, t: &Type) -> String {
    match t {
        Type::Class(id) | Type::Enum(id) | Type::Iface(id) => prog
            .decls
            .get(*id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| t.to_string()),
        Type::Generic(name, args) => {
            let mut s = name.clone();
            s.push('<');
            for (i, a) in args.iter().enumerate() {
                if i > 0 { s.push(','); }
                s.push_str(&suffix_type_name(prog, a));
            }
            s.push('>');
            s
        }
        Type::Adt { base, args } => {
            let mut s = prog.decls.get(*base).map(|d| d.name.clone()).unwrap_or_else(|| t.to_string());
            s.push('<');
            for (i, a) in args.iter().enumerate() {
                if i > 0 { s.push(','); }
                s.push_str(&suffix_type_name(prog, a));
            }
            s.push('>');
            s
        }
        Type::Param(n) => n.clone(),
        Type::Ptr(e) => format!("{}*", suffix_type_name(prog, e)),
        Type::Array(e) => format!("{}[]", suffix_type_name(prog, e)),
        Type::Option(e) => format!("Option<{}>", suffix_type_name(prog, e)),
        Type::Result(a, b) => format!(
            "Result<{},{}>",
            suffix_type_name(prog, a),
            suffix_type_name(prog, b)
        ),
        _ => t.to_string(),
    }
}

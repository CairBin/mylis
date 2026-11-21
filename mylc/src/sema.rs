//! 语义分析：第 13 章 12 条强制检查 + 类型解析 + 鸭子类型接口判定。
//!
//! 检查号与规范一一对应：
//!   MYL001 单文件最多一个 pub class
//!   MYL002 pub class 必须与文件名一致
//!   MYL003 局部变量必须初始化
//!   MYL004 类字段全覆盖初始化
//!   MYL005 跨文件私有访问
//!   MYL006 ? 语法合法性
//!   MYL007 裸指针仅 unsafe 可用
//!   MYL008 无体 unsafe fn 仅限 *_bind.myl
//!   MYL009 @header 仅可用于 C 绑定
//!   MYL010 注解只能挂载合法节点（语句/局部变量侧在 parser 中报告）
//!   MYL011 循环导入（program.rs）
//!   MYL012 严格类型匹配、参数数量匹配

use crate::ast::*;
use crate::diag::{Diag, E_GENERAL};
use std::collections::HashSet;

pub struct Sema<'a> {
    pub prog: &'a mut Program,
    diags: &'a mut Diag,
    cur_mod: usize,
    cur_file: String,
    cur_fn: Option<usize>,
    cur_class: Option<usize>,
    scopes: Vec<Vec<(String, Type, bool)>>,
    in_ctor: bool,
    const_writes: Vec<(String, u32)>,
    unsafe_depth: usize,
    loop_depth: usize,
    ret_ty: Type,
    visible: Vec<usize>,
    in_field_init: bool,
    /// 当前模板体检查时的类型参数 -> bound interface id
    type_param_bounds: std::collections::HashMap<String, Vec<usize>>,
    /// 当前环境里所有类型参数名（包括没有 bound 的参数）
    type_param_names: HashSet<String>,
    /// 只有 interface 方法签名里允许 `Self`，普通代码里使用应报未知类型。
    allow_self: bool,
}

/// 接口组合：把 `interface A { B; C; fn ... }` 里内嵌接口的方法集**展平**进 A 的 members。
///
/// 规则（对齐 Go 的并集语义）：
/// - 同名同签名 → 合并成一份，静默通过；
/// - 同名不同签名 → 报 MYL013（不允许在外层“重写”消歧）；
/// - 内嵌的名字必须是 interface，否则 MYL013；
/// - 循环内嵌 → MYL013。
///
/// 展平后，vtable 生成、鸭子类型一致性检查、接口方法调用都不用再关心继承关系。
fn flatten_ifaces(prog: &mut Program, diags: &mut Diag) {
    let n = prog.decls.len();
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];

    for id in 0..n {
        if prog.decls[id].kind != DeclKind::Iface {
            continue;
        }
        let mod_id = prog.decls[id].mod_id;
        let path = prog.modules[mod_id].path.clone();
        let iname = prog.decls[id].name.clone();
        let exts = prog.decls[id].extends.clone();
        let mut ps: Vec<usize> = Vec::new();
        for (name, line, col) in exts {
            match prog.find_type_in(mod_id, &name) {
                Some(tid) if prog.decls[tid].kind == DeclKind::Iface => {
                    if prog.decls[tid].mod_id != mod_id && !prog.decls[tid].is_pub {
                        diags.err(
                            13,
                            &path,
                            line,
                            col,
                            format!("接口 `{}` 内嵌的 `{}` 不是 pub，跨文件不可见", iname, name),
                        );
                    }
                    ps.push(tid);
                }
                Some(_) => diags.err(
                    13,
                    &path,
                    line,
                    col,
                    format!("`{}` 不是 interface，不能用在接口内嵌位置", name),
                ),
                None => diags.err(
                    13,
                    &path,
                    line,
                    col,
                    format!("接口 `{}` 内嵌的 `{}` 找不到", iname, name),
                ),
            }
        }
        parents[id] = ps;
    }

    // 0=未访问 1=在栈上 2=已完成；后序遍历保证父接口先展平
    let mut state = vec![0u8; n];
    let mut flat: Vec<Vec<usize>> = vec![Vec::new(); n];

    for start in 0..n {
        if prog.decls[start].kind != DeclKind::Iface || state[start] == 2 {
            continue;
        }
        state[start] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some((cur, pi)) = stack.pop() {
            if pi < parents[cur].len() {
                stack.push((cur, pi + 1));
                let p = parents[cur][pi];
                match state[p] {
                    1 => {
                        let path = prog.modules[prog.decls[cur].mod_id].path.clone();
                        let (l, c) = (prog.decls[cur].line, prog.decls[cur].col);
                        let (a, b) = (
                            prog.decls[cur].name.clone(),
                            prog.decls[p].name.clone(),
                        );
                        diags.err(
                            13,
                            &path,
                            l,
                            c,
                            format!("接口 `{}` 与 `{}` 形成循环内嵌", a, b),
                        );
                    }
                    0 => {
                        state[p] = 1;
                        stack.push((p, 0));
                    }
                    _ => {}
                }
                continue;
            }

            // 父接口全部就绪：合并自身方法 + 各父接口已展平的方法
            let path = prog.modules[prog.decls[cur].mod_id].path.clone();
            let iname = prog.decls[cur].name.clone();
            let own = prog.decls[cur].members.clone();
            let mut merged: Vec<usize> = Vec::new();
            for m in own.iter().chain(
                parents[cur]
                    .iter()
                    .flat_map(|p| flat[*p].iter()),
            ) {
                push_member(prog, diags, &path, &iname, &mut merged, *m, *m);
            }
            flat[cur] = merged.clone();
            prog.decls[cur].members = merged;
            state[cur] = 2;
        }
    }
}

/// 把成员 `mid` 并入 `merged`：同名同签名合并，同名不同签名报错。
fn push_member(
    prog: &Program,
    diags: &mut Diag,
    path: &str,
    iface_name: &str,
    merged: &mut Vec<usize>,
    mid: usize,
    from: usize,
) {
    let name = prog.decls[mid].name.clone();
    for existing in merged.iter().copied() {
        if prog.decls[existing].name != name {
            continue;
        }
        let same = prog.decls[existing].ret == prog.decls[mid].ret
            && prog.decls[existing].params.len() == prog.decls[mid].params.len()
            && prog
                .decls[existing]
                .params
                .iter()
                .zip(prog.decls[mid].params.iter())
                .all(|(a, b)| a.ty == b.ty);
        if same {
            return; // 同名同签名：静默合并
        }
        let d = &prog.decls[from];
        diags.err(
            14,
            path,
            d.line,
            d.col,
            format!(
                "接口 `{}` 的方法 `{}` 与内嵌接口传来的同名方法签名不一致（{} vs {}）",
                iface_name,
                name,
                sig_text(prog, existing),
                sig_text(prog, mid)
            ),
        );
        return;
    }
    merged.push(mid);
}

fn sig_text(prog: &Program, mid: usize) -> String {
    let d = &prog.decls[mid];
    let mut s = format!("fn {}(", d.name);
    for (i, p) in d.params.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("{}", p.ty));
    }
    s.push(')');
    s.push_str(&format!(" -> {}", d.ret));
    s
}

pub fn analyze(prog: &mut Program, diags: &mut Diag) {
    set_type_names(prog.decls.iter().map(|d| d.name.clone()).collect());
    prog.tostring_iface = prog
        .decls
        .iter()
        .position(|d| d.kind == DeclKind::Iface && d.name == "ToString");
    build_symbols(prog, diags);
    flatten_ifaces(prog, diags);

    // ---------- 结构检查（MYL001/002/008/009/010） ----------
    check_module_structure(prog, diags);
    // 每个类最多一个构造器（与类同名的方法 / init）
    for cid in 0..prog.decls.len() {
        if prog.decls[cid].kind != DeclKind::Class {
            continue;
        }
        let ctors: Vec<usize> = prog.decls[cid]
            .members
            .iter()
            .copied()
            .filter(|m| prog.decls[*m].is_ctor)
            .collect();
        if ctors.len() > 1 {
            let ms: Vec<String> = ctors.iter().map(|m| prog.decls[*m].name.clone()).collect();
            let d = &prog.decls[ctors[1]];
            diags.err(
                E_GENERAL,
                &prog.modules[d.mod_id].path,
                d.line,
                d.col,
                format!(
                    "class `{}` 定义了多个构造器（{}）：一个类最多一个构造器，其余请写成静态工厂方法",
                    prog.decls[cid].name,
                    ms.join(", ")
                ),
            );
        }
    }

    // 未知注解告警（防拼错导致静默失效，如 @exprot）
    const KNOWN_ANNOTS: &[&str] = &["export", "header", "deprecated", "inline", "derive"];
    fn annot_suggest(name: &str) -> Option<&'static str> {
        let mut best: Option<(usize, &'static str)> = None;
        for k in KNOWN_ANNOTS {
            let d = edit_distance(name, k);
            if d <= 2 && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, k));
            }
        }
        best.map(|(_, k)| k)
    }
    let mut unknown: Vec<(usize, String, u32, u32)> = Vec::new();
    for id in 0..prog.decls.len() {
        let mod_id = prog.decls[id].mod_id;
        if prog.decls[id].owner.is_some() {
            // 方法：随所属声明的注解一起检查（这里单独扫）
        }
        for a in prog.decls[id].annots.clone() {
            if !KNOWN_ANNOTS.contains(&a.name.as_str()) {
                unknown.push((mod_id, a.name.clone(), a.line, a.col));
            }
        }
        for f in prog.decls[id].fields.iter() {
            for a in f.annots.clone() {
                if !KNOWN_ANNOTS.contains(&a.name.as_str()) {
                    unknown.push((mod_id, a.name.clone(), a.line, a.col));
                }
            }
        }
    }
    // 只对「与内置注解高度相似」的名字告警（防拼错静默失效）；
    // 用户自定义注解（@route / @col 等纯元数据）保持静默，不打扰。
    for (mod_id, name, line, col) in unknown {
        if let Some(k) = annot_suggest(&name) {
            diags.warn(
                200,
                &prog.modules[mod_id].path,
                line,
                col,
                format!("未识别的注解 @{}：是否想写 @{}？", name, k),
            );
        }
    }

    // 入口 `main` 不能带参数（C 侧按 int main(void) 调用）
    for d in prog.decls.iter() {
        if d.kind == DeclKind::Fn
            && d.owner.is_none()
            && d.name == "main"
            && !d.params.is_empty()
        {
            diags.err(
                12,
                &prog.modules[d.mod_id].path,
                d.line,
                d.col,
                format!("`main` 不能带参数（实际 {} 个），需要命令行参数请用 class/函数自行定义", d.params.len()),
            );
        }
    }

    // @export 只能用于顶层 pub fn（导出为稳定 C 符号）
    for d in prog.decls.iter() {
        let has_export = d.annots.iter().any(|a| a.name == "export");
        if !has_export {
            continue;
        }
        if d.kind != DeclKind::Fn || d.owner.is_some() || !d.is_pub {
            diags.err(
                E_GENERAL,
                &prog.modules[d.mod_id].path,
                d.line,
                d.col,
                "`@export` 只能标注在顶层 `pub fn` 上（用于 --lib 导出 C 符号）",
            );
        }
    }

    // init 形态检查（MYL012 / MYL102）
    for d in prog.decls.iter() {
        if !d.is_ctor {
            continue;
        }
        if d.ret != Type::Void {
            diags.err(
                12,
                &prog.modules[d.mod_id].path,
                d.line,
                d.col,
                format!(
                    "构造器 `{}` 不能有返回类型（省略 `->` 即为 void），实际是 {}",
                    d.name, d.ret
                ),
            );
        }
        if d.is_static {
            diags.err(
                E_GENERAL,
                &prog.modules[d.mod_id].path,
                d.line,
                d.col,
                "构造器不能声明为 static",
            );
        }
    }
    if diags.errors > 0 {
        return;
    }

    // ---------- 类型解析 ----------
    for id in 0..prog.decls.len() {
        let mut s = Sema::new(&mut *prog, &mut *diags, id);
        s.resolve_decl_types(id);
    }
    if diags.errors > 0 {
        return;
    }

    // ---------- 检查字段初始化 / 顶层函数体 ----------
    for id in 0..prog.decls.len() {
        if prog.decls[id].owner.is_some() {
            continue; // 类方法 / 接口签名在下一阶段处理
        }
        let mut s = Sema::new(&mut *prog, &mut *diags, id);
        s.check_decl_body(id);
    }

    // ---------- 类方法体（带 this 上下文） ----------
    check_all_method_bodies(&mut *prog, &mut *diags);

    // 泛型函数实例可能在类方法体里才产生，补检查一遍顶层函数。
    loop {
        let mut todo = None;
        for id in 0..prog.decls.len() {
            let d = &prog.decls[id];
            if d.kind == DeclKind::Fn && d.owner.is_none() && d.body.is_some()
                && d.type_params.is_empty() && !d.checked
            {
                todo = Some(id);
                break;
            }
        }
        match todo {
            Some(id) => {
                let mut s = Sema::new(&mut *prog, &mut *diags, id);
                s.check_decl_body(id);
            }
            None => break,
        }
    }

    // ---------- MYL004：类字段全覆盖初始化 ----------
    check_class_fields(prog, diags);

    // ---------- late monomorphization ----------
    monomorphize(prog, diags);
    if diags.errors > 0 {
        return;
    }

    // 在模板被中和前保存 API 快照，供 .mylib 导出泛型签名。
    {
        let mut api = Vec::new();
        for d in prog.decls.iter() {
            if d.owner.is_none() && !d.type_params.is_empty() {
                let mut x = d.clone();
                x.members.clear();
                x.member_asts = d.members.iter().map(|m| prog.decls[*m].clone()).collect();
                api.push(x);
            }
        }
        prog.template_api = api;
    }

    // 泛型模板只用于实例化，不进入后端。置为 Alias，避免 codegen/MIR/LLVM
    // 看到未替换的 T。
    let template_ids: Vec<usize> = prog.decls.iter().enumerate()
        .filter(|(_, d)| !d.type_params.is_empty())
        .map(|(i, _)| i)
        .collect();
    for d in prog.decls.iter_mut() {
        let owner_template = d.owner.map(|o| template_ids.contains(&o)).unwrap_or(false);
        if !d.type_params.is_empty() || owner_template {
            d.kind = DeclKind::Alias;
            d.alias_target = Type::Infer;
            d.fields.clear();
            d.members.clear();
            d.body = None;
            d.params.clear();
            d.ret = Type::Infer;
        }
    }

    // 泛型模板体检查时可能用类型参数（如 T）实例化出 Vec<T> 之类的
    // “具体”类。它们不是可生成代码的类型；这里一并中和，避免后端
    // 看到未替换的 T。
    let tparam_names: HashSet<String> = prog.decls.iter()
        .flat_map(|d| d.type_params.iter().cloned())
        .collect();
    // 先找出直接带 T 的实例，再沿 Type::Class 引用传递闭包，
    // 这样 Vec<Vec<T>> 这类外层实例也会被识别。
    let mut unresolved_ids: HashSet<usize> = HashSet::new();
    loop {
        let mut changed = false;
        for (_, args, id) in prog.generic_instances.clone() {
            if unresolved_ids.contains(&id) {
                continue;
            }
            if args.iter().any(|a| type_contains_param_or_instance(a, &tparam_names, &unresolved_ids)) {
                unresolved_ids.insert(id);
                changed = true;
            }
        }
        if !changed { break; }
    }
    // 这些占位实例也可能已经进入 Result/Option/数组类型表；一并清掉，
    // 否则 codegen 仍会为 C_Vec_Vec__T 生成辅助结构。
    prog.insts.retain(|t| {
        !type_contains_param_or_instance(t, &tparam_names, &unresolved_ids)
    });
    prog.arrays.retain(|t| {
        !type_contains_param_or_instance(t, &tparam_names, &unresolved_ids)
    });
    for id in unresolved_ids {
        let members = prog.decls[id].members.clone();
        neuter_generic_instance(&mut prog.decls[id]);
        for mid in members {
            neuter_generic_instance(&mut prog.decls[mid]);
        }
    }
}

// ============================================================ 单态化
// 类型检查阶段只保留 (模板, 类型实参)；进入后端前，把所有真正会被执行的
// `Adt` 和 `Res::Adt*` 收敛成具体的 class/enum/interface 声明。
// 这一步是“late monomorphization”：模板函数体只检查一次，实例按需生成。
pub(crate) fn monomorphize(prog: &mut Program, diags: &mut Diag) {
    let mut rounds = 0;
    loop {
        let before = prog.decls.len();
        let mut i = 0;
        while i < prog.decls.len() {
            let mut d = prog.decls[i].clone();
            mono_decl(&mut d, prog, diags);
            prog.decls[i] = d;
            i += 1;
        }
        if prog.decls.len() == before {
            break;
        }
        rounds += 1;
        if rounds > 64 {
            break;
        }
    }
    // `Program::insts/arrays` 里可能还残留 Adt 类型；用实际声明重建。
    let mut insts: Vec<Type> = Vec::new();
    let mut arrays: Vec<Type> = Vec::new();
    for d in prog.decls.iter() {
        collect_decl_types(d, &mut insts, &mut arrays);
    }
    prog.insts = insts;
    prog.arrays = arrays;
}

fn collect_type_sets(t: &Type, insts: &mut Vec<Type>, arrays: &mut Vec<Type>) {
    match t {
        Type::Result(a, b) => {
            if !insts.contains(t) { insts.push(t.clone()); }
            collect_type_sets(a, insts, arrays);
            collect_type_sets(b, insts, arrays);
        }
        Type::Option(e) => {
            if !insts.contains(t) { insts.push(t.clone()); }
            collect_type_sets(e, insts, arrays);
        }
        Type::Adt { args, .. } => for a in args { collect_type_sets(a, insts, arrays); },
        Type::Ptr(e) | Type::Array(e) => {
            if matches!(t, Type::Array(_)) && !arrays.contains(t) { arrays.push(t.clone()); }
            collect_type_sets(e, insts, arrays);
        }
        Type::Generic(_, args) => for a in args { collect_type_sets(a, insts, arrays); },
        _ => {}
    }
}

fn collect_decl_types(d: &Decl, insts: &mut Vec<Type>, arrays: &mut Vec<Type>) {
    collect_type_sets(&d.ret, insts, arrays);
    for p in d.params.iter() { collect_type_sets(&p.ty, insts, arrays); }
    for f in d.fields.iter() { collect_type_sets(&f.ty, insts, arrays); }
    for v in d.variants.iter() { for t in v.fields.iter() { collect_type_sets(t, insts, arrays); } }
    collect_type_sets(&d.alias_target, insts, arrays);
    if let Some(b) = &d.body { collect_block_types(b, insts, arrays); }
}

fn collect_block_types(b: &Block, insts: &mut Vec<Type>, arrays: &mut Vec<Type>) {
    for s in b.stmts.iter() { collect_stmt_types(s, insts, arrays); }
}

fn collect_stmt_types(s: &Stmt, insts: &mut Vec<Type>, arrays: &mut Vec<Type>) {
    match &s.kind {
        StmtKind::Var { ty, init, .. } => {
            collect_type_sets(ty, insts, arrays);
            if let Some(e) = init { collect_expr_types(e, insts, arrays); }
        }
        StmtKind::Assign { target, value } => { collect_expr_types(target, insts, arrays); collect_expr_types(value, insts, arrays); }
        StmtKind::ExprStmt(e) | StmtKind::Return(Some(e)) => collect_expr_types(e, insts, arrays),
        StmtKind::If { cond, then_b, else_b } => {
            collect_expr_types(cond, insts, arrays);
            collect_block_types(then_b, insts, arrays);
            if let Some(x) = else_b { collect_stmt_types(x, insts, arrays); }
        }
        StmtKind::While { cond, body } => { collect_expr_types(cond, insts, arrays); collect_block_types(body, insts, arrays); }
        StmtKind::For { init, cond, step, body } => {
            collect_stmt_types(init, insts, arrays);
            if let Some(c) = cond { collect_expr_types(c, insts, arrays); }
            if let Some(st) = step { collect_stmt_types(st, insts, arrays); }
            collect_block_types(body, insts, arrays);
        }
        StmtKind::Unsafe(b) | StmtKind::Block(b) => collect_block_types(b, insts, arrays),
        StmtKind::Match { subject, arms } => {
            collect_expr_types(subject, insts, arrays);
            for a in arms.iter() { collect_block_types(&a.body, insts, arrays); }
        }
        _ => {}
    }
}

fn collect_expr_types(e: &Expr, insts: &mut Vec<Type>, arrays: &mut Vec<Type>) {
    collect_type_sets(&e.ty, insts, arrays);
    if let Some((t, _)) = &e.box_to { collect_type_sets(t, insts, arrays); }
    match &e.kind {
        ExprKind::Bin(_, a, b) => { collect_expr_types(a, insts, arrays); collect_expr_types(b, insts, arrays); }
        ExprKind::Un(_, a) | ExprKind::Field(a, _) | ExprKind::Try(a) | ExprKind::Ok(a)
        | ExprKind::Err(a) | ExprKind::Some_(a) => collect_expr_types(a, insts, arrays),
        ExprKind::Call(_, targs, args) => {
            for t in targs { collect_type_sets(t, insts, arrays); }
            for a in args { collect_expr_types(a, insts, arrays); }
        }
        ExprKind::Method(recv, _, targs, args) => {
            collect_expr_types(recv, insts, arrays);
            for t in targs { collect_type_sets(t, insts, arrays); }
            for a in args { collect_expr_types(a, insts, arrays); }
        }
        ExprKind::New(_, targs, args) => {
            for t in targs { collect_type_sets(t, insts, arrays); }
            for a in args { collect_expr_types(a, insts, arrays); }
        }
        ExprKind::Cast(a, t, _) => { collect_expr_types(a, insts, arrays); collect_type_sets(t, insts, arrays); }
        ExprKind::Index(a, b) => { collect_expr_types(a, insts, arrays); collect_expr_types(b, insts, arrays); }
        ExprKind::ArrayLit(items) => for a in items { collect_expr_types(a, insts, arrays); },
        ExprKind::ArrayNew(t, n) => { collect_type_sets(t, insts, arrays); collect_expr_types(n, insts, arrays); }
        ExprKind::Interp(parts) => for p in parts {
            if let InterpPart::Expr(a) = p { collect_expr_types(a, insts, arrays); }
        },
        ExprKind::StaticPath { targs, args, .. } => {
            for t in targs { collect_type_sets(t, insts, arrays); }
            for a in args { collect_expr_types(a, insts, arrays); }
        }
        _ => {}
    }
}

fn mono_decl(d: &mut Decl, prog: &mut Program, diags: &mut Diag) {
    mono_type(&mut d.ret, prog, diags);
    for p in d.params.iter_mut() { mono_type(&mut p.ty, prog, diags); }
    for f in d.fields.iter_mut() {
        mono_type(&mut f.ty, prog, diags);
        if let Some(e) = &mut f.init { mono_expr(e, prog, diags); }
    }
    for v in d.variants.iter_mut() {
        for t in v.fields.iter_mut() { mono_type(t, prog, diags); }
    }
    mono_type(&mut d.alias_target, prog, diags);
    if let Some(b) = &mut d.body { mono_block(b, prog, diags); }
}

fn mono_block(b: &mut Block, prog: &mut Program, diags: &mut Diag) {
    for s in b.stmts.iter_mut() { mono_stmt(s, prog, diags); }
}

fn mono_stmt(s: &mut Stmt, prog: &mut Program, diags: &mut Diag) {
    match &mut s.kind {
        StmtKind::Var { ty, init, .. } => {
            mono_type(ty, prog, diags);
            if let Some(e) = init { mono_expr(e, prog, diags); }
        }
        StmtKind::Assign { target, value } => { mono_expr(target, prog, diags); mono_expr(value, prog, diags); }
        StmtKind::ExprStmt(e) | StmtKind::Return(Some(e)) => mono_expr(e, prog, diags),
        StmtKind::If { cond, then_b, else_b } => {
            mono_expr(cond, prog, diags);
            mono_block(then_b, prog, diags);
            if let Some(x) = else_b { mono_stmt(x, prog, diags); }
        }
        StmtKind::While { cond, body } => { mono_expr(cond, prog, diags); mono_block(body, prog, diags); }
        StmtKind::For { init, cond, step, body } => {
            mono_stmt(init, prog, diags);
            if let Some(c) = cond { mono_expr(c, prog, diags); }
            if let Some(st) = step { mono_stmt(st, prog, diags); }
            mono_block(body, prog, diags);
        }
        StmtKind::Unsafe(b) | StmtKind::Block(b) => mono_block(b, prog, diags),
        StmtKind::Match { subject, arms } => {
            mono_expr(subject, prog, diags);
            let concrete_enum = match &subject.ty { Type::Enum(id) => Some(*id), _ => None };
            for a in arms.iter_mut() {
                if let (Some(eid), Pattern::Variant { enum_id, .. }) = (concrete_enum, &mut a.pat) {
                    *enum_id = eid;
                }
                mono_block(&mut a.body, prog, diags);
            }
        }
        _ => {}
    }
}

fn mono_expr(e: &mut Expr, prog: &mut Program, diags: &mut Diag) {
    mono_type(&mut e.ty, prog, diags);
    if let Some((cb, ca, ib, ia)) = e.coerce_adt.clone() {
        let cid = ensure_instance(prog, diags, cb, &ca);
        let iid = ensure_instance(prog, diags, ib, &ia);
        if let (Some(cid), Some(iid)) = (cid, iid) {
            e.coerce = Some((cid, iid));
            e.coerce_adt = None;
        }
    }
    if let Some((t, id)) = e.box_to.clone() {
        let mut nt = t;
        mono_type(&mut nt, prog, diags);
        e.box_to = Some((nt, id));
    }
    mono_res(&mut e.res, prog, diags);
    match &mut e.kind {
        ExprKind::Bin(_, a, b) => { mono_expr(a, prog, diags); mono_expr(b, prog, diags); }
        ExprKind::Un(_, a) | ExprKind::Field(a, _) | ExprKind::Try(a) | ExprKind::Ok(a)
        | ExprKind::Err(a) | ExprKind::Some_(a) => mono_expr(a, prog, diags),
        ExprKind::Call(_, targs, args) => {
            for t in targs.iter_mut() { mono_type(t, prog, diags); }
            for a in args.iter_mut() { mono_expr(a, prog, diags); }
        }
        ExprKind::Method(recv, _, targs, args) => {
            mono_expr(recv, prog, diags);
            for t in targs.iter_mut() { mono_type(t, prog, diags); }
            for a in args.iter_mut() { mono_expr(a, prog, diags); }
        }
        ExprKind::New(_, targs, args) => {
            for t in targs.iter_mut() { mono_type(t, prog, diags); }
            for a in args.iter_mut() { mono_expr(a, prog, diags); }
        }
        ExprKind::Cast(a, t, _) => { mono_expr(a, prog, diags); mono_type(t, prog, diags); }
        ExprKind::Index(a, b) => { mono_expr(a, prog, diags); mono_expr(b, prog, diags); }
        ExprKind::ArrayLit(items) => for a in items.iter_mut() { mono_expr(a, prog, diags); },
        ExprKind::ArrayNew(t, n) => { mono_type(t, prog, diags); mono_expr(n, prog, diags); }
        ExprKind::Interp(parts) => for p in parts.iter_mut() {
            if let InterpPart::Expr(a) = p { mono_expr(a, prog, diags); }
        },
        ExprKind::StaticPath { targs, args, .. } => {
            for t in targs.iter_mut() { mono_type(t, prog, diags); }
            for a in args.iter_mut() { mono_expr(a, prog, diags); }
        }
        _ => {}
    }
    // 泛型方法体里对 `T: Eq/Ord` 的调用在实例化后已经变成具体标量。
    // 后端不认识接口方法 id，改写为对应的内建比较。
    // 接口方法则按具体接口实例重新定位声明，供 vtable 分派。
    let new_res: Option<Res> = match (&e.kind, &e.res) {
        (ExprKind::Method(recv, name, _, _), Res::Method(mid)) => {
            let from_iface = prog.decls[*mid].owner
                .map(|o| prog.decls[o].kind == DeclKind::Iface)
                .unwrap_or(false);
            if !from_iface { None } else {
                let rt = recv.ty.clone();
                match rt {
                    Type::I8 | Type::I16 | Type::I32 | Type::I64
                    | Type::U8 | Type::U16 | Type::U32 | Type::U64
                    | Type::F32 | Type::F64 | Type::Bool | Type::Str => match name.as_str() {
                        "equals" => Some(Res::Builtin(Builtin::ValueEq)),
                        "compare" => Some(Res::Builtin(Builtin::ValueCmp)),
                        "hash" => Some(Res::Builtin(Builtin::ValueHash)),
                        _ => None,
                    },
                    Type::Class(cid) | Type::Enum(cid) => prog.decls[cid].members.iter().copied()
                        .find(|m| {
                            let m = &prog.decls[*m];
                            m.name == *name && !m.is_ctor && !m.is_static
                        })
                        .map(Res::Method),
                    Type::Iface(iid) => prog.decls[iid].members.iter().copied()
                        .find(|m| prog.decls[*m].name == *name)
                        .map(Res::Method),
                    _ => None,
                }
            }
        }
        _ => None,
    };
    if let Some(n) = new_res { e.res = n; }
}

fn mono_type(t: &mut Type, prog: &mut Program, diags: &mut Diag) {
    match t {
        Type::Adt { base, args } => {
            for a in args.iter_mut() { mono_type(a, prog, diags); }
            if args.iter().any(type_is_param) { return; }
            let base = *base;
            if let Some(id) = find_instance(prog, base, args) {
                *t = concrete_type(prog, base, id);
            } else if let Some(id) = ensure_instance(prog, diags, base, args) {
                *t = concrete_type(prog, base, id);
            }
        }
        Type::Ptr(e) | Type::Array(e) | Type::Option(e) => mono_type(e, prog, diags),
        Type::Result(a, b) => { mono_type(a, prog, diags); mono_type(b, prog, diags); }
        Type::Generic(name, args) => {
            for a in args.iter_mut() { mono_type(a, prog, diags); }
            if args.iter().any(type_is_param) { return; }
            let base = prog.decls.iter().position(|d| {
                d.owner.is_none()
                    && !d.type_params.is_empty()
                    && d.type_params.len() == args.len()
                    && d.name == *name
                    && matches!(d.kind, DeclKind::Class | DeclKind::Enum | DeclKind::Iface)
            });
            if let Some(base) = base {
                if let Some(id) = ensure_instance(prog, diags, base, args) {
                    *t = concrete_type(prog, base, id);
                }
            }
        },
        _ => {}
    }
}

fn type_is_param(t: &Type) -> bool {
    match t {
        Type::Param(_) | Type::Named(_) | Type::Infer => true,
        Type::Ptr(e) | Type::Array(e) | Type::Option(e) => type_is_param(e),
        Type::Result(a, b) => type_is_param(a) || type_is_param(b),
        Type::Generic(_, args) | Type::Adt { args, .. } => args.iter().any(type_is_param),
        _ => false,
    }
}

fn concrete_type(prog: &Program, base: usize, id: usize) -> Type {
    match prog.decls[base].kind {
        DeclKind::Enum => Type::Enum(id),
        DeclKind::Iface => Type::Iface(id),
        _ => Type::Class(id),
    }
}

fn find_instance(prog: &Program, base: usize, args: &[Type]) -> Option<usize> {
    prog.generic_instances.iter()
        .find(|(b, a, _)| *b == base && a.len() == args.len() && a.iter().zip(args.iter()).all(|(x, y)| x == y))
        .map(|(_, _, id)| *id)
}

fn ensure_instance(prog: &mut Program, diags: &mut Diag, base: usize, args: &[Type]) -> Option<usize> {
    if prog.decls[base].type_params.is_empty() { return Some(base); }
    if args.iter().any(type_is_param) { return None; }
    if let Some(id) = find_instance(prog, base, args) { return Some(id); }
    let kind = prog.decls[base].kind;
    let mut s = Sema::new(prog, diags, base);
    match kind {
        DeclKind::Class => s.instantiate_class(base, args),
        DeclKind::Enum => s.instantiate_enum(base, args),
        DeclKind::Iface => s.instantiate_interface(base, args),
        _ => None,
    }
}

fn mono_res(r: &mut Res, prog: &mut Program, diags: &mut Diag) {
    let original = r.clone();
    match original {
        Res::AdtCtor { base, mut args } => {
            for a in args.iter_mut() { mono_type(a, prog, diags); }
            if let Some(cid) = ensure_instance(prog, diags, base, &args) {
                *r = Res::Ctor(cid);
            }
        }
        Res::AdtVariant { base, variant, mut args } => {
            for a in args.iter_mut() { mono_type(a, prog, diags); }
            if let Some(eid) = ensure_instance(prog, diags, base, &args) {
                *r = Res::Variant(eid, variant);
            }
        }
        Res::AdtFn { base, mut args } => {
            for a in args.iter_mut() { mono_type(a, prog, diags); }
            if args.iter().any(type_is_param) { return; }
            let id = if let Some(id) = find_instance(prog, base, &args) { Some(id) } else {
                let mut s = Sema::new(prog, diags, base);
                s.instantiate_fn(base, &args)
            };
            if let Some(id) = id { *r = Res::Fn(id); }
        }
        Res::AdtField { base, field, mut class_args } => {
            for a in class_args.iter_mut() { mono_type(a, prog, diags); }
            if let Some(cid) = ensure_instance(prog, diags, base, &class_args) {
                if field < prog.decls[cid].fields.len() {
                    *r = Res::Field(cid, field);
                }
            }
        }
        Res::AdtStaticField { base, field, mut class_args } => {
            for a in class_args.iter_mut() { mono_type(a, prog, diags); }
            if let Some(cid) = ensure_instance(prog, diags, base, &class_args) {
                if field < prog.decls[cid].fields.len() {
                    *r = Res::StaticField(cid, field);
                }
            }
        }
        Res::AdtMethod { base, method, mut class_args, mut method_args } => {
            for a in class_args.iter_mut() { mono_type(a, prog, diags); }
            for a in method_args.iter_mut() { mono_type(a, prog, diags); }
            if let Some(cid) = ensure_instance(prog, diags, base, &class_args) {
                let orig = prog.decls[method].clone();
                if let Some(mid) = prog.decls[cid].members.iter().copied().find(|m| {
                    let m = &prog.decls[*m];
                    m.name == orig.name && m.is_static == orig.is_static && m.is_ctor == orig.is_ctor
                }) {
                    if method_args.is_empty() {
                        *r = Res::Method(mid);
                    } else {
                        let mut s = Sema::new(prog, diags, mid);
                        if let Some(id) = s.instantiate_method(mid, &method_args) {
                            *r = Res::Method(id);
                        }
                    }
                }
            }
        }
        Res::AdtStaticMethod { base, method, mut class_args, mut method_args } => {
            for a in class_args.iter_mut() { mono_type(a, prog, diags); }
            for a in method_args.iter_mut() { mono_type(a, prog, diags); }
            if let Some(cid) = ensure_instance(prog, diags, base, &class_args) {
                let orig = prog.decls[method].clone();
                if let Some(mid) = prog.decls[cid].members.iter().copied().find(|m| {
                    let m = &prog.decls[*m];
                    m.name == orig.name && m.is_static == orig.is_static && m.is_ctor == orig.is_ctor
                }) {
                    if method_args.is_empty() {
                        *r = Res::StaticMethod(cid, mid);
                    } else {
                        let mut s = Sema::new(prog, diags, mid);
                        if let Some(id) = s.instantiate_method(mid, &method_args) {
                            *r = Res::StaticMethod(cid, id);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn type_contains_param_or_instance(
    t: &Type,
    names: &HashSet<String>,
    unresolved: &HashSet<usize>,
) -> bool {
    match t {
        Type::Named(n) | Type::Param(n) => names.contains(n),
        Type::Class(id) | Type::Enum(id) | Type::Iface(id) => unresolved.contains(id),
        Type::Adt { args, .. } => args
            .iter()
            .any(|a| type_contains_param_or_instance(a, names, unresolved)),
        Type::Generic(_, args) => args
            .iter()
            .any(|a| type_contains_param_or_instance(a, names, unresolved)),
        Type::Ptr(e) | Type::Array(e) | Type::Option(e) => {
            type_contains_param_or_instance(e, names, unresolved)
        }
        Type::Result(a, b) => {
            type_contains_param_or_instance(a, names, unresolved)
                || type_contains_param_or_instance(b, names, unresolved)
        }
        _ => false,
    }
}

fn neuter_generic_instance(d: &mut Decl) {
    d.kind = DeclKind::Alias;
    d.alias_target = Type::Infer;
    d.fields.clear();
    d.members.clear();
    d.body = None;
    d.params.clear();
    d.ret = Type::Infer;
}

/// 简单编辑距离：用于注解拼写提示
fn edit_distance(a: &str, b: &str) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..=n {
        dp[i][0] = i;
    }
    for j in 0..=m {
        dp[0][j] = j;
    }
    for (i, ca) in a.chars().enumerate() {
        for (j, cb) in b.chars().enumerate() {
            dp[i + 1][j + 1] = (dp[i][j + 1] + 1)
                .min(dp[i + 1][j] + 1)
                .min(dp[i][j] + if ca == cb { 0 } else { 1 });
        }
    }
    dp[n][m]
}

fn check_module_structure(prog: &mut Program, diags: &mut Diag) {
    for m in 0..prog.modules.len() {
        let file = prog.modules[m].path.clone();
        let stem = prog.modules[m].stem.clone();
        let is_bind = prog.modules[m].is_bind;
        let virtual_api = file.contains(".mylib");
        let mut pub_classes = 0usize;
        let decl_ids = prog.modules[m].decls.clone();
        for id in decl_ids {
            let (kind, name, is_pub, line, col, has_body, is_unsafe, annots) = {
                let d = &prog.decls[id];
                (
                    d.kind,
                    d.name.clone(),
                    d.is_pub,
                    d.line,
                    d.col,
                    d.body.is_some(),
                    d.is_unsafe,
                    d.annots.clone(),
                )
            };
            if kind == DeclKind::Enum && is_pub && name != stem && !virtual_api {
                diags.err(
                    2,
                    &file,
                    line,
                    col,
                    format!("pub enum `{}` 必须与文件名 `{}.myl` 完全一致", name, stem),
                );
            }
            if matches!(kind, DeclKind::Class | DeclKind::Enum) {
                // 方法名不可重复（禁止重载）
                let mut seen_methods: Vec<String> = Vec::new();
                for fid in prog.decls[id].members.clone() {
                    let (mname, mline2, mcol2) = {
                        let m = &prog.decls[fid];
                        (m.name.clone(), m.line, m.col)
                    };
                    if seen_methods.contains(&mname) {
                        diags.err(
                            E_GENERAL,
                            &file,
                            mline2,
                            mcol2,
                            format!("`{}` 内重复的方法 `{}`（Mylis 禁止重载）", name, mname),
                        );
                    } else {
                        seen_methods.push(mname);
                    }
                }
            }
            if kind == DeclKind::Class && is_pub {
                pub_classes += 1;
                if pub_classes > 1 && !virtual_api {
                    diags.err(
                        1,
                        &file,
                        line,
                        col,
                        "一个 .myl 文件最多只能有一个 pub class（此文件已有多个）",
                    );
                }
                if name != stem && !virtual_api {
                    diags.err(
                        2,
                        &file,
                        line,
                        col,
                        format!("pub class `{}` 必须与文件名 `{}.myl` 完全一致", name, stem),
                    );
                }
            }
            if kind == DeclKind::Class {
                // 同一 class 内字段名 / 方法名不可重复（禁止重载）
                let mut seen_fields: Vec<String> = Vec::new();
                let mut seen_methods: Vec<String> = Vec::new();
                let mut dups: Vec<(String, u32, u32, &str)> = Vec::new();
                for fid in prog.decls[id].members.clone() {
                    let (mname, mline, mcol) = {
                        let m = &prog.decls[fid];
                        (m.name.clone(), m.line, m.col)
                    };
                    if seen_methods.contains(&mname) {
                        dups.push((mname.clone(), mline, mcol, "方法"));
                    } else {
                        seen_methods.push(mname);
                    }
                }
                for f in &prog.decls[id].fields.clone() {
                    if seen_fields.contains(&f.name) {
                        dups.push((f.name.clone(), f.line, f.col, "字段"));
                    } else {
                        seen_fields.push(f.name.clone());
                    }
                }
                for (n, l, c, what) in dups {
                    let file2 = file.clone();
                    diags.err(
                        E_GENERAL,
                        &file2,
                        l,
                        c,
                        format!("class 内重复的{}名 `{}`（禁止重载 / 重名）", what, n),
                    );
                }
                // 字段名与方法名也不可冲突
                let cname = prog.decls[id].name.clone();
                let _ = cname;
            }
            if kind == DeclKind::Fn {
                if !has_body {
                    // `.mylib` 注入的 API 声明模块是虚拟模块，允许无体 pub fn。
                    let virtual_api = file.contains(".mylib");
                    if !is_unsafe && !virtual_api {
                        diags.err(
                            E_GENERAL,
                            &file,
                            line,
                            col,
                            "无函数体的声明只能是 `unsafe fn`（C 外部符号）",
                        );
                    }
                    if !is_bind && !virtual_api {
                        diags.err(
                            8,
                            &file,
                            line,
                            col,
                            "无体 unsafe fn（C 绑定）只能写在 *_bind.myl 文件中；业务文件禁止出现 C 绑定",
                        );
                    }
                }
                for a in &annots {
                    if a.name == "header" {
                        let ok = !has_body && is_unsafe && is_bind;
                        if !ok {
                            diags.err(
                                9,
                                &file,
                                a.line,
                                a.col,
                                "@header 只能用于 *_bind.myl 中的无体 unsafe fn（C 绑定）",
                            );
                        }
                        if a.args.len() != 1 || !matches!(a.args[0], AnnotArg::Str(_)) {
                            diags.err(
                                9,
                                &file,
                                a.line,
                                a.col,
                                "@header 需要一个字符串参数，例如 @header(\"stdio.h\")",
                            );
                        } else if let AnnotArg::Str(h) = &a.args[0] {
                            prog.decls[id].c_header = Some(h.clone());
                        }
                    }
                }
            }
        }
    }
}

impl<'a> Sema<'a> {
    fn new(prog: &'a mut Program, diags: &'a mut Diag, decl_id: usize) -> Sema<'a> {
        let mod_id = prog.decls[decl_id].mod_id;
        let file = prog.modules[mod_id].path.clone();
        let mut s = Sema {
            prog,
            diags,
            cur_mod: mod_id,
            cur_file: file,
            cur_fn: None,
            cur_class: None,
            scopes: vec![Vec::new()],
            unsafe_depth: 0,
            loop_depth: 0,
            ret_ty: Type::Void,
            visible: Vec::new(),
            in_field_init: false,
            in_ctor: false,
            const_writes: Vec::new(),
            type_param_bounds: std::collections::HashMap::new(),
            type_param_names: HashSet::new(),
            allow_self: false,
        };
        s.visible = s.compute_visible(mod_id);
        s
    }

    /// Rust 风格的可见性：只看直接 import；被 `pub import` 再导出的模块继续传递。
    fn compute_visible(&self, mod_id: usize) -> Vec<usize> {
        let mut out = vec![mod_id];
        let mut seen: HashSet<usize> = HashSet::from([mod_id]);
        let mut queue: Vec<(usize, bool)> = Vec::new();
        // 第一层：本模块的直接 import（无论是否 pub，都对本模块可见）
        for t in &self.prog.modules[mod_id].imports {
            if seen.insert(*t) {
                out.push(*t);
                queue.push((*t, true));
            }
        }
        // 后续层：只有 pub import 才会把依赖再导出
        let mut i = 0;
        while i < queue.len() {
            let (m, _) = queue[i];
            i += 1;
            for (k, t) in self.prog.modules[m].imports.iter().enumerate() {
                let (is_pub, is_selective) = self.prog.modules[m]
                    .import_raw
                    .get(k)
                    .map(|r| (r.3, r.4))
                    .unwrap_or((false, false));
                // 只有 `pub import "module"` 才把目标模块整体暴露给上层；
                // `pub import { A } from "module"` 只再导出名字，不暴露 module 本身。
                if is_pub && !is_selective && seen.insert(*t) {
                    out.push(*t);
                    queue.push((*t, false));
                }
            }
        }
        out
    }

    // ---------------------------------------------------------- 类型解析
    /// 在声明自己的模块作用域里解析类型。泛型实例是在调用方作用域里
    /// 创建的；若直接用调用方作用域解析，实例模块中本来就有的导入名
    /// （如 Map.myl 的 `Vec`）会看不到。
    fn resolve_instance_decl_types(&mut self, id: usize) {
        let mod_id = self.prog.decls[id].mod_id;
        let old_mod = self.cur_mod;
        let old_file = self.cur_file.clone();
        self.cur_mod = mod_id;
        self.cur_file = self.prog.modules[mod_id].path.clone();
        self.resolve_decl_types(id);
        self.cur_mod = old_mod;
        self.cur_file = old_file;
    }

    fn resolve_decl_types(&mut self, id: usize) {
        if !self.prog.decls[id].type_params.is_empty() {
            return;
        }
        if let Some(owner) = self.prog.decls[id].owner {
            if !self.prog.decls[owner].type_params.is_empty() {
                return;
            }
        }
        let is_iface_member = self.prog.decls[id].owner
            .map(|o| self.prog.decls[o].kind == DeclKind::Iface)
            .unwrap_or(false);
        let old_allow_self = self.allow_self;
        self.allow_self = is_iface_member;
        let (kind, mut ret, mut params, mut fields) = {
            let d = &self.prog.decls[id];
            (
                d.kind,
                d.ret.clone(),
                d.params.clone(),
                d.fields.clone(),
            )
        };
        match kind {
            DeclKind::Alias => {
                let mut t = self.prog.decls[id].alias_target.clone();
                self.resolve_type(&mut t, id);
                self.prog.decls[id].alias_target = t;
            }
            DeclKind::Const => {
                self.resolve_type(&mut ret, id);
                self.prog.decls[id].ret = ret;
            }
            DeclKind::Fn => {
                self.resolve_type(&mut ret, id);
                for p in params.iter_mut() {
                    self.resolve_type(&mut p.ty, id);
                }
                self.prog.decls[id].ret = ret;
                self.prog.decls[id].params = params;
            }
            DeclKind::Class => {
                for f in fields.iter_mut() {
                    self.resolve_type(&mut f.ty, id);
                    if f.is_static {
                        if let Some(init) = &f.init {
                            let ok = matches!(
                                init.kind,
                                ExprKind::Int(_)
                                    | ExprKind::IntU64(_)
                                    | ExprKind::Float(_)
                                    | ExprKind::Str(_)
                                    | ExprKind::Bool(_)
                                    | ExprKind::Null
                            );
                            if !ok {
                                self.diags.err(
                                    12,
                                    &self.cur_file.clone(),
                                    f.line,
                                    f.col,
                                    format!(
                                        "静态字段 `{}` 的初值必须是常量字面量（int/float/string/bool）",
                                        f.name
                                    ),
                                );
                            }
                        }
                    }
                }
                self.prog.decls[id].fields = fields;
            }
            DeclKind::Iface => {
                for p in params.iter_mut() {
                    self.resolve_type(&mut p.ty, id);
                }
                self.prog.decls[id].ret = ret;
                self.prog.decls[id].params = params;
            }
            DeclKind::Enum => {
                let mut variants = self.prog.decls[id].variants.clone();
                for v in variants.iter_mut() {
                    for ft in v.fields.iter_mut() {
                        self.resolve_type(ft, id);
                    }
                }
                self.prog.decls[id].variants = variants;
            }
        }
        // 方法/签名成员由各自 decl 的循环处理
        self.allow_self = old_allow_self;
    }

    fn resolve_type(&mut self, t: &mut Type, at_decl: usize) {
        let (line, col) = {
            let d = &self.prog.decls[at_decl];
            (d.line, d.col)
        };
        match t {
            Type::Generic(name, args) => {
                let base = match self.lookup_type(name) {
                    Some(id) => id,
                    None => {
                        self.diags.err(E_GENERAL, &self.cur_file, line, col, format!("未知类型 `{}`", name));
                        *t = Type::Infer;
                        return;
                    }
                };
                if self.prog.decls[base].kind != DeclKind::Class
                    && self.prog.decls[base].kind != DeclKind::Iface
                    && self.prog.decls[base].kind != DeclKind::Enum
                {
                    self.diags.err(E_GENERAL, &self.cur_file, line, col, format!("`{}` 不是泛型 class/interface/enum", name));
                    *t = Type::Infer;
                    return;
                }
                if self.prog.decls[base].type_params.is_empty() {
                    self.diags.err(E_GENERAL, &self.cur_file, line, col, format!("`{}` 不是泛型类型，不能带类型实参", name));
                    *t = Type::Infer;
                    return;
                }
                let mut rargs = args.clone();
                for a in rargs.iter_mut() { self.resolve_type(a, at_decl); }
                if rargs.len() != self.prog.decls[base].type_params.len() {
                    self.diags.err(E_GENERAL, &self.cur_file, line, col, format!("`{}` 需要 {} 个类型实参，实际 {}", name, self.prog.decls[base].type_params.len(), rargs.len()));
                    *t = Type::Infer;
                    return;
                }
                // 类型检查阶段只保留规范化的 (模板, 实参) 类型；
                // 具体 Decl 在单态化阶段再生成。
                *t = Type::Adt { base, args: rargs };
            }
            Type::Named(name) if name == "Self" => {
                if !self.allow_self {
                    self.diags.err(E_GENERAL, &self.cur_file, line, col, "`Self` 只能出现在 interface 方法签名中".to_string());
                    *t = Type::Infer;
                }
            }
            Type::Named(name) if self.type_param_names.contains(name) => {
                *t = Type::Param(name.clone());
            }
            Type::Named(name) => match self.lookup_type(name) {
                Some(tid) if self.prog.decls[tid].kind == DeclKind::Alias => {
                    if self.prog.decls[tid].mod_id != self.cur_mod && !self.prog.decls[tid].is_pub {
                        self.diags.err(
                            5,
                            &self.cur_file,
                            line,
                            col,
                            format!("`{}` 不是 pub，跨文件不可见", name),
                        );
                    }
                    let mut target = self.prog.decls[tid].alias_target.clone();
                    if matches!(&target, Type::Named(n) if n == name) {
                        self.diags.err(
                            15,
                            &self.cur_file,
                            line,
                            col,
                            format!("类型别名 `{}` 指向自身（循环别名）", name),
                        );
                        *t = Type::Infer;
                        return;
                    }
                    self.resolve_type(&mut target, at_decl);
                    *t = target;
                }
                Some(tid) => {
                    if self.prog.decls[tid].mod_id != self.cur_mod && !self.prog.decls[tid].is_pub {
                        self.diags.err(
                            5,
                            &self.cur_file,
                            line,
                            col,
                            format!("`{}` 不是 pub，跨文件不可见", name),
                        );
                    }
                    *t = match self.prog.decls[tid].kind {
                        DeclKind::Class => Type::Class(tid),
                        DeclKind::Enum => Type::Enum(tid),
                        _ => Type::Iface(tid),
                    };
                }
                None => {
                    self.diags.err(
                        E_GENERAL,
                        &self.cur_file,
                        line,
                        col,
                        format!("未知类型 `{}`", name),
                    );
                    *t = Type::Infer;
                }
            },
            Type::Ptr(e) => self.resolve_type(e, at_decl),
            Type::Array(e) => {
                self.resolve_type(e, at_decl);
                let is_tparam = matches!(e.as_ref(), Type::Named(n) if self.type_param_bounds.contains_key(n));
                if !e.is_value() && !is_tparam {
                    self.diags.err(
                        12,
                        &self.cur_file,
                        line,
                        col,
                        format!("数组元素类型必须是值类型，不能是 {}", e),
                    );
                }
            }
            Type::Result(a, b) => {
                self.resolve_type(a, at_decl);
                self.resolve_type(b, at_decl);
                if !a.is_value() && !matches!(a.as_ref(), Type::Iface(_)) {
                    self.diags.err(
                        12,
                        &self.cur_file,
                        line,
                        col,
                        format!("Result 的 Ok 载荷类型必须是值类型，不能是 {}", a),
                    );
                }
                if !b.is_value() && !matches!(b.as_ref(), Type::Iface(_)) {
                    self.diags.err(
                        12,
                        &self.cur_file,
                        line,
                        col,
                        format!("Result 的 Err 载荷类型必须是值类型，不能是 {}", b),
                    );
                }
            }
            Type::Option(a) => {
                self.resolve_type(a, at_decl);
                if !a.is_value() && !matches!(a.as_ref(), Type::Iface(_)) {
                    self.diags.err(
                        12,
                        &self.cur_file,
                        line,
                        col,
                        format!("Option 的载荷类型必须是值类型，不能是 {}", a),
                    );
                }
            }
            _ => {}
        }
        if !matches!(t, Type::Named(_)) {
            self.prog.note_inst(t);
        }
    }

    fn args_contain_type_param(&self, args: &[Type]) -> bool {
        let names: HashSet<String> = self.type_param_bounds.keys().cloned().collect();
        args.iter().any(|a| type_contains_param_or_instance(a, &names, &HashSet::new()))
    }

    fn instantiate_method(&mut self, base: usize, args: &[Type]) -> Option<usize> {
        if !self.check_type_bounds(base, args) { return None; }
        if let Some((_, _, id)) = self.prog.generic_instances.iter()
            .find(|(b, a, _)| {
                *b == base && a.len() == args.len()
                    && a.iter().zip(args.iter()).all(|(x, y)| self.same_type(x, y))
            })
        {
            return Some(*id);
        }
        let params = self.prog.decls[base].type_params.clone();
        if params.len() != args.len() { return None; }
        let mut map = std::collections::HashMap::new();
        for (p, a) in params.iter().zip(args.iter()) {
            map.insert(p.clone(), a.clone());
        }
        let mut f = self.prog.decls[base].clone();
        f.type_params.clear();
        f.name = format!("{}{}", f.name, crate::generics::generic_suffix(&self.prog, args));
        f.generic_base = Some(base);
        f.checked = false;
        crate::generics::subst_decl(&mut f, &map);
        let new_id = self.prog.decls.len();
        let owner = f.owner.unwrap_or(0);
        self.prog.decls.push(f);
        self.prog.decls[owner].members.push(new_id);
        self.prog.generic_instances.push((base, args.to_vec(), new_id));
        if !self.args_contain_type_param(args) {
            self.resolve_instance_decl_types(new_id);
        }
        Some(new_id)
    }

    fn instantiate_fn(&mut self, base: usize, args: &[Type]) -> Option<usize> {
        if !self.check_type_bounds(base, args) { return None; }
        if let Some((_, _, id)) = self.prog.generic_instances.iter()
            .find(|(b, a, _)| {
                *b == base && a.len() == args.len()
                    && a.iter().zip(args.iter()).all(|(x, y)| self.same_type(x, y))
            })
        {
            return Some(*id);
        }
        let params = self.prog.decls[base].type_params.clone();
        if params.len() != args.len() { return None; }
        let mut map = std::collections::HashMap::new();
        for (p, a) in params.iter().zip(args.iter()) {
            map.insert(p.clone(), a.clone());
        }
        let mut f = self.prog.decls[base].clone();
        f.type_params.clear();
        f.name = format!("{}{}", f.name, crate::generics::generic_suffix(&self.prog, args));
        f.generic_base = Some(base);
        f.checked = false;
        crate::generics::subst_decl(&mut f, &map);
        let new_id = self.prog.decls.len();
        let mod_id = f.mod_id;
        self.prog.decls.push(f);
        self.prog.modules[mod_id].decls.push(new_id);
        self.prog.generic_instances.push((base, args.to_vec(), new_id));
        if !self.args_contain_type_param(args) {
            self.resolve_instance_decl_types(new_id);
        }
        Some(new_id)
    }

    fn generic_args_for_concrete(&self, base: usize, concrete: usize) -> Option<Vec<Type>> {
        self.prog.generic_instances.iter()
            .find(|(b, _, id)| *b == base && *id == concrete)
            .map(|(_, a, _)| a.clone())
    }

    /// 把已经单态化的 Class/Enum 还原成「模板名 + 类型实参」。
    fn concrete_generic_name_args(&self, id: usize) -> Option<(String, Vec<Type>)> {
        let base = self.prog.decls.get(id)?.generic_base?;
        let name = self.prog.decls.get(base)?.name.clone();
        let args = self.generic_args_for_concrete(base, id)?;
        Some((name, args))
    }

    /// 判断两个 sema 类型是否表示同一个类型。
    ///
    /// `Vec<i64>` 在源码/签名里可能是 `Generic("Vec", [I64])`，在实例化后
    /// 又会变成 `Class(Vec__i64)`；两者不能只靠 `==` 比较。
    fn same_type(&self, a: &Type, b: &Type) -> bool {
        if a == b { return true; }
        if matches!(a, Type::Infer) || matches!(b, Type::Infer) { return true; }
        match (a, b) {
            (Type::Ptr(x), Type::Ptr(y)) => self.same_type(x, y),
            (Type::Array(x), Type::Array(y)) => self.same_type(x, y),
            (Type::Option(x), Type::Option(y)) => self.same_type(x, y),
            (Type::Result(xa, xb), Type::Result(ya, yb)) => {
                self.same_type(xa, ya) && self.same_type(xb, yb)
            }
            (Type::Class(c), Type::Generic(n, args))
            | (Type::Enum(c), Type::Generic(n, args)) => {
                match self.concrete_generic_name_args(*c) {
                    Some((cn, ca)) => {
                        cn == *n && ca.len() == args.len()
                            && ca.iter().zip(args.iter()).all(|(x, y)| self.same_type(x, y))
                    }
                    None => false,
                }
            }
            (Type::Generic(n, args), Type::Class(c))
            | (Type::Generic(n, args), Type::Enum(c)) => {
                self.same_type(b, a)
            }
            (Type::Class(a), Type::Class(b))
            | (Type::Enum(a), Type::Enum(b))
            | (Type::Iface(a), Type::Iface(b)) => {
                match (self.concrete_generic_name_args(*a), self.concrete_generic_name_args(*b)) {
                    (Some((na, aa)), Some((nb, ab))) => {
                        na == nb && aa.len() == ab.len()
                            && aa.iter().zip(ab.iter()).all(|(x, y)| self.same_type(x, y))
                    }
                    _ => false,
                }
            }
            (Type::Generic(n, xa), Type::Generic(m, xb)) => {
                n == m && xa.len() == xb.len()
                    && xa.iter().zip(xb.iter()).all(|(x, y)| self.same_type(x, y))
            }
            (Type::Adt { base: ba, args: aa }, Type::Adt { base: bb, args: ab }) => {
                ba == bb && aa.len() == ab.len()
                    && aa.iter().zip(ab.iter()).all(|(x, y)| self.same_type(x, y))
            }
            (Type::Adt { base, args }, Type::Generic(n, ga))
            | (Type::Generic(n, ga), Type::Adt { base, args }) => {
                let bn = self.prog.decls[*base].name.clone();
                bn == *n && args.len() == ga.len()
                    && args.iter().zip(ga.iter()).all(|(x, y)| self.same_type(x, y))
            }
            (Type::Param(a), Type::Param(b)) => a == b,
            _ => false,
        }
    }

    fn expr_type_hint(&self, e: &Expr) -> Option<Type> {
        Some(match &e.kind {
            ExprKind::Int(_) => Type::I32,
            ExprKind::IntU64(_) => Type::U64,
            ExprKind::Float(_) => Type::F64,
            ExprKind::Str(_) => Type::Str,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Cast(_, t, _) => t.clone(),
            _ => return None,
        })
    }

    fn bind_type_param(
        &self,
        tmpl: &Type,
        actual: &Type,
        params: &[String],
        map: &mut std::collections::HashMap<String, Type>,
    ) -> bool {
        match tmpl {
            Type::Named(n) | Type::Param(n) if params.contains(n) => {
                if let Some(old) = map.get(n) {
                    self.same_type(old, actual)
                } else {
                    map.insert(n.clone(), actual.clone());
                    true
                }
            }
            Type::Ptr(a) => match actual {
                Type::Ptr(b) => self.bind_type_param(a, b, params, map),
                _ => true,
            },
            Type::Array(a) => match actual {
                Type::Array(b) => self.bind_type_param(a, b, params, map),
                _ => true,
            },
            Type::Option(a) => match actual {
                Type::Option(b) => self.bind_type_param(a, b, params, map),
                _ => true,
            },
            Type::Result(a, b) => match actual {
                Type::Result(c, d) => {
                    self.bind_type_param(a, c, params, map)
                        && self.bind_type_param(b, d, params, map)
                }
                _ => true,
            },
            Type::Generic(name, targs) => {
                let got = match actual {
                    Type::Class(cid) | Type::Enum(cid) | Type::Iface(cid) => self
                        .lookup_type_pub(*cid, name),
                    Type::Generic(an, aa) if an == name => Some(aa.clone()),
                    Type::Adt { base, args } if self.prog.decls[*base].name == *name => Some(args.clone()),
                    _ => None,
                };
                if let Some(got) = got {
                    if got.len() == targs.len() {
                        for (t, a) in targs.iter().zip(got.iter()) {
                            if !self.bind_type_param(t, a, params, map) { return false; }
                        }
                    }
                }
                true
            }
            Type::Adt { base, args } => {
                let got = match actual {
                    Type::Adt { base: ab, args: aa } if *ab == *base => Some(aa.clone()),
                    Type::Generic(n, aa) if self.prog.decls[*base].name == *n => Some(aa.clone()),
                    Type::Class(cid) | Type::Enum(cid) | Type::Iface(cid) => {
                        self.generic_args_for_concrete(*base, *cid)
                    }
                    _ => None,
                };
                if let Some(got) = got {
                    if got.len() == args.len() {
                        for (t, a) in args.iter().zip(got.iter()) {
                            if !self.bind_type_param(t, a, params, map) { return false; }
                        }
                    }
                }
                true
            }
            _ => true,
        }
    }

    fn lookup_type_pub(&self, concrete: usize, base_name: &str) -> Option<Vec<Type>> {
        let base = self.lookup_type(base_name)?;
        self.generic_args_for_concrete(base, concrete)
    }

    fn infer_targs_from_parts(
        &mut self,
        base: usize,
        ret: &Type,
        params: &[Param],
        expected: Option<&Type>,
        args: &[Expr],
    ) -> Option<Vec<Type>> {
        let names = self.prog.decls[base].type_params.clone();
        if names.is_empty() { return Some(Vec::new()); }
        let mut map = std::collections::HashMap::new();
        if let Some(exp) = expected {
            if *exp != Type::Infer {
                self.bind_type_param(ret, exp, &names, &mut map);
            }
        }
        for (p, a) in params.iter().zip(args.iter()) {
            let hint = match self.expr_type_hint(a) {
                Some(h) => h,
                None => {
                    let mut tmp = a.clone();
                    self.check_expr(&mut tmp, None)
                },
            };
            if hint != Type::Infer {
                self.bind_type_param(&p.ty, &hint, &names, &mut map);
            }
        }
        let mut out = Vec::new();
        for n in &names {
            match map.get(n) {
                Some(t) => out.push(t.clone()),
                None => return None,
            }
        }
        Some(out)
    }

    fn check_type_bounds(&mut self, base: usize, args: &[Type]) -> bool {
        let (line, col) = {
            let d = &self.prog.decls[base];
            (d.line, d.col)
        };
        let params = self.prog.decls[base].type_params.clone();
        let bounds = self.prog.decls[base].type_bounds.clone();
        let mut ok = true;
        for (i, pname) in params.iter().enumerate() {
            let Some(arg) = args.get(i) else { continue };
            for bname in bounds.iter()
                .find(|(n, _)| n == pname)
                .map(|(_, b)| b.clone())
                .unwrap_or_default()
            {
                let bid = {
                    // bound 名称属于声明所在模块的导入环境，不能在调用方模块里查找；
                    // 否则跨模块使用 `contains<T: Eq>` 时会找不到 Eq。
                    let old_mod = self.cur_mod;
                    self.cur_mod = self.prog.decls[base].mod_id;
                    let got = self.lookup_type(&bname);
                    self.cur_mod = old_mod;
                    match got {
                        Some(id) => id,
                        None => {
                            self.err_at(line, col, E_GENERAL, format!("未知 bound 接口 `{}`", bname));
                            ok = false;
                            continue;
                        }
                    }
                };
                if self.prog.decls[bid].kind != DeclKind::Iface {
                    self.err_at(line, col, E_GENERAL, format!("bound `{}` 不是 interface", bname));
                    ok = false;
                    continue;
                }
                let iface_name = self.prog.decls[bid].name.clone();
                let satisfied = match arg {
                    Type::Class(cid) | Type::Enum(cid) => {
                        self.iface_compatible(*cid, bid, line, col)
                    }
                    Type::Iface(iid) => *iid == bid,
                    Type::Param(n) | Type::Named(n) => self
                        .type_param_bounds
                        .get(n)
                        .map(|ids| ids.contains(&bid))
                        .unwrap_or(false),
                    Type::Adt { base, .. } => {
                        self.iface_compatible(*base, bid, line, col)
                    }
                    Type::I8 | Type::I16 | Type::I32 | Type::I64
                    | Type::U8 | Type::U16 | Type::U32 | Type::U64
                    | Type::F32 | Type::F64 | Type::Bool | Type::Str
                        if matches!(iface_name.as_str(), "ToString" | "Eq" | "Ord" | "Hash") => true,
                    _ => false,
                };
                if !satisfied {
                    self.err_at(
                        line,
                        col,
                        E_GENERAL,
                        format!("类型参数 `{}` 的实参 {} 不满足 bound `{}`", pname, arg, bname),
                    );
                    ok = false;
                }
            }
        }
        ok
    }

    fn instantiate_enum(&mut self, base: usize, args: &[Type]) -> Option<usize> {
        if !self.check_type_bounds(base, args) { return None; }
        if let Some((_, _, id)) = self.prog.generic_instances.iter()
            .find(|(b, a, _)| {
                *b == base && a.len() == args.len()
                    && a.iter().zip(args.iter()).all(|(x, y)| self.same_type(x, y))
            })
        {
            return Some(*id);
        }
        let params = self.prog.decls[base].type_params.clone();
        if params.len() != args.len() { return None; }
        let mut map = std::collections::HashMap::new();
        for (p, a) in params.iter().zip(args.iter()) { map.insert(p.clone(), a.clone()); }
        let mut e = self.prog.decls[base].clone();
        e.type_params.clear();
        e.name = format!("{}{}", e.name, crate::generics::generic_suffix(&self.prog, args));
        e.generic_base = Some(base);
        e.members.clear();
        crate::generics::subst_decl(&mut e, &map);
        let new_id = self.prog.decls.len();
        let old_members = self.prog.decls[base].members.clone();
        self.prog.decls.push(e);
        for mid in old_members {
            let mut m = self.prog.decls[mid].clone();
            m.owner = Some(new_id);
            m.checked = false;
            crate::generics::subst_decl(&mut m, &map);
            let nid = self.prog.decls.len();
            self.prog.decls.push(m);
            self.prog.decls[new_id].members.push(nid);
        }
        self.prog.generic_instances.push((base, args.to_vec(), new_id));
        if !self.args_contain_type_param(args) {
            self.resolve_instance_decl_types(new_id);
            let members = self.prog.decls[new_id].members.clone();
            for mid in members {
                self.resolve_instance_decl_types(mid);
            }
        }
        Some(new_id)
    }

    fn instantiate_interface(&mut self, base: usize, args: &[Type]) -> Option<usize> {
        if !self.check_type_bounds(base, args) { return None; }
        if let Some((_, _, id)) = self.prog.generic_instances.iter()
            .find(|(b, a, _)| {
                *b == base && a.len() == args.len()
                    && a.iter().zip(args.iter()).all(|(x, y)| self.same_type(x, y))
            })
        {
            return Some(*id);
        }
        let params = self.prog.decls[base].type_params.clone();
        if params.len() != args.len() { return None; }
        let mut map = std::collections::HashMap::new();
        for (p, a) in params.iter().zip(args.iter()) {
            map.insert(p.clone(), a.clone());
        }
        let mut iface = self.prog.decls[base].clone();
        iface.type_params.clear();
        iface.name = format!("{}{}", iface.name, crate::generics::generic_suffix(&self.prog, args));
        iface.generic_base = Some(base);
        iface.members.clear();
        crate::generics::subst_decl(&mut iface, &map);
        let new_id = self.prog.decls.len();
        let old_members = self.prog.decls[base].members.clone();
        self.prog.decls.push(iface);
        for mid in old_members {
            let mut m = self.prog.decls[mid].clone();
            m.owner = Some(new_id);
            m.checked = false;
            crate::generics::subst_decl(&mut m, &map);
            let nid = self.prog.decls.len();
            self.prog.decls.push(m);
            self.prog.decls[new_id].members.push(nid);
        }
        self.prog.generic_instances.push((base, args.to_vec(), new_id));
        if !self.args_contain_type_param(args) {
            self.resolve_instance_decl_types(new_id);
            let members = self.prog.decls[new_id].members.clone();
            for mid in members {
                self.resolve_instance_decl_types(mid);
            }
        }
        Some(new_id)
    }

    fn instantiate_class(&mut self, base: usize, args: &[Type]) -> Option<usize> {
        if !self.check_type_bounds(base, args) { return None; }
        if let Some((_, _, id)) = self.prog.generic_instances.iter()
            .find(|(b, a, _)| {
                *b == base && a.len() == args.len()
                    && a.iter().zip(args.iter()).all(|(x, y)| self.same_type(x, y))
            })
        {
            return Some(*id);
        }
        let params = self.prog.decls[base].type_params.clone();
        if params.len() != args.len() { return None; }
        let mut map = std::collections::HashMap::new();
        for (p, a) in params.iter().zip(args.iter()) {
            map.insert(p.clone(), a.clone());
        }
        let mut class = self.prog.decls[base].clone();
        class.type_params.clear();
        class.name = format!("{}{}", class.name, crate::generics::generic_suffix(&self.prog, args));
        class.generic_base = Some(base);
        let _ = args;
        class.members.clear();
        let mut class = class;
        crate::generics::subst_decl(&mut class, &map);
        let new_id = self.prog.decls.len();
        self.prog.decls.push(class);
        self.prog.generic_instances.push((base, args.to_vec(), new_id));

        let base_members = self.prog.decls[base].members.clone();
        let mut new_members = Vec::new();
        for mid in base_members {
            let mut m = self.prog.decls[mid].clone();
            m.owner = Some(new_id);
            crate::generics::subst_decl(&mut m, &map);
            // 泛型方法保留自己的类型参数；普通方法直接属于具体类。
            let nid = self.prog.decls.len();
            self.prog.decls.push(m);
            new_members.push(nid);
        }
        self.prog.decls[new_id].members = new_members.clone();
        if !self.args_contain_type_param(args) {
            self.resolve_instance_decl_types(new_id);
            for mid in new_members {
                self.resolve_instance_decl_types(mid);
            }
        }
        Some(new_id)
    }

    fn module_full_name(&self, m: usize) -> String {
        self.prog.modules[m].full_name()
    }

    fn find_exported_fn(&self, m: usize, name: &str, seen: &mut HashSet<usize>) -> Option<usize> {
        if !seen.insert(m) { return None; }
        if let Some(id) = self.prog.find_fn_in(m, name) { return Some(id); }
        for (n, t) in &self.prog.modules[m].reexports {
            if n == name {
                if let Some(id) = self.find_exported_fn(*t, name, seen) { return Some(id); }
            }
        }
        None
    }

    fn find_exported_const(&self, m: usize, name: &str, seen: &mut HashSet<usize>) -> Option<usize> {
        if !seen.insert(m) { return None; }
        if let Some(id) = self.prog.find_const_in(m, name) { return Some(id); }
        for (n, t) in &self.prog.modules[m].reexports {
            if n == name {
                if let Some(id) = self.find_exported_const(*t, name, seen) { return Some(id); }
            }
        }
        None
    }

    fn find_exported_type(&self, m: usize, name: &str, seen: &mut HashSet<usize>) -> Option<usize> {
        if !seen.insert(m) { return None; }
        if let Some(id) = self.prog.find_type_in(m, name) { return Some(id); }
        for (n, t) in &self.prog.modules[m].reexports {
            if n == name {
                if let Some(id) = self.find_exported_type(*t, name, seen) { return Some(id); }
            }
        }
        None
    }

    fn lookup_type(&self, name: &str) -> Option<usize> {
        if let Some((scope, leaf)) = name.rsplit_once("::") {
            // 编译器生成的库 API 源允许看见全部已加载模块，以便跨包签名自洽；
            // 用户模块必须显式 import 目标模块，限定名不再绕过可见性。
            let api_module = self.prog.modules[self.cur_mod].path.contains(".mylib");
            let candidates: Vec<usize> = if api_module {
                (0..self.prog.modules.len()).collect()
            } else {
                self.visible.clone()
            };
            for m in candidates {
                let info = &self.prog.modules[m];
                let full = self.module_full_name(m);
                let leaf_scope = format!("{}::{}", scope, leaf);
                if full == scope || full == leaf_scope {
                    let mut seen = HashSet::new();
                    if let Some(id) = self.find_exported_type(m, leaf, &mut seen) {
                        return Some(id);
                    }
                }
            }
            return None;
        }
        let mut seen = HashSet::new();
        if let Some(id) = self.find_exported_type(self.cur_mod, name, &mut seen) {
            return Some(id);
        }
        for (n, m) in &self.prog.modules[self.cur_mod].imported {
            if n == name {
                let mut seen = HashSet::new();
                if let Some(id) = self.find_exported_type(*m, name, &mut seen) {
                    return Some(id);
                }
            }
        }
        None
    }

    fn lookup_const(&self, name: &str) -> Option<usize> {
        if let Some((scope, leaf)) = name.rsplit_once("::") {
            let api_module = self.prog.modules[self.cur_mod].path.contains(".mylib");
            let candidates: Vec<usize> = if api_module {
                (0..self.prog.modules.len()).collect()
            } else {
                self.visible.clone()
            };
            for m in candidates {
                if self.module_full_name(m) == scope {
                    let mut seen = HashSet::new();
                    if let Some(id) = self.find_exported_const(m, leaf, &mut seen) { return Some(id); }
                }
            }
            return None;
        }
        let mut seen = HashSet::new();
        if let Some(id) = self.find_exported_const(self.cur_mod, name, &mut seen) { return Some(id); }
        for (n, m) in &self.prog.modules[self.cur_mod].imported {
            if n == name {
                let mut seen = HashSet::new();
                if let Some(id) = self.find_exported_const(*m, name, &mut seen) { return Some(id); }
            }
        }
        None
    }

    fn lookup_fn(&self, name: &str) -> Option<usize> {
        if let Some((scope, leaf)) = name.rsplit_once("::") {
            let api_module = self.prog.modules[self.cur_mod].path.contains(".mylib");
            let candidates: Vec<usize> = if api_module {
                (0..self.prog.modules.len()).collect()
            } else {
                self.visible.clone()
            };
            for m in candidates {
                let info = &self.prog.modules[m];
                let full = self.module_full_name(m);
                if full == scope {
                    let mut seen = HashSet::new();
                    if let Some(id) = self.find_exported_fn(m, leaf, &mut seen) {
                        return Some(id);
                    }
                }
            }
            return None;
        }
        let mut seen = HashSet::new();
        if let Some(id) = self.find_exported_fn(self.cur_mod, name, &mut seen) {
            return Some(id);
        }
        for (n, m) in &self.prog.modules[self.cur_mod].imported {
            if n == name {
                let mut seen = HashSet::new();
                if let Some(id) = self.find_exported_fn(*m, name, &mut seen) {
                    return Some(id);
                }
            }
        }
        None
    }

    // ---------------------------------------------------------- 作用域
    fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn declare(&mut self, name: &str, ty: Type, line: u32, col: u32) {
        self.declare_ex(name, ty, false, line, col)
    }
    fn declare_ex(&mut self, name: &str, ty: Type, is_const: bool, line: u32, col: u32) {
        if let Some(top) = self.scopes.last() {
            if top.iter().any(|(n, _, _)| n == name) {
                self.diags.err(
                    E_GENERAL,
                    &self.cur_file,
                    line,
                    col,
                    format!("同一作用域内重复声明变量 `{}`", name),
                );
            }
        }
        if let Some(top) = self.scopes.last_mut() {
            top.push((name.to_string(), ty, is_const));
        }
    }
    fn lookup_var(&self, name: &str) -> Option<Type> {
        for s in self.scopes.iter().rev() {
            if let Some((_, t, _)) = s.iter().rev().find(|(n, _, _)| n == name) {
                return Some(t.clone());
            }
        }
        None
    }
    fn var_is_const(&self, name: &str) -> bool {
        for s in self.scopes.iter().rev() {
            if let Some((_, _, c)) = s.iter().rev().find(|(n, _, _)| n == name) {
                return *c;
            }
        }
        false
    }
    fn err_at(&mut self, line: u32, col: u32, code: u32, msg: impl Into<String>) {
        let f = self.cur_file.clone();
        self.diags.err(code, &f, line, col, msg);
    }

    // ------------------------------------------------------ 检查函数体
    fn enter_type_env(
        &mut self,
        decl_ids: &[usize],
    ) -> (std::collections::HashMap<String, Vec<usize>>, HashSet<String>) {
        let old_bounds = std::mem::take(&mut self.type_param_bounds);
        let old_names = std::mem::take(&mut self.type_param_names);
        let mut bounds = Vec::new();
        for did in decl_ids {
            for p in &self.prog.decls[*did].type_params {
                self.type_param_names.insert(p.clone());
            }
            for (n, bs) in self.prog.decls[*did].type_bounds.clone() {
                bounds.push((n, bs));
            }
        }
        for (n, bs) in bounds {
            let mut ids = Vec::new();
            for b in bs {
                match self.lookup_type(&b) {
                    Some(id) if self.prog.decls[id].kind == DeclKind::Iface => ids.push(id),
                    Some(_) => {
                        let (l, c) = (self.prog.decls[decl_ids[0]].line, self.prog.decls[decl_ids[0]].col);
                        self.err_at(l, c, E_GENERAL, format!("bound `{}` 不是 interface", b));
                    }
                    None => {
                        let (l, c) = (self.prog.decls[decl_ids[0]].line, self.prog.decls[decl_ids[0]].col);
                        self.err_at(l, c, E_GENERAL, format!("未知 bound 接口 `{}`", b));
                    }
                }
            }
            self.type_param_bounds.insert(n, ids);
        }
        (old_bounds, old_names)
    }

    fn check_decl_body(&mut self, id: usize) {
        self.prog.decls[id].checked = true;
        if self.prog.decls[id].owner.is_some() {
            return;
        }
        let old_env = self.enter_type_env(&[id]);
        let (kind, is_ctor_of) = (self.prog.decls[id].kind, self.prog.decls[id].is_ctor);
        match kind {
            DeclKind::Fn => {
                let body = self.prog.decls[id].body.take();
                if let Some(mut b) = body {
                    self.cur_fn = Some(id);
                    self.cur_class = None;
                    let mut ret = self.prog.decls[id].ret.clone();
                    self.resolve_type(&mut ret, id);
                    self.ret_ty = ret;
                    self.scopes.clear();
                    self.scopes.push(Vec::new());
                    for mut p in self.prog.decls[id].params.clone() {
                        self.resolve_type(&mut p.ty, id);
                        if p.ty.has_raw_ptr() && !self.prog.decls[id].is_unsafe {
                            self.err_at(
                                p.line,
                                p.col,
                                7,
                                format!("裸指针参数 `{}` 只能出现在 unsafe fn 中", p.name),
                            );
                        }
                        self.declare_ex(&p.name, p.ty, p.is_const, p.line, p.col);
                    }
                    // unsafe fn 的函数体整体处于 unsafe 域
                    if self.prog.decls[id].is_unsafe {
                        self.unsafe_depth += 1;
                    }
                    self.check_block(&mut b);
                    if self.prog.decls[id].is_unsafe {
                        self.unsafe_depth -= 1;
                    }
                    let ret = self.prog.decls[id].ret.clone();
                    if ret != Type::Void && !block_returns(&b) {
                        let (l, c) = (self.prog.decls[id].line, self.prog.decls[id].col);
                        self.err_at(
                            l,
                            c,
                            E_GENERAL,
                            "非 void 函数在所有路径上都必须 return",
                        );
                    }
                    self.prog.decls[id].body = Some(b);
                    self.cur_fn = None;
                } else if is_ctor_of {
                    // interface 签名：无体，检查参数类型已在解析阶段完成
                }
            }
            DeclKind::Class => {
                // 字段初始化表达式（无 this 环境，安全域内检查）
                let mut fields = self.prog.decls[id].fields.clone();
                for f in fields.iter_mut() {
                    self.resolve_type(&mut f.ty, id);
                }
                self.cur_class = Some(id);
                self.cur_fn = None;
                self.ret_ty = Type::Void;
                self.in_field_init = true;
                for f in fields.iter_mut() {
                    if let Some(init) = f.init.take() {
                        let mut e = init;
                        let declared = f.ty.clone();
                        let got = self.check_expr(&mut e, Some(&declared));
                        if !self.same_type(&got, &declared) {
                            self.err_at(
                                e.line,
                                e.col,
                                12,
                                format!("字段 `{}` 初始化类型不匹配：声明 {}，实际 {}", f.name, declared, got),
                            );
                        }
                        f.init = Some(e);
                    }
                }
                self.in_field_init = false;
                self.prog.decls[id].fields = fields;
            }
            DeclKind::Iface => {}
            DeclKind::Enum => {}
            DeclKind::Alias => {}
            DeclKind::Const => {
                let expr = self.prog.decls[id].const_expr.take();
                if let Some(mut e) = expr {
                    let declared = self.prog.decls[id].ret.clone();
                    if !is_const_literal(&e) {
                        self.err_at(e.line, e.col, E_GENERAL, "顶层 const 初值必须是字面量（可带一元负号）");
                    }
                    let got = self.check_expr(&mut e, Some(&declared));
                    if !self.same_type(&got, &declared) && !matches!(got, Type::Infer) {
                        self.err_at(
                            e.line,
                            e.col,
                            12,
                            format!("常量 `{}` 类型不匹配：声明 {}，实际 {}", self.prog.decls[id].name, declared, got),
                        );
                    }
                    self.prog.decls[id].const_expr = Some(e);
                }
            }
        }

        // 类/接口的成员单独检查（它们是独立 decl）
        self.type_param_bounds = old_env.0;
        self.type_param_names = old_env.1;
    }

    // 类方法体需要所属 class 上下文

    fn check_method_body(&mut self, id: usize, owner: usize) {
        let mut body = match self.prog.decls[id].body.take() {
            Some(b) => b,
            None => return,
        };
        self.cur_fn = Some(id);
        let old_env = self.enter_type_env(&[id, owner]);
        self.cur_class = if self.prog.decls[id].is_static {
            None
        } else {
            Some(owner)
        };
        let is_ctor = self.prog.decls[id].is_ctor;
        self.in_ctor = is_ctor;
        self.const_writes.clear();
        let mut ret = self.prog.decls[id].ret.clone();
        self.resolve_type(&mut ret, id);
        self.ret_ty = ret;
        self.scopes.clear();
        self.scopes.push(Vec::new());
        for mut p in self.prog.decls[id].params.clone() {
            self.resolve_type(&mut p.ty, id);
            if p.ty.has_raw_ptr() && !self.prog.decls[id].is_unsafe {
                self.err_at(
                    p.line,
                    p.col,
                    7,
                    format!("裸指针参数 `{}` 只能出现在 unsafe fn 中", p.name),
                );
            }
            self.declare_ex(&p.name, p.ty, p.is_const, p.line, p.col);
        }
        if self.prog.decls[id].is_unsafe {
            self.unsafe_depth += 1;
        }
        self.check_block(&mut body);
        if self.prog.decls[id].is_unsafe {
            self.unsafe_depth -= 1;
        }
        let ret = self.prog.decls[id].ret.clone();
        if ret != Type::Void && !block_returns(&body) {
            let (l, c) = (self.prog.decls[id].line, self.prog.decls[id].col);
            self.err_at(l, c, E_GENERAL, "非 void 方法在所有路径上都必须 return");
        }
        self.prog.decls[id].body = Some(body);
        // const 字段必须在构造器内被赋值一次（除非声明处已初始化）
        if is_ctor {
            self.in_ctor = false;
            let fields: Vec<(String, bool, bool, u32, u32)> = self.prog.decls[owner]
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.is_const, f.init.is_some(), f.line, f.col))
                .collect();
            let writes = std::mem::take(&mut self.const_writes);
            for (name, is_const, has_init, fl, fc) in fields {
                if !is_const || has_init {
                    continue;
                }
                if !writes.iter().any(|(n, _)| *n == name) {
                    self.err_at(
                        fl,
                        fc,
                        16,
                        format!("const 字段 `{}` 必须在构造器中被赋值", name),
                    );
                }
            }
        }
        self.cur_fn = None;
        self.cur_class = None;
        self.type_param_bounds = old_env.0;
        self.type_param_names = old_env.1;
    }

    fn check_block(&mut self, b: &mut Block) {
        self.push_scope();
        for s in b.stmts.iter_mut() {
            self.check_stmt(s);
        }
        self.pop_scope();
    }

    fn check_stmt(&mut self, s: &mut Stmt) {
        match &mut s.kind {
            StmtKind::Var { ty, name, init, is_const } => {
                let mut declared = ty.clone();
                let at = self.cur_fn.unwrap_or(0);
                self.resolve_type(&mut declared, at);
                *ty = declared.clone();
                if declared.has_raw_ptr() && self.unsafe_depth == 0 {
                    self.err_at(
                        s.line,
                        s.col,
                        7,
                        format!("裸指针变量 `{}` 只能在 unsafe 域内声明", name),
                    );
                }
                match init {
                    None => {
                        self.err_at(
                            s.line,
                            s.col,
                            3,
                            format!("局部变量 `{}` 必须初始化（Mylis 禁止未初始化变量）", name),
                        );
                    }
                    Some(e) => {
                        let got = self.check_expr(e, Some(&declared));
                        if !self.same_type(&got, &declared)
                            && !self.try_iface_coerce(e, &declared)
                            && !matches!(got, Type::Infer)
                        {
                            self.err_at(
                                e.line,
                                e.col,
                                12,
                                format!(
                                    "变量 `{}` 初始化类型不匹配：声明 {}，实际 {}",
                                    name, declared, got
                                ),
                            );
                        }
                    }
                }
                let line = s.line;
                let col = s.col;
                self.declare_ex(name, declared, *is_const, line, col);
            }
            StmtKind::Assign { target, value } => {
                let tt = self.check_expr(target, None);
                // const 绑定：局部变量 / 参数 / 字段
                if let ExprKind::Ident(n) = &target.kind {
                    if self.var_is_const(n) {
                        let (l, c) = (target.line, target.col);
                        self.err_at(
                            l,
                            c,
                            15,
                            format!("不能给 const 变量 `{}` 重新赋值（const 绑定只读）", n),
                        );
                    }
                }
                if let ExprKind::Field(recv, fname) = &target.kind {
                    let is_this = matches!(recv.kind, ExprKind::This);
                    let owner = match &recv.ty {
                        Type::Class(cid) => Some(*cid),
                        Type::Adt { base, .. } => Some(*base),
                        _ => None,
                    };
                    let fname = fname.clone();
                    let owner = if is_this {
                        self.cur_class
                    } else {
                        owner
                    };
                    let is_const_field = owner
                        .and_then(|cid| {
                            self.prog.decls[cid]
                                .fields
                                .iter()
                                .find(|f| f.name == fname)
                                .map(|f| (f.is_const, f.init.is_some(), f.line, f.col))
                        })
                        .filter(|(c, _, _, _)| *c);
                    if let Some((_, has_init, fl, fc)) = is_const_field {
                        let in_own_ctor = self.in_ctor && (is_this || owner == self.cur_class);
                        if !in_own_ctor {
                            let (l, c) = (target.line, target.col);
                            self.err_at(
                                l,
                                c,
                                15,
                                format!("const 字段 `{}` 只能在构造器中赋值", fname),
                            );
                        } else if has_init {
                            let (l, c) = (target.line, target.col);
                            self.err_at(
                                l,
                                c,
                                15,
                                format!("const 字段 `{}` 已在声明处初始化，构造器中不能再赋值", fname),
                            );
                        } else if self.const_writes.iter().any(|(n, _)| *n == fname) {
                            let (l, c) = (target.line, target.col);
                            self.err_at(
                                l,
                                c,
                                15,
                                format!("const 字段 `{}` 在构造器中重复赋值", fname),
                            );
                        } else {
                            self.const_writes.push((fname.clone(), target.line));
                            let _ = (fl, fc);
                        }
                    }
                }
                if !is_place(target) {
                    let (l, c) = (target.line, target.col);
                    self.err_at(l, c, E_GENERAL, "赋值目标必须是变量、字段或指针解引用");
                }
                let vt = self.check_expr(value, Some(&tt));
                if !self.same_type(&vt, &tt) {
                    if self.try_iface_coerce(value, &tt) {
                        // 已插入接口转换
                    } else if !matches!(vt, Type::Infer) {
                        let (l, c) = (value.line, value.col);
                        self.err_at(
                            l,
                            c,
                            12,
                            format!("赋值类型不匹配：目标 {}，实际 {}", tt, vt),
                        );
                    }
                }
            }
            StmtKind::ExprStmt(e) => {
                self.check_expr(e, None);
            }
            StmtKind::If {
                cond,
                then_b,
                else_b,
            } => {
                let ct = self.check_expr(cond, Some(&Type::Bool));
                if ct != Type::Bool {
                    let (l, c) = (cond.line, cond.col);
                    self.err_at(l, c, 12, format!("if 条件必须是 bool，实际是 {}", ct));
                }
                self.check_block(then_b);
                if let Some(eb) = else_b {
                    self.check_stmt(eb);
                }
            }
            StmtKind::While { cond, body } => {
                let ct = self.check_expr(cond, Some(&Type::Bool));
                if ct != Type::Bool {
                    let (l, c) = (cond.line, cond.col);
                    self.err_at(l, c, 12, format!("while 条件必须是 bool，实际是 {}", ct));
                }
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                self.push_scope();
                self.check_stmt(init);
                if let Some(c) = cond {
                    let ct = self.check_expr(c, Some(&Type::Bool));
                    if ct != Type::Bool {
                        let (l, cc) = (c.line, c.col);
                        self.err_at(l, cc, 12, format!("for 条件必须是 bool，实际是 {}", ct));
                    }
                }
                if let Some(st) = step {
                    self.check_stmt(st);
                }
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }
            StmtKind::Return(e) => {
                let ret = self.ret_ty.clone();
                match e {
                    None => {
                        if ret != Type::Void {
                            self.err_at(
                                s.line,
                                s.col,
                                12,
                                format!("函数返回类型是 {}，return 必须带值", ret),
                            );
                        }
                    }
                    Some(ex) => {
                        if ret == Type::Void {
                            self.err_at(s.line, s.col, 12, "void 函数不能 return 值");
                        } else {
                            let got = self.check_expr(ex, Some(&ret));
                            if !self.same_type(&got, &ret) && !self.try_iface_coerce(ex, &ret) && !matches!(got, Type::Infer)
                            {
                                self.err_at(
                                    ex.line,
                                    ex.col,
                                    12,
                                    format!("return 类型不匹配：声明 {}，实际 {}", ret, got),
                                );
                            }
                        }
                    }
                }
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.loop_depth == 0 {
                    self.err_at(s.line, s.col, E_GENERAL, "break/continue 只能用于循环内");
                }
            }
            StmtKind::Unsafe(b) => {
                self.unsafe_depth += 1;
                self.check_block(b);
                self.unsafe_depth -= 1;
            }
            StmtKind::Block(b) => self.check_block(b),
            StmtKind::Match { subject, arms } => {
                self.check_match(subject, arms, s.line, s.col);
            }
        }
    }

    fn check_match(&mut self, subject: &mut Expr, arms: &mut [MatchArm], line: u32, col: u32) {
        let st = self.check_expr(subject, None);
        let (payload_ok, payload_err) = match &st {
            Type::Result(a, b) => (Some((**a).clone()), Some((**b).clone())),
            _ => (None, None),
        };
        let payload_some = match &st {
            Type::Option(a) => Some((**a).clone()),
            _ => None,
        };
        let mut seen_wild = false;
        let mut seen_ok = false;
        let mut seen_err = false;
        let mut seen_some = false;
        let mut seen_none = false;
        let mut seen_true = false;
        let mut seen_false = false;
        let (enum_id, enum_args) = match &st {
            Type::Enum(id) => (Some(*id), Vec::new()),
            Type::Adt { base, args } if self.prog.decls[*base].kind == DeclKind::Enum => {
                (Some(*base), args.clone())
            }
            _ => (None, Vec::new()),
        };
        let mut seen_variants: Vec<usize> = Vec::new();
        for arm in arms.iter_mut() {
            let (al, ac) = (arm.line, arm.col);
            let _ = (al, ac);
            // 模式合法性
            let pat_snap = arm.pat.clone();
            let mut resolved_variant: Option<(usize, usize)> = None;
            match &pat_snap {
                Pattern::Wildcard => seen_wild = true,
                Pattern::Ok(b) => match &payload_ok {
                    Some(t) => {
                        seen_ok = true;
                        self.check_binding(b, t.clone(), arm.line, arm.col);
                    }
                    None => self.err_at(
                        arm.line,
                        arm.col,
                        12,
                        format!("Ok(...) 模式只能用于 Result，实际是 {}", st),
                    ),
                },
                Pattern::Err(b) => match &payload_err {
                    Some(t) => {
                        seen_err = true;
                        self.check_binding(b, t.clone(), arm.line, arm.col);
                    }
                    None => self.err_at(
                        arm.line,
                        arm.col,
                        12,
                        format!("Err(...) 模式只能用于 Result，实际是 {}", st),
                    ),
                },
                Pattern::Some(b) => match &payload_some {
                    Some(t) => {
                        seen_some = true;
                        self.check_binding(b, t.clone(), arm.line, arm.col);
                    }
                    None => self.err_at(
                        arm.line,
                        arm.col,
                        12,
                        format!("Some(...) 模式只能用于 Option，实际是 {}", st),
                    ),
                },
                Pattern::None => {
                    if payload_some.is_none() {
                        self.err_at(arm.line, arm.col, 12, format!("None 模式只能用于 Option，实际是 {}", st));
                    }
                    seen_none = true;
                }
                Pattern::Bool(b) => {
                    if st != Type::Bool {
                        self.err_at(arm.line, arm.col, 12, format!("bool 模式只能用于 bool，实际是 {}", st));
                    }
                    if *b {
                        seen_true = true
                    } else {
                        seen_false = true
                    }
                }
                Pattern::Int(_v) => {
                    if !st.is_integer() && !st.is_float() {
                        self.err_at(arm.line, arm.col, 12, format!("整数字面量模式不能用于 {}", st));
                    }
                }
                Pattern::IntU64(_v) => {
                    if st != Type::U64 {
                        self.err_at(arm.line, arm.col, 12, format!("超出 i64 的整数字面量模式只能用于 u64，实际是 {}", st));
                    }
                }
                Pattern::Float(_) => {
                    if !st.is_float() {
                        self.err_at(arm.line, arm.col, 12, format!("浮点字面量模式只能用于浮点类型，实际是 {}", st));
                    }
                }
                Pattern::Str(_) => {
                    if st != Type::Str {
                        self.err_at(arm.line, arm.col, 12, format!("字符串模式只能用于 string，实际是 {}", st));
                    }
                }
                Pattern::Null => {
                    if !matches!(st, Type::Ptr(_) | Type::Class(_) | Type::Enum(_)) {
                        self.err_at(arm.line, arm.col, 12, "null 模式只能用于指针 / class / enum");
                    }
                }
                Pattern::Variant { ename, name, binds, .. } => match enum_id {
                    Some(eid) => {
                        let real = self.prog.decls[eid].name.clone();
                        let base_name = self.prog.decls[eid]
                            .generic_base
                            .map(|b| self.prog.decls[b].name.clone());
                        let name_ok = *ename == real
                            || base_name.as_deref() == Some(ename.as_str());
                        if !name_ok {
                            self.err_at(
                                arm.line,
                                arm.col,
                                12,
                                format!("变体模式 `{}::{}` 与匹配对象类型 `{}` 不一致", ename, name, real),
                            );
                        }
                        match self.prog.decls[eid]
                            .variants
                            .iter()
                            .position(|v| v.name == *name)
                        {
                            Some(vi) => {
                                let mut cmap = std::collections::HashMap::new();
                                for (pname, a) in self.prog.decls[eid].type_params.iter().zip(enum_args.iter()) {
                                    cmap.insert(pname.clone(), a.clone());
                                }
                                let vfields: Vec<Type> = self.prog.decls[eid].variants[vi].fields
                                    .iter()
                                    .map(|ft| crate::generics::subst_type(ft, &cmap))
                                    .collect();
                                if binds.len() != vfields.len() {
                                    self.err_at(
                                        arm.line,
                                        arm.col,
                                        12,
                                        format!(
                                            "变体 `{}::{}` 需要 {} 个绑定，实际 {} 个",
                                            real,
                                            name,
                                            vfields.len(),
                                            binds.len()
                                        ),
                                    );
                                }
                                for (b, ft) in binds.iter().zip(vfields.iter()) {
                                    self.check_binding(b, ft.clone(), arm.line, arm.col);
                                }
                                resolved_variant = Some((eid, vi));
                                seen_variants.push(vi);
                            }
                            None => {
                                self.err_at(
                                    arm.line,
                                    arm.col,
                                    E_GENERAL,
                                    format!("enum `{}` 没有变体 `{}`", real, name),
                                );
                            }
                        }
                    }
                    None => self.err_at(
                        arm.line,
                        arm.col,
                        12,
                        format!("变体模式只能用于 enum，实际是 {}", st),
                    ),
                },
            }
            if let Some((eid, vi)) = resolved_variant {
                if let Pattern::Variant { enum_id: pid, vidx, .. } = &mut arm.pat {
                    *pid = eid;
                    *vidx = vi;
                }
            }
            // 分支体：拥有独立作用域，绑定变量已声明
            self.push_scope();
            self.check_block_inner(&mut arm.body);
            self.pop_scope();
        }
        // 穷尽性
        let variants_complete = match enum_id {
            Some(eid) => {
                let n = self.prog.decls[eid].variants.len();
                (0..n).all(|i| seen_variants.contains(&i))
            }
            None => false,
        };
        let complete = seen_wild
            || variants_complete
            || (payload_some.is_some() && seen_some && seen_none)
            || (payload_ok.is_some() && seen_ok && seen_err)
            || (st == Type::Bool && seen_true && seen_false);
        if !complete {
            self.err_at(
                line,
                col,
                E_GENERAL,
                format!("match 未穷尽：{} 的所有情况必须被覆盖，或添加 `_ =>` 分支", st),
            );
        }
    }

    /// @deprecated 调用点告警（不阻断编译）
    fn warn_deprecated(&mut self, id: usize, line: u32, col: u32, what: &str) {
        let (name, msg) = match self.prog.decls[id]
            .annots
            .iter()
            .find(|a| a.name == "deprecated")
        {
            Some(a) => {
                let m = a
                    .args
                    .iter()
                    .find_map(|x| match x {
                        AnnotArg::Str(s2) => Some(s2.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                (self.prog.decls[id].name.clone(), m)
            }
            None => return,
        };
        let tail = if msg.is_empty() {
            "：请改用新接口".to_string()
        } else {
            format!("：{}", msg)
        };
        let file = self.cur_file.clone();
        self.diags.warn(
            201,
            &file,
            line,
            col,
            format!("{} `{}` 已废弃{}", what, name, tail),
        );
    }

    fn check_binding(&mut self, bind: &Option<String>, ty: Type, line: u32, col: u32) {
        if let Some(n) = bind {
            self.declare(n, ty, line, col);
        }
    }

    /// check_block 的内部版本：由调用方负责 push/pop（供 match 分支复用）
    fn check_block_inner(&mut self, b: &mut Block) {
        for s in b.stmts.iter_mut() {
            self.check_stmt(s);
        }
    }

    // ---------------------------------------------------------- 表达式
    fn check_expr(&mut self, e: &mut Expr, expected: Option<&Type>) -> Type {
        let mut pending_res = Res::None;
        let mut pending_box: Option<(Type, usize)> = None;
        let ty = match &mut e.kind {
            ExprKind::Int(v) => {
                let t = match expected {
                    Some(t) if t.is_integer() => t.clone(),
                    Some(t) if t.is_float() => t.clone(),
                    _ => Type::I32,
                };
                if let Err(msg) = int_fits(*v, &t) {
                    self.err_at(e.line, e.col, 12, msg);
                }
                t
            }
            ExprKind::IntU64(v) => {
                let t = match expected {
                    Some(t) if t.is_integer() => t.clone(),
                    Some(t) if t.is_float() => t.clone(),
                    _ => Type::U64,
                };
                if let Err(msg) = int_fits_u64(*v, &t) {
                    self.err_at(e.line, e.col, 12, msg);
                }
                t
            }
            ExprKind::Float(_) => match expected {
                Some(t) if t.is_float() => t.clone(),
                _ => Type::F64,
            },
            ExprKind::Str(_) => Type::Str,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Null => Type::Ptr(Box::new(Type::Void)),
            ExprKind::This => {
                let this_ty = self
                    .cur_class
                    .map(|c| {
                        if !self.prog.decls[c].type_params.is_empty() {
                            Type::Adt {
                                base: c,
                                args: self.prog.decls[c]
                                    .type_params
                                    .iter()
                                    .cloned()
                                    .map(Type::Param)
                                    .collect(),
                            }
                        } else {
                            match self.prog.decls[c].kind {
                                DeclKind::Enum => Type::Enum(c),
                                _ => Type::Class(c),
                            }
                        }
                    })
                    .unwrap_or(Type::Infer);
                if self.in_field_init {
                    self.err_at(
                        e.line,
                        e.col,
                        E_GENERAL,
                        "字段初值表达式不能使用 `this`：此时实例字段尚未全部初始化，请把这段逻辑写进 `init`",
                    );
                    return Type::Infer;
                }
                if self.unsafe_depth > 0 {
                    // 无特殊语义
                }
                match self.cur_class {
                    Some(_) => this_ty,
                    None => {
                        self.err_at(e.line, e.col, E_GENERAL, "`this` 只能出现在类/枚举方法内");
                        Type::Infer
                    }
                }
            }
            ExprKind::Ident(name) => match self.lookup_var(name) {
                Some(t) => {
                    e.res = Res::Var;
                    t
                }
                None => {
                    let cname = name.clone();
                    if let Some(cid) = self.lookup_const(&cname) {
                        if let Some(value) = self.prog.decls[cid].const_expr.clone() {
                            let declared = self.prog.decls[cid].ret.clone();
                            *e = (*value).clone();
                            return self.check_expr(e, Some(&declared));
                        }
                    }
                    self.err_at(
                        e.line,
                        e.col,
                        E_GENERAL,
                        format!("未声明的标识符 `{}`（顶层函数需要以调用形式使用）", name),
                    );
                    Type::Infer
                }
            },
            ExprKind::Bin(op, l, r) => {
                let lt = self.check_expr(l, expected);
                let rt_expect = if lt.is_numeric() { Some(lt.clone()) } else { None };
                let rt = self.check_expr(r, rt_expect.as_ref());
                // 字面量适配：`1 + x`
                if is_numeric_literal(l) && rt.is_numeric() && lt.is_numeric() && lt != rt {
                    l.ty = rt.clone();
                }
                self.check_bin(*op, &lt, &rt, e.line, e.col)
            }
            ExprKind::Un(op, x) => {
                let t = self.check_expr(x, expected);
                match op {
                    UnOp::Neg => {
                        if !t.is_numeric() {
                            self.err_at(e.line, e.col, 12, format!("一元 '-' 需要数值类型，实际是 {}", t));
                            Type::Infer
                        } else {
                            t
                        }
                    }
                    UnOp::Not => {
                        if t != Type::Bool {
                            self.err_at(e.line, e.col, 12, format!("'!' 需要 bool，实际是 {}", t));
                        }
                        Type::Bool
                    }
                    UnOp::BitNot => {
                        if !t.is_integer() {
                            self.err_at(e.line, e.col, 12, format!("'~' 需要整数类型，实际是 {}", t));
                            Type::Infer
                        } else {
                            t
                        }
                    }
                    UnOp::Deref => {
                        if self.unsafe_depth == 0 {
                            self.err_at(e.line, e.col, 7, "裸指针解引用只能在 unsafe 域内");
                        }
                        match t {
                            Type::Ptr(inner) => *inner,
                            _ => {
                                self.err_at(e.line, e.col, 12, format!("'*' 需要指针类型，实际是 {}", t));
                                Type::Infer
                            }
                        }
                    }
                    UnOp::AddrOf => {
                        if self.unsafe_depth == 0 {
                            self.err_at(e.line, e.col, 7, "取地址只能在 unsafe 域内");
                        }
                        if !is_place(x) {
                            self.err_at(e.line, e.col, 12, "取地址的目标必须是变量或字段");
                        }
                        Type::ptr(t)
                    }
                }
            }
            ExprKind::Call(name, targs, args) => {
                match self.lookup_fn(name) {
                    Some(fid) => {
                        if !self.prog.decls[fid].type_params.is_empty() {
                            let mut ra = if targs.is_empty() {
                                let ret = self.prog.decls[fid].ret.clone();
                                let params = self.prog.decls[fid].params.clone();
                                match self.infer_targs_from_parts(fid, &ret, &params, expected, args) {
                                    Some(a) => a,
                                    None => {
                                        self.err_at(e.line, e.col, E_GENERAL, format!("无法推断函数 `{}` 的类型实参，请显式写成 `{}<...>(...)`", name, name));
                                        return Type::Infer;
                                    }
                                }
                            } else {
                                let mut a = targs.clone();
                                for x in a.iter_mut() { self.resolve_type(x, fid); }
                                a
                            };
                            if ra.len() != self.prog.decls[fid].type_params.len() {
                                self.err_at(e.line, e.col, E_GENERAL, format!("函数 `{}` 类型实参数量不匹配", name));
                                return Type::Infer;
                            }
                            // 只做签名替换和类型检查；具体函数在单态化阶段生成。
                            let mut m = self.prog.decls[fid].clone();
                            let mut fmap = std::collections::HashMap::new();
                            for (p, a) in self.prog.decls[fid].type_params.iter().zip(ra.iter()) {
                                fmap.insert(p.clone(), a.clone());
                            }
                            crate::generics::subst_decl(&mut m, &fmap);
                            self.check_args(&format!("函数 `{}`", name), &m.params, args, e.line, e.col);
                            e.res = Res::AdtFn { base: fid, args: ra.clone() };
                            let ret = m.ret.clone();
                            e.ty = ret.clone();
                            self.prog.note_inst(&ret);
                            return ret;
                        }
                        if !targs.is_empty() {
                            self.err_at(e.line, e.col, E_GENERAL, format!("函数 `{}` 不是泛型函数，不能带类型实参", name));
                        }
                        if self.prog.decls[fid].mod_id != self.cur_mod && !self.prog.decls[fid].is_pub {
                            self.err_at(
                                e.line,
                                e.col,
                                5,
                                format!("函数 `{}` 不是 pub，跨文件不可见", name),
                            );
                        }
                        if self.prog.decls[fid].is_unsafe && self.unsafe_depth == 0 {
                            self.err_at(
                                e.line,
                                e.col,
                                7,
                                format!("调用 unsafe fn `{}` 必须位于 unsafe 域内", name),
                            );
                        }
                        let params = self.prog.decls[fid].params.clone();
                        self.check_args(&format!("函数 `{}`", name), &params, args, e.line, e.col);
                        self.warn_deprecated(fid, e.line, e.col, "函数");
                        e.res = Res::Fn(fid);
                        let ret = self.prog.decls[fid].ret.clone();
                        self.prog.note_inst(&ret);
                        ret
                    }
                    None => {
                        self.err_at(e.line, e.col, E_GENERAL, format!("未声明的函数 `{}`", name));
                        Type::Infer
                    }
                }
            }
            ExprKind::Method(recv, mname, targs, args) => {
                let rt = self.check_expr(recv, None);
                let (rty, rres) = self.check_method_call(recv, &rt, mname, targs, expected, args, e.line, e.col);
                if !matches!(rres, Res::None) {
                    pending_res = rres;
                }
                rty
            }
            ExprKind::Field(recv, fname) => {
                let rt = self.check_expr(recv, None);
                if let Type::Enum(eid) = &rt {
                    let en = self.prog.decls[*eid].name.clone();
                    self.err_at(
                        e.line,
                        e.col,
                        E_GENERAL,
                        format!(
                            "enum `{}` 没有字段 `{}`：变体载荷只能通过 `match` 解构",
                            en, fname
                        ),
                    );
                    return Type::Infer;
                }
                match rt {
                    Type::Class(cid) => match find_class_field(&self.prog.decls[cid], fname) {
                        Some(idx) => {
                            if self.prog.decls[cid].mod_id != self.cur_mod
                                && !self.prog.decls[cid].fields[idx].is_pub
                            {
                                self.err_at(
                                    e.line,
                                    e.col,
                                    5,
                                    format!("字段 `{}` 不是 pub，跨文件不可见", fname),
                                );
                            }
                            e.res = Res::Field(cid, idx);
                            let ft = self.prog.decls[cid].fields[idx].ty.clone();
                            self.prog.note_inst(&ft);
                            ft
                        }
                        None => {
                            self.err_at(
                                e.line,
                                e.col,
                                E_GENERAL,
                                format!("类型 {} 没有字段 `{}`", self.type_name(&Type::Class(cid)), fname),
                            );
                            Type::Infer
                        }
                    },
                    Type::Adt { base, args } => {
                        let base = base;
                        match find_class_field(&self.prog.decls[base], fname) {
                            Some(idx) => {
                                if self.prog.decls[base].mod_id != self.cur_mod
                                    && !self.prog.decls[base].fields[idx].is_pub
                                {
                                    self.err_at(
                                        e.line,
                                        e.col,
                                        5,
                                        format!("字段 `{}` 不是 pub，跨文件不可见", fname),
                                    );
                                }
                                e.res = Res::AdtField { base, field: idx, class_args: args.clone() };
                                let mut cmap = std::collections::HashMap::new();
                                for (pname, a) in self.prog.decls[base].type_params.iter().zip(args.iter()) {
                                    cmap.insert(pname.clone(), a.clone());
                                }
                                let ft = crate::generics::subst_type(&self.prog.decls[base].fields[idx].ty, &cmap);
                                self.prog.note_inst(&ft);
                                ft
                            }
                            None => {
                                self.err_at(
                                    e.line,
                                    e.col,
                                    E_GENERAL,
                                    format!("类型 {} 没有字段 `{}`", self.type_name(&Type::Adt { base, args: args.clone() }), fname),
                                );
                                Type::Infer
                            }
                        }
                    }
                    Type::Iface(_) => {
                        self.err_at(e.line, e.col, E_GENERAL, "interface 值不能直接访问字段");
                        Type::Infer
                    }
                    other => {
                        self.err_at(e.line, e.col, 12, format!("{} 没有字段 `{}`", other, fname));
                        Type::Infer
                    }
                }
            }
            ExprKind::New(cname, targs, args) => {
                let cid;
                let mut class_args: Vec<Type> = Vec::new();
                let base_id = match self.lookup_type(cname) {
                    Some(id) if self.prog.decls[id].kind == DeclKind::Class => {
                        if !self.prog.decls[id].type_params.is_empty() {
                            let mut ra = if targs.is_empty() {
                                if let Some(Type::Adt { base: eb, args: ea }) = expected {
                                    if *eb == id { ea.clone() } else {
                                        let ctor = self.prog.decls[id].members.iter().copied().find(|m| self.prog.decls[*m].is_ctor);
                                        let (ret, params) = if let Some(mid) = ctor {
                                            (self.prog.decls[mid].ret.clone(), self.prog.decls[mid].params.clone())
                                        } else { (Type::Void, Vec::new()) };
                                        match self.infer_targs_from_parts(id, &ret, &params, None, args) {
                                            Some(a) => a,
                                            None => {
                                                self.err_at(e.line, e.col, E_GENERAL, format!("无法推断 `new {}` 的类型实参，请显式写成 `new {}<...>(...)`", cname, cname));
                                                return Type::Infer;
                                            }
                                        }
                                    }
                                } else if let Some(Type::Class(ecid)) = expected {
                                    if let Some(a) = self.generic_args_for_concrete(id, *ecid) {
                                        a
                                    } else {
                                        let ctor = self.prog.decls[id].members.iter().copied().find(|m| self.prog.decls[*m].is_ctor);
                                        let (ret, params) = if let Some(mid) = ctor {
                                            (self.prog.decls[mid].ret.clone(), self.prog.decls[mid].params.clone())
                                        } else { (Type::Void, Vec::new()) };
                                        match self.infer_targs_from_parts(id, &ret, &params, expected, args) {
                                            Some(a) => a,
                                            None => {
                                                self.err_at(e.line, e.col, E_GENERAL, format!("无法推断 `new {}` 的类型实参，请显式写成 `new {}<...>(...)`", cname, cname));
                                                return Type::Infer;
                                            }
                                        }
                                    }
                                } else {
                                    let ctor = self.prog.decls[id].members.iter().copied().find(|m| self.prog.decls[*m].is_ctor);
                                    let (ret, params) = if let Some(mid) = ctor {
                                        (self.prog.decls[mid].ret.clone(), self.prog.decls[mid].params.clone())
                                    } else { (Type::Void, Vec::new()) };
                                    match self.infer_targs_from_parts(id, &ret, &params, None, args) {
                                        Some(a) => a,
                                        None => {
                                            self.err_at(e.line, e.col, E_GENERAL, format!("无法推断 `new {}` 的类型实参，请显式写成 `new {}<...>(...)`", cname, cname));
                                            return Type::Infer;
                                        }
                                    }
                                }
                            } else {
                                let mut a = targs.clone();
                                for x in a.iter_mut() { self.resolve_type(x, id); }
                                a
                            };
                            if ra.len() != self.prog.decls[id].type_params.len() {
                                self.err_at(e.line, e.col, E_GENERAL, format!("class `{}` 类型实参数量不匹配", cname));
                                return Type::Infer;
                            }
                            // 类型检查只返回规范的 Adt；具体 Decl 推迟到单态化。
                            class_args = ra;
                            id
                        } else {
                            if !targs.is_empty() {
                                self.err_at(e.line, e.col, E_GENERAL, format!("`{}` 不是泛型 class，不能带类型实参", cname));
                            }
                            class_args = Vec::new();
                            id
                        }
                    }
                    Some(id) if self.prog.decls[id].kind == DeclKind::Enum => {
                        self.err_at(
                            e.line,
                            e.col,
                            E_GENERAL,
                            format!(
                                "enum `{}` 不能用 new 实例化，请用 `{}::变体名(...)` 构造",
                                cname, cname
                            ),
                        );
                        return Type::Infer;
                    }
                    Some(_) => {
                        self.err_at(e.line, e.col, 12, format!("`{}` 是 interface，不能用 new 实例化", cname));
                        return Type::Infer;
                    }
                    None => {
                        self.err_at(e.line, e.col, E_GENERAL, format!("未知 class `{}`", cname));
                        return Type::Infer;
                    }
                };
                // 泛型类的 cid 是模板 id；非泛型才是具体 Class。
                cid = base_id;
                if self.prog.decls[base_id].mod_id != self.cur_mod && !self.prog.decls[base_id].is_pub {
                    self.err_at(e.line, e.col, 5, format!("class `{}` 不是 pub，跨文件不可见", cname));
                }
                let ctor = self.prog.decls[cid]
                    .members
                    .iter()
                    .copied()
                    .find(|m| self.prog.decls[*m].is_ctor);
                match ctor {
                    Some(mid) => {
                        let mut m = self.prog.decls[mid].clone();
                        if !class_args.is_empty() {
                            let base = self.prog.decls[base_id].type_params.clone();
                            let mut cmap = std::collections::HashMap::new();
                            for (p, a) in base.iter().zip(class_args.iter()) {
                                cmap.insert(p.clone(), a.clone());
                            }
                            crate::generics::subst_decl(&mut m, &cmap);
                        }
                        self.check_args(&format!("`new {}`", cname), &m.params, args, e.line, e.col);
                    }
                    None => {
                        if !args.is_empty() {
                            self.err_at(
                                e.line,
                                e.col,
                                12,
                                format!("class `{}` 没有 init，new 不能带参数", cname),
                            );
                        }
                    }
                }
                self.warn_deprecated(base_id, e.line, e.col, "类型");
                if !class_args.is_empty() {
                    e.res = Res::AdtCtor { base: base_id, args: class_args.clone() };
                    Type::Adt { base: base_id, args: class_args }
                } else {
                    e.res = Res::Ctor(base_id);
                    Type::Class(base_id)
                }
            }
            ExprKind::Ok(inner) => match expected {
                Some(Type::Result(ok, _)) => {
                    let ok = (**ok).clone();
                    let got = self.check_expr(inner, Some(&ok));
                    if !self.same_type(&got, &ok) && self.try_iface_coerce(inner, &ok) {
                        // 已插入 class/enum → interface 或装箱转换
                    } else if !self.same_type(&got, &ok) && !matches!(got, Type::Infer) {
                        self.err_at(
                            inner.line,
                            inner.col,
                            12,
                            format!("Ok(...) 载荷类型不匹配：期望 {}，实际 {}", ok, got),
                        );
                    }
                    expected.unwrap().clone()
                }
                _ => {
                    self.err_at(e.line, e.col, E_GENERAL, "无法确定 Ok(...) 的 Result 类型，请在声明或返回处写明");
                    Type::Infer
                }
            },
            ExprKind::Err(inner) => match expected {
                Some(Type::Result(_, er)) => {
                    let er = (**er).clone();
                    let got = self.check_expr(inner, Some(&er));
                    if !self.same_type(&got, &er) && self.try_iface_coerce(inner, &er) {
                        // 已插入 class/enum → interface 或装箱转换
                    } else if !self.same_type(&got, &er) && !matches!(got, Type::Infer) {
                        self.err_at(
                            inner.line,
                            inner.col,
                            12,
                            format!("Err(...) 载荷类型不匹配：期望 {}，实际 {}", er, got),
                        );
                    }
                    expected.unwrap().clone()
                }
                _ => {
                    self.err_at(e.line, e.col, E_GENERAL, "无法确定 Err(...) 的 Result 类型，请在声明或返回处写明");
                    Type::Infer
                }
            },
            ExprKind::Some_(inner) => match expected {
                Some(Type::Option(t)) => {
                    let t = (**t).clone();
                    let got = self.check_expr(inner, Some(&t));
                    if !self.same_type(&got, &t) && self.try_iface_coerce(inner, &t) {
                        // 已插入 class/enum → interface 或装箱转换
                    } else if !self.same_type(&got, &t) && !matches!(got, Type::Infer) {
                        self.err_at(
                            inner.line,
                            inner.col,
                            12,
                            format!("Some(...) 载荷类型不匹配：期望 {}，实际 {}", t, got),
                        );
                    }
                    expected.unwrap().clone()
                }
                _ => {
                    self.err_at(e.line, e.col, E_GENERAL, "无法确定 Some(...) 的 Option 类型");
                    Type::Infer
                }
            },
            ExprKind::None_ => match expected {
                Some(Type::Option(_)) => expected.unwrap().clone(),
                _ => {
                    self.err_at(e.line, e.col, E_GENERAL, "无法确定 None 的 Option 类型");
                    Type::Infer
                }
            },
            ExprKind::Cast(inner, target, checked) => {
                let src = self.check_expr(inner, None);
                let mut target_ty = target.clone();
                self.resolve_type(&mut target_ty, self.cur_fn.unwrap_or(0));
                *target = target_ty.clone();
                let target = target_ty;
                let checked = *checked;
                if checked {
                    if !matches!(src, Type::Iface(_)) {
                        self.err_at(e.line, e.col, 12, "`as?` 只能用于接口值");
                    }
                    if matches!(target, Type::Ptr(_) | Type::Void) {
                        self.err_at(e.line, e.col, 12, format!("`as?` 不支持目标类型 {}", target));
                    }
                    Type::Option(Box::new(target))
                } else {
                    if let Type::Iface(iid) = &target {
                        if boxable(&src) && self.empty_iface(*iid) {
                            pending_box = Some((src.clone(), *iid));
                            target.clone()
                        } else {
                            let (rty, rres) = self.check_cast(&src, &target, e.line, e.col);
                            if !matches!(rres, Res::None) { pending_res = rres; }
                            rty
                        }
                    } else {
                        let (rty, rres) = self.check_cast(&src, &target, e.line, e.col);
                        if !matches!(rres, Res::None) { pending_res = rres; }
                        rty
                    }
                }
            }
            ExprKind::Try(inner) => {
                let src = self.check_expr(inner, None);
                let ret = self.ret_ty.clone();
                match (&ret, &src) {
                    (Type::Result(_, rerr), Type::Result(ok, serr)) => {
                        if **rerr != **serr {
                            self.err_at(
                                e.line,
                                e.col,
                                6,
                                format!(
                                    "`?` 的 Err 类型必须与函数返回一致：函数 {}，表达式 {}",
                                    rerr, serr
                                ),
                            );
                        }
                        e.res = Res::None;
                        (**ok).clone()
                    }
                    (Type::Option(rok), Type::Option(sok)) => {
                        if **rok != **sok {
                            self.err_at(
                                e.line,
                                e.col,
                                6,
                                format!(
                                    "`?` 的 Option 载荷类型必须与函数返回一致：函数 {}，表达式 {}",
                                    rok, sok
                                ),
                            );
                        }
                        (**rok).clone()
                    }
                    _ => {
                        self.err_at(
                            e.line,
                            e.col,
                            6,
                            format!(
                                "`?` 只能在返回 Result 的函数内用于 Result 值，或在返回 Option 的函数内用于 Option 值（当前函数返回 {}，表达式是 {}）",
                                ret, src
                            ),
                        );
                        Type::Infer
                    }
                }
            }
            ExprKind::Index(recv, idx) => {
                let rt = self.check_expr(recv, None);
                let it = self.check_expr(idx, Some(&Type::I64));
                if !it.is_integer() {
                    self.err_at(idx.line, idx.col, 12, format!("下标必须是整数，实际是 {}", it));
                }
                match rt {
                    Type::Array(elem) => {
                        let et = (*elem).clone();
                        self.prog.note_inst(&et);
                        et
                    }
                    other => {
                        self.err_at(
                            e.line,
                            e.col,
                            12,
                            format!("下标只能用于数组，{} 不是数组", other),
                        );
                        Type::Infer
                    }
                }
            }
            ExprKind::ArrayLit(items) => {
                let elem = match expected {
                    Some(Type::Array(el)) => (**el).clone(),
                    Some(other) => {
                        self.err_at(
                            e.line,
                            e.col,
                            12,
                            format!("数组字面量期望 {}，不是数组类型", other),
                        );
                        if items.is_empty() {
                            Type::Infer
                        } else {
                            self.check_expr(&mut items[0], None)
                        }
                    }
                    None => {
                        if items.is_empty() {
                            self.err_at(
                                e.line,
                                e.col,
                                E_GENERAL,
                                "空数组字面量 `[]` 需要由变量的声明类型确定元素类型，例如 `i32[] a = [];`",
                            );
                            Type::Infer
                        } else {
                            let t = self.check_expr(&mut items[0], None);
                            if matches!(t, Type::Infer) {
                                Type::Infer
                            } else {
                                t
                            }
                        }
                    }
                };
                if !matches!(elem, Type::Infer) {
                    for it in items.iter_mut() {
                        let got = self.check_expr(it, Some(&elem));
                        if !self.same_type(&got, &elem) && !self.try_iface_coerce(it, &elem) && !matches!(got, Type::Infer) {
                            self.err_at(
                                it.line,
                                it.col,
                                12,
                                format!("数组元素类型不匹配：期望 {}，实际 {}", elem, got),
                            );
                        }
                    }
                    Type::array(elem)
                } else {
                    Type::Infer
                }
            }
            ExprKind::ArrayNew(elem_ty, len) => {
                let mut el = elem_ty.clone();
                let at = self.cur_fn.unwrap_or(0);
                self.resolve_type(&mut el, at);
                *elem_ty = el.clone();
                let lt = self.check_expr(len, Some(&Type::U64));
                if !lt.is_integer() {
                    self.err_at(len.line, len.col, 12, format!("数组长度必须是整数，实际是 {}", lt));
                }
                Type::array(el)
            }
            ExprKind::Interp(parts) => {
                let tid = self.prog.tostring_iface;
                for part in parts.iter_mut() {
                    if let InterpPart::Expr(inner) = part {
                        self.check_expr(inner, None);
                        self.check_interp_piece(inner, tid);
                    }
                }
                Type::Str
            }
            ExprKind::StaticPath {
                class,
                name,
                targs,
                args,
                call,
            } => {
                // `A::ADD(...)`：当 A 是可见包/模块而不是 class/enum 时，按包函数调用解析。
                let qualified = format!("{}::{}", class, name);
                if let Some(cid) = self.lookup_const(&qualified) {
                    if *call {
                        self.err_at(e.line, e.col, E_GENERAL, format!("常量 `{}` 不能调用", qualified));
                        return Type::Infer;
                    }
                    if let Some(value) = self.prog.decls[cid].const_expr.clone() {
                        let declared = self.prog.decls[cid].ret.clone();
                        *e = (*value).clone();
                        return self.check_expr(e, Some(&declared));
                    }
                }
                if let Some(fid) = self.lookup_fn(&qualified) {
                    if !*call {
                        self.err_at(e.line, e.col, E_GENERAL, format!("包函数 `{}` 需要调用", qualified));
                        return Type::Infer;
                    }
                    if self.prog.decls[fid].mod_id != self.cur_mod && !self.prog.decls[fid].is_pub {
                        self.err_at(e.line, e.col, 5, format!("函数 `{}` 不是 pub，跨文件不可见", qualified));
                    }
                    if self.prog.decls[fid].is_unsafe && self.unsafe_depth == 0 {
                        self.err_at(e.line, e.col, 7, format!("调用 unsafe fn `{}` 必须位于 unsafe 域内", qualified));
                    }
                    let params = self.prog.decls[fid].params.clone();
                    self.check_args(&format!("函数 `{}`", qualified), &params, args, e.line, e.col);
                    self.warn_deprecated(fid, e.line, e.col, "函数");
                    e.res = Res::Fn(fid);
                    let mut ret = self.prog.decls[fid].ret.clone();

                    if let Type::Named(n) = ret.clone() {
                        if let Some(tid) = self.lookup_type(&n) {
                            ret = match self.prog.decls[tid].kind {
                                DeclKind::Class => Type::Class(tid),
                                DeclKind::Enum => Type::Enum(tid),
                                DeclKind::Alias => ret,
                                _ => ret,
                            };
                        }
                    }
                    self.prog.note_inst(&ret);
                    e.ty = ret.clone();
                    return ret;
                }
                let mut class_args: Vec<Type> = Vec::new();
                let mut cid = match self.lookup_type(class) {
                    Some(id)
                        if matches!(
                            self.prog.decls[id].kind,
                            DeclKind::Class | DeclKind::Enum
                        ) =>
                    {
                        id
                    }
                    Some(_) => {
                        self.err_at(
                            e.line,
                            e.col,
                            12,
                            format!("`{}` 不是 class / enum，不能使用 ::", class),
                        );
                        return Type::Infer;
                    }
                    None => {
                        self.err_at(e.line, e.col, E_GENERAL, format!("未知类型 `{}`", class));
                        return Type::Infer;
                    }
                };
                if self.prog.decls[cid].type_params.is_empty() {
                    if !targs.is_empty() {
                        self.err_at(e.line, e.col, E_GENERAL, format!("`{}` 不是泛型类型，不能带类型实参", class));
                    }
                } else {
                    let mut ra = if targs.is_empty() {
                        match expected {
                            Some(Type::Adt { base: eb, args: ea }) if *eb == cid => ea.clone(),
                            Some(Type::Enum(ecid)) => self.generic_args_for_concrete(cid, *ecid).unwrap_or_default(),
                            _ => Vec::new(),
                        }
                    } else {
                        targs.clone()
                    };
                    if ra.is_empty() && targs.is_empty() {
                        let vi = self.prog.decls[cid].variants.iter().position(|v| v.name == *name);
                        let mut map = std::collections::HashMap::new();
                        if let Some(vi) = vi {
                            let fields = self.prog.decls[cid].variants[vi].fields.clone();
                            for (ft, a) in fields.iter().zip(args.iter()) {
                                if let Some(h) = self.expr_type_hint(a) {
                                    self.bind_type_param(ft, &h, &self.prog.decls[cid].type_params.clone(), &mut map);
                                }
                            }
                        }
                        let names = self.prog.decls[cid].type_params.clone();
                        for n in names {
                            if let Some(t) = map.get(&n) { ra.push(t.clone()); }
                        }
                    }
                    if ra.is_empty() {
                        self.err_at(e.line, e.col, E_GENERAL, format!("泛型类型 `{}` 需要显式类型实参，例如 `{}<i64>::{}`", class, class, name));
                        return Type::Infer;
                    }
                    for a in ra.iter_mut() { self.resolve_type(a, cid); }
                    if ra.len() != self.prog.decls[cid].type_params.len() {
                        self.err_at(e.line, e.col, E_GENERAL, format!("`{}` 需要 {} 个类型实参，实际 {}", class, self.prog.decls[cid].type_params.len(), ra.len()));
                        return Type::Infer;
                    }
                    // 只记录规范化的类实参；具体 Decl 推迟到单态化。
                    class_args = ra;
                }

                let is_enum = self.prog.decls[cid].kind == DeclKind::Enum;
                if is_enum {
                    // 变体构造：E::Variant(...) / E::Variant
                    if let Some(vi) = self.prog.decls[cid]
                        .variants
                        .iter()
                        .position(|v| v.name == *name)
                    {
                        let mut cmap = std::collections::HashMap::new();
                        for (pname, a) in self.prog.decls[cid].type_params.iter().zip(class_args.iter()) {
                            cmap.insert(pname.clone(), a.clone());
                        }
                        let vfields: Vec<Type> = self.prog.decls[cid].variants[vi]
                            .fields
                            .iter()
                            .map(|ft| crate::generics::subst_type(ft, &cmap))
                            .collect();
                        let vname = self.prog.decls[cid].variants[vi].name.clone();
                        if !*call && !vfields.is_empty() {
                            self.err_at(
                                e.line,
                                e.col,
                                12,
                                format!("变体 `{}::{}` 需要 {} 个参数", class, vname, vfields.len()),
                            );
                            return Type::Infer;
                        }
                        if args.len() != vfields.len() {
                            self.err_at(
                                e.line,
                                e.col,
                                12,
                                format!(
                                    "变体 `{}::{}` 需要 {} 个参数，实际 {} 个",
                                    class,
                                    vname,
                                    vfields.len(),
                                    args.len()
                                ),
                            );
                        }
                        for (i, (a, ft)) in args.iter_mut().zip(vfields.iter()).enumerate() {
                            let got = self.check_expr(a, Some(ft));
                            if !self.same_type(&got, ft) {
                                self.err_at(
                                    a.line,
                                    a.col,
                                12,
                                    format!("变体 `{}::{}` 第 {} 个参数类型不匹配：期望 {}，实际 {}", class, vname, i + 1, ft, got),
                                );
                            }
                        }
                        if self.prog.decls[cid].mod_id != self.cur_mod
                            && !self.prog.decls[cid].is_pub
                        {
                            self.err_at(
                                e.line,
                                e.col,
                                5,
                                format!("enum `{}` 不是 pub，跨文件不可见", class),
                            );
                        }
                        if class_args.is_empty() {
                            e.res = Res::Variant(cid, vi);
                            Type::Enum(cid)
                        } else {
                            e.res = Res::AdtVariant { base: cid, variant: vi, args: class_args.clone() };
                            Type::Adt { base: cid, args: class_args.clone() }
                        }
                    } else if *call {
                        // enum 的静态方法（如 E::unit()）
                        let found = self.prog.decls[cid]
                            .members
                            .iter()
                            .copied()
                            .find(|m| {
                                self.prog.decls[*m].name == *name
                                    && self.prog.decls[*m].is_static
                            });
                        match found {
                            Some(mid) => {
                                let (ret, res) = self.check_static_method_signature(
                                    cid,
                                    &class_args,
                                    mid,
                                    targs,
                                    expected,
                                    args,
                                    e.line,
                                    e.col,
                                );
                                self.warn_deprecated(mid, e.line, e.col, "静态方法");
                                e.res = res;
                                self.prog.note_inst(&ret);
                                ret
                            }
                            None => {
                                self.err_at(
                                    e.line,
                                    e.col,
                                    E_GENERAL,
                                    format!("enum `{}` 没有变体或静态方法 `{}`", class, name),
                                );
                                Type::Infer
                            }
                        }
                    } else {
                        self.err_at(
                            e.line,
                            e.col,
                            E_GENERAL,
                            format!("enum `{}` 没有变体 `{}`", class, name),
                        );
                        Type::Infer
                    }
                } else {
                if *call {
                    let found = self.prog.decls[cid]
                        .members
                        .iter()
                        .copied()
                        .find(|m| self.prog.decls[*m].name == *name && self.prog.decls[*m].is_static);
                    match found {
                        Some(mid) => {
                            if self.prog.decls[mid].mod_id != self.cur_mod && !self.prog.decls[mid].is_pub {
                                self.err_at(e.line, e.col, 5, format!("静态方法 `{}` 不是 pub，跨文件不可见", name));
                            }
                            if self.prog.decls[mid].is_unsafe && self.unsafe_depth == 0 {
                                self.err_at(
                                    e.line,
                                    e.col,
                                    7,
                                    format!("调用 unsafe 静态方法 `{}` 必须位于 unsafe 域内", name),
                                );
                            }
                            let (ret, res) = self.check_static_method_signature(
                                cid,
                                &class_args,
                                mid,
                                targs,
                                expected,
                                args,
                                e.line,
                                e.col,
                            );
                            self.warn_deprecated(mid, e.line, e.col, "静态方法");
                            e.res = res;
                            self.prog.note_inst(&ret);
                            ret
                        }
                        None => {
                            self.err_at(
                                e.line,
                                e.col,
                                E_GENERAL,
                                format!("class `{}` 没有静态方法 `{}`", class, name),
                            );
                            Type::Infer
                        }
                    }
                } else {
                    let idx = self.prog.decls[cid]
                        .fields
                        .iter()
                        .position(|f| f.name == *name && f.is_static);
                    match idx {
                        Some(i) => {
                            if self.prog.decls[cid].mod_id != self.cur_mod
                                && !self.prog.decls[cid].fields[i].is_pub
                            {
                                self.err_at(e.line, e.col, 5, format!("静态字段 `{}` 不是 pub，跨文件不可见", name));
                            }
                            let mut cmap = std::collections::HashMap::new();
                            for (pname, a) in self.prog.decls[cid].type_params.iter().zip(class_args.iter()) {
                                cmap.insert(pname.clone(), a.clone());
                            }
                            let ft = crate::generics::subst_type(&self.prog.decls[cid].fields[i].ty, &cmap);
                            if class_args.is_empty() {
                                e.res = Res::StaticField(cid, i);
                            } else {
                                e.res = Res::AdtStaticField { base: cid, field: i, class_args: class_args.clone() };
                            }
                            self.prog.note_inst(&ft);
                            ft
                        }
                        None => {
                            self.err_at(
                                e.line,
                                e.col,
                                E_GENERAL,
                                format!("class `{}` 没有静态字段 `{}`", class, name),
                            );
                            Type::Infer
                        }
                    }
                }
                }
            }
        };
        if !matches!(pending_res, Res::None) {
            e.res = pending_res;
        }
        if let Some(b) = pending_box {
            e.box_to = Some(b);
        }
        e.ty = ty.clone();
        if !matches!(ty, Type::Infer) {
            self.prog.note_inst(&ty);
        }
        ty
    }

    /// 检查一个插值片段能否转成字符串；class 需要自动转换为 ToString 接口
    fn check_interp_piece(&mut self, e: &mut Expr, tid: Option<usize>) {
        let t = e.ty.clone();
        if t.is_numeric() || t == Type::Bool || t == Type::Str {
            return;
        }
        if let (Type::Named(n), Some(want)) = (&t, tid) {
            if self.type_param_bounds.get(n).map(|v| v.contains(&want)).unwrap_or(false) {
                return;
            }
        }
        if !self.formattable(&t, e.line, e.col, tid) {
            return;
        }
        let type_id = match t {
            Type::Class(cid) | Type::Enum(cid) => Some(cid),
            _ => None,
        };
        if let (Some(cid), Some(iid)) = (type_id, tid) {
            if self.iface_compatible(cid, iid, e.line, e.col) {
                self.prog.note_impl(cid, iid);
                // 保留 class/enum 的原始类型，让格式化阶段直接调用它的
                // toString/to_string；不要把它改写成 interface cast，否则
                // 字段访问的 Res::Field 会被覆盖，MIR-C 无法生成字段指令。
            }
        }
    }

    /// 该类型是否可被格式化（数组按元素递归判定）
    fn formattable(&mut self, t: &Type, line: u32, col: u32, tid: Option<usize>) -> bool {
        match t {
            Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::F32
            | Type::F64
            | Type::Bool
            | Type::Str => true,
            Type::Class(cid) => {
                let cid = *cid;
                let has = self.prog.decls[cid]
                    .members
                    .iter()
                    .copied()
                    .any(|m| {
                        matches!(self.prog.decls[m].name.as_str(), "toString" | "to_string")
                            && self.prog.decls[m].params.is_empty()
                            && self.prog.decls[m].ret == Type::Str
                    });
                if has {
                    true
                } else {
                    self.err_at(
                        line,
                        col,
                        E_GENERAL,
                        format!(
                            "无法插入 class `{}`：需要实现 ToString 接口（定义 `pub fn toString() -> string`）",
                            self.prog.decls[cid].name
                        ),
                    );
                    false
                }
            }
            Type::Enum(eid) => {
                let eid = *eid;
                let has = self.prog.decls[eid].members.iter().copied().any(|m| {
                    matches!(self.prog.decls[m].name.as_str(), "toString" | "to_string")
                        && self.prog.decls[m].params.is_empty()
                        && self.prog.decls[m].ret == Type::Str
                });
                if has {
                    true
                } else {
                    self.err_at(
                        line,
                        col,
                        E_GENERAL,
                        format!(
                            "无法插入 enum `{}`：需要定义 `pub fn toString() -> string`",
                            self.prog.decls[eid].name
                        ),
                    );
                    false
                }
            }
            Type::Iface(iid) => {
                let iid = *iid;
                let has = self.prog.decls[iid]
                    .members
                    .iter()
                    .copied()
                    .any(|m| matches!(self.prog.decls[m].name.as_str(), "toString" | "to_string"));
                if has {
                    true
                } else {
                    self.err_at(
                        line,
                        col,
                        E_GENERAL,
                        format!(
                            "无法插入 interface `{}`：它没有声明 `fn toString() -> string`",
                            self.prog.decls[iid].name
                        ),
                    );
                    false
                }
            }
            Type::Array(elem) => {
                let el = (**elem).clone();
                self.formattable(&el, line, col, tid)
            }
            other => {
                self.err_at(
                    line,
                    col,
                    E_GENERAL,
                    format!("类型 {} 不能插入字符串（仅数值/bool/string/ToString/数组）", other),
                );
                false
            }
        }
    }

    fn type_name(&self, t: &Type) -> String {
        match t {
            Type::Class(id) => format!("class `{}`", self.prog.decls[*id].name),
            Type::Iface(id) => format!("interface `{}`", self.prog.decls[*id].name),
            other => other.to_string(),
        }
    }

    fn check_bin(&mut self, op: BinOp, lt: &Type, rt: &Type, line: u32, col: u32) -> Type {
        use BinOp::*;
        match op {
            Add if *lt == Type::Str && *rt == Type::Str => Type::Str,
            Add | Sub | Mul | Div | Rem => {
                if !lt.is_numeric() || !rt.is_numeric() {
                    self.err_at(
                        line,
                        col,
                        12,
                        format!("算术运算需要数值类型，实际是 {} 和 {}", lt, rt),
                    );
                    return Type::Infer;
                }
                if lt != rt {
                    self.err_at(
                        line,
                        col,
                        12,
                        format!("算术运算两侧类型必须一致：{} 与 {}", lt, rt),
                    );
                    return Type::Infer;
                }
                lt.clone()
            }
            Eq | Ne => {
                if matches!(lt, Type::Array(_)) && matches!(rt, Type::Array(_)) {
                    self.err_at(
                        line,
                        col,
                        E_GENERAL,
                        "数组是按引用的 GC 对象，不能用 == 比较；请比较 .len() 后逐元素比较",
                    );
                    return Type::Infer;
                }
                if matches!(lt, Type::Enum(_)) || matches!(rt, Type::Enum(_)) {
                    self.err_at(
                        line,
                        col,
                        E_GENERAL,
                        "enum 不能用 == 比较（比较的是指针身份）：请用 match 对变体分别处理",
                    );
                    return Type::Infer;
                }
                if lt == rt {
                    Type::Bool
                } else {
                    self.err_at(line, col, 12, format!("比较两侧类型必须一致：{} 与 {}", lt, rt));
                    Type::Infer
                }
            }
            Lt | Le | Gt | Ge => {
                let ok = (lt.is_numeric() && rt.is_numeric()) || (*lt == Type::Str && *rt == Type::Str);
                if !ok || lt != rt {
                    self.err_at(line, col, 12, format!("比较不合法：{} 与 {}", lt, rt));
                    return Type::Infer;
                }
                Type::Bool
            }
            AndAnd | OrOr => {
                if *lt != Type::Bool || *rt != Type::Bool {
                    self.err_at(line, col, 12, format!("逻辑运算需要 bool，实际是 {} 和 {}", lt, rt));
                }
                Type::Bool
            }
            BitAnd | BitOr | BitXor | Shl | Shr => {
                if !lt.is_integer() || !rt.is_integer() || lt != rt {
                    self.err_at(line, col, 12, format!("位运算需要两侧同型整数：{} 与 {}", lt, rt));
                    return Type::Infer;
                }
                lt.clone()
            }
        }
    }

    fn check_args(&mut self, what: &str, params: &[Param], args: &mut [Expr], line: u32, col: u32) {
        if args.len() != params.len() {
            self.err_at(
                line,
                col,
                12,
                format!("{} 参数数量不匹配：期望 {} 个，实际 {}", what, params.len(), args.len()),
            );
            return;
        }
        for (a, p) in args.iter_mut().zip(params.iter()) {
            let pt = p.ty.clone();
            let got = self.check_expr(a, Some(&pt));
            if !self.same_type(&got, &pt) && !self.try_iface_coerce(a, &pt) && !matches!(got, Type::Infer) {
                self.err_at(
                    a.line,
                    a.col,
                    12,
                    format!("{} 参数 `{}` 类型不匹配：期望 {}，实际 {}", what, p.name, pt, got),
                );
            }
        }
    }

    /// 静态方法签名的泛型替换；不创建具体 Decl。
    fn check_static_method_signature(
        &mut self,
        base: usize,
        class_args: &[Type],
        mid: usize,
        mtargs: &[Type],
        expected: Option<&Type>,
        args: &mut [Expr],
        line: u32,
        col: u32,
    ) -> (Type, Res) {
        let mut m = self.prog.decls[mid].clone();
        if !class_args.is_empty() {
            let cparams = self.prog.decls[base].type_params.clone();
            let mut cmap = std::collections::HashMap::new();
            for (p, a) in cparams.iter().zip(class_args.iter()) {
                cmap.insert(p.clone(), a.clone());
            }
            crate::generics::subst_decl(&mut m, &cmap);
        }
        let mut method_args: Vec<Type> = Vec::new();
        if !m.type_params.is_empty() {
            let ma = if mtargs.is_empty() {
                let ret = m.ret.clone();
                let params = m.params.clone();
                match self.infer_targs_from_parts(mid, &ret, &params, expected, args) {
                    Some(a) => a,
                    None => {
                        self.err_at(line, col, E_GENERAL, format!("无法推断静态方法 `{}` 的类型实参", m.name));
                        return (Type::Infer, Res::None);
                    }
                }
            } else {
                let mut a = mtargs.to_vec();
                for x in a.iter_mut() { self.resolve_type(x, mid); }
                a
            };
            if ma.len() != m.type_params.len() {
                self.err_at(line, col, E_GENERAL, format!("静态方法 `{}` 类型实参数量不匹配", m.name));
                return (Type::Infer, Res::None);
            }
            let mut mm = std::collections::HashMap::new();
            for (p, a) in m.type_params.iter().zip(ma.iter()) { mm.insert(p.clone(), a.clone()); }
            crate::generics::subst_decl(&mut m, &mm);
            method_args = ma;
        }
        self.check_args(&format!("静态方法 `{}`", m.name), &m.params, args, line, col);
        let ret = m.ret.clone();
        let res = if class_args.is_empty() && method_args.is_empty() {
            Res::StaticMethod(base, mid)
        } else {
            Res::AdtStaticMethod {
                base,
                method: mid,
                class_args: class_args.to_vec(),
                method_args,
            }
        };
        (ret, res)
    }

    /// 检查形如 `Vec<T>` / `Box<T>` 的模板接收者上的方法调用。
    ///
    /// 这一步只使用模板方法的签名做类型检查；真正的实例化发生在
    /// 外层函数/方法被单态化并重新检查时。
    fn check_generic_receiver_method(
        &mut self,
        base: usize,
        gname: &str,
        gargs: &[Type],
        mname: &str,
        mtargs: &[Type],
        expected: Option<&Type>,
        args: &mut [Expr],
        line: u32,
        col: u32,
    ) -> (Type, Res) {
        if !matches!(self.prog.decls[base].kind, DeclKind::Class | DeclKind::Enum) {
            self.err_at(line, col, E_GENERAL, format!("`{}` 不是 class/enum，不能调用实方法", gname));
            return (Type::Infer, Res::None);
        }
        let Some(mid) = self.prog.decls[base]
            .members
            .iter()
            .copied()
            .find(|m| self.prog.decls[*m].name == mname && !self.prog.decls[*m].is_ctor)
        else {
            let kw = if self.prog.decls[base].kind == DeclKind::Enum { "enum" } else { "class" };
            self.err_at(line, col, E_GENERAL, format!("{} `{}` 没有方法 `{}`", kw, gname, mname));
            return (Type::Infer, Res::None);
        };
        if self.prog.decls[mid].is_static {
            self.err_at(
                line,
                col,
                E_GENERAL,
                format!("`{}` 是静态方法，请用 `{}::{}(...)` 调用", mname, gname, mname),
            );
            return (Type::Infer, Res::None);
        }

        let cparams = self.prog.decls[base].type_params.clone();
        let mut cmap = std::collections::HashMap::new();
        for (p, a) in cparams.iter().zip(gargs.iter()) { cmap.insert(p.clone(), a.clone()); }
        let mut m = self.prog.decls[mid].clone();
        crate::generics::subst_decl(&mut m, &cmap);
        let mut method_args: Vec<Type> = Vec::new();

        if !m.type_params.is_empty() {
            let mut ma = if mtargs.is_empty() {
                match self.infer_targs_from_parts(mid, &m.ret, &m.params, expected, args) {
                    Some(a) => a,
                    None => {
                        self.err_at(line, col, E_GENERAL, format!("无法推断方法 `{}` 的类型实参，请显式写成 `{}.{}<...>(...)`", mname, mname, mname));
                        return (Type::Infer, Res::None);
                    }
                }
            } else {
                let mut a = mtargs.to_vec();
                for x in a.iter_mut() { self.resolve_type(x, mid); }
                a
            };
            if ma.len() != m.type_params.len() {
                self.err_at(line, col, E_GENERAL, format!("方法 `{}` 类型实参数量不匹配", mname));
                return (Type::Infer, Res::None);
            }
            let mut mm = std::collections::HashMap::new();
            for (p, a) in m.type_params.iter().zip(ma.iter()) { mm.insert(p.clone(), a.clone()); }
            crate::generics::subst_decl(&mut m, &mm);
            method_args = ma;
        }

        if m.mod_id != self.cur_mod && !m.is_pub {
            self.err_at(line, col, 5, format!("方法 `{}` 不是 pub，跨文件不可见", mname));
        }
        if m.is_unsafe && self.unsafe_depth == 0 {
            self.err_at(line, col, 7, format!("调用 unsafe 方法 `{}` 必须位于 unsafe 域内", mname));
        }
        self.check_args(&format!("方法 `{}`", mname), &m.params, args, line, col);
        self.warn_deprecated(mid, line, col, "方法");
        let ret = m.ret.clone();
        self.prog.note_inst(&ret);
        if !gargs.is_empty() {
            (
                ret,
                Res::AdtMethod {
                    base,
                    method: mid,
                    class_args: gargs.to_vec(),
                    method_args,
                },
            )
        } else {
            (ret, Res::Method(mid))
        }
    }

    fn check_method_call(
        &mut self,
        recv: &mut Expr,
        rty: &Type,
        mname: &str,
        targs: &[Type],
        expected: Option<&Type>,
        args: &mut [Expr],
        line: u32,
        col: u32,
    ) -> (Type, Res) {
        let _ = recv;
        if let Type::Named(n) | Type::Param(n) = rty {
            if let Some(ids) = self.type_param_bounds.get(n).cloned() {
                for iid in ids {
                    let found = self.prog.decls[iid]
                        .members
                        .iter()
                        .copied()
                        .find(|m| self.prog.decls[*m].name == mname);
                    if let Some(mid) = found {
                        if !targs.is_empty() {
                            self.err_at(line, col, E_GENERAL, "接口方法不能带类型实参");
                            return (Type::Infer, Res::None);
                        }
                        let actual = Type::Param(n.clone());
                        let params: Vec<Param> = self.prog.decls[mid]
                            .params
                            .iter()
                            .map(|p| Param {
                                ty: crate::generics::subst_self_type(&p.ty, &actual),
                                ..p.clone()
                            })
                            .collect();
                        let ret = crate::generics::subst_self_type(
                            &self.prog.decls[mid].ret,
                            &actual,
                        );
                        let _ = expected;
                        self.check_args(&format!("接口方法 `{}`", mname), &params, args, line, col);
                        self.prog.note_inst(&ret);
                        return (ret, Res::Method(mid));
                    }
                }
                self.err_at(line, col, E_GENERAL, format!("类型参数 `{}` 的 bound 中没有方法 `{}`", n, mname));
                return (Type::Infer, Res::None);
            }
        }
        // 标量/字符串的内建 Eq/Ord 实现：泛型单态化后，T 的实参是内置类型时
        // `a.equals(b)` / `a.compare(b)` 直接落到这里的 Builtin。
        if matches!(rty, Type::I8 | Type::I16 | Type::I32 | Type::I64
            | Type::U8 | Type::U16 | Type::U32 | Type::U64
            | Type::F32 | Type::F64 | Type::Bool | Type::Str)
        {
            let b = match mname {
                "equals" => Some(Builtin::ValueEq),
                "compare" => Some(Builtin::ValueCmp),
                "hash" => Some(Builtin::ValueHash),
                _ => None,
            };
            if let Some(b) = b {
                if !targs.is_empty() {
                    self.err_at(line, col, E_GENERAL, "内置比较/哈希方法不能带类型实参");
                    return (Type::Infer, Res::None);
                }
                if matches!(b, Builtin::ValueHash) {
                    if !args.is_empty() {
                        self.err_at(line, col, 12, "hash() 不接受参数");
                        return (Type::Infer, Res::None);
                    }
                    return (Type::U64, Res::Builtin(b));
                }
                if args.len() != 1 {
                    self.err_at(line, col, 12, format!("{}() 需要一个参数", mname));
                    return (Type::Infer, Res::None);
                }
                let got = self.check_expr(&mut args[0], Some(rty));
                if !self.same_type(&got, rty) && !matches!(got, Type::Infer) {
                    let _ = expected;
                    self.err_at(
                        args[0].line,
                        args[0].col,
                        12,
                        format!("{} 参数类型不匹配：期望 {}，实际 {}", mname, rty, got),
                    );
                }
                let ret = if matches!(b, Builtin::ValueEq) { Type::Bool } else { Type::I32 };
                return (ret, Res::Builtin(b));
            }
        }
        match rty {
            Type::Generic(gname, gargs) => {
                let base = match self.lookup_type(gname) {
                    Some(id) => id,
                    None => {
                        self.err_at(line, col, E_GENERAL, format!("未知类型 `{}`", gname));
                        return (Type::Infer, Res::None);
                    }
                };
                return self.check_generic_receiver_method(
                    base, gname, gargs, mname, targs, expected, args, line, col,
                );
            }
            Type::Adt { base, args: gargs } => {
                if self.prog.decls[*base].kind == DeclKind::Iface {
                    let mid = match self.prog.decls[*base]
                        .members
                        .iter()
                        .copied()
                        .find(|m| self.prog.decls[*m].name == mname)
                    {
                        Some(m) => m,
                        None => {
                            self.err_at(line, col, E_GENERAL, format!("interface `{}` 没有方法 `{}`", self.prog.decls[*base].name, mname));
                            return (Type::Infer, Res::None);
                        }
                    };
                    let mut m = self.prog.decls[mid].clone();
                    let mut imap = std::collections::HashMap::new();
                    for (p, a) in self.prog.decls[*base].type_params.iter().zip(gargs.iter()) {
                        imap.insert(p.clone(), a.clone());
                    }
                    crate::generics::subst_decl(&mut m, &imap);
                    if !targs.is_empty() {
                        self.err_at(line, col, E_GENERAL, "接口方法不支持显式类型实参");
                    }
                    self.check_args(&format!("接口方法 `{}`", mname), &m.params, args, line, col);
                    let ret = m.ret.clone();
                    self.prog.note_inst(&ret);
                    return (ret, Res::Method(mid));
                }
                return self.check_generic_receiver_method(
                    *base, "", gargs, mname, targs, expected, args, line, col,
                );
            }
            Type::Class(cid) | Type::Enum(cid) => {
                let cid = *cid;
                let found = self.prog.decls[cid]
                    .members
                    .iter()
                    .copied()
                    .find(|m| self.prog.decls[*m].name == mname && !self.prog.decls[*m].is_ctor);
                if let Some(mid) = found {
                    if self.prog.decls[mid].is_static {
                        self.err_at(
                            line,
                            col,
                            E_GENERAL,
                            format!("`{}` 是静态方法，请用 `{}::{}(...)` 调用", mname, self.prog.decls[cid].name, mname),
                        );
                        return (Type::Infer, Res::None);
                    }
                }
                let found = match found {
                    Some(mid) if !self.prog.decls[mid].type_params.is_empty() => {
                        let mut ra = if targs.is_empty() {
                            let ret = self.prog.decls[mid].ret.clone();
                            let params = self.prog.decls[mid].params.clone();
                            match self.infer_targs_from_parts(mid, &ret, &params, expected, args) {
                                Some(a) => a,
                                None => {
                                    self.err_at(line, col, E_GENERAL, format!("无法推断方法 `{}` 的类型实参，请显式写成 `{}.{}<...>(...)`", mname, mname, mname));
                                    return (Type::Infer, Res::None);
                                }
                            }
                        } else {
                            let mut a = targs.to_vec();
                            for x in a.iter_mut() { self.resolve_type(x, mid); }
                            a
                        };
                        if ra.len() != self.prog.decls[mid].type_params.len() {
                            self.err_at(line, col, E_GENERAL, format!("方法 `{}` 类型实参数量不匹配", mname));
                            return (Type::Infer, Res::None);
                        }
                        if self.prog.decls[mid].mod_id != self.cur_mod && !self.prog.decls[mid].is_pub {
                            self.err_at(line, col, 5, format!("方法 `{}` 不是 pub，跨文件不可见", mname));
                        }
                        if self.prog.decls[mid].is_unsafe && self.unsafe_depth == 0 {
                            self.err_at(line, col, 7, format!("调用 unsafe 方法 `{}` 必须位于 unsafe 域内", mname));
                        }
                        let mut m = self.prog.decls[mid].clone();
                        let mut mm = std::collections::HashMap::new();
                        for (p, a) in self.prog.decls[mid].type_params.iter().zip(ra.iter()) {
                            mm.insert(p.clone(), a.clone());
                        }
                        crate::generics::subst_decl(&mut m, &mm);
                        self.check_args(&format!("方法 `{}`", mname), &m.params, args, line, col);
                        self.warn_deprecated(mid, line, col, "方法");
                        let ret = m.ret.clone();
                        self.prog.note_inst(&ret);
                        return (ret, Res::AdtMethod {
                            base: cid,
                            method: mid,
                            class_args: Vec::new(),
                            method_args: ra,
                        });
                    }
                    Some(mid) => {
                        if !targs.is_empty() {
                            self.err_at(line, col, E_GENERAL, format!("方法 `{}` 不是泛型方法，不能带类型实参", mname));
                        }
                        Some(mid)
                    }
                    None => None,
                };
                match found {
                    Some(mid) => {
                        if self.prog.decls[mid].mod_id != self.cur_mod && !self.prog.decls[mid].is_pub {
                            self.err_at(line, col, 5, format!("方法 `{}` 不是 pub，跨文件不可见", mname));
                        }
                        if self.prog.decls[mid].is_unsafe && self.unsafe_depth == 0 {
                            self.err_at(
                                line,
                                col,
                                7,
                                format!("调用 unsafe 方法 `{}` 必须位于 unsafe 域内", mname),
                            );
                        }
                        let params = self.prog.decls[mid].params.clone();
                        self.check_args(&format!("方法 `{}`", mname), &params, args, line, col);
                        self.warn_deprecated(mid, line, col, "方法");
                        let ret = self.prog.decls[mid].ret.clone();
                        self.prog.note_inst(&ret);
                        (ret, Res::Method(mid))
                    }
                    None => {
                        let cname = self.prog.decls[cid].name.clone();
                        let kw = if self.prog.decls[cid].kind == DeclKind::Enum {
                            "enum"
                        } else {
                            "class"
                        };
                        self.err_at(
                            line,
                            col,
                            E_GENERAL,
                            format!("{} `{}` 没有方法 `{}`", kw, cname, mname),
                        );
                        (Type::Infer, Res::None)
                    }
                }
            }
            Type::Iface(iid) => {
                let iid = *iid;
                let found = self.prog.decls[iid]
                    .members
                    .iter()
                    .copied()
                    .find(|m| self.prog.decls[*m].name == mname);
                match found {
                    Some(mid) => {
                        let params = self.prog.decls[mid].params.clone();
                        self.check_args(&format!("接口方法 `{}`", mname), &params, args, line, col);
                        let ret = self.prog.decls[mid].ret.clone();
                        self.prog.note_inst(&ret);
                        (ret, Res::Method(mid))
                    }
                    None => {
                        let iname = self.prog.decls[iid].name.clone();
                        self.err_at(
                            line,
                            col,
                            E_GENERAL,
                            format!("interface `{}` 没有方法 `{}`", iname, mname),
                        );
                        (Type::Infer, Res::None)
                    }
                }
            }
            Type::Array(_) => match mname {
                "len" => {
                    if !args.is_empty() {
                        self.err_at(line, col, 12, "数组.len() 不接受参数");
                    }
                    (Type::I32, Res::Builtin(Builtin::ArrLen))
                }
                _ => {
                    self.err_at(line, col, E_GENERAL, format!("数组没有方法 `{}`（仅支持 .len()）", mname));
                    (Type::Infer, Res::None)
                }
            },
            Type::Str => match mname {
                "len" => {
                    if !args.is_empty() {
                        self.err_at(line, col, 12, "string.len() 不接受参数");
                    }
                    (Type::U64, Res::Builtin(Builtin::StrLen))
                }
                _ => {
                    self.err_at(line, col, E_GENERAL, format!("string 没有方法 `{}`", mname));
                    (Type::Infer, Res::None)
                }
            },
            Type::Result(ok, er) => {
                let (ok, er) = ((**ok).clone(), (**er).clone());
                match mname {
                    "is_ok" | "is_err" => {
                        if !args.is_empty() {
                            self.err_at(line, col, 12, format!("{}.{}() 不接受参数", rty, mname));
                        }
                        let b = if mname == "is_ok" {
                            Builtin::ResIsOk
                        } else {
                            Builtin::ResIsErr
                        };
                        (Type::Bool, Res::Builtin(b))
                    }
                    "unwrap" => {
                        if !args.is_empty() {
                            self.err_at(line, col, 12, "unwrap() 不接受参数");
                        }
                        (ok, Res::Builtin(Builtin::ResUnwrap))
                    }
                    "unwrap_err" => {
                        if !args.is_empty() {
                            self.err_at(line, col, 12, "unwrap_err() 不接受参数");
                        }
                        (er, Res::Builtin(Builtin::ResUnwrapErr))
                    }
                    "unwrap_or" => {
                        if args.len() != 1 {
                            self.err_at(line, col, 12, "unwrap_or() 需要一个参数");
                        } else {
                            let got = self.check_expr(&mut args[0], Some(&ok));
                            if !self.same_type(&got, &ok) && !matches!(got, Type::Infer) {
                                self.err_at(
                                    args[0].line,
                                    args[0].col,
                                    12,
                                    format!("unwrap_or 参数类型不匹配：期望 {}，实际 {}", ok, got),
                                );
                            }
                        }
                        (ok, Res::Builtin(Builtin::ResUnwrapOr))
                    }
                    _ => {
                        self.err_at(line, col, E_GENERAL, format!("Result 没有方法 `{}`", mname));
                        (Type::Infer, Res::None)
                    }
                }
            }
            Type::Option(t) => {
                let t = (**t).clone();
                match mname {
                    "is_some" | "is_none" => {
                        if !args.is_empty() {
                            self.err_at(line, col, 12, format!("{}.{}() 不接受参数", rty, mname));
                        }
                        let b = if mname == "is_some" {
                            Builtin::OptIsSome
                        } else {
                            Builtin::OptIsNone
                        };
                        (Type::Bool, Res::Builtin(b))
                    }
                    "unwrap" => {
                        if !args.is_empty() {
                            self.err_at(line, col, 12, "unwrap() 不接受参数");
                        }
                        (t, Res::Builtin(Builtin::OptUnwrap))
                    }
                    "unwrap_or" => {
                        if args.len() != 1 {
                            self.err_at(line, col, 12, "unwrap_or() 需要一个参数");
                        } else {
                            let got = self.check_expr(&mut args[0], Some(&t));
                            if !self.same_type(&got, &t) && !matches!(got, Type::Infer) {
                                self.err_at(
                                    args[0].line,
                                    args[0].col,
                                    12,
                                    format!("unwrap_or 参数类型不匹配：期望 {}，实际 {}", t, got),
                                );
                            }
                        }
                        (t, Res::Builtin(Builtin::OptUnwrapOr))
                    }
                    _ => {
                        self.err_at(line, col, E_GENERAL, format!("Option 没有方法 `{}`", mname));
                        (Type::Infer, Res::None)
                    }
                }
            }
            other => {
                self.err_at(line, col, 12, format!("{} 没有方法 `{}`", other, mname));
                (Type::Infer, Res::None)
            }
        }
    }

    fn check_cast(&mut self, src: &Type, target: &Type, line: u32, col: u32) -> (Type, Res) {
        if src == target {
            return (target.clone(), Res::None);
        }
        match (src, target) {
            (a, b) if a.is_numeric() && b.is_numeric() => (target.clone(), Res::None),
            (Type::Ptr(_), Type::Ptr(_)) => {
                if self.unsafe_depth == 0 {
                    self.err_at(line, col, 7, "指针类型转换只能在 unsafe 域内");
                }
                (target.clone(), Res::None)
            }
            (Type::Class(cid), Type::Iface(iid)) => {
                let (cid, iid) = (*cid, *iid);
                if self.iface_compatible(cid, iid, line, col) {
                    self.prog.note_impl(cid, iid);
                    (target.clone(), Res::IfaceCast(cid, iid))
                } else {
                    (Type::Infer, Res::None)
                }
            }
            (Type::Iface(_), Type::Class(_)) => (target.clone(), Res::None),
            (Type::Iface(_), Type::Iface(_)) => (target.clone(), Res::None),
            (Type::Iface(_), t) if boxable(t) => (target.clone(), Res::None),
            _ => {
                self.err_at(
                    line,
                    col,
                    12,
                    format!("不允许的显式转换：{} -> {}", src, target),
                );
                (Type::Infer, Res::None)
            }
        }
    }

    fn empty_iface(&self, iid: usize) -> bool {
        self.prog.decls[iid].members.is_empty()
    }

    /// class 是否实现 interface（鸭子类型：方法签名匹配即实现）
    fn iface_compatible_adt(
        &mut self,
        cid: usize,
        cargs: &[Type],
        iid: usize,
        iargs: &[Type],
        line: u32,
        col: u32,
    ) -> bool {
        let iname = self.prog.decls[iid].name.clone();
        let cname = self.prog.decls[cid].name.clone();
        let mut cmap = std::collections::HashMap::new();
        let mut imap = std::collections::HashMap::new();
        for (p, a) in self.prog.decls[cid].type_params.iter().zip(cargs.iter()) {
            cmap.insert(p.clone(), a.clone());
        }
        for (p, a) in self.prog.decls[iid].type_params.iter().zip(iargs.iter()) {
            imap.insert(p.clone(), a.clone());
        }
        let iface_self = Type::Adt { base: iid, args: iargs.to_vec() };
        let sigs: Vec<usize> = self.prog.decls[iid].members.clone();
        let mut ok = true;
        for sid in sigs {
            let (sname, sparams, sret) = {
                let s = self.prog.decls[sid].clone();
                let subst = |t: &Type| {
                    let t = crate::generics::subst_type(t, &imap);
                    crate::generics::subst_self_type(&t, &iface_self)
                };
                (
                    s.name.clone(),
                    s.params.iter().map(|p| subst(&p.ty)).collect::<Vec<_>>(),
                    subst(&s.ret),
                )
            };
            let found = self.prog.decls[cid]
                .members
                .iter()
                .copied()
                .find(|m| {
                    let method = &self.prog.decls[*m];
                    !method.is_ctor
                        && (method.name == sname
                            || (sname == "toString" && method.name == "to_string"))
                });
            match found {
                None => {
                    self.err_at(line, col, E_GENERAL,
                        format!("class `{}` 缺少接口 `{}` 要求的方法 `{}`", cname, iname, sname));
                    ok = false;
                }
                Some(mid) => {
                    let mut m = self.prog.decls[mid].clone();
                    crate::generics::subst_decl(&mut m, &cmap);
                    if m.params.len() != sparams.len() {
                        self.err_at(line, col, 12,
                            format!("方法 `{}` 参数数量与接口 `{}` 声明不一致", sname, iname));
                        ok = false;
                        continue;
                    }
                    for (a, b) in m.params.iter().zip(sparams.iter()) {
                        if !self.same_type(&a.ty, b) {
                            self.err_at(line, col, 12,
                                format!("方法 `{}` 参数类型与接口 `{}` 声明不一致：{} 与 {}", sname, iname, a.ty, b));
                            ok = false;
                        }
                    }
                    if !self.same_type(&m.ret, &sret) {
                        self.err_at(line, col, 12,
                            format!("方法 `{}` 返回类型与接口 `{}` 声明不一致：{} 与 {}", sname, iname, m.ret, sret));
                        ok = false;
                    }
                }
            }
        }
        ok
    }

    fn iface_compatible(&mut self, cid: usize, iid: usize, line: u32, col: u32) -> bool {
        let iname = self.prog.decls[iid].name.clone();
        let cname = self.prog.decls[cid].name.clone();
        let self_ty = if self.prog.decls[cid].kind == DeclKind::Enum {
            Type::Enum(cid)
        } else {
            Type::Class(cid)
        };
        let sigs: Vec<usize> = self.prog.decls[iid].members.clone();
        let mut ok = true;
        for sid in sigs {
            let (sname, sparams, sret) = {
                let s = &self.prog.decls[sid];
                (
                    s.name.clone(),
                    s.params
                        .iter()
                        .map(|p| crate::generics::subst_self_type(&p.ty, &self_ty))
                        .collect::<Vec<_>>(),
                    crate::generics::subst_self_type(&s.ret, &self_ty),
                )
            };
            let found = self.prog.decls[cid]
                .members
                .iter()
                .copied()
                .find(|m| {
                    let method = &self.prog.decls[*m];
                    !method.is_ctor
                        && (method.name == sname
                            || (sname == "toString" && method.name == "to_string"))
                });
            match found {
                None => {
                    self.err_at(
                        line,
                        col,
                        E_GENERAL,
                        format!("class `{}` 缺少接口 `{}` 要求的方法 `{}`", cname, iname, sname),
                    );
                    ok = false;
                }
                Some(mid) => {
                    let (mparams, mret) = {
                        let m = &self.prog.decls[mid];
                        (m.params.clone(), m.ret.clone())
                    };
                    if mparams.len() != sparams.len() {
                        self.err_at(
                            line,
                            col,
                            12,
                            format!("方法 `{}` 参数数量与接口 `{}` 声明不一致", sname, iname),
                        );
                        ok = false;
                        continue;
                    }
                    for (a, b) in mparams.iter().zip(sparams.iter()) {
                        if a.ty != *b {
                            self.err_at(
                                line,
                                col,
                                12,
                                format!(
                                    "方法 `{}` 参数类型与接口 `{}` 声明不一致：{} 与 {}",
                                    sname, iname, a.ty, b
                                ),
                            );
                            ok = false;
                        }
                    }
                    if mret != sret {
                        self.err_at(
                            line,
                            col,
                            12,
                            format!(
                                "方法 `{}` 返回类型与接口 `{}` 声明不一致：{} 与 {}",
                                sname, iname, mret, sret
                            ),
                        );
                        ok = false;
                    }
                }
            }
        }
        ok
    }

    /// 在需要的位置把 class 值自动转换为 interface 值
    fn try_iface_coerce(&mut self, e: &mut Expr, expected: &Type) -> bool {
        let orig = e.ty.clone();
        let original = orig.clone();
        let (iid, iargs, iface_generic) = match expected {
            Type::Iface(iid) => (*iid, Vec::new(), false),
            Type::Adt { base, args } if self.prog.decls[*base].kind == DeclKind::Iface => {
                (*base, args.clone(), true)
            }
            _ => return false,
        };
        let (cid, cargs) = match &orig {
            Type::Class(cid) | Type::Enum(cid) => (*cid, Vec::new()),
            Type::Adt { base, args } => (*base, args.clone()),
            _ => {
                if !iface_generic && self.empty_iface(iid) && boxable(&orig) {
                    e.box_to = Some((orig, iid));
                    e.ty = expected.clone();
                    return true;
                }
                return false;
            }
        };
        let compatible = if iface_generic {
            self.iface_compatible_adt(cid, &cargs, iid, &iargs, e.line, e.col)
        } else {
            self.iface_compatible_adt(cid, &cargs, iid, &iargs, e.line, e.col)
        };
        if compatible {
            e.coerce = Some((cid, iid));
            e.coerce_adt = Some((cid, cargs.clone(), iid, iargs.clone()));
            self.prog.note_impl(cid, iid);
            // 具体实例在 mono 阶段替换；这里保留 class/enum 原始类型。
            e.ty = original;
            return true;
        }
        if !iface_generic && self.empty_iface(iid) && boxable(&orig) {
            e.box_to = Some((orig, iid));
            e.ty = expected.clone();
            return true;
        }
        false
    }

}

/// 类方法体的外部入口（供 analyze 使用）
fn check_all_method_bodies(prog: &mut Program, diags: &mut Diag) {
    let mut items: Vec<(usize, usize)> = Vec::new();
    for cid in 0..prog.decls.len() {
        if matches!(prog.decls[cid].kind, DeclKind::Class | DeclKind::Enum) {
            for m in prog.decls[cid].members.clone() {
                items.push((m, cid));
            }
        }
    }
    for (mid, cid) in items {
        if prog.decls[mid].body.is_none() {
            continue;
        }
        let mut s = Sema::new(&mut *prog, &mut *diags, mid);
        s.check_method_body(mid, cid);
    }
}

fn boxable(t: &Type) -> bool {
    matches!(
        t,
        Type::I8 | Type::I16 | Type::I32 | Type::I64
            | Type::U8 | Type::U16 | Type::U32 | Type::U64
            | Type::F32 | Type::F64 | Type::Bool | Type::Str
    )
}

fn is_numeric_literal(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Int(_) | ExprKind::IntU64(_) | ExprKind::Float(_))
}

fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Ident(_) => true,
        ExprKind::Index(recv, _) => is_place(recv),
        ExprKind::StaticPath { call, .. } => !*call,
        ExprKind::Field(recv, _) => is_place(recv) || matches!(recv.kind, ExprKind::This),
        ExprKind::Un(UnOp::Deref, _) => true,
        _ => false,
    }
}

fn int_fits(v: i64, t: &Type) -> Result<(), String> {
    let ok = match t {
        Type::I8 => v >= -128 && v <= 127,
        Type::I16 => v >= -32768 && v <= 32767,
        Type::I32 => v >= i32::MIN as i64 && v <= i32::MAX as i64,
        Type::I64 => true,
        Type::U8 => v >= 0 && v <= 255,
        Type::U16 => v >= 0 && v <= 65535,
        Type::U32 => v >= 0 && v <= u32::MAX as i64,
        Type::U64 => v >= 0,
        Type::F32 | Type::F64 => true,
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("整数字面量 {} 超出 {} 的取值范围", v, t))
    }
}

fn int_fits_u64(v: u64, t: &Type) -> Result<(), String> {
    let ok = match t {
        Type::I8 => v <= 127,
        Type::I16 => v <= 32767,
        Type::I32 => v <= i32::MAX as u64,
        Type::I64 => v <= i64::MAX as u64,
        Type::U8 => v <= 255,
        Type::U16 => v <= 65535,
        Type::U32 => v <= u32::MAX as u64,
        Type::U64 => true,
        Type::F32 | Type::F64 => true,
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("整数字面量 {} 超出 {} 的取值范围", v, t))
    }
}

fn block_returns(b: &Block) -> bool {
    match b.stmts.last() {
        Some(s) => stmt_returns(s),
        None => false,
    }
}

fn is_const_literal(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::IntU64(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Null => true,
        ExprKind::Un(UnOp::Neg, inner) => matches!(&inner.kind, ExprKind::Int(_) | ExprKind::IntU64(_) | ExprKind::Float(_)),
        _ => false,
    }
}

fn stmt_returns(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Return(_) => true,
        StmtKind::Block(b) => block_returns(b),
        StmtKind::Unsafe(b) => block_returns(b),
        StmtKind::If {
            then_b, else_b, ..
        } => match else_b {
            Some(eb) => block_returns(then_b) && stmt_returns(eb),
            None => false,
        },
        StmtKind::Match { arms, .. } => {
            !arms.is_empty() && arms.iter().all(|a| block_returns(&a.body))
        }
        _ => false,
    }
}

// ------------------------------------------------------------ 符号表
fn build_symbols(prog: &mut Program, diags: &mut Diag) {
    prog.type_syms = vec![Vec::new(); prog.modules.len()];
    prog.fn_syms = vec![Vec::new(); prog.modules.len()];
    prog.const_syms = vec![Vec::new(); prog.modules.len()];
    let mut dup: Vec<(usize, String, u32, u32)> = Vec::new();
    for id in 0..prog.decls.len() {
        if prog.decls[id].owner.is_some() {
            continue; // 方法 / 接口签名的名字属于其所属类型的作用域
        }
        let (kind, name, mod_id, line, col) = {
            let d = &prog.decls[id];
            (d.kind, d.name.clone(), d.mod_id, d.line, d.col)
        };
        let table = match kind {
            DeclKind::Fn => &mut prog.fn_syms[mod_id],
            DeclKind::Const => &mut prog.const_syms[mod_id],
            _ => &mut prog.type_syms[mod_id],
        };
        if table.iter().any(|(n, _)| *n == name) {
            dup.push((mod_id, name.clone(), line, col));
            continue;
        }
        table.push((name, id));
    }
    for (mod_id, name, line, col) in dup {
        let file = prog.modules[mod_id].path.clone();
        diags.err(
            E_GENERAL,
            &file,
            line,
            col,
            format!("同一文件内重复声明 `{}`（Mylis 禁止函数重载 / 重名）", name),
        );
    }
}

// --------------------------------------------------- MYL004 字段全覆盖
fn check_class_fields(prog: &mut Program, diags: &mut Diag) {
    let mut reports: Vec<(usize, u32, u32, String)> = Vec::new();
    for cid in 0..prog.decls.len() {
        if prog.decls[cid].kind != DeclKind::Class {
            continue;
        }
        let cname = prog.decls[cid].name.clone();
        let missing: Vec<(String, u32, u32)> = prog.decls[cid]
            .fields
            .iter()
            .filter(|f| !f.is_static && f.init.is_none())
            .map(|f| (f.name.clone(), f.line, f.col))
            .collect();
        if missing.is_empty() {
            continue;
        }
        let ctor = prog.decls[cid]
            .members
            .iter()
            .copied()
            .find(|m| prog.decls[*m].is_ctor);
        let assigned: Vec<String> = match ctor {
            Some(mid) => match &prog.decls[mid].body {
                Some(b) => collect_this_assigns(b).into_iter().collect(),
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        let virtual_api = prog.modules[prog.decls[cid].mod_id].path.contains(".mylib");
        for (fname, line, col) in missing {
            if virtual_api { continue; }
            if !assigned.contains(&fname) {
                reports.push((
                    cid,
                    line,
                    col,
                    format!(
                        "class `{}` 的字段 `{}` 未初始化：字段要么在声明处赋初值，要么在 init 中被赋值",
                        cname, fname
                    ),
                ));
            }
        }
        if ctor.is_none() && !assigned.is_empty() {
            // 理论不可达，保留结构完整性
        }
    }
    for (cid, line, col, msg) in reports {
        let file = prog.modules[prog.decls[cid].mod_id].path.clone();
        diags.err(4, &file, line, col, msg);
    }
}

fn collect_this_assigns(b: &Block) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in &b.stmts {
        collect_stmt_assigns(s, &mut out);
    }
    out
}

fn collect_stmt_assigns(s: &Stmt, out: &mut HashSet<String>) {
    match &s.kind {
        StmtKind::Assign { target, value } => {
            if let ExprKind::Field(recv, fname) = &target.kind {
                if matches!(recv.kind, ExprKind::This) {
                    out.insert(fname.clone());
                }
            }
            if let ExprKind::Field(recv, fname) = &value.kind {
                if matches!(recv.kind, ExprKind::This) {
                    out.insert(fname.clone());
                }
            }
        }
        StmtKind::If {
            then_b, else_b, ..
            } => {
            for s in &then_b.stmts {
                collect_stmt_assigns(s, out);
            }
            if let Some(eb) = else_b {
                collect_stmt_assigns(eb, out);
            }
        }
        StmtKind::While { body, .. } => {
            for s in &body.stmts {
                collect_stmt_assigns(s, out);
            }
        }
        StmtKind::For { init, body, .. } => {
            collect_stmt_assigns(init, out);
            for s in &body.stmts {
                collect_stmt_assigns(s, out);
            }
        }
        StmtKind::Block(b) | StmtKind::Unsafe(b) => {
            for s in &b.stmts {
                collect_stmt_assigns(s, out);
            }
        }
        _ => {}
    }
}

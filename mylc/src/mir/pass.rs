//! MIR 优化 pass 框架。
//!
//! 第一个 pass 只做控制流图整理：从入口块出发做可达性分析，删除不可达块。
//! 它不改变语义，只是给后续常量折叠/DCE/内联提供统一的 pass 入口。

use super::{MirFunction, MirInst, MirInstKind, MirOperand, MirProgram, MirTerm, MirConst};
use crate::ast::{BinOp, Res, Type};
use std::collections::{HashMap, HashSet};

pub fn simplify(prog: &mut MirProgram) {
    inline_calls(prog);
    for f in &mut prog.funcs {
        prune_unreachable(f);
        fold_constants(f);
        eliminate_dead(f);
    }
    prog.funcs.retain(|f| !f.blocks.is_empty());
    inline_cleanup(prog);
    for f in &mut prog.funcs {
        annotate_safepoints(f);
    }
}

/// 类型是否可能包含 GC 引用。接口/枚举/聚合内部也可能有引用。
fn has_gc_ref(t: &Type) -> bool {
    match t {
        Type::Str | Type::Class(_) | Type::Array(_) | Type::Iface(_) | Type::Enum(_) => true,
        Type::Option(e) => has_gc_ref(e),
        Type::Result(a, b) => has_gc_ref(a) || has_gc_ref(b),
        _ => false,
    }
}

/// 活跃引用集合。局部槽和 SSA 值分开存，避免给 MirOperand 加 Hash 依赖。
#[derive(Clone, Default, PartialEq, Eq)]
struct LiveSet {
    locals: HashSet<u32>,
    values: HashSet<u32>,
}

impl LiveSet {
    fn add_op(&mut self, o: MirOperand) {
        match o {
            MirOperand::Local(l) => { self.locals.insert(l); }
            MirOperand::Value(v) => { self.values.insert(v); }
        }
    }

    fn contains_op(&self, o: MirOperand) -> bool {
        match o {
            MirOperand::Local(l) => self.locals.contains(&l),
            MirOperand::Value(v) => self.values.contains(&v),
        }
    }

    fn remove_op(&mut self, o: MirOperand) {
        match o {
            MirOperand::Local(l) => { self.locals.remove(&l); }
            MirOperand::Value(v) => { self.values.remove(&v); }
        }
    }

    fn union(&mut self, other: &LiveSet) {
        self.locals.extend(other.locals.iter().copied());
        self.values.extend(other.values.iter().copied());
    }

    fn subtract(&mut self, other: &LiveSet) {
        for l in &other.locals { self.locals.remove(l); }
        for v in &other.values { self.values.remove(v); }
    }

    fn is_empty(&self) -> bool { self.locals.is_empty() && self.values.is_empty() }
}

fn term_uses(t: &Option<MirTerm>, out: &mut LiveSet, local_ref: &HashSet<u32>, value_ref: &HashSet<u32>) {
    let add = |out: &mut LiveSet, o: MirOperand| match o {
        MirOperand::Local(l) if local_ref.contains(&l) => { out.locals.insert(l); }
        MirOperand::Value(v) if value_ref.contains(&v) => { out.values.insert(v); }
        _ => {}
    };
    match t {
        Some(MirTerm::Return(Some(v))) => add(out, *v),
        Some(MirTerm::Branch { cond, .. }) => add(out, *cond),
        _ => {}
    }
}

/// 在调用/分配点前插入 `safepoint`，roots 使用反向活跃性分析的结果：
/// 只保留在该点之后（包括调用参数本身）仍然会被读取的 GC 引用。
fn annotate_safepoints(f: &mut MirFunction) {
    if f.blocks.is_empty() { return; }

    let local_ref: HashSet<u32> = f.locals.iter().enumerate()
        .filter(|(_, l)| has_gc_ref(&l.ty))
        .map(|(i, _)| i as u32)
        .collect();
    let mut value_ref: HashSet<u32> = HashSet::new();
    for b in &f.blocks {
        for inst in &b.insts {
            if let Some(d) = inst.dst {
                if has_gc_ref(&inst.ty) { value_ref.insert(d); }
            }
        }
    }
    if local_ref.is_empty() && value_ref.is_empty() { return; }

    let labels: HashMap<&str, usize> = f.blocks.iter().enumerate()
        .map(|(i, b)| (b.label.as_str(), i)).collect();
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); f.blocks.len()];
    for (i, b) in f.blocks.iter().enumerate() {
        match &b.term {
            Some(MirTerm::Goto(t)) => {
                if let Some(j) = labels.get(t.as_str()) { succs[i].push(*j); }
            }
            Some(MirTerm::Branch { then_label, else_label, .. }) => {
                for t in [then_label, else_label] {
                    if let Some(j) = labels.get(t.as_str()) { succs[i].push(*j); }
                }
            }
            _ => {}
        }
    }

    // 块级 use/def：def 是 SSA dst、Store 的局部槽、BindPattern 绑定的局部槽。
    let mut uses: Vec<LiveSet> = vec![LiveSet::default(); f.blocks.len()];
    let mut defs: Vec<LiveSet> = vec![LiveSet::default(); f.blocks.len()];
    for (bi, b) in f.blocks.iter().enumerate() {
        let mut block_defs = LiveSet::default();
        for inst in &b.insts {
            for op in operands(&inst.kind) {
                let is_ref = match op {
                    MirOperand::Local(l) => local_ref.contains(&l),
                    MirOperand::Value(v) => value_ref.contains(&v),
                };
                if is_ref && !block_defs.contains_op(op) {
                    uses[bi].add_op(op);
                }
            }
            if let Some(d) = inst.dst {
                if value_ref.contains(&d) {
                    defs[bi].values.insert(d);
                    block_defs.values.insert(d);
                }
            }
            if let MirInstKind::Store { local, .. } = &inst.kind {
                if local_ref.contains(local) {
                    defs[bi].locals.insert(*local);
                    block_defs.locals.insert(*local);
                }
            }
            if let MirInstKind::BindPattern { locals, .. } = &inst.kind {
                for l in locals {
                    if local_ref.contains(l) {
                        defs[bi].locals.insert(*l);
                        block_defs.locals.insert(*l);
                    }
                }
            }
        }
        term_uses(&b.term, &mut uses[bi], &local_ref, &value_ref);
    }

    // 反向数据流到不动点。
    let mut live_in: Vec<LiveSet> = vec![LiveSet::default(); f.blocks.len()];
    let mut live_out: Vec<LiveSet> = vec![LiveSet::default(); f.blocks.len()];
    loop {
        let mut changed = false;
        for bi in (0..f.blocks.len()).rev() {
            let mut out = LiveSet::default();
            for s in &succs[bi] { out.union(&live_in[*s]); }
            let mut in_set = uses[bi].clone();
            in_set.union(&out);
            in_set.subtract(&defs[bi]);
            if out != live_out[bi] || in_set != live_in[bi] {
                live_out[bi] = out;
                live_in[bi] = in_set;
                changed = true;
            }
        }
        if !changed { break; }
    }

    // 每个块内反向扫描，给每个调用/分配点算出 roots。
    // roots = 该点之后的活跃引用 ∪ 该指令自己的引用操作数（调用参数在 GC 前必须保活）。
    let mut additions: HashMap<usize, Vec<(usize, Vec<MirOperand>)>> = HashMap::new();
    for (bi, b) in f.blocks.iter().enumerate() {
        let mut live = live_out[bi].clone();
        let mut tu = LiveSet::default();
        term_uses(&b.term, &mut tu, &local_ref, &value_ref);
        live.union(&tu);
        for ii in (0..b.insts.len()).rev() {
            let inst = &b.insts[ii];
            let is_alloc_or_call = matches!(
                inst.kind,
                MirInstKind::Call { .. }
                    | MirInstKind::MethodCall { .. }
                    | MirInstKind::New { .. }
                    | MirInstKind::ArrayNew { .. }
                    | MirInstKind::ArrayLit { .. }
                    | MirInstKind::Interp { .. }
                    | MirInstKind::Box { .. }
            );
            if is_alloc_or_call {
                let mut roots = live.clone();
                for op in operands(&inst.kind) {
                    match op {
                        MirOperand::Local(l) if local_ref.contains(&l) => { roots.locals.insert(l); }
                        MirOperand::Value(v) if value_ref.contains(&v) => { roots.values.insert(v); }
                        _ => {}
                    }
                }
                // 本次调用的 dst 在调用返回前尚不存在，不能出现在根标注里。
                if let Some(d) = inst.dst { roots.values.remove(&d); }
                let mut ls: Vec<u32> = roots.locals.iter().copied().collect();
                ls.sort_unstable();
                let mut vs: Vec<u32> = roots.values.iter().copied().collect();
                vs.sort_unstable();
                let roots: Vec<MirOperand> = ls.into_iter().map(MirOperand::Local)
                    .chain(vs.into_iter().map(MirOperand::Value))
                    .collect();
                additions.entry(bi).or_default().push((ii, roots));
            }
            for op in operands(&inst.kind) {
                match op {
                    MirOperand::Local(l) if local_ref.contains(&l) => { live.locals.insert(l); }
                    MirOperand::Value(v) if value_ref.contains(&v) => { live.values.insert(v); }
                    _ => {}
                }
            }
            if let Some(d) = inst.dst { live.remove_op(MirOperand::Value(d)); }
            if let MirInstKind::Store { local, .. } = &inst.kind {
                live.remove_op(MirOperand::Local(*local));
            }
            if let MirInstKind::BindPattern { locals, .. } = &inst.kind {
                for l in locals { live.remove_op(MirOperand::Local(*l)); }
            }
        }
    }

    // 按原指令顺序插入，避免多次调用点相互覆盖索引。
    for (bi, adds) in additions {
        let mut by_idx: HashMap<usize, Vec<MirOperand>> = HashMap::new();
        for (ii, roots) in adds { by_idx.insert(ii, roots); }
        let old = std::mem::take(&mut f.blocks[bi].insts);
        let mut new_insts = Vec::with_capacity(old.len() * 2);
        for (ii, inst) in old.into_iter().enumerate() {
            if let Some(roots) = by_idx.get(&ii) {
                new_insts.push(MirInst {
                    dst: None,
                    ty: Type::Void,
                    res: Res::None,
                    coerce: None,
                    kind: MirInstKind::Safepoint { roots: roots.clone() },
                    line: inst.line,
                });
            }
            new_insts.push(inst);
        }
        f.blocks[bi].insts = new_insts;
    }
}

fn inline_cleanup(prog: &mut MirProgram) {
    // 内联后可能产生空的基本块或只剩跳转的块；可达性清理保持 CFG 紧凑。
    for f in &mut prog.funcs {
        prune_unreachable(f);
        remove_empty_gotos(f);
        fold_constants(f);
        eliminate_dead(f);
    }
}

fn remove_empty_gotos(f: &mut MirFunction) {
    // 只合并“无指令、只 goto 下一块”的块，避免改变分支语义。
    let mut changed = true;
    while changed {
        changed = false;
        let mut redirect: HashMap<String, String> = HashMap::new();
        for b in &f.blocks {
            if b.insts.is_empty() {
                if let Some(MirTerm::Goto(t)) = &b.term {
                    redirect.insert(b.label.clone(), t.clone());
                }
            }
        }
        if redirect.is_empty() {
            break;
        }
        let nblocks = f.blocks.len();
        for b in &mut f.blocks {
            let t = match &b.term {
                Some(MirTerm::Goto(t)) => Some(t.clone()),
                Some(MirTerm::Branch { then_label, else_label, .. }) => {
                    let mut nt = then_label.clone();
                    for _ in 0..nblocks { match redirect.get(&nt) { Some(x) => nt = x.clone(), None => break } }
                    let mut ne = else_label.clone();
                    for _ in 0..nblocks { match redirect.get(&ne) { Some(x) => ne = x.clone(), None => break } }
                    b.term = Some(MirTerm::Branch {
                        cond: match &b.term { Some(MirTerm::Branch { cond, .. }) => *cond, _ => unreachable!() },
                        then_label: nt,
                        else_label: ne,
                    });
                    None
                }
                _ => None,
            };
            if let Some(mut t) = t {
                for _ in 0..nblocks { match redirect.get(&t) { Some(x) => t = x.clone(), None => break } }
                b.term = Some(MirTerm::Goto(t));
            }
        }
        f.blocks.retain(|b| !redirect.contains_key(&b.label));
        changed = true;
    }
}

fn scalar_abi(t: &Type) -> bool {
    matches!(
        t,
        Type::Void | Type::I8 | Type::I16 | Type::I32 | Type::I64
            | Type::U8 | Type::U16 | Type::U32 | Type::U64
            | Type::F32 | Type::F64 | Type::Bool | Type::Str
    )
}

fn inline_calls(prog: &mut MirProgram) {
    let snapshot: Vec<MirFunction> = prog.funcs.clone();
    let mut small: HashMap<usize, MirFunction> = HashMap::new();
    for f in &snapshot {
        if f.owner.is_none()
            && !f.is_ctor
            && !f.name.ends_with("::main")
            && !f.name.contains("__g")
            && f.blocks.len() <= 4
            && f.blocks.iter().map(|b| b.insts.len()).sum::<usize>() <= 24
            && scalar_abi(&f.ret)
            && f.params.iter().all(|p| scalar_abi(&p.ty))
            && !f.blocks.iter().flat_map(|b| &b.insts).any(|i| matches!(i.kind, MirInstKind::Opaque(_)))
        {
            small.insert(f.decl_id, f.clone());
        }
    }
    let mut callees: HashMap<String, HashSet<String>> = HashMap::new();
    for f in &snapshot {
        let mut set = HashSet::new();
        for b in &f.blocks {
            for i in &b.insts {
                if let MirInstKind::Call { callee, .. } = &i.kind {
                    set.insert(callee.clone());
                }
            }
        }
        callees.insert(f.name.clone(), set);
    }
    for idx in 0..prog.funcs.len() {
        let caller = prog.funcs[idx].name.clone();
        inline_in_function(&mut prog.funcs[idx], &small, &callees, &caller);
    }
}

fn reaches(from: &str, target: &str, graph: &HashMap<String, HashSet<String>>, seen: &mut HashSet<String>) -> bool {
    if from == target {
        return true;
    }
    if !seen.insert(from.to_string()) {
        return false;
    }
    graph.get(from).map(|next| next.iter().any(|n| reaches(n, target, graph, seen))).unwrap_or(false)
}

fn remap_op(o: MirOperand, values: &HashMap<u32, u32>, locals: &HashMap<u32, u32>) -> MirOperand {
    match o {
        MirOperand::Value(v) => MirOperand::Value(values.get(&v).copied().unwrap_or(v)),
        MirOperand::Local(l) => MirOperand::Local(locals.get(&l).copied().unwrap_or(l)),
    }
}

fn inline_in_function(
    f: &mut MirFunction,
    small: &HashMap<usize, MirFunction>,
    graph: &HashMap<String, HashSet<String>>,
    caller: &str,
) {
    let mut next_value = f
        .blocks
        .iter()
        .flat_map(|b| &b.insts)
        .filter_map(|i| i.dst)
        .max()
        .unwrap_or(0)
        + 1;
    let mut next_local = f.locals.len() as u32;
    let mut serial = 0usize;
    let mut out = Vec::new();
    for b in f.blocks.drain(..) {
        let mut current = super::MirBlock { label: b.label.clone(), insts: Vec::new(), term: b.term.clone() };
        for i in b.insts {
            let call = match &i.kind {
                MirInstKind::Call { callee, .. } => callee.clone(),
                _ => { current.insts.push(i); continue; }
            };
            let call_args = match &i.kind {
                MirInstKind::Call { args, .. } => args.clone(),
                _ => Vec::new(),
            };
            let callee_id = match i.res {
                crate::ast::Res::Fn(id) => Some(id),
                _ => None,
            };
            let callee_fn = match callee_id.and_then(|id| small.get(&id)) {
                Some(x) if x.name != caller => x.clone(),
                _ => { current.insts.push(i); continue; }
            };
            let mut seen = HashSet::new();
            if reaches(&call, caller, graph, &mut seen) {
                current.insts.push(i);
                continue;
            }
            serial += 1;
            let prefix = format!("inl{}_{}_", serial, call.replace("::", "_"));
            let entry = format!("{}{}", prefix, callee_fn.blocks.first().map(|b| b.label.as_str()).unwrap_or("entry"));
            let after = format!("{}after", prefix);
            let ret_dst = i.dst.unwrap_or_else(|| { let v = next_value; next_value += 1; v });
            let mut values = HashMap::new();
            let mut locals = HashMap::new();
            for dst in callee_fn.blocks.iter().flat_map(|b| &b.insts).filter_map(|i| i.dst) {
                values.insert(dst, next_value);
                next_value += 1;
            }
            for li in 0..callee_fn.locals.len() as u32 {
                locals.insert(li, next_local);
                next_local += 1;
            }
            // 参数槽：复制到调用者新局部槽。
            let param_local = |name: &str, ty: &Type| {
                callee_fn.locals.iter().position(|l| &l.name == name && &l.ty == ty).map(|x| x as u32)
            };
            for (pi, arg) in call_args.iter().enumerate() {
                if let Some(li) = callee_fn.params.get(pi).and_then(|p| param_local(&p.name, &p.ty)) {
                    if let Some(nl) = locals.get(&li) {
                        current.insts.push(super::MirInst {
                            dst: None,
                            ty: Type::Void,
                            res: crate::ast::Res::None,
                            coerce: None,
                            kind: MirInstKind::Store { local: *nl, value: *arg },
                            line: i.line,
                        });
                    }
                }
            }
            f.locals.extend(callee_fn.locals.iter().map(|l| l.clone()));
            current.term = Some(MirTerm::Goto(entry.clone()));
            out.push(current);

            let mut inlined = Vec::new();
            for cb in &callee_fn.blocks {
                let label = format!("{}{}", prefix, cb.label);
                let mut insts = Vec::new();
                for ci in &cb.insts {
                    insts.push(clone_inst(ci, &values, &locals));
                }
                let term = match &cb.term {
                    Some(MirTerm::Return(Some(v))) => {
                        insts.push(super::MirInst {
                            dst: Some(ret_dst),
                            ty: callee_fn.ret.clone(),
                            res: crate::ast::Res::None,
                            coerce: None,
                            kind: MirInstKind::Copy(remap_op(*v, &values, &locals)),
                            line: 0,
                        });
                        Some(MirTerm::Goto(after.clone()))
                    }
                    Some(MirTerm::Return(None)) => Some(MirTerm::Goto(after.clone())),
                    Some(MirTerm::Goto(t)) => Some(MirTerm::Goto(format!("{}{}", prefix, t))),
                    Some(MirTerm::Branch { cond, then_label, else_label }) => Some(MirTerm::Branch {
                        cond: remap_op(*cond, &values, &locals),
                        then_label: format!("{}{}", prefix, then_label),
                        else_label: format!("{}{}", prefix, else_label),
                    }),
                    Some(MirTerm::Unreachable) => Some(MirTerm::Unreachable),
                    None => None,
                };
                inlined.push(super::MirBlock { label, insts, term });
            }
            out.extend(inlined);
            let empty_term = None;
            current = super::MirBlock { label: after, insts: Vec::new(), term: empty_term };
        }
        // 剩余块尾
        current.term = b.term.clone();
        out.push(current);
    }
    f.blocks = out;
}

fn clone_inst(i: &super::MirInst, values: &HashMap<u32, u32>, locals: &HashMap<u32, u32>) -> super::MirInst {
    use MirInstKind::*;
    let kind = match &i.kind {
        Const(c) => Const(c.clone()),
        Undef => Undef,
        Copy(v) => Copy(remap_op(*v, values, locals)),
        Store { local, value } => Store { local: locals.get(local).copied().unwrap_or(*local), value: remap_op(*value, values, locals) },
        Bin(op, a, b) => Bin(*op, remap_op(*a, values, locals), remap_op(*b, values, locals)),
        Un(op, a) => Un(*op, remap_op(*a, values, locals)),
        Call { callee, args } => Call { callee: callee.clone(), args: args.iter().map(|o| remap_op(*o, values, locals)).collect() },
        MethodCall { recv, name, args } => MethodCall { recv: remap_op(*recv, values, locals), name: name.clone(), args: args.iter().map(|o| remap_op(*o, values, locals)).collect() },
        Field { base, name } => Field { base: remap_op(*base, values, locals), name: name.clone() },
        StoreField { base, name, value } => StoreField { base: remap_op(*base, values, locals), name: name.clone(), value: remap_op(*value, values, locals) },
        Index { base, index } => Index { base: remap_op(*base, values, locals), index: remap_op(*index, values, locals) },
        StoreIndex { base, index, value } => StoreIndex { base: remap_op(*base, values, locals), index: remap_op(*index, values, locals), value: remap_op(*value, values, locals) },
        New { class, args } => New { class: class.clone(), args: args.iter().map(|o| remap_op(*o, values, locals)).collect() },
        ArrayNew { elem, len } => ArrayNew { elem: elem.clone(), len: remap_op(*len, values, locals) },
        ArrayLit { elem, values: vals } => ArrayLit { elem: elem.clone(), values: vals.iter().map(|o| remap_op(*o, values, locals)).collect() },
        Cast { value, target, checked } => Cast { value: remap_op(*value, values, locals), target: target.clone(), checked: *checked },
        Box { value, from } => Box { value: remap_op(*value, values, locals), from: from.clone() },
        Try { value } => Try { value: remap_op(*value, values, locals) },
        Interp { parts } => Interp { parts: parts.iter().map(|p| match p {
            super::MirInterpPart::Lit(x) => super::MirInterpPart::Lit(x.clone()),
            super::MirInterpPart::Value(o) => super::MirInterpPart::Value(remap_op(*o, values, locals)),
        }).collect() },
        Global(n) => Global(n.clone()),
        StaticRef(n) => StaticRef(n.clone()),
        StoreStatic { name, value } => StoreStatic { name: name.clone(), value: remap_op(*value, values, locals) },
        StorePtr { ptr, value } => StorePtr { ptr: remap_op(*ptr, values, locals), value: remap_op(*value, values, locals) },
        MatchTest { subject, pattern } => MatchTest { subject: remap_op(*subject, values, locals), pattern: pattern.clone() },
        BindPattern { subject, pattern, locals: ls } => BindPattern {
            subject: remap_op(*subject, values, locals), pattern: pattern.clone(),
            locals: ls.iter().map(|l| locals.get(l).copied().unwrap_or(*l)).collect(),
        },
        Safepoint { roots } => Safepoint { roots: roots.iter().map(|o| remap_op(*o, values, locals)).collect() },
        Opaque(t) => Opaque(t.clone()),
    };
    super::MirInst { dst: i.dst.map(|d| values.get(&d).copied().unwrap_or(d)), ty: i.ty.clone(), res: i.res.clone(), coerce: i.coerce, kind, line: i.line }
}

fn fold_constants(f: &mut MirFunction) {
    for _ in 0..2 {
        let mut cmap: HashMap<u32, MirConst> = HashMap::new();
        for b in &f.blocks {
            for i in &b.insts {
                if let (Some(d), MirInstKind::Const(c)) = (i.dst, &i.kind) {
                    cmap.insert(d, c.clone());
                }
            }
        }
        fold_with(f, &mut cmap);
    }
}

fn fold_with(f: &mut MirFunction, cmap: &mut HashMap<u32, MirConst>) {
    for b in &mut f.blocks {
        for i in &mut b.insts {
            let c = match &i.kind {
                MirInstKind::Bin(op, a, b) => {
                    match (const_of(a, cmap), const_of(b, cmap)) {
                        (Some(MirConst::Int(x)), Some(MirConst::Int(y))) => int_bin(*op, x, y, &i.ty),
                        (Some(MirConst::Float(x)), Some(MirConst::Float(y))) => float_bin(*op, x, y, &i.ty),
                        (Some(MirConst::Bool(x)), Some(MirConst::Bool(y))) => bool_bin(*op, x, y),
                        _ => None,
                    }
                }
                MirInstKind::Un(op, a) => match const_of(a, cmap) {
                    Some(MirConst::Int(x)) => int_un(*op, x, &i.ty),
                    Some(MirConst::Bool(x)) => bool_un(*op, x),
                    Some(MirConst::Float(x)) => float_un(*op, x, &i.ty),
                    _ => None,
                },
                _ => None,
            };
            if let Some(c) = c {
                if let Some(d) = i.dst {
                    cmap.insert(d, c.clone());
                }
                i.kind = MirInstKind::Const(c);
            }
        }
    }
}

fn const_of(o: &MirOperand, cmap: &HashMap<u32, MirConst>) -> Option<MirConst> {
    match o {
        MirOperand::Value(v) => cmap.get(v).cloned(),
        MirOperand::Local(_) => None,
    }
}

fn fits(ty: &Type, v: i128) -> bool {
    match ty {
        Type::I8 => i128::from(i8::MIN) <= v && v <= i128::from(i8::MAX),
        Type::I16 => i128::from(i16::MIN) <= v && v <= i128::from(i16::MAX),
        Type::I32 => i128::from(i32::MIN) <= v && v <= i128::from(i32::MAX),
        Type::I64 => i128::from(i64::MIN) <= v && v <= i128::from(i64::MAX),
        Type::U8 => 0 <= v && v <= i128::from(u8::MAX),
        Type::U16 => 0 <= v && v <= i128::from(u16::MAX),
        Type::U32 => 0 <= v && v <= i128::from(u32::MAX),
        Type::U64 => 0 <= v && v <= i128::from(u64::MAX),
        _ => false,
    }
}

fn int_bin(op: BinOp, x: i64, y: i64, ty: &Type) -> Option<MirConst> {
    match op {
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let b = match op {
                BinOp::Eq => x == y,
                BinOp::Ne => x != y,
                BinOp::Lt => x < y,
                BinOp::Le => x <= y,
                BinOp::Gt => x > y,
                BinOp::Ge => x >= y,
                _ => unreachable!(),
            };
            Some(MirConst::Bool(b))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul => {
            let v = match op {
                BinOp::Add => i128::from(x) + i128::from(y),
                BinOp::Sub => i128::from(x) - i128::from(y),
                BinOp::Mul => i128::from(x) * i128::from(y),
                _ => unreachable!(),
            };
            fits(ty, v).then(|| MirConst::Int(v as i64))
        }
        BinOp::Div | BinOp::Rem => {
            if y == 0 || (x == i64::MIN && y == -1) {
                return None;
            }
            let v = match op {
                BinOp::Div => i128::from(x / y),
                BinOp::Rem => i128::from(x % y),
                _ => unreachable!(),
            };
            fits(ty, v).then(|| MirConst::Int(v as i64))
        }
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            let v = match op {
                BinOp::BitAnd => x & y,
                BinOp::BitOr => x | y,
                BinOp::BitXor => x ^ y,
                _ => unreachable!(),
            };
            Some(MirConst::Int(v))
        }
        BinOp::Shl | BinOp::Shr => {
            let bits = match ty {
                Type::I8 | Type::U8 => 8,
                Type::I16 | Type::U16 => 16,
                Type::I32 | Type::U32 => 32,
                Type::I64 | Type::U64 => 64,
                _ => return None,
            };
            if !(0..bits).contains(&y) {
                return None;
            }
            let v = if op == BinOp::Shl {
                i128::from(x).checked_shl(y as u32)?
            } else {
                i128::from(x >> y)
            };
            fits(ty, v).then(|| MirConst::Int(v as i64))
        }
        BinOp::AndAnd | BinOp::OrOr => None,
    }
}

fn float_bin(op: BinOp, x: f64, y: f64, ty: &Type) -> Option<MirConst> {
    if !matches!(ty, Type::F64) {
        return None;
    }
    match op {
        BinOp::Add => Some(MirConst::Float(x + y)),
        BinOp::Sub => Some(MirConst::Float(x - y)),
        BinOp::Mul => Some(MirConst::Float(x * y)),
        BinOp::Div if y != 0.0 => Some(MirConst::Float(x / y)),
        _ => None,
    }
    .filter(|c| match c { MirConst::Float(v) => v.is_finite(), _ => true })
}

fn bool_bin(op: BinOp, x: bool, y: bool) -> Option<MirConst> {
    match op {
        BinOp::AndAnd => Some(MirConst::Bool(x && y)),
        BinOp::OrOr => Some(MirConst::Bool(x || y)),
        BinOp::Eq => Some(MirConst::Bool(x == y)),
        BinOp::Ne => Some(MirConst::Bool(x != y)),
        _ => None,
    }
}

fn int_un(op: crate::ast::UnOp, x: i64, ty: &Type) -> Option<MirConst> {
    match op {
        crate::ast::UnOp::Neg => {
            let v = -i128::from(x);
            fits(ty, v).then(|| MirConst::Int(v as i64))
        }
        crate::ast::UnOp::BitNot => {
            if matches!(ty, Type::I8 | Type::I16 | Type::I32 | Type::I64) {
                Some(MirConst::Int(!x))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn bool_un(op: crate::ast::UnOp, x: bool) -> Option<MirConst> {
    match op {
        crate::ast::UnOp::Not => Some(MirConst::Bool(!x)),
        _ => None,
    }
}

fn float_un(op: crate::ast::UnOp, x: f64, ty: &Type) -> Option<MirConst> {
    if !matches!(ty, Type::F64) {
        return None;
    }
    match op {
        crate::ast::UnOp::Neg => Some(MirConst::Float(-x)),
        _ => None,
    }
}

fn pure(i: &super::MirInst) -> bool {
    matches!(
        i.kind,
        MirInstKind::Const(_)
            | MirInstKind::Copy(_)
            | MirInstKind::Undef
            | MirInstKind::Bin(..)
            | MirInstKind::Un(..)
            | MirInstKind::Field { .. }
            | MirInstKind::Index { .. }
            | MirInstKind::Cast { .. }
            | MirInstKind::Box { .. }
            | MirInstKind::Try { .. }
            | MirInstKind::Global(_)
            | MirInstKind::StaticRef(_)
            | MirInstKind::MatchTest { .. }
    )
}

fn eliminate_dead(f: &mut MirFunction) {
    let mut used: HashSet<u32> = HashSet::new();
    for b in &f.blocks {
        for i in &b.insts {
            for o in operands(&i.kind) {
                if let MirOperand::Value(v) = o {
                    used.insert(v);
                }
            }
        }
        if let Some(MirTerm::Return(Some(o)) | MirTerm::Branch { cond: o, .. }) = &b.term {
            if let MirOperand::Value(v) = o {
                used.insert(*v);
            }
        }
    }
    for b in &mut f.blocks {
        b.insts.retain(|i| match i.dst {
            None => true,
            Some(d) => !pure(i) || used.contains(&d),
        });
    }
}

fn operands(k: &MirInstKind) -> Vec<MirOperand> {
    use MirInstKind::*;
    match k {
        Const(_) | Undef | Global(_) | StaticRef(_) | Opaque(_) => vec![],
        Copy(v) => vec![*v],
        Store { value, .. } => vec![*value],
        Bin(_, a, b) => vec![*a, *b],
        Un(_, a) => vec![*a],
        Call { args, .. } => args.clone(),
        MethodCall { recv, args, .. } => std::iter::once(*recv).chain(args.iter().copied()).collect(),
        Field { base, .. } => vec![*base],
        StoreField { base, value, .. } => vec![*base, *value],
        Index { base, index } => vec![*base, *index],
        StoreIndex { base, index, value } => vec![*base, *index, *value],
        New { args, .. } => args.clone(),
        ArrayNew { len, .. } => vec![*len],
        ArrayLit { values, .. } => values.clone(),
        Cast { value, .. } | Try { value } => vec![*value],
        Box { value, .. } => vec![*value],
        Interp { parts } => parts.iter().filter_map(|p| match p {
            super::MirInterpPart::Value(o) => Some(*o),
            _ => None,
        }).collect(),
        StoreStatic { value, .. } => vec![*value],
        StorePtr { ptr, value } => vec![*ptr, *value],
        MatchTest { subject, .. } => vec![*subject],
        BindPattern { subject, .. } => vec![*subject],
        Safepoint { roots } => roots.clone(),
    }
}

fn prune_unreachable(f: &mut MirFunction) {
    if f.blocks.is_empty() {
        return;
    }
    let mut labels: HashMap<&str, usize> = HashMap::new();
    for (i, b) in f.blocks.iter().enumerate() {
        labels.insert(b.label.as_str(), i);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        if !seen.insert(i) {
            continue;
        }
        let next: Vec<usize> = match &f.blocks[i].term {
            Some(MirTerm::Goto(t)) => labels.get(t.as_str()).copied().into_iter().collect(),
            Some(MirTerm::Branch { then_label, else_label, .. }) => {
                [then_label, else_label]
                    .iter()
                    .filter_map(|t| labels.get(t.as_str()).copied())
                    .collect()
            }
            _ => Vec::new(),
        };
        stack.extend(next);
    }
    let mut keep = vec![false; f.blocks.len()];
    for i in seen {
        keep[i] = true;
    }
    let mut idx = 0;
    f.blocks.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Res, Type};
    use crate::mir::{MirBlock, MirInst, MirLocal};

    fn block(label: &str, term: MirTerm) -> MirBlock {
        MirBlock { label: label.into(), insts: Vec::new(), term: Some(term) }
    }

    #[test]
    fn removes_unreachable_blocks() {
        let mut p = MirProgram { funcs: Vec::new(), classes: Vec::new(), enums: Vec::new(), ifaces: Vec::new() };
        p.funcs.push(MirFunction {
            name: "main".into(),
            decl_id: 0,
            owner: None,
            is_static: false,
            is_ctor: false,
            params: Vec::<MirLocal>::new(),
            locals: Vec::new(),
            ret: Type::I32,
            blocks: vec![
                block("entry", MirTerm::Return(None)),
                block("dead", MirTerm::Return(None)),
            ],
        });
        simplify(&mut p);
        assert_eq!(p.funcs[0].blocks.len(), 1);
        assert_eq!(p.funcs[0].blocks[0].label, "entry");
    }

    fn inst(dst: u32, kind: MirInstKind, ty: Type) -> MirInst {
        MirInst { dst: Some(dst), ty, res: Res::None, coerce: None, kind, line: 0 }
    }

    #[test]
    fn folds_constants_and_drops_dead_values() {
        let mut p = MirProgram { funcs: Vec::new(), classes: Vec::new(), enums: Vec::new(), ifaces: Vec::new() };
        let b = MirBlock {
            label: "entry".into(),
            insts: vec![
                inst(1, MirInstKind::Const(MirConst::Int(2)), Type::I32),
                inst(2, MirInstKind::Const(MirConst::Int(3)), Type::I32),
                inst(3, MirInstKind::Bin(BinOp::Add, MirOperand::Value(1), MirOperand::Value(2)), Type::I32),
                inst(9, MirInstKind::Const(MirConst::Int(99)), Type::I32),
            ],
            term: Some(MirTerm::Return(Some(MirOperand::Value(3)))),
        };
        p.funcs.push(MirFunction {
            name: "f".into(), decl_id: 0, owner: None, is_static: false, is_ctor: false,
            params: Vec::new(), locals: Vec::new(), ret: Type::Void,
            blocks: vec![b],
        });
        simplify(&mut p);
        let kinds: Vec<&MirInstKind> = p.funcs[0].blocks[0].insts.iter().map(|i| &i.kind).collect();
        assert_eq!(kinds.len(), 1, "folded inputs and dead constant should be removed");
        assert!(matches!(kinds[0], MirInstKind::Const(MirConst::Int(5))));
    }

    #[test]
    fn inlines_small_function_with_return_copy() {
        let mut p = MirProgram { funcs: Vec::new(), classes: Vec::new(), enums: Vec::new(), ifaces: Vec::new() };
        let callee = MirFunction {
            name: "mod::inc".into(), decl_id: 0, owner: None, is_static: false, is_ctor: false,
            params: vec![MirLocal { name: "x".into(), ty: Type::I32, mutable: false }],
            locals: vec![MirLocal { name: "x".into(), ty: Type::I32, mutable: false }],
            ret: Type::I32,
            blocks: vec![MirBlock {
                label: "entry".into(),
                insts: vec![
                    inst(1, MirInstKind::Const(MirConst::Int(1)), Type::I32),
                    inst(2, MirInstKind::Bin(BinOp::Add, MirOperand::Local(0), MirOperand::Value(1)), Type::I32),
                ],
                term: Some(MirTerm::Return(Some(MirOperand::Value(2)))),
            }],
        };
        let caller = MirFunction {
            name: "mod::main".into(), decl_id: 1, owner: None, is_static: false, is_ctor: false,
            params: Vec::new(), locals: Vec::new(), ret: Type::I32,
            blocks: vec![MirBlock {
                label: "entry".into(),
                insts: vec![
                    inst(1, MirInstKind::Const(MirConst::Int(2)), Type::I32),
                    super::super::MirInst {
                        dst: Some(2), ty: Type::I32, res: Res::Fn(0), coerce: None, line: 0,
                        kind: MirInstKind::Call { callee: "mod::inc".into(), args: vec![MirOperand::Value(1)] },
                    },
                ],
                term: Some(MirTerm::Return(Some(MirOperand::Value(2)))),
            }],
        };
        p.funcs.push(callee);
        p.funcs.push(caller);
        simplify(&mut p);
        let all: Vec<&MirInstKind> = p.funcs[1].blocks.iter().flat_map(|b| &b.insts).map(|i| &i.kind).collect();
        assert!(!all.iter().any(|k| matches!(k, MirInstKind::Call { .. })), "call should be inlined");
        assert!(all.iter().any(|k| matches!(k, MirInstKind::Copy(_))), "return should become copy");
    }
}

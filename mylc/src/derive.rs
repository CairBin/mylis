//! 编译期反射：内置派生。
//!
//! 当前只实现 `@derive(ToString)`：编译器读取 class 的实例字段，在
//! 编译期生成 `pub fn toString() -> string`，不保留任何运行时类型表。
//! 生成代码使用字符串插值，因此字段类型必须能按语言规则插入字符串。

use crate::ast::{AnnotArg, Decl, DeclKind};
use crate::diag::{Diag, E_GENERAL};
use crate::parser::Parser;

pub fn expand(d: &mut Decl, path: &str, diags: &mut Diag) {
    let mut derives: Vec<(String, u32, u32)> = Vec::new();
    for a in &d.annots {
        if a.name != "derive" { continue; }
        if a.args.is_empty() {
            diags.err(E_GENERAL, path, a.line, a.col, "`@derive` 至少需要一个派生名");
            continue;
        }
        for arg in &a.args {
            match arg {
                AnnotArg::Str(s) | AnnotArg::Ident(s) => derives.push((s.clone(), a.line, a.col)),
                _ => diags.err(
                    E_GENERAL,
                    path,
                    a.line,
                    a.col,
                    "`@derive` 的参数必须是派生名，例如 `@derive(ToString)`",
                ),
            }
        }
    }
    for (name, line, col) in derives {
        match name.as_str() {
            "ToString" => derive_to_string(d, path, line, col, diags),
            _ => diags.err(
                E_GENERAL,
                path,
                line,
                col,
                format!("未知或尚未实现的派生 `{}`（当前支持 ToString）", name),
            ),
        }
    }
}

fn derive_to_string(d: &mut Decl, path: &str, line: u32, col: u32, diags: &mut Diag) {
    if d.kind != DeclKind::Class {
        diags.err(E_GENERAL, path, line, col, "`@derive(ToString)` 只能用于 class");
        return;
    }
    if !d.type_params.is_empty() {
        diags.err(
            E_GENERAL,
            path,
            line,
            col,
            "`@derive(ToString)` 暂不支持泛型 class（字段类型 T 的字符串化尚未打通）",
        );
        return;
    }
    if d.member_asts
        .iter()
        .any(|m| m.kind == DeclKind::Fn && matches!(m.name.as_str(), "toString" | "to_string"))
    {
        diags.err(
            E_GENERAL,
            path,
            line,
            col,
            format!("class `{}` 已经定义了 toString，不能再派生 ToString", d.name),
        );
        return;
    }

    let fields: Vec<&crate::ast::Field> = d.fields.iter().filter(|f| !f.is_static).collect();
    let mut msg = format!("{} {{{{", d.name);
    for (i, f) in fields.iter().enumerate() {
        if i == 0 { msg.push(' '); } else { msg.push_str(", "); }
        let field_expr = match &f.ty {
            // class/enum 字段显式调用 toString，避免插值在字段类型上再走一次
            // 接口转换；primitive/string/类型参数继续使用内建插值。
            crate::ast::Type::Named(n) if !d.type_params.iter().any(|p| p == n) => {
                format!("{{this.{}.toString()}}", f.name)
            }
            _ => format!("{{this.{}}}", f.name),
        };
        msg.push_str(&format!("{}: {}", f.name, field_expr));
    }
    if fields.is_empty() {
        msg.push_str(" }}");
    } else {
        msg.push(' ');
        msg.push_str("}}");
    }

    let src = format!(
        "class __MylisDerive {{\n    pub fn toString() -> string {{\n        return \"{}\";\n    }}\n}}\n",
        msg
    );
    let mut p = Parser::new(path, &src, diags);
    let module = p.parse_module();
    let method = module.decls.into_iter().find_map(|c| {
        if c.kind == DeclKind::Class && c.name == "__MylisDerive" {
            c.member_asts
                .into_iter()
                .find(|m| m.kind == DeclKind::Fn && m.name == "toString")
        } else {
            None
        }
    });
    let Some(mut m) = method else {
        return;
    };
    m.line = line;
    m.col = col;
    d.member_asts.push(m);
}

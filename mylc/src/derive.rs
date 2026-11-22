//! 编译期反射：内置派生。
//!
//! 当前实现 `@derive(ToString)` 与 `@derive(Json)`：编译器读取 class 的实例字段，
//! 在编译期生成 `pub fn toString() -> string` / `pub fn toJson() -> Json`，
//! 不保留任何运行时类型表。primitive / string / class / 数组字段使用字符串插值；
//! `Option<T>` 生成 Some/None 分支，`Vec<T>` 生成逐元素拼接，均不依赖运行时反射。

use crate::ast::{AnnotArg, Decl, DeclKind, Type};
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
            "Json" => derive_json(d, path, line, col, diags),
            _ => diags.err(
                E_GENERAL,
                path,
                line,
                col,
                format!("未知或尚未实现的派生 `{}`（当前支持 ToString / Json）", name),
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
    let mut body = format!("        string __mylis_s = \"{} {{{{\";\n", d.name);
    for (i, f) in fields.iter().enumerate() {
        if i == 0 {
            body.push_str("        __mylis_s = __mylis_s + \" \";\n");
        } else {
            body.push_str("        __mylis_s = __mylis_s + \", \";\n");
        }
        body.push_str(&format!(
            "        __mylis_s = __mylis_s + \"{}: \";\n",
            f.name
        ));
        match &f.ty {
            Type::Option(_) => {
                body.push_str(&format!(
                    "        if (this.{}.is_some()) {{\n",
                    f.name
                ));
                body.push_str(&format!(
                    "            __mylis_s = __mylis_s + \"Some({{this.{}.unwrap()}})\";\n",
                    f.name
                ));
                body.push_str("        } else {\n");
                body.push_str("            __mylis_s = __mylis_s + \"None\";\n");
                body.push_str("        }\n");
            }
            Type::Generic(name, _) if name.rsplit("::").next() == Some("Vec") => {
                body.push_str("        __mylis_s = __mylis_s + \"[\";\n");
                body.push_str(&format!("        i64 __mylis_i_{} = 0;\n", i));
                body.push_str(&format!(
                    "        while (__mylis_i_{i} < this.{}.len()) {{\n",
                    f.name,
                    i = i
                ));
                body.push_str(&format!(
                    "            if (__mylis_i_{i} > 0) {{ __mylis_s = __mylis_s + \", \"; }}\n",
                    i = i
                ));
                body.push_str(&format!(
                    "            __mylis_s = __mylis_s + \"{{this.{}.get(__mylis_i_{i})}}\";\n",
                    f.name,
                    i = i
                ));
                body.push_str(&format!(
                    "            __mylis_i_{i} = __mylis_i_{i} + 1;\n",
                    i = i
                ));
                body.push_str("        }\n");
                body.push_str("        __mylis_s = __mylis_s + \"]\";\n");
            }
            _ => {
                body.push_str(&format!(
                    "        __mylis_s = __mylis_s + \"{{this.{}}}\";\n",
                    f.name
                ));
            }
        }
    }
    body.push_str("        __mylis_s = __mylis_s + \" }\";\n");
    body.push_str("        return __mylis_s;\n");

    let src = format!(
        "class __MylisDerive {{\n    pub fn toString() -> string {{\n{body}    }}\n}}\n"
    );
    let Some(mut m) = parse_generated_method(&src, path, diags, "toString", line, col) else {
        return;
    };
    m.line = line;
    m.col = col;
    d.member_asts.push(m);
}

/// 解析一段临时 class 源码，取出其中的方法声明；行号/列号回填到注解位置。
fn parse_generated_method(
    src: &str,
    path: &str,
    diags: &mut Diag,
    method_name: &str,
    line: u32,
    col: u32,
) -> Option<Decl> {
    let mut p = Parser::new(path, src, diags);
    let module = p.parse_module();
    let mut method = module.decls.into_iter().find_map(|c| {
        if c.kind == DeclKind::Class && c.name == "__MylisDerive" {
            c.member_asts
                .into_iter()
                .find(|m| m.kind == DeclKind::Fn && m.name == method_name)
        } else {
            None
        }
    });
    if let Some(m) = method.as_mut() {
        m.line = line;
        m.col = col;
    }
    method
}

/// 生成 `toJson`：对象字段顺序与 class 声明顺序一致，值通过 `std/Json` 构造。
///
/// JSON 值生成是递归的：`Option` 生成 null/内层值，`Vec`/数组逐元素生成，
/// `Map<string, T>` 逐键生成。每个容器使用独立的临时变量，因此任意层嵌套
/// 都不会共享循环变量。
fn derive_json(d: &mut Decl, path: &str, line: u32, col: u32, diags: &mut Diag) {
    if d.kind != DeclKind::Class {
        diags.err(E_GENERAL, path, line, col, "`@derive(Json)` 只能用于 class");
        return;
    }
    if !d.type_params.is_empty() {
        diags.err(
            E_GENERAL,
            path,
            line,
            col,
            "`@derive(Json)` 暂不支持泛型 class",
        );
        return;
    }
    if d.member_asts
        .iter()
        .any(|m| m.kind == DeclKind::Fn && m.name == "toJson")
    {
        diags.err(
            E_GENERAL,
            path,
            line,
            col,
            format!("class `{}` 已经定义了 toJson，不能再派生 Json", d.name),
        );
        return;
    }

    let fields: Vec<&crate::ast::Field> = d.fields.iter().filter(|f| !f.is_static).collect();
    let mut body = String::new();
    let mut counter = 0usize;
    body.push_str("        Json __mylis_obj = Json::Null();\n");
    body.push_str("        __mylis_obj.kind = 6;\n");
    for f in fields.iter() {
        let out = fresh_name("v", &mut counter);
        body.push_str(&format!("        Json {out} = Json::Null();\n"));
        if !emit_json_value(&f.ty, &format!("this.{}", f.name), &out, &mut counter, 8, &mut body) {
            diags.err(
                E_GENERAL,
                path,
                line,
                col,
                format!(
                    "`@derive(Json)` 暂不支持字段 `{}` 的类型（当前支持 primitive / string / class / 数组 / Option / Vec / Map<string, T>，容器可嵌套）",
                    f.name
                ),
            );
            return;
        }
        body.push_str(&format!(
            "        __mylis_obj.objectValue.set(\"{}\", {out});\n",
            f.name
        ));
    }
    body.push_str("        return __mylis_obj;\n");

    let src =
        format!("class __MylisDerive {{\n    pub fn toJson() -> Json {{\n{body}    }}\n}}\n");
    let Some(mut m) = parse_generated_method(&src, path, diags, "toJson", line, col) else {
        return;
    };
    m.line = line;
    m.col = col;
    d.member_asts.push(m);
}

fn fresh_name(prefix: &str, counter: &mut usize) -> String {
    let id = *counter;
    *counter += 1;
    format!("__mylis_{prefix}_{id}")
}

fn is_string_type(ty: &Type) -> bool {
    matches!(ty, Type::Str) || matches!(ty, Type::Named(name) if name == "string")
}

fn json_scalar_expr(ty: &Type, expr: &str) -> Option<String> {
    let out = match ty {
        Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::U8 | Type::U16 | Type::U32
        | Type::U64 => format!("Json::Int({expr} as i64)"),
        Type::F32 | Type::F64 => format!("Json::Float({expr} as f64)"),
        Type::Bool => format!("Json::Bool({expr})"),
        Type::Str => format!("Json::Str({expr})"),
        // 解析前字段类型通常还是 Named；class 统一走递归 toJson。enum 先按字符串输出。
        Type::Named(name) => {
            if name == "string" {
                format!("Json::Str({expr})")
            } else if name == "bool" {
                format!("Json::Bool({expr})")
            } else {
                format!("{expr}.toJson()")
            }
        }
        Type::Class(_) | Type::Adt { .. } => format!("{expr}.toJson()"),
        Type::Enum(_) => format!("Json::Str(\"{{{expr}}}\")"),
        _ => return None,
    };
    Some(out)
}

/// 生成把 `expr` 转成 `out`（一个 Json 临时变量）的语句。
/// `indent` 是当前语句块的缩进宽度；返回 false 表示类型不受支持。
fn emit_json_value(
    ty: &Type,
    expr: &str,
    out: &str,
    counter: &mut usize,
    indent: usize,
    body: &mut String,
) -> bool {
    let pad = " ".repeat(indent);
    if let Some(scalar) = json_scalar_expr(ty, expr) {
        body.push_str(&format!("{pad}{out} = {scalar};\n"));
        return true;
    }

    match ty {
        Type::Option(inner) => {
            body.push_str(&format!("{pad}if ({expr}.is_some()) {{\n"));
            if !emit_json_value(
                inner,
                &format!("{expr}.unwrap()"),
                out,
                counter,
                indent + 4,
                body,
            ) {
                return false;
            }
            body.push_str(&format!("{pad}}} else {{\n"));
            body.push_str(&format!("{pad}    {out} = Json::Null();\n"));
            body.push_str(&format!("{pad}}}\n"));
            true
        }
        Type::Array(inner) => {
            let idx = fresh_name("i", counter);
            let elem = fresh_name("v", counter);
            body.push_str(&format!("{pad}{out}.kind = 5;\n"));
            body.push_str(&format!("{pad}i64 {idx} = 0;\n"));
            body.push_str(&format!(
                "{pad}while ({idx} < ({expr}.len() as i64)) {{\n"
            ));
            body.push_str(&format!(
                "{pad}    Json {elem} = Json::Null();\n"
            ));
            if !emit_json_value(
                inner,
                &format!("{expr}[{idx}]"),
                &elem,
                counter,
                indent + 4,
                body,
            ) {
                return false;
            }
            body.push_str(&format!(
                "{pad}    {out}.arrayValue.push({elem});\n"
            ));
            body.push_str(&format!("{pad}    {idx} = {idx} + 1;\n"));
            body.push_str(&format!("{pad}}}\n"));
            true
        }
        Type::Generic(name, args)
            if name.rsplit("::").next() == Some("Vec") && args.len() == 1 =>
        {
            let idx = fresh_name("i", counter);
            let elem = fresh_name("v", counter);
            body.push_str(&format!("{pad}{out}.kind = 5;\n"));
            body.push_str(&format!("{pad}i64 {idx} = 0;\n"));
            body.push_str(&format!(
                "{pad}while ({idx} < ({expr}.len() as i64)) {{\n"
            ));
            body.push_str(&format!(
                "{pad}    Json {elem} = Json::Null();\n"
            ));
            if !emit_json_value(
                &args[0],
                &format!("{expr}.get({idx})"),
                &elem,
                counter,
                indent + 4,
                body,
            ) {
                return false;
            }
            body.push_str(&format!(
                "{pad}    {out}.arrayValue.push({elem});\n"
            ));
            body.push_str(&format!("{pad}    {idx} = {idx} + 1;\n"));
            body.push_str(&format!("{pad}}}\n"));
            true
        }
        Type::Generic(name, args)
            if name.rsplit("::").next() == Some("Map") && args.len() == 2 =>
        {
            if !is_string_type(&args[0]) {
                return false;
            }
            let idx = fresh_name("i", counter);
            let key = fresh_name("k", counter);
            let elem = fresh_name("v", counter);
            body.push_str(&format!("{pad}{out}.kind = 6;\n"));
            body.push_str(&format!("{pad}i64 {idx} = 0;\n"));
            body.push_str(&format!(
                "{pad}while ({idx} < {expr}.count) {{\n"
            ));
            body.push_str(&format!(
                "{pad}    string {key} = {expr}.keys[{idx}];\n"
            ));
            body.push_str(&format!(
                "{pad}    Json {elem} = Json::Null();\n"
            ));
            if !emit_json_value(
                &args[1],
                &format!("{expr}.values[{idx}]"),
                &elem,
                counter,
                indent + 4,
                body,
            ) {
                return false;
            }
            body.push_str(&format!(
                "{pad}    {out}.objectValue.set({key}, {elem});\n"
            ));
            body.push_str(&format!("{pad}    {idx} = {idx} + 1;\n"));
            body.push_str(&format!("{pad}}}\n"));
            true
        }
        _ => false,
    }
}

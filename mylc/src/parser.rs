//! 语法分析：递归下降 + 优先级爬升，输出 AST。

use crate::ast::*;
use crate::diag::{Diag, E_SYNTAX};
use crate::lexer::{tok_desc, Lexer, Tok, Token};

pub struct ImportAst {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub is_pub: bool,
    /// `import { A, B } from "x"` 的显式导入名；空表示只导入模块，不做裸名绑定。
    pub names: Vec<String>,
    pub selective: bool,
}

pub struct ModuleAst {
    pub decls: Vec<Decl>,
    pub imports: Vec<ImportAst>,
}

pub struct Parser<'a> {
    file: &'a str,
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    diags: &'a mut Diag,
    quiet: usize,
}

impl<'a> Parser<'a> {
    pub fn new(file: &'a str, src: &'a str, diags: &'a mut Diag) -> Self {
        let (toks, lex_errors) = Lexer::new(file, src).tokenize();
        for (l, c, m) in lex_errors {
            diags.err(crate::diag::E_LEX, file, l, c, m);
        }
        Parser {
            file,
            src,
            toks,
            pos: 0,
            diags,
            quiet: 0,
        }
    }

    // ------------------------------------------------------------ token 基元
    fn tok(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn kind(&self) -> Tok {
        self.tok().tok
    }

    fn nth(&self, n: usize) -> Tok {
        self.toks
            .get(self.pos + n)
            .map(|t| t.tok)
            .unwrap_or(Tok::Eof)
    }

    fn line(&self) -> u32 {
        self.tok().line
    }

    fn col(&self) -> u32 {
        self.tok().col
    }

    fn bump(&mut self) -> Token {
        let t = self.tok().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at(&self, t: Tok) -> bool {
        self.kind() == t
    }

    fn eat(&mut self, t: Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error(&mut self, line: u32, col: u32, msg: impl Into<String>) {
        if self.quiet == 0 {
            self.diags.err(E_SYNTAX, self.file, line, col, msg);
        }
    }

    fn expect(&mut self, t: Tok, ctx: &str) -> Option<Token> {
        if self.at(t) {
            Some(self.bump())
        } else {
            let (l, c) = (self.line(), self.col());
            self.error(
                l,
                c,
                format!("{ctx}：期望 {}，实际是 {}", tok_desc(t), tok_desc(self.kind())),
            );
            None
        }
    }

    fn expect_ident(&mut self, ctx: &str) -> Option<Token> {
        if self.at(Tok::Ident) {
            Some(self.bump())
        } else {
            let (l, c) = (self.line(), self.col());
            self.error(
                l,
                c,
                format!("{ctx}：期望标识符，实际是 {}", tok_desc(self.kind())),
            );
            None
        }
    }

    /// 顶层/类体出错时跳过一个 token，避免连锁报错
    fn recover(&mut self) {
        if !self.at(Tok::Eof) {
            self.bump();
        }
    }

    // ============================================================== 模块
    pub fn parse_module(&mut self) -> ModuleAst {
        let mut decls = Vec::new();
        let mut imports = Vec::new();
        while !self.at(Tok::Eof) {
            // `pub import "X.myl";` —— 再导出（对应 Rust 的 `pub use`）
            let mut is_pub_import = false;
            if self.at(Tok::KwPub) && self.nth(1) == Tok::KwImport {
                self.bump(); // 吃掉 pub
                is_pub_import = true;
            }
            if self.at(Tok::KwImport) {
                self.bump();
                let mut names = Vec::new();
                let mut selective = false;
                if self.at(Tok::LBrace) {
                    selective = true;
                    self.bump();
                    while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
                        let name = self.bump();
                        if name.tok != Tok::Ident {
                            self.error(name.line, name.col, "import { } 中需要名字");
                        } else {
                            names.push(name.text.clone());
                        }
                        if self.at(Tok::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    self.expect(Tok::RBrace, "import { }");
                    let from = self.bump();
                    if from.tok != Tok::Ident || from.text != "from" {
                        self.error(from.line, from.col, "import { } 后需要 from");
                    }
                }
                let path = self.bump();
                if path.tok != Tok::StrLit {
                    self.error(path.line, path.col, "import 后需要字符串路径字面量");
                } else {
                    imports.push(ImportAst {
                        path: path.text.clone(),
                        line: path.line,
                        col: path.col,
                        is_pub: is_pub_import,
                        names,
                        selective,
                    });
                }
                self.expect(Tok::Semi, "import 语句");
                continue;
            }
            let before = self.pos;
            if let Some(d) = self.parse_top_decl() {
                decls.push(d);
            } else {
                self.recover();
            }
            if self.pos == before {
                self.recover();
            }
        }
        ModuleAst { decls, imports }
    }

    fn source_span(&self, start_line: u32, start_col: u32) -> String {
        let start = self.source_offset(start_line, start_col);
        let end = self.source_offset(self.line(), self.col());
        if end <= start { return String::new(); }
        self.src.get(start..end).unwrap_or("").trim().to_string()
    }

    fn source_offset(&self, line: u32, col: u32) -> usize {
        let mut off = 0usize;
        for (i, text) in self.src.split('\n').enumerate() {
            if i + 1 == line as usize {
                let mut chars = text.char_indices();
                for _ in 1..col {
                    if chars.next().is_none() { break; }
                }
                return off + chars.next().map(|(b, _)| b).unwrap_or(text.len());
            }
            off += text.len() + 1;
        }
        self.src.len()
    }

    fn parse_top_decl(&mut self) -> Option<Decl> {
        let sl = self.line();
        let sc = self.col();
        let mut d = self.parse_top_decl_inner();
        if let Some(x) = &mut d {
            if !x.type_params.is_empty() {
                x.source = Some(self.source_span(sl, sc));
            }
        }
        d
    }

    fn parse_top_decl_inner(&mut self) -> Option<Decl> {
        let annots = self.parse_annots();
        let mut is_pub = false;
        if self.at(Tok::KwPub) {
            self.bump();
            is_pub = true;
        }
        let (line, col) = (self.line(), self.col());
        // 上下文关键字 `type A = B;`（`type` 不作全局关键字，避免破坏字段/变量名）
        if matches!(self.kind(), Tok::Ident) && self.tok().text == "type" {
            self.bump();
            let name = match self.expect_ident("类型别名") {
                Some(t) => t.text,
                None => return None,
            };
            let mut d = Decl::new(DeclKind::Alias, name, is_pub, 0, line, col);
            d.annots = annots;
            if self.expect(Tok::Assign, "别名后的 =").is_none() {
                return Some(d);
            }
            d.alias_target = self.parse_type();
            self.expect(Tok::Semi, "类型别名末尾的 ;");
            return Some(d);
        }
        match self.kind() {
            Tok::KwClass => self.parse_class(is_pub, annots),
            Tok::KwInterface => self.parse_iface(is_pub, annots),
            Tok::KwEnum => self.parse_enum(is_pub, annots),
            Tok::KwConst => self.parse_const(is_pub, annots),
            Tok::KwFn => self.parse_fn(is_pub, annots, false, false),
            Tok::KwUnsafe => {
                self.bump();
                if self.at(Tok::KwFn) {
                    self.parse_fn(is_pub, annots, true, false)
                } else if self.at(Tok::LBrace) {
                    self.error(line, col, "顶层 unsafe 只允许出现在 fn 或语句块中");
                    None
                } else {
                    self.error(line, col, "unsafe 后需要 fn");
                    None
                }
            }
            Tok::KwImport => {
                self.error(line, col, "import 只能出现在文件顶层且不能在声明之后重复使用");
                None
            }
            _ => {
                self.error(
                    line,
                    col,
                    format!("期望顶层声明（class/interface/enum/fn/const），实际是 {}", tok_desc(self.kind())),
                );
                None
            }
        }
    }

    // ============================================================== 注解
    fn parse_annots(&mut self) -> Vec<Annot> {
        let mut out = Vec::new();
        while self.at(Tok::At) {
            let (line, col) = (self.line(), self.col());
            self.bump();
            let name = match self.expect_ident("注解名") {
                Some(t) => t.text,
                None => continue,
            };
            let mut args = Vec::new();
            if self.eat(Tok::LParen) {
                if !self.at(Tok::RParen) {
                    loop {
                        let t = self.bump();
                        match t.tok {
                            Tok::Int => {
                                if t.uval > i64::MAX as u64 {
                                    self.error(t.line, t.col, "注解整型参数超出 i64 取值范围");
                                } else {
                                    args.push(AnnotArg::Int(t.ival));
                                }
                            }
                            Tok::Float => args.push(AnnotArg::Float(t.fval)),
                            Tok::StrLit => args.push(AnnotArg::Str(t.text.clone())),
                            Tok::Ident => args.push(AnnotArg::Ident(t.text.clone())),
                            Tok::KwTrue => args.push(AnnotArg::Bool(true)),
                            Tok::KwFalse => args.push(AnnotArg::Bool(false)),
                            _ => self.error(
                                t.line,
                                t.col,
                                "注解参数只允许常量字面量或标识符（int/float/string/bool/name）",
                            ),
                        }
                        if self.eat(Tok::Comma) {
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RParen, "注解参数列表");
            }
            out.push(Annot {
                name,
                args,
                line,
                col,
            });
        }
        out
    }

    // ============================================================== 类型
    fn parse_type(&mut self) -> Type {
        let mut t = self.parse_type_atom();
        loop {
            if self.at(Tok::Star) {
                self.bump();
                t = Type::ptr(t);
            } else if self.at(Tok::LBracket) {
                self.bump();
                if self.expect(Tok::RBracket, "数组类型后缀").is_none() {
                    break;
                }
                t = Type::array(t);
            } else {
                break;
            }
        }
        t
    }

    fn parse_type_atom(&mut self) -> Type {
        let (line, col) = (self.line(), self.col());
        if self.at(Tok::KwVoid) {
            self.bump();
            return Type::Void;
        }
        let mut name = match self.expect_ident("类型名") {
            Some(t) => t.text,
            None => return Type::Named("<error>".to_string()),
        };
        while self.at(Tok::ColonColon) {
            self.bump();
            match self.expect_ident("限定类型名") {
                Some(t) => {
                    name.push_str("::");
                    name.push_str(&t.text);
                }
                None => break,
            }
        }
        let prim = match name.as_str() {
            "i8" => Some(Type::I8),
            "i16" => Some(Type::I16),
            "i32" => Some(Type::I32),
            "i64" => Some(Type::I64),
            "u8" => Some(Type::U8),
            "u16" => Some(Type::U16),
            "u32" => Some(Type::U32),
            "u64" => Some(Type::U64),
            "f32" => Some(Type::F32),
            "f64" => Some(Type::F64),
            "bool" => Some(Type::Bool),
            "string" => Some(Type::Str),
            _ => None,
        };
        if let Some(p) = prim {
            return p;
        }
        if name == "Result" && self.at(Tok::Lt) {
            self.bump();
            let a = self.parse_type();
            self.expect(Tok::Comma, "Result<T, E> 类型参数");
            let b = self.parse_type();
            self.close_angle(line, col, "Result<T, E>");
            return Type::result(a, b);
        }
        if name == "Option" && self.at(Tok::Lt) {
            self.bump();
            let a = self.parse_type();
            self.close_angle(line, col, "Option<T>");
            return Type::option(a);
        }
        if name == "Result" || name == "Option" {
            self.error(
                line,
                col,
                format!("{} 需要类型参数，如 {}<...>", name, name),
            );
        }
        if self.at(Tok::Lt) {
            let args = self.parse_type_args("泛型类型");
            return Type::Generic(name, args);
        }
        Type::Named(name)
    }

    /// 关闭尖括号，必要时把 `>>` 拆成两个 `>`
    fn close_angle(&mut self, line: u32, col: u32, ctx: &str) {
        if self.at(Tok::Gt) {
            self.bump();
            return;
        }
        if self.at(Tok::Shr) {
            let mut first = self.toks[self.pos].clone();
            first.tok = Tok::Gt;
            let mut second = self.toks[self.pos].clone();
            second.tok = Tok::Gt;
            second.col += 1;
            self.toks[self.pos] = first;
            self.toks.insert(self.pos + 1, second);
            self.bump();
            return;
        }
        self.error(
            line,
            col,
            format!("{ctx} 未闭合：期望 '>'，实际是 {}", tok_desc(self.kind())),
        );
    }

    fn parse_type_params(&mut self) -> (Vec<String>, Vec<(String, Vec<String>)>) {
        let mut names = Vec::new();
        let mut bounds = Vec::new();
        if !self.at(Tok::Lt) { return (names, bounds); }
        let (line, col) = (self.line(), self.col());
        self.bump();
        loop {
            let name = match self.expect_ident("类型参数名") {
                Some(t) => t.text,
                None => break,
            };
            names.push(name.clone());
            let mut bs = Vec::new();
            if self.eat(Tok::Colon) {
                loop {
                    let mut b = match self.expect_ident("bound 接口名") {
                        Some(t) => t.text,
                        None => break,
                    };
                    while self.eat(Tok::ColonColon) {
                        if let Some(t) = self.expect_ident("bound 限定名") {
                            b.push_str("::");
                            b.push_str(&t.text);
                        } else {
                            break;
                        }
                    }
                    bs.push(b);
                    if self.eat(Tok::Plus) { continue; }
                    break;
                }
            }
            bounds.push((name, bs));
            if self.eat(Tok::Comma) { continue; }
            break;
        }
        self.close_angle(line, col, "泛型声明");
        (names, bounds)
    }

    fn parse_type_args(&mut self, ctx: &str) -> Vec<Type> {
        let mut out = Vec::new();
        let (line, col) = (self.line(), self.col());
        if !self.at(Tok::Lt) { return out; }
        self.bump();
        if !self.at(Tok::Gt) && !self.at(Tok::Shr) {
            loop {
                out.push(self.parse_type());
                if self.eat(Tok::Comma) { continue; }
                break;
            }
        }
        self.close_angle(line, col, ctx);
        out
    }

    /// 仅当 `<...>` 后面紧跟 `(` 时才当作调用的显式类型实参；
    /// 否则回退，让 `<` 继续按比较运算符解析。
    fn looks_like_call_type_args(&self) -> bool {
        if !self.at(Tok::Lt) { return false; }
        let mut depth = 0i32;
        let mut i = self.pos;
        while i < self.toks.len() {
            match self.toks[i].tok {
                Tok::Lt => depth += 1,
                Tok::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        return self.toks.get(i + 1).map(|t| t.tok) == Some(Tok::LParen);
                    }
                }
                Tok::Shr => {
                    depth -= 2;
                    if depth <= 0 {
                        return self.toks.get(i + 1).map(|t| t.tok) == Some(Tok::LParen);
                    }
                }
                Tok::Semi | Tok::LBrace | Tok::RBrace | Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn looks_like_static_type_args(&self) -> bool {
        if !self.at(Tok::Lt) { return false; }
        let mut depth = 0i32;
        let mut i = self.pos;
        while i < self.toks.len() {
            match self.toks[i].tok {
                Tok::Lt => depth += 1,
                Tok::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        return self.toks.get(i + 1).map(|t| t.tok) == Some(Tok::ColonColon);
                    }
                }
                Tok::Shr => {
                    depth -= 2;
                    if depth <= 0 {
                        return self.toks.get(i + 1).map(|t| t.tok) == Some(Tok::ColonColon);
                    }
                }
                Tok::Semi | Tok::LBrace | Tok::RBrace | Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn try_static_type_args(&mut self) -> Vec<Type> {
        if !self.looks_like_static_type_args() { return Vec::new(); }
        self.parse_type_args("泛型类型")
    }

    fn try_call_type_args(&mut self) -> Vec<Type> {
        if !self.looks_like_call_type_args() { return Vec::new(); }
        self.parse_type_args("泛型调用")
    }

    // ============================================================== class
    fn parse_const(&mut self, is_pub: bool, annots: Vec<Annot>) -> Option<Decl> {
        let kw = self.bump(); // const
        let ty = self.parse_type();
        let name = match self.expect_ident("常量名") {
            Some(t) => t.text,
            None => {
                self.recover();
                return None;
            }
        };
        let mut d = Decl::new(DeclKind::Const, name, is_pub, 0, kw.line, kw.col);
        d.annots = annots;
        d.ret = ty;
        if self.expect(Tok::Assign, "常量后的 =").is_none() {
            return Some(d);
        }
        d.const_expr = Some(Box::new(self.parse_expr()));
        self.expect(Tok::Semi, "常量声明末尾的 ;");
        Some(d)
    }

    fn parse_class(&mut self, is_pub: bool, annots: Vec<Annot>) -> Option<Decl> {

        let kw = self.bump(); // class
        let name = self.expect_ident("class 名")?.text;
        let mut d = Decl::new(DeclKind::Class, name, is_pub, 0, kw.line, kw.col);
        let (tp, tb) = self.parse_type_params();
        d.type_params = tp;
        d.type_bounds = tb;
        d.annots = annots;
        if self.expect(Tok::LBrace, "class 体").is_none() {
            return Some(d);
        }
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let f_annots = self.parse_annots();
            let mut f_pub = false;
            if self.at(Tok::KwPub) {
                self.bump();
                f_pub = true;
            }
            let mut f_static = false;
            if self.at(Tok::KwStatic) {
                self.bump();
                f_static = true;
            }
            let mut f_const = false;
            if self.at(Tok::KwConst) {
                self.bump();
                f_const = true;
            }
            let (line, col) = (self.line(), self.col());
            if self.at(Tok::KwFn) {
                if let Some(m) = self.parse_fn(f_pub, f_annots, false, f_static) {
                    d.member_asts.push(m);
                }
                continue;
            }
            if self.at(Tok::KwUnsafe) {
                self.bump();
                if let Some(m) = self.parse_fn(f_pub, f_annots, true, f_static) {
                    d.member_asts.push(m);
                }
                continue;
            }
            // 字段
            let ty = self.parse_type();
            let fname = match self.expect_ident("字段名") {
                Some(t) => t.text,
                None => {
                    self.recover();
                    continue;
                }
            };
            let init = if self.eat(Tok::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            d.fields.push(Field {
                name: fname,
                ty,
                init,
                is_pub: f_pub,
                is_static: f_static,
                is_const: f_const,
                annots: f_annots,
                line,
                col,
            });
            self.expect(Tok::Semi, "字段声明末尾");
        }
        self.expect(Tok::RBrace, "class 体结束");
        Some(d)
    }

    // ============================================================ interface
    fn parse_iface(&mut self, is_pub: bool, annots: Vec<Annot>) -> Option<Decl> {
        let kw = self.bump(); // interface
        let name = self.expect_ident("interface 名")?.text;
        let mut d = Decl::new(DeclKind::Iface, name, is_pub, 0, kw.line, kw.col);
        let (tp, tb) = self.parse_type_params();
        d.type_params = tp;
        d.type_bounds = tb;
        d.annots = annots;
        if self.expect(Tok::LBrace, "interface 体").is_none() {
            return Some(d);
        }
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let (line, col) = (self.line(), self.col());
            // 接口组合：裸接口名 + 分号，例如 `Reader;` / `Writer;`
            if self.at(Tok::Ident) && self.toks.get(self.pos + 1).map(|t| t.tok) == Some(Tok::Semi) {
                let name = self.bump().text;
                self.bump(); // ';'
                d.extends.push((name, line, col));
                continue;
            }
            let s_annots = self.parse_annots();
            let (line, col) = (self.line(), self.col());
            if !self.at(Tok::KwFn) {
                self.error(line, col, "interface 体内只允许方法签名或内嵌接口名（如 Reader;）");
                self.recover();
                continue;
            }
            if let Some(m) = self.parse_fn(false, s_annots, false, false) {
                if m.body.is_some() {
                    self.error(m.line, m.col, "interface 方法签名不允许有函数体");
                }
                d.member_asts.push(m);
            }
        }
        self.expect(Tok::RBrace, "interface 体结束");
        Some(d)
    }

    // ================================================================ fn
    fn parse_fn(
        &mut self,
        is_pub: bool,
        annots: Vec<Annot>,
        is_unsafe: bool,
        is_static: bool,
    ) -> Option<Decl> {
        let kw = self.bump(); // fn
        let name = self.expect_ident("函数名")?.text;
        let mut d = Decl::new(DeclKind::Fn, name, is_pub, 0, kw.line, kw.col);
        let (tp, tb) = self.parse_type_params();
        d.type_params = tp;
        d.type_bounds = tb;
        d.annots = annots;
        d.is_unsafe = is_unsafe;
        d.is_static = is_static;
        d.is_ctor = false; // loader 判定
        if self.expect(Tok::LParen, "参数列表").is_some() {
            if !self.at(Tok::RParen) {
                loop {
                    let (line, col) = (self.line(), self.col());
                    let mut p_const = false;
                    if self.at(Tok::KwConst) {
                        self.bump();
                        p_const = true;
                    }
                    let ty = self.parse_type();
                    let pname = match self.expect_ident("参数名") {
                        Some(t) => t.text,
                        None => break,
                    };
                    d.params.push(Param {
                        name: pname,
                        ty,
                        is_const: p_const,
                        line,
                        col,
                    });
                    if self.eat(Tok::Comma) {
                        continue;
                    }
                    break;
                }
            }
            self.expect(Tok::RParen, "参数列表结束");
        }
        if self.eat(Tok::Arrow) {
            d.ret = self.parse_type();
        } else {
            d.ret = Type::Void;
        }
        if self.at(Tok::LBrace) {
            d.body = Some(self.parse_block());
        } else {
            self.expect(Tok::Semi, "无函数体的声明必须以 ';' 结束");
            d.body = None;
        }
        Some(d)
    }

    // ================================================================ enum
    fn parse_enum(&mut self, is_pub: bool, annots: Vec<Annot>) -> Option<Decl> {
        let kw = self.bump(); // enum
        let name = self.expect_ident("enum 名")?.text;
        let mut d = Decl::new(DeclKind::Enum, name, is_pub, 0, kw.line, kw.col);
        let (tp, tb) = self.parse_type_params();
        d.type_params = tp;
        d.type_bounds = tb;
        d.annots = annots;
        if self.expect(Tok::LBrace, "enum 体").is_none() {
            return Some(d);
        }
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let m_annots = self.parse_annots();
            let mut m_pub = false;
            if self.at(Tok::KwPub) {
                self.bump();
                m_pub = true;
            }
            let mut m_static = false;
            if self.at(Tok::KwStatic) {
                self.bump();
                m_static = true;
            }
            let (line, col) = (self.line(), self.col());
            if self.at(Tok::KwFn) {
                if let Some(m) = self.parse_fn(m_pub, m_annots, false, m_static) {
                    d.member_asts.push(m);
                }
                continue;
            }
            if self.at(Tok::KwUnsafe) {
                self.bump();
                if let Some(m) = self.parse_fn(m_pub, m_annots, true, m_static) {
                    d.member_asts.push(m);
                }
                continue;
            }
            if m_pub || m_static {
                self.error(line, col, "enum 体内 pub/static 只能用于方法");
                self.recover();
                continue;
            }
            let vname = match self.expect_ident("变体名") {
                Some(t) => t.text,
                None => {
                    self.recover();
                    continue;
                }
            };
            let mut fields: Vec<Type> = Vec::new();
            if self.at(Tok::LParen) {
                self.bump();
                if !self.at(Tok::RParen) {
                    loop {
                        fields.push(self.parse_type());
                        if self.eat(Tok::Comma) {
                            if self.at(Tok::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RParen, "变体载荷结束");
            }
            let v = Variant {
                name: vname,
                fields,
                line,
                col,
            };
            if d.variants.iter().any(|x| x.name == v.name) {
                self.error(line, col, format!("enum `{}` 重复的变体 `{}`", d.name, v.name));
            }
            d.variants.push(v);
            if !self.eat(Tok::Comma) {
                if self.at(Tok::RBrace)
                    || self.at(Tok::KwFn)
                    || self.at(Tok::KwPub)
                    || self.at(Tok::KwStatic)
                    || self.at(Tok::At)
                {
                    continue;
                }
                self.error(line, col, "变体之间需要逗号");
            }
        }
        self.expect(Tok::RBrace, "enum 体结束");
        Some(d)
    }

    // ============================================================== 语句
    fn parse_block(&mut self) -> Block {
        let (line, col) = (self.line(), self.col());
        self.expect(Tok::LBrace, "语句块");
        let mut b = Block {
            stmts: Vec::new(),
            line,
            col,
        };
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let before = self.pos;
            if let Some(s) = self.parse_stmt() {
                b.stmts.push(s);
            } else {
                self.recover();
            }
            if self.pos == before {
                self.recover(); // 防止死循环
            }
        }
        self.expect(Tok::RBrace, "语句块结束");
        b
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        let (line, col) = (self.line(), self.col());

        // 注解（第 13 章第 10 条：语句/局部变量禁止挂载）
        if self.at(Tok::At) {
            let annots = self.parse_annots();
            self.diags.err(
                10,
                self.file,
                line,
                col,
                format!("注解 @{} 不能挂载在语句或局部变量上", annots.first().map(|a| a.name.as_str()).unwrap_or("?")),
            );
        }

        match self.kind() {
            Tok::LBrace => {
                let b = self.parse_block();
                return Some(Stmt::new(StmtKind::Block(b), line, col));
            }
            Tok::KwIf => return Some(self.parse_if()),
            Tok::KwWhile => {
                self.bump();
                self.expect(Tok::LParen, "while 条件");
                let cond = self.parse_expr();
                self.expect(Tok::RParen, "while 条件结束");
                let body = self.parse_block();
                return Some(Stmt::new(StmtKind::While { cond, body }, line, col));
            }
            Tok::KwFor => return Some(self.parse_for()),
            Tok::KwReturn => {
                self.bump();
                let e = if self.at(Tok::Semi) {
                    None
                } else {
                    Some(self.parse_expr())
                };
                self.expect(Tok::Semi, "return 语句");
                return Some(Stmt::new(StmtKind::Return(e), line, col));
            }
            Tok::KwBreak => {
                self.bump();
                self.expect(Tok::Semi, "break 语句");
                return Some(Stmt::new(StmtKind::Break, line, col));
            }
            Tok::KwContinue => {
                self.bump();
                self.expect(Tok::Semi, "continue 语句");
                return Some(Stmt::new(StmtKind::Continue, line, col));
            }
            Tok::KwUnsafe => {
                self.bump();
                let b = self.parse_block();
                return Some(Stmt::new(StmtKind::Unsafe(b), line, col));
            }
            Tok::KwMatch => return Some(self.parse_match()),
            _ => {}
        }

        // 局部变量声明（类型前置）
        if let Some(s) = self.try_var_decl() {
            return Some(s);
        }

        let e = self.parse_expr();
        if self.eat(Tok::Assign) {
            let v = self.parse_expr();
            self.expect(Tok::Semi, "赋值语句");
            return Some(Stmt::new(
                StmtKind::Assign {
                    target: e,
                    value: v,
                },
                line,
                col,
            ));
        }
        self.expect(Tok::Semi, "表达式语句");
        Some(Stmt::new(StmtKind::ExprStmt(e), line, col))
    }

    /// 试探性解析 `类型 名字 [= init] ;`，失败则回退（不产生诊断）
    fn try_var_decl(&mut self) -> Option<Stmt> {
        // 只有「标识符 / void / const」才可能是类型的开头；避免 `*p = v;` 被误判为声明
        if !matches!(self.kind(), Tok::Ident | Tok::KwVoid | Tok::KwConst) {
            return None;
        }
        let save_pos = self.pos;
        let save_errors = self.diags.errors;
        self.quiet += 1;
        let (line, col) = (self.line(), self.col());
        let mut is_const = false;
        if self.at(Tok::KwConst) {
            self.bump();
            is_const = true;
        }
        let ty = self.parse_type();
        let ok_shape = self.at(Tok::Ident)
            && matches!(self.nth(1), Tok::Assign | Tok::Semi);
        self.quiet -= 1;
        if !ok_shape || self.diags.errors != save_errors {
            self.pos = save_pos;
            return None;
        }
        let name = self.bump().text;
        let init = if self.eat(Tok::Assign) {
            Some(self.parse_expr())
        } else {
            None
        };
        self.expect(Tok::Semi, "变量声明末尾");
        Some(Stmt::new(
            StmtKind::Var { ty, name, init, is_const },
            line,
            col,
        ))
    }

    fn parse_match(&mut self) -> Stmt {
        let (line, col) = (self.line(), self.col());
        self.bump(); // match
        let subject = if self.at(Tok::LParen) {
            self.bump();
            let e = self.parse_expr();
            self.expect(Tok::RParen, "match 主语括号结束");
            e
        } else {
            self.parse_expr()
        };
        if self.expect(Tok::LBrace, "match 分支体").is_none() {
            return Stmt::new(StmtKind::Match { subject, arms: Vec::new() }, line, col);
        }
        let mut arms = Vec::new();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let before = self.pos;
            let (pl, pc) = (self.line(), self.col());
            let pat = self.parse_pattern();
            if self.expect(Tok::FatArrow, "match 分支箭头 '=>'").is_none() {
                self.recover();
                continue;
            }
            let body = if self.at(Tok::LBrace) {
                self.parse_block()
            } else {
                // 允许单条语句分支：包装成块
                let st = self.parse_stmt();
                let mut b = Block { stmts: Vec::new(), line: pl, col: pc };
                if let Some(s) = st {
                    b.stmts.push(s);
                }
                b
            };
            arms.push(MatchArm { pat, body, line: pl, col: pc });
            if self.pos == before {
                self.recover();
            }
        }
        self.expect(Tok::RBrace, "match 分支体结束");
        Stmt::new(StmtKind::Match { subject, arms }, line, col)
    }

    fn parse_pattern(&mut self) -> Pattern {
        let (line, col) = (self.line(), self.col());
        let bind = |p: &mut Parser| -> Option<String> {
            if p.expect(Tok::LParen, "模式绑定").is_none() {
                return None;
            }
            let name = if p.at(Tok::Ident) {
                let t = p.bump();
                if t.text == "_" { None } else { Some(t.text) }
            } else {
                p.error(p.line(), p.col(), "模式绑定需要标识符或 '_'");
                None
            };
            p.expect(Tok::RParen, "模式绑定结束");
            name
        };
        match self.kind() {
            Tok::KwOk => {
                self.bump();
                Pattern::Ok(bind(self))
            }
            Tok::KwErr => {
                self.bump();
                Pattern::Err(bind(self))
            }
            Tok::KwSome => {
                self.bump();
                Pattern::Some(bind(self))
            }
            Tok::KwNone => {
                self.bump();
                Pattern::None
            }
            Tok::KwNull => {
                self.bump();
                Pattern::Null
            }
            Tok::KwTrue => {
                self.bump();
                Pattern::Bool(true)
            }
            Tok::KwFalse => {
                self.bump();
                Pattern::Bool(false)
            }
            Tok::Int => {
                let t = self.bump();
                if t.uval > i64::MAX as u64 {
                    Pattern::IntU64(t.uval)
                } else {
                    Pattern::Int(t.ival)
                }
            }
            Tok::Float => {
                let t = self.bump();
                Pattern::Float(t.fval)
            }
            Tok::StrLit => {
                let t = self.bump();
                if t.text.contains('{') {
                    self.error(line, col, "match 模式中的字符串不能包含插值");
                }
                Pattern::Str(t.text)
            }
            Tok::Ident => {
                let t = self.bump();
                if t.text == "_" {
                    return Pattern::Wildcard;
                }
                // Enum::Variant 或 Enum::Variant(a, b)
                if self.at(Tok::ColonColon) {
                    self.bump();
                    let vname = match self.expect_ident("变体名") {
                        Some(x) => x.text,
                        None => String::from("<error>"),
                    };
                    let mut binds: Vec<Option<String>> = Vec::new();
                    if self.at(Tok::LParen) {
                        self.bump();
                        if !self.at(Tok::RParen) {
                            loop {
                                if self.at(Tok::Ident) {
                                    let b = self.bump();
                                    binds.push(if b.text == "_" { None } else { Some(b.text) });
                                } else {
                                    self.error(self.line(), self.col(), "变体模式绑定需要标识符或 '_'");
                                    binds.push(None);
                                    break;
                                }
                                if self.eat(Tok::Comma) {
                                    if self.at(Tok::RParen) {
                                        break;
                                    }
                                    continue;
                                }
                                break;
                            }
                        }
                        self.expect(Tok::RParen, "变体模式绑定结束");
                    }
                    return Pattern::Variant {
                        ename: t.text,
                        name: vname,
                        binds,
                        enum_id: usize::MAX,
                        vidx: usize::MAX,
                    };
                }
                self.error(line, col, "不支持的模式（可用 Ok/Err/Some/None/Enum::Variant(...)/字面量/_; 绑定请写成 Ok(_)）");
                Pattern::Wildcard
            }
            _ => {
                self.error(line, col, format!("期望模式，实际是 {}", tok_desc(self.kind())));
                Pattern::Wildcard
            }
        }
    }

    /// 把 "a {expr} b" 拆成字面量与表达式片段；`{{` / `}}` 表示花括号字面量
    fn split_interp(&mut self, text: &str, line: u32) -> Vec<InterpPart> {
        let mut parts: Vec<InterpPart> = Vec::new();
        let mut lit = String::new();
        let bytes: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == '{' {
                if i + 1 < bytes.len() && bytes[i + 1] == '{' {
                    lit.push('{');
                    i += 2;
                    continue;
                }
                // 找配对的 '}'
                let mut depth = 1usize;
                let mut j = i + 1;
                let mut expr = String::new();
                while j < bytes.len() {
                    match bytes[j] {
                        '{' => {
                            depth += 1;
                            expr.push('{');
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            expr.push('}');
                        }
                        ch => expr.push(ch),
                    }
                    j += 1;
                }
                if depth != 0 {
                    self.error(line, 1, "字符串插值缺少配对的 '}'");
                    break;
                }
                if expr.trim().is_empty() {
                    self.error(line, 1, "字符串插值 `{}` 内需要表达式");
                } else {
                    if !lit.is_empty() {
                        parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
                    }
                    // 子解析：行号对齐，便于诊断定位
                    let mut src = String::new();
                    for _ in 1..line {
                        src.push('\n');
                    }
                    src.push_str(&expr);
                    let mut sub = Parser::new(self.file, &src, &mut *self.diags);
                    let e = sub.parse_expr();
                    if !sub.at(Tok::Eof) {
                        sub.error(sub.line(), sub.col(), "字符串插值表达式多余内容");
                    }
                    parts.push(InterpPart::Expr(Box::new(e)));
                }
                i = j + 1;
                continue;
            }
            if c == '}' {
                if i + 1 < bytes.len() && bytes[i + 1] == '}' {
                    lit.push('}');
                    i += 2;
                    continue;
                }
                self.error(line, 1, "字符串中单独的 '}' 需要写成 '}}'");
                lit.push('}');
                i += 1;
                continue;
            }
            lit.push(c);
            i += 1;
        }
        if !lit.is_empty() {
            parts.push(InterpPart::Lit(lit));
        }
        if parts.is_empty() {
            parts.push(InterpPart::Lit(String::new()));
        }
        parts
    }

    fn parse_if(&mut self) -> Stmt {
        let (line, col) = (self.line(), self.col());
        self.bump(); // if
        self.expect(Tok::LParen, "if 条件");
        let cond = self.parse_expr();
        self.expect(Tok::RParen, "if 条件结束");
        let then_b = self.parse_block();
        let else_b = if self.eat(Tok::KwElse) {
            if self.at(Tok::KwIf) {
                Some(Box::new(self.parse_if()))
            } else if self.at(Tok::LBrace) {
                let l = self.line();
                let c = self.col();
                let b = self.parse_block();
                Some(Box::new(Stmt::new(StmtKind::Block(b), l, c)))
            } else {
                let l = self.line();
                let c = self.col();
                self.error(l, c, "else 后需要 if 或语句块");
                None
            }
        } else {
            None
        };
        Stmt::new(
            StmtKind::If {
                cond,
                then_b,
                else_b,
            },
            line,
            col,
        )
    }

    fn parse_for(&mut self) -> Stmt {
        let (line, col) = (self.line(), self.col());
        self.bump(); // for
        self.expect(Tok::LParen, "for 头部");
        let init: Stmt = if let Some(v) = self.try_var_decl() {
            v
        } else {
            let l = self.line();
            let c = self.col();
            let e = self.parse_expr();
            if self.eat(Tok::Assign) {
                let v = self.parse_expr();
                self.expect(Tok::Semi, "for 初始化子句");
                Stmt::new(StmtKind::Assign { target: e, value: v }, l, c)
            } else {
                self.expect(Tok::Semi, "for 初始化子句");
                Stmt::new(StmtKind::ExprStmt(e), l, c)
            }
        };
        let cond = if self.at(Tok::Semi) {
            None
        } else {
            Some(self.parse_expr())
        };
        self.expect(Tok::Semi, "for 条件子句");
        let step = if self.at(Tok::RParen) {
            None
        } else {
            let (sl, sc) = (self.line(), self.col());
            let e = self.parse_expr();
            if self.eat(Tok::Assign) {
                let v = self.parse_expr();
                Some(Box::new(Stmt::new(
                    StmtKind::Assign { target: e, value: v },
                    sl,
                    sc,
                )))
            } else {
                Some(Box::new(Stmt::new(StmtKind::ExprStmt(e), sl, sc)))
            }
        };
        self.expect(Tok::RParen, "for 头部结束");
        let body = self.parse_block();
        Stmt::new(
            StmtKind::For {
                init: Box::new(init),
                cond,
                step,
                body,
            },
            line,
            col,
        )
    }

    // ============================================================== 表达式
    fn parse_expr(&mut self) -> Expr {
        self.parse_binary(1)
    }

    fn parse_binary(&mut self, min_prec: u8) -> Expr {
        let mut lhs = self.parse_cast();
        loop {
            let (op, prec) = match bin_op(self.kind()) {
                Some(x) => x,
                None => break,
            };
            if prec < min_prec {
                break;
            }
            let (line, col) = (self.line(), self.col());
            self.bump();
            let rhs = self.parse_binary(prec + 1);
            lhs = Expr::new(ExprKind::Bin(op, Box::new(lhs), Box::new(rhs)), line, col);
        }
        lhs
    }

    /// `as` 的优先级低于一元运算、高于乘除：`*p as i32` == `(*p) as i32`
    fn parse_cast(&mut self) -> Expr {
        let mut e = self.parse_unary();
        while self.at(Tok::KwAs) {
            let (el, ec) = (e.line, e.col);
            self.bump();
            let checked = self.at(Tok::Question);
            if checked { self.bump(); }
            let ty = self.parse_type();
            e = Expr::new(ExprKind::Cast(Box::new(e), ty, checked), el, ec);
        }
        e
    }

    fn parse_unary(&mut self) -> Expr {
        let (line, col) = (self.line(), self.col());
        let op = match self.kind() {
            Tok::Bang => Some(UnOp::Not),
            Tok::Minus => Some(UnOp::Neg),
            Tok::Tilde => Some(UnOp::BitNot),
            Tok::Star => Some(UnOp::Deref),
            Tok::Amp => Some(UnOp::AddrOf),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.parse_unary();
            return Expr::new(ExprKind::Un(op, Box::new(e)), line, col);
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut e = self.parse_primary();
        loop {
            match self.kind() {
                Tok::Dot => {
                    self.bump();
                    let name = match self.expect_ident("成员名") {
                        Some(t) => t.text,
                        None => break,
                    };
                    let (el, ec) = (e.line, e.col);
                    let targs = if self.at(Tok::Lt) { self.try_call_type_args() } else { Vec::new() };
                    if self.at(Tok::LParen) {
                        let args = self.parse_call_args();
                        e = Expr::new(ExprKind::Method(Box::new(e), name, targs, args), el, ec);
                    } else {
                        e = Expr::new(ExprKind::Field(Box::new(e), name), el, ec);
                    }
                }
                Tok::Question => {
                    self.bump();
                    let (el, ec) = (e.line, e.col);
                    e = Expr::new(ExprKind::Try(Box::new(e)), el, ec);
                }
                Tok::LBracket => {
                    self.bump();
                    let idx = self.parse_expr();
                    self.expect(Tok::RBracket, "下标结束");
                    let (el, ec) = (e.line, e.col);
                    e = Expr::new(ExprKind::Index(Box::new(e), Box::new(idx)), el, ec);
                }
                _ => break,
            }
        }
        e
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        self.expect(Tok::LParen, "实参列表");
        if !self.at(Tok::RParen) {
            loop {
                args.push(self.parse_expr());
                if self.eat(Tok::Comma) {
                    continue;
                }
                break;
            }
        }
        self.expect(Tok::RParen, "实参列表结束");
        args
    }

    fn parse_primary(&mut self) -> Expr {
        let (line, col) = (self.line(), self.col());
        let tok = self.tok().clone();
        match tok.tok {
            Tok::Int => {
                self.bump();
                if tok.uval > i64::MAX as u64 {
                    Expr::new(ExprKind::IntU64(tok.uval), line, col)
                } else {
                    Expr::new(ExprKind::Int(tok.ival), line, col)
                }
            }
            Tok::Float => {
                self.bump();
                Expr::new(ExprKind::Float(tok.fval), line, col)
            }
            Tok::StrLit => {
                self.bump();
                if tok.text.contains('{') {
                    let parts = self.split_interp(&tok.text, line);
                    Expr::new(ExprKind::Interp(parts), line, col)
                } else {
                    Expr::new(ExprKind::Str(tok.text.clone()), line, col)
                }
            }
            Tok::LBracket => {
                self.bump();
                let mut items = Vec::new();
                if !self.at(Tok::RBracket) {
                    loop {
                        items.push(self.parse_expr());
                        if self.eat(Tok::Comma) {
                            if self.at(Tok::RBracket) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RBracket, "数组字面量结束");
                Expr::new(ExprKind::ArrayLit(items), line, col)
            }
            Tok::KwTrue => {
                self.bump();
                Expr::new(ExprKind::Bool(true), line, col)
            }
            Tok::KwFalse => {
                self.bump();
                Expr::new(ExprKind::Bool(false), line, col)
            }
            Tok::KwNull => {
                self.bump();
                Expr::new(ExprKind::Null, line, col)
            }
            Tok::KwThis => {
                self.bump();
                Expr::new(ExprKind::This, line, col)
            }
            Tok::KwOk => {
                self.bump();
                self.expect(Tok::LParen, "Ok(...)");
                let inner = self.parse_expr();
                self.expect(Tok::RParen, "Ok(...) 结束");
                Expr::new(ExprKind::Ok(Box::new(inner)), line, col)
            }
            Tok::KwErr => {
                self.bump();
                self.expect(Tok::LParen, "Err(...)");
                let inner = self.parse_expr();
                self.expect(Tok::RParen, "Err(...) 结束");
                Expr::new(ExprKind::Err(Box::new(inner)), line, col)
            }
            Tok::KwSome => {
                self.bump();
                self.expect(Tok::LParen, "Some(...)");
                let inner = self.parse_expr();
                self.expect(Tok::RParen, "Some(...) 结束");
                Expr::new(ExprKind::Some_(Box::new(inner)), line, col)
            }
            Tok::KwNone => {
                self.bump();
                Expr::new(ExprKind::None_, line, col)
            }
            Tok::KwNew => {
                self.bump();
                // 先尝试 `new T[n]`
                let save_pos = self.pos;
                let save_errs = self.diags.errors;
                self.quiet += 1;
                let ty = self.parse_type_atom();
                let is_arr = self.at(Tok::LBracket);
                self.quiet -= 1;
                if is_arr && self.diags.errors == save_errs {
                    self.bump(); // '['
                    let len = self.parse_expr();
                    self.expect(Tok::RBracket, "new T[n] 长度结束");
                    return Expr::new(ExprKind::ArrayNew(ty, Box::new(len)), line, col);
                }
                self.pos = save_pos;
                let name = match self.expect_ident("class 名") {
                    Some(t) => t.text,
                    None => String::from("<error>"),
                };
                let targs = if self.at(Tok::Lt) { self.parse_type_args("new 类型实参") } else { Vec::new() };
                let mut args = Vec::new();
                if self.at(Tok::LParen) {
                    args = self.parse_call_args();
                }
                Expr::new(ExprKind::New(name, targs, args), line, col)
            }
            Tok::LParen => {
                self.bump();
                let e = self.parse_expr();
                self.expect(Tok::RParen, "括号表达式");
                e
            }
            Tok::Ident => {
                let mut class = tok.text.clone();
                self.bump();
                let mut static_targs = Vec::new();
                if self.at(Tok::Lt) && self.looks_like_static_type_args() {
                    static_targs = self.try_static_type_args();
                }
                if self.at(Tok::ColonColon) {
                    let mut sname = String::new();
                    loop {
                        self.bump(); // ::
                        let seg = match self.expect_ident("限定名") {
                            Some(t) => t.text,
                            None => String::from("<error>"),
                        };
                        if self.at(Tok::ColonColon) {
                            class.push_str("::");
                            class.push_str(&seg);
                        } else {
                            sname = seg;
                            break;
                        }
                    }
                    let mut args = Vec::new();
                    let call = self.at(Tok::LParen);
                    if call {
                        args = self.parse_call_args();
                    }
                    return Expr::new(
                        ExprKind::StaticPath {
                            class,
                            name: sname,
                            targs: static_targs,
                            args,
                            call,
                        },
                        line,
                        col,
                    );
                }
                let targs = if !static_targs.is_empty() {
                    static_targs
                } else if self.at(Tok::Lt) {
                    self.try_call_type_args()
                } else {
                    Vec::new()
                };
                if self.at(Tok::LParen) {
                    let args = self.parse_call_args();
                    Expr::new(ExprKind::Call(class.clone(), targs, args), line, col)
                } else {
                    Expr::new(ExprKind::Ident(class), line, col)
                }
            }
            _ => {
                self.error(
                    line,
                    col,
                    format!("期望表达式，实际是 {}", tok_desc(tok.tok)),
                );
                Expr::new(ExprKind::Int(0), line, col)
            }
        }
    }
}

fn bin_op(t: Tok) -> Option<(BinOp, u8)> {
    Some(match t {
        Tok::OrOr => (BinOp::OrOr, 1),
        Tok::AndAnd => (BinOp::AndAnd, 2),
        Tok::Pipe => (BinOp::BitOr, 3),
        Tok::Caret => (BinOp::BitXor, 4),
        Tok::Amp => (BinOp::BitAnd, 5),
        Tok::EqEq => (BinOp::Eq, 6),
        Tok::Ne => (BinOp::Ne, 6),
        Tok::Lt => (BinOp::Lt, 7),
        Tok::Le => (BinOp::Le, 7),
        Tok::Gt => (BinOp::Gt, 7),
        Tok::Ge => (BinOp::Ge, 7),
        Tok::Shl => (BinOp::Shl, 8),
        Tok::Shr => (BinOp::Shr, 8),
        Tok::Plus => (BinOp::Add, 9),
        Tok::Minus => (BinOp::Sub, 9),
        Tok::Star => (BinOp::Mul, 10),
        Tok::Slash => (BinOp::Div, 10),
        Tok::Percent => (BinOp::Rem, 10),
        _ => return None,
    })
}

//! 词法分析：Mylis 全部 token 定义与扫描器。
//! 关键字固定，标识符 ASCII，字符串字面量支持 \n \t \r \0 \\ \" \' 转义。

use crate::diag::E_LEX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Eof,
    Ident,
    Int,
    Float,
    StrLit,
    // 关键字
    KwFn,
    KwClass,
    KwInterface,
    KwEnum,
    KwPub,
    KwUnsafe,
    KwNew,
    KwIf,
    KwElse,
    KwWhile,
    KwFor,
    KwReturn,
    KwBreak,
    KwContinue,
    KwImport,
    KwThis,
    KwAs,
    KwTrue,
    KwFalse,
    KwNull,
    KwVoid,
    KwOk,
    KwErr,
    KwSome,
    KwNone,
    KwStatic,
    KwConst,
    KwMatch,
    // 标点
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Dot,
    Colon,
    ColonColon,
    Arrow,
    FatArrow,
    Assign,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Shl,
    Shr,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Bang,
    AndAnd,
    OrOr,
    Question,
    At,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub text: String,
    pub ival: i64,
    pub uval: u64,
    pub fval: f64,
    pub line: u32,
    pub col: u32,
}

impl Token {
    pub fn dummy(tok: Tok, line: u32, col: u32) -> Token {
        Token {
            tok,
            text: String::new(),
            ival: 0,
            uval: 0,
            fval: 0.0,
            line,
            col,
        }
    }
}

pub struct Lexer<'a> {
    #[allow(dead_code)]
    pub file: &'a str,
    src: &'a str,
    b: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
    pub errors: Vec<(u32, u32, String)>,
}

const KEYWORDS: &[(&str, Tok)] = &[
    ("fn", Tok::KwFn),
    ("class", Tok::KwClass),
    ("interface", Tok::KwInterface),
    ("enum", Tok::KwEnum),
    ("pub", Tok::KwPub),
    ("unsafe", Tok::KwUnsafe),
    ("new", Tok::KwNew),
    ("if", Tok::KwIf),
    ("else", Tok::KwElse),
    ("while", Tok::KwWhile),
    ("for", Tok::KwFor),
    ("return", Tok::KwReturn),
    ("break", Tok::KwBreak),
    ("continue", Tok::KwContinue),
    ("import", Tok::KwImport),
    ("this", Tok::KwThis),
    ("as", Tok::KwAs),
    ("true", Tok::KwTrue),
    ("false", Tok::KwFalse),
    ("null", Tok::KwNull),
    ("void", Tok::KwVoid),
    ("Ok", Tok::KwOk),
    ("Err", Tok::KwErr),
    ("Some", Tok::KwSome),
    ("None", Tok::KwNone),
    ("static", Tok::KwStatic),
    ("const", Tok::KwConst),
    ("match", Tok::KwMatch),
];

impl<'a> Lexer<'a> {
    pub fn new(file: &'a str, src: &'a str) -> Self {
        Lexer {
            file,
            src,
            b: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            errors: Vec::new(),
        }
    }

    fn cur(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn at(&self, off: usize) -> Option<u8> {
        self.b.get(self.pos + off).copied()
    }

    fn adv(&mut self) {
        if let Some(c) = self.cur() {
            if c == b'\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            self.pos += 1;
        }
    }

    fn error(&mut self, line: u32, col: u32, msg: impl Into<String>) {
        self.errors.push((line, col, msg.into()));
    }

    fn skip_ws(&mut self) {
        loop {
            match self.cur() {
                Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') | Some(0x0b) | Some(0x0c) => {
                    self.adv();
                }
                Some(b'/') if self.at(1) == Some(b'/') => {
                    while let Some(c) = self.cur() {
                        if c == b'\n' {
                            break;
                        }
                        self.adv();
                    }
                }
                Some(b'/') if self.at(1) == Some(b'*') => {
                    let (l, c) = (self.line, self.col);
                    self.adv();
                    self.adv();
                    loop {
                        match self.cur() {
                            None => {
                                self.error(l, c, "未闭合的块注释");
                                break;
                            }
                            Some(b'*') if self.at(1) == Some(b'/') => {
                                self.adv();
                                self.adv();
                                break;
                            }
                            _ => self.adv(),
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn scan_string(&mut self) -> String {
        let (l, c) = (self.line, self.col);
        self.adv(); // 开引号
        let mut out = String::new();
        loop {
            match self.cur() {
                None => {
                    self.error(l, c, "未闭合的字符串字面量");
                    break;
                }
                Some(b'"') => {
                    self.adv();
                    break;
                }
                Some(b'\n') => {
                    self.error(l, c, "字符串字面量不能跨行");
                    break;
                }
                Some(b'\\') => {
                    self.adv();
                    let e = self.cur();
                    match e {
                        Some(b'n') => {
                            out.push('\n');
                            self.adv();
                        }
                        Some(b't') => {
                            out.push('\t');
                            self.adv();
                        }
                        Some(b'r') => {
                            out.push('\r');
                            self.adv();
                        }
                        Some(b'0') => {
                            out.push('\0');
                            self.adv();
                        }
                        Some(b'\\') => {
                            out.push('\\');
                            self.adv();
                        }
                        Some(b'"') => {
                            out.push('"');
                            self.adv();
                        }
                        Some(b'\'') => {
                            out.push('\'');
                            self.adv();
                        }
                        Some(x) => {
                            let (el, ec) = (self.line, self.col);
                            self.error(el, ec, format!("未知转义序列 '\\{}'", x as char));
                            self.adv();
                        }
                        None => {
                            self.error(l, c, "未闭合的字符串字面量");
                            break;
                        }
                    }
                }
                Some(_) => {
                    // 整段原样拷贝（保证 UTF-8 完整）
                    let start = self.pos;
                    while let Some(x) = self.cur() {
                        if x == b'"' || x == b'\\' || x == b'\n' {
                            break;
                        }
                        self.adv();
                    }
                    out.push_str(&self.src[start..self.pos]);
                }
            }
        }
        out
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_ws();
        let (line, col) = (self.line, self.col);
        let c = match self.cur() {
            None => return Token::dummy(Tok::Eof, line, col),
            Some(c) => c,
        };

        // 标识符 / 关键字
        if c == b'_' || c.is_ascii_alphabetic() {
            let start = self.pos;
            while let Some(x) = self.cur() {
                if x == b'_' || x.is_ascii_alphanumeric() {
                    self.adv();
                } else {
                    break;
                }
            }
            let word = &self.src[start..self.pos];
            let tok = KEYWORDS
                .iter()
                .find(|(w, _)| *w == word)
                .map(|(_, t)| *t)
                .unwrap_or(Tok::Ident);
            let mut t = Token::dummy(tok, line, col);
            t.text = word.to_string();
            return t;
        }

        // 数字
        if c.is_ascii_digit() {
            let start = self.pos;
            let mut is_float = false;
            if c == b'0' && (self.at(1) == Some(b'x') || self.at(1) == Some(b'X')) {
                self.adv();
                self.adv();
                while let Some(x) = self.cur() {
                    if x.is_ascii_hexdigit() {
                        self.adv();
                    } else {
                        break;
                    }
                }
            } else {
                while let Some(x) = self.cur() {
                    if x.is_ascii_digit() {
                        self.adv();
                    } else {
                        break;
                    }
                }
                if self.cur() == Some(b'.') && self.at(1).map_or(false, |d| d.is_ascii_digit()) {
                    is_float = true;
                    self.adv();
                    while let Some(x) = self.cur() {
                        if x.is_ascii_digit() {
                            self.adv();
                        } else {
                            break;
                        }
                    }
                }
                if matches!(self.cur(), Some(b'e') | Some(b'E')) {
                    is_float = true;
                    self.adv();
                    if matches!(self.cur(), Some(b'+') | Some(b'-')) {
                        self.adv();
                    }
                    while let Some(x) = self.cur() {
                        if x.is_ascii_digit() {
                            self.adv();
                        } else {
                            break;
                        }
                    }
                }
            }
            let raw = &self.src[start..self.pos];
            let mut t = Token::dummy(if is_float { Tok::Float } else { Tok::Int }, line, col);
            t.text = raw.to_string();
            if is_float {
                t.fval = raw.parse::<f64>().unwrap_or(0.0);
            } else {
                let digits = raw.replace('_', "");
                let parsed = if digits.starts_with("0x") || digits.starts_with("0X") {
                    u64::from_str_radix(&digits[2..], 16).ok()
                } else {
                    digits.parse::<u64>().ok()
                };
                let u = match parsed {
                    Some(v) => v,
                    None => {
                        self.error(line, col, format!("整数字面量 {} 超出 u64 取值范围", raw));
                        0
                    }
                };
                t.uval = u;
                t.ival = u as i64;
            }
            return t;
        }

        // 字符串
        if c == b'"' {
            let text = self.scan_string();
            let mut t = Token::dummy(Tok::StrLit, line, col);
            t.text = text;
            return t;
        }

        self.adv();
        let tok = match c {
            b'(' => Tok::LParen,
            b')' => Tok::RParen,
            b'[' => Tok::LBracket,
            b']' => Tok::RBracket,
            b'{' => Tok::LBrace,
            b'}' => Tok::RBrace,
            b';' => Tok::Semi,
            b',' => Tok::Comma,
            b'.' => Tok::Dot,
            b':' => {
                if self.cur() == Some(b':') {
                    self.adv();
                    Tok::ColonColon
                } else {
                    Tok::Colon
                }
            }
            b'@' => Tok::At,
            b'?' => Tok::Question,
            b'~' => Tok::Tilde,
            b'^' => Tok::Caret,
            b'%' => Tok::Percent,
            b'=' => {
                if self.cur() == Some(b'=') {
                    self.adv();
                    Tok::EqEq
                } else if self.cur() == Some(b'>') {
                    self.adv();
                    Tok::FatArrow
                } else {
                    Tok::Assign
                }
            }
            b'!' => {
                if self.cur() == Some(b'=') {
                    self.adv();
                    Tok::Ne
                } else {
                    Tok::Bang
                }
            }
            b'<' => {
                if self.cur() == Some(b'=') {
                    self.adv();
                    Tok::Le
                } else if self.cur() == Some(b'<') {
                    self.adv();
                    Tok::Shl
                } else {
                    Tok::Lt
                }
            }
            b'>' => {
                if self.cur() == Some(b'=') {
                    self.adv();
                    Tok::Ge
                } else if self.cur() == Some(b'>') {
                    self.adv();
                    Tok::Shr
                } else {
                    Tok::Gt
                }
            }
            b'+' => Tok::Plus,
            b'-' => {
                if self.cur() == Some(b'>') {
                    self.adv();
                    Tok::Arrow
                } else {
                    Tok::Minus
                }
            }
            b'*' => Tok::Star,
            b'/' => Tok::Slash,
            b'&' => {
                if self.cur() == Some(b'&') {
                    self.adv();
                    Tok::AndAnd
                } else {
                    Tok::Amp
                }
            }
            b'|' => {
                if self.cur() == Some(b'|') {
                    self.adv();
                    Tok::OrOr
                } else {
                    Tok::Pipe
                }
            }
            other => {
                self.error(line, col, format!("非法字符 '{}' (0x{:02x})", other as char, other));
                return self.next_token();
            }
        };
        Token::dummy(tok, line, col)
    }

    /// 一次性扫描全部 token（parser 使用 token 向量，便于回溯与 `>>` 拆分）
    pub fn tokenize(mut self) -> (Vec<Token>, Vec<(u32, u32, String)>) {
        let mut out = Vec::new();
        loop {
            let t = self.next_token();
            let end = t.tok == Tok::Eof;
            out.push(t);
            if end {
                break;
            }
        }
        (out, self.errors)
    }
}

pub fn tok_desc(t: Tok) -> &'static str {
    match t {
        Tok::Eof => "文件结束",
        Tok::Ident => "标识符",
        Tok::Int => "整数字面量",
        Tok::Float => "浮点字面量",
        Tok::StrLit => "字符串字面量",
        Tok::KwFn => "'fn'",
        Tok::KwClass => "'class'",
        Tok::KwInterface => "'interface'",
        Tok::KwEnum => "'enum'",
        Tok::KwPub => "'pub'",
        Tok::KwUnsafe => "'unsafe'",
        Tok::KwNew => "'new'",
        Tok::KwIf => "'if'",
        Tok::KwElse => "'else'",
        Tok::KwWhile => "'while'",
        Tok::KwFor => "'for'",
        Tok::KwReturn => "'return'",
        Tok::KwBreak => "'break'",
        Tok::KwContinue => "'continue'",
        Tok::KwImport => "'import'",
        Tok::KwThis => "'this'",
        Tok::KwAs => "'as'",
        Tok::KwTrue => "'true'",
        Tok::KwFalse => "'false'",
        Tok::KwNull => "'null'",
        Tok::KwVoid => "'void'",
        Tok::KwOk => "'Ok'",
        Tok::KwErr => "'Err'",
        Tok::KwSome => "'Some'",
        Tok::KwNone => "'None'",
        Tok::KwStatic => "'static'",
        Tok::KwConst => "'const'",
        Tok::KwMatch => "'match'",
        Tok::LParen => "'('",
        Tok::RParen => "')'",
        Tok::LBracket => "'['",
        Tok::RBracket => "']'",
        Tok::LBrace => "'{'",
        Tok::RBrace => "'}'",
        Tok::Semi => "';'",
        Tok::Comma => "','",
        Tok::Dot => "'.'",
        Tok::Colon => "':'",
        Tok::ColonColon => "'::'",
        Tok::Arrow => "'->'",
        Tok::FatArrow => "'=>'",
        Tok::Assign => "'='",
        Tok::EqEq => "'=='",
        Tok::Ne => "'!='",
        Tok::Lt => "'<'",
        Tok::Le => "'<='",
        Tok::Gt => "'>'",
        Tok::Ge => "'>='",
        Tok::Shl => "'<<'",
        Tok::Shr => "'>>'",
        Tok::Plus => "'+'",
        Tok::Minus => "'-'",
        Tok::Star => "'*'",
        Tok::Slash => "'/'",
        Tok::Percent => "'%'",
        Tok::Amp => "'&'",
        Tok::Pipe => "'|'",
        Tok::Caret => "'^'",
        Tok::Tilde => "'~'",
        Tok::Bang => "'!'",
        Tok::AndAnd => "'&&'",
        Tok::OrOr => "'||'",
        Tok::Question => "'?'",
        Tok::At => "'@'",
    }
}

pub const _E_LEX_UNUSED: u32 = E_LEX;

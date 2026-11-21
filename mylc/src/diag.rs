//! 诊断输出：所有错误统一带 MYLnnn 编号。
//! 1..=12 对应规范第 13 章的 12 条强制检查，其余为词法/语法/通用错误。

pub const E_LEX: u32 = 100;
pub const E_SYNTAX: u32 = 101;
pub const E_GENERAL: u32 = 102;
pub const E_IO: u32 = 103;
pub const E_INTERNAL: u32 = 104;

pub fn label(code: u32) -> String {
    match code {
        1 => "MYL001".to_string(),
        2 => "MYL002".to_string(),
        3 => "MYL003".to_string(),
        4 => "MYL004".to_string(),
        5 => "MYL005".to_string(),
        6 => "MYL006".to_string(),
        7 => "MYL007".to_string(),
        8 => "MYL008".to_string(),
        9 => "MYL009".to_string(),
        10 => "MYL010".to_string(),
        11 => "MYL011".to_string(),
        12 => "MYL012".to_string(),
        13 => "MYL013".to_string(),
        14 => "MYL014".to_string(),
        15 => "MYL015".to_string(),
        16 => "MYL016".to_string(),
        E_LEX => "MYL100".to_string(),
        E_SYNTAX => "MYL101".to_string(),
        E_GENERAL => "MYL102".to_string(),
        E_IO => "MYL103".to_string(),
        200 => "MYLW01".to_string(),
        201 => "MYLW02".to_string(),
        _ => "MYL104".to_string(),
    }
}

/// 一条结构化诊断（语言服务器用；命令行模式仍直接打印）
#[derive(Clone, Debug)]
pub struct Item {
    pub code: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub msg: String,
    pub warning: bool,
}

pub struct Diag {
    pub errors: usize,
    pub notes: usize,
    pub warnings: usize,
    /// 非空时进入“收集模式”：不打印，改为记录条目
    pub items: Option<Vec<Item>>,
}

impl Diag {
    pub fn new() -> Self {
        Diag {
            errors: 0,
            notes: 0,
            warnings: 0,
            items: None,
        }
    }

    /// 切换到收集模式（语言服务器：诊断通过 JSON 返回）
    pub fn collect_mode(&mut self) {
        self.items = Some(Vec::new());
    }

    fn push(&mut self, code: String, file: &str, line: u32, col: u32, msg: &str, warning: bool) {
        if let Some(items) = self.items.as_mut() {
            items.push(Item {
                code,
                file: file.to_string(),
                line,
                col,
                msg: msg.to_string(),
                warning,
            });
        }
    }

    /// 警告：不阻断编译，编译成功时统一汇总
    pub fn warn(&mut self, code: u32, file: &str, line: u32, col: u32, msg: impl Into<String>) {
        let msg = msg.into();
        if self.items.is_some() {
            self.push(label(code), file, line, col, &msg, true);
        } else {
            eprintln!(
                "{}:{}:{}: warning[{}]: {}",
                file,
                line,
                col,
                label(code),
                msg
            );
        }
        self.warnings += 1;
    }

    pub fn err(&mut self, code: u32, file: &str, line: u32, col: u32, msg: impl Into<String>) {
        let msg = msg.into();
        if self.items.is_some() {
            self.push(label(code), file, line, col, &msg, false);
        } else {
            eprintln!(
                "{}:{}:{}: error[{}]: {}",
                file,
                line,
                col,
                label(code),
                msg
            );
        }
        self.errors += 1;
    }

    pub fn note(&mut self, file: &str, line: u32, col: u32, msg: impl Into<String>) {
        let msg = msg.into();
        if self.items.is_none() {
            eprintln!("{}:{}:{}: note: {}", file, line, col, msg);
        }
        self.notes += 1;
    }
}

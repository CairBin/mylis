//! Mylis AST 定义（与规范第 12 章 1:1 对应）。

use std::fmt;
use std::sync::{OnceLock, RwLock};

/// 声明名注册表：让 Type 的 Display 能打印 class/interface 的真实名字。
static TYPE_NAMES: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

fn type_names() -> &'static RwLock<Vec<String>> {
    TYPE_NAMES.get_or_init(|| RwLock::new(Vec::new()))
}

pub fn set_type_names(names: Vec<String>) {
    *type_names().write().unwrap() = names;
}

fn lookup_name(id: usize) -> String {
    if let Ok(g) = type_names().read() {
        if let Some(n) = g.get(id) {
            return n.clone();
        }
    }
    format!("#{}", id)
}

// ============================================================== 类型系统
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Void,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Str,
    Result(Box<Type>, Box<Type>),
    Option(Box<Type>),
    Ptr(Box<Type>),
    /// T[]
    Array(Box<Type>),
    /// 解析前：带类型实参的类型名，如 Vec<i64>
    Generic(String, Vec<Type>),
    /// 解析前：源码里的类型名
    Named(String),
    /// 解析后：class 声明 id
    Class(usize),
    /// 解析后：interface 声明 id
    Iface(usize),
    /// 解析后：enum 声明 id
    Enum(usize),
    /// 解析后的泛型名义类型：模板声明 id + 类型实参。
    /// 类型身份由 (base, args) 决定，不再和某个单态化 Decl id 绑定。
    Adt { base: usize, args: Vec<Type> },
    /// 解析后的类型参数（模板内的 T）
    Param(String),
    /// 内部占位：Ok/Err/Some/None 的待定类型
    Infer,
}

impl Type {
    pub fn ptr(elem: Type) -> Type {
        Type::Ptr(Box::new(elem))
    }
    pub fn result(ok: Type, err: Type) -> Type {
        Type::Result(Box::new(ok), Box::new(err))
    }
    pub fn option(inner: Type) -> Type {
        Type::Option(Box::new(inner))
    }
    pub fn array(elem: Type) -> Type {
        Type::Array(Box::new(elem))
    }

    /// 是否含裸指针（决定是否必须处于 unsafe 域）
    pub fn has_raw_ptr(&self) -> bool {
        match self {
            Type::Ptr(_) => true,
            Type::Array(a) => a.has_raw_ptr(),
            Type::Generic(_, args) => args.iter().any(|a| a.has_raw_ptr()),
            Type::Adt { args, .. } => args.iter().any(|a| a.has_raw_ptr()),
            Type::Result(a, b) => a.has_raw_ptr() || b.has_raw_ptr(),
            Type::Option(a) => a.has_raw_ptr(),
            _ => false,
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
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
        )
    }

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
        )
    }

    pub fn is_signed(&self) -> bool {
        matches!(self, Type::I8 | Type::I16 | Type::I32 | Type::I64)
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Type::F32 | Type::F64)
    }

    pub fn is_value(&self) -> bool {
        match self {
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
            | Type::Str
            | Type::Result(_, _)
            | Type::Option(_) => true,
            // class / enum / array 是引用语义，但作为载荷时是「一个指针大小的值」
            Type::Class(_) | Type::Enum(_) | Type::Array(_) | Type::Adt { .. } | Type::Param(_) => true,
            _ => false,
        }
    }

    /// 已解析完成（不含 Named / Infer）
    pub fn is_resolved(&self) -> bool {
        match self {
            Type::Named(_) | Type::Infer => false,
            Type::Ptr(e) => e.is_resolved(),
            Type::Array(e) => e.is_resolved(),
            Type::Generic(_, args) => args.iter().all(|a| a.is_resolved()),
            Type::Adt { args, .. } => args.iter().all(|a| a.is_resolved()),
            Type::Param(_) => true,
            Type::Result(a, b) => a.is_resolved() && b.is_resolved(),
            Type::Option(a) => a.is_resolved(),
            _ => true,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Void => write!(f, "void"),
            Type::I8 => write!(f, "i8"),
            Type::I16 => write!(f, "i16"),
            Type::I32 => write!(f, "i32"),
            Type::I64 => write!(f, "i64"),
            Type::U8 => write!(f, "u8"),
            Type::U16 => write!(f, "u16"),
            Type::U32 => write!(f, "u32"),
            Type::U64 => write!(f, "u64"),
            Type::F32 => write!(f, "f32"),
            Type::F64 => write!(f, "f64"),
            Type::Bool => write!(f, "bool"),
            Type::Str => write!(f, "string"),
            Type::Result(a, b) => write!(f, "Result<{}, {}>", a, b),
            Type::Option(a) => write!(f, "Option<{}>", a),
            Type::Ptr(e) => write!(f, "{}*", e),
            Type::Array(e) => write!(f, "{}[]", e),
            Type::Generic(name, args) => {
                write!(f, "{}<", name)?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", a)?;
                }
                write!(f, ">")
            }
            Type::Named(n) => write!(f, "{}", n),
            Type::Class(i) => write!(f, "{}", lookup_name(*i)),
            Type::Iface(i) => write!(f, "{}", lookup_name(*i)),
            Type::Enum(i) => write!(f, "{}", lookup_name(*i)),
            Type::Adt { base, args } => {
                write!(f, "{}<", lookup_name(*base))?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", a)?;
                }
                write!(f, ">")
            }
            Type::Param(n) => write!(f, "{}", n),
            Type::Infer => write!(f, "<待定>"),
        }
    }
}

// =============================================================== 注解
#[derive(Clone, Debug)]
pub enum AnnotArg {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
}

#[derive(Clone, Debug)]
pub struct Annot {
    pub name: String,
    pub args: Vec<AnnotArg>,
    pub line: u32,
    pub col: u32,
}

// ============================================================== 表达式
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    Deref,
    AddrOf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    ResIsOk,
    ResIsErr,
    ResUnwrap,
    ResUnwrapErr,
    ResUnwrapOr,
    OptIsSome,
    OptIsNone,
    OptUnwrap,
    OptUnwrapOr,
    StrLen,
    ArrLen,
    /// `T.equals(other)` 的标量/字符串内建实现
    ValueEq,
    /// `T.compare(other)` 的标量/字符串内建实现
    ValueCmp,
    /// `T.hash()` 的标量/字符串内建实现
    ValueHash,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Res {
    None,
    /// 顶层函数 / 方法 声明 id
    Fn(usize),
    Method(usize),
    /// (class 声明 id, 字段下标)
    Field(usize, usize),
    /// 局部变量 / 参数（codegen 按作用域解析）
    Var,
    /// new 的 class 声明 id
    Ctor(usize),
    /// class/enum -> interface 自动实现转换 (类型 id, iface id)
    IfaceCast(usize, usize),
    /// enum 变体构造 (enum id, 变体下标)
    Variant(usize, usize),
    /// 静态方法（class id, 方法 decl id）
    StaticMethod(usize, usize),
    /// 静态字段（class id, 字段下标）
    StaticField(usize, usize),
    /// 泛型构造：模板 id + 类型实参，等待单态化
    AdtCtor { base: usize, args: Vec<Type> },
    /// 泛型函数调用：模板 id + 类型实参
    AdtFn { base: usize, args: Vec<Type> },
    /// 泛型方法调用：模板 id + 类实参 + 方法实参
    AdtMethod {
        base: usize,
        method: usize,
        class_args: Vec<Type>,
        method_args: Vec<Type>,
    },
    /// 泛型静态方法调用
    AdtStaticMethod {
        base: usize,
        method: usize,
        class_args: Vec<Type>,
        method_args: Vec<Type>,
    },
    /// 泛型实例字段
    AdtField {
        base: usize,
        field: usize,
        class_args: Vec<Type>,
    },
    /// 泛型静态字段
    AdtStaticField {
        base: usize,
        field: usize,
        class_args: Vec<Type>,
    },
    /// 泛型 enum 变体
    AdtVariant {
        base: usize,
        variant: usize,
        args: Vec<Type>,
    },
    Builtin(Builtin),
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    /// 超出 i64::MAX 的正整数字面量；避免在 lexer 阶段被截断为 0。
    IntU64(u64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
    This,
    Ident(String),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Un(UnOp, Box<Expr>),
    Call(String, Vec<Type>, Vec<Expr>),
    Method(Box<Expr>, String, Vec<Type>, Vec<Expr>),
    Field(Box<Expr>, String),
    New(String, Vec<Type>, Vec<Expr>),
    Ok(Box<Expr>),
    Err(Box<Expr>),
    Some_(Box<Expr>),
    None_,
    Cast(Box<Expr>, Type, bool), // 第三项：as? -> true
    Try(Box<Expr>),
    /// a[i]
    Index(Box<Expr>, Box<Expr>),
    /// [a, b, c]
    ArrayLit(Vec<Expr>),
    /// new T[n]
    ArrayNew(Type, Box<Expr>),
    /// "文本 {expr} 文本"
    Interp(Vec<InterpPart>),
    /// Class::name 或 Class::name(args)
    StaticPath {
        class: String,
        name: String,
        targs: Vec<Type>,
        args: Vec<Expr>,
        call: bool,
    },
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
    pub col: u32,
    /// sema 填充的类型
    pub ty: Type,
    /// sema 填充的解析结果
    pub res: Res,
    /// sema 填充：enum -> interface 的自动转换（(enum id, iface id)），与 res 分离以免覆盖变体构造信息
    pub coerce: Option<(usize, usize)>,
    /// 泛型 class/interface 的转换必然涉及类型实参；mono 阶段据此生成具体实例。
    pub coerce_adt: Option<(usize, Vec<Type>, usize, Vec<Type>)>,
    /// sema 填充：基本类型 -> 空接口的自动装箱（(源类型, 目标接口 id)）
    pub box_to: Option<(Type, usize)>,
}

impl Expr {
    pub fn new(kind: ExprKind, line: u32, col: u32) -> Expr {
        Expr {
            kind,
            line,
            col,
            ty: Type::Infer,
            res: Res::None,
            coerce: None,
            coerce_adt: None,
            box_to: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    Expr(Box<Expr>),
}

// ================================================================ 语句
#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Var {
        ty: Type,
        name: String,
        init: Option<Expr>,
        is_const: bool,
    },
    Assign {
        target: Expr,
        value: Expr,
    },
    ExprStmt(Expr),
    If {
        cond: Expr,
        then_b: Block,
        else_b: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    For {
        init: Box<Stmt>,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>,
        body: Block,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    Unsafe(Block),
    Block(Block),
    Match {
        subject: Expr,
        arms: Vec<MatchArm>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    Wildcard,
    Int(i64),
    IntU64(u64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
    /// Ok(bind) / Err(bind) / Some(bind)；绑定名为 None 表示 `_`
    Ok(Option<String>),
    Err(Option<String>),
    Some(Option<String>),
    None,
    /// Enum::Variant 或 Enum::Variant(binds)
    Variant {
        /// 枚举名（用于诊断）
        ename: String,
        /// 变体名
        name: String,
        /// 绑定名（`_` 对应 None），数量必须等于变体载荷个数
        binds: Vec<Option<String>>,
        /// sema 填充：enum 声明 id
        enum_id: usize,
        /// sema 填充：变体下标
        vidx: usize,
    },
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pat: Pattern,
    pub body: Block,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
    pub col: u32,
}

impl Stmt {
    pub fn new(kind: StmtKind, line: u32, col: u32) -> Stmt {
        Stmt { kind, line, col }
    }
}

// =============================================================== 声明
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind {
    Class,
    Iface,
    Fn,
    Enum,
    /// `type A = B;` 类型重命名（纯别名，不产生新类型）
    Alias,
    /// `pub const T NAME = literal;` 编译期常量
    Const,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub is_const: bool,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub init: Option<Expr>,
    pub is_pub: bool,
    pub is_static: bool,
    pub is_const: bool,
    pub annots: Vec<Annot>,
    pub line: u32,
    pub col: u32,
}

/// enum 变体：`Name`（unit）或 `Name(T1, T2)`（tuple）
#[derive(Clone, Debug)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Type>,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct Decl {
    pub kind: DeclKind,
    pub name: String,
    /// 泛型声明的类型参数；空表示普通声明
    pub type_params: Vec<String>,
    /// 类型参数的 bound：参数名 -> 接口名列表，例如 T: ToString
    pub type_bounds: Vec<(String, Vec<String>)>,
    /// 单态化实例对应的泛型模板 id
    pub generic_base: Option<usize>,
    /// 泛型模板的原始源码片段（可含方法体），供 .mylib API 导出
    pub source: Option<String>,
    pub is_pub: bool,
    pub annots: Vec<Annot>,
    pub mod_id: usize,
    pub line: u32,
    pub col: u32,

    // class
    pub fields: Vec<Field>,
    // enum 变体
    pub variants: Vec<Variant>,

    // interface 组合：内嵌的父接口名（sema 阶段解析并展平）
    pub extends: Vec<(String, u32, u32)>,

    // class 方法 / interface 签名（loader 之后指向 decls 的 id）
    pub members: Vec<usize>,
    // 解析阶段的成员 AST，loader 展开后清空
    pub member_asts: Vec<Decl>,

    // fn
    pub params: Vec<Param>,
    pub ret: Type,
    pub body: Option<Block>,
    pub is_unsafe: bool,
    pub is_ctor: bool,
    /// 静态方法 / 静态字段
    pub is_static: bool,
    /// @header("...") 的 C 头文件名（仅无体 unsafe fn 允许）
    pub c_header: Option<String>,

    /// 若为 class 方法 / interface 签名，指向所属声明
    pub owner: Option<usize>,

    /// 类型别名（DeclKind::Alias）的目标类型
    pub alias_target: Type,

    /// 顶层 / 静态常量初值（DeclKind::Const）
    pub const_expr: Option<Box<Expr>>,

    // sema
    pub checked: bool,
}

impl Decl {
    pub fn new(kind: DeclKind, name: String, is_pub: bool, mod_id: usize, line: u32, col: u32) -> Decl {
        Decl {
            kind,
            name,
            type_params: Vec::new(),
            type_bounds: Vec::new(),
            generic_base: None,
            source: None,
            is_pub,
            annots: Vec::new(),
            mod_id,
            line,
            col,
            fields: Vec::new(),
            variants: Vec::new(),
            extends: Vec::new(),
            members: Vec::new(),
            member_asts: Vec::new(),
            params: Vec::new(),
            ret: Type::Void,
            body: None,
            is_unsafe: false,
            is_ctor: false,
            is_static: false,
            alias_target: Type::Infer,
            const_expr: None,
            c_header: None,
            owner: None,
            checked: false,
        }
    }
}

pub fn find_class_field(d: &Decl, name: &str) -> Option<usize> {
    d.fields.iter().position(|f| f.name == name)
}

// =============================================================== 模块 / 程序
pub struct ModuleInfo {
    pub path: String,
    pub stem: String,
    /// 相对包根的模块路径；包根为空，`src/vector/A.myl` 为 ["vector", "A"]。
    /// 非包模块保留单个 stem 段，避免改变 std/独立文件的既有命名。
    pub module_path: Vec<String>,
    /// 所属包名；std/普通目录模块为 None。
    pub pkg_name: Option<String>,
    pub dir: String,
    pub is_bind: bool,
    pub prefix: String,
    pub import_raw: Vec<(String, u32, u32, bool, bool)>,
    /// 与 `imports` 对齐的显式导入名；空列表表示只做模块导入。
    pub import_selected: Vec<Vec<String>>,
    /// 显式导入名 -> 模块 id；裸名查找只查这里。
    pub imported: Vec<(String, usize)>,
    /// `pub import { ... }` 再导出的名字 -> 模块 id。
    pub reexports: Vec<(String, usize)>,
    pub imports: Vec<usize>,
    pub decls: Vec<usize>,
    pub state: u8, // 0 未访问 1 访问中 2 已完成
    pub topo: usize,
}

impl ModuleInfo {
    /// 模块的完整名：包名::路径::...；包根就是包名，std 模块就是 stem。
    pub fn full_name(&self) -> String {
        if let Some(pkg) = &self.pkg_name {
            if self.module_path.is_empty() {
                pkg.clone()
            } else {
                format!("{}::{}", pkg, self.module_path.join("::"))
            }
        } else if self.module_path.is_empty() {
            self.stem.clone()
        } else {
            self.module_path.join("::")
        }
    }

    /// 模块路径的最后一个名字；包根没有路径段。
    pub fn module_leaf(&self) -> Option<&str> {
        self.module_path.last().map(|s| s.as_str())
    }
}

pub struct Program {
    pub modules: Vec<ModuleInfo>,
    pub decls: Vec<Decl>,
    pub type_syms: Vec<Vec<(String, usize)>>,
    pub fn_syms: Vec<Vec<(String, usize)>>,
    pub const_syms: Vec<Vec<(String, usize)>>,
    /// Result / Option 实例化类型表
    pub insts: Vec<Type>,
    /// 数组类型实例表 T[]
    pub arrays: Vec<Type>,
    /// 预置 ToString 接口的声明 id
    pub tostring_iface: Option<usize>,
    /// (class id, iface id) 转换点
    pub impls: Vec<(usize, usize)>,
    /// 泛型实例缓存：(模板 id, 实参列表, 具体 id)
    pub generic_instances: Vec<(usize, Vec<Type>, usize)>,
    /// 泛型模板的原始声明的 API 快照（在模板被中和前捕获，供 .mylib 导出）
    pub template_api: Vec<Decl>,
    pub entry: usize,
    /// 拓扑编译顺序（依赖在前）
    pub order: Vec<usize>,
}

impl Program {
    pub fn new() -> Program {
        Program {
            modules: Vec::new(),
            decls: Vec::new(),
            type_syms: Vec::new(),
            fn_syms: Vec::new(),
            const_syms: Vec::new(),
            insts: Vec::new(),
            arrays: Vec::new(),
            tostring_iface: None,
            impls: Vec::new(),
            generic_instances: Vec::new(),
            template_api: Vec::new(),
            entry: 0,
            order: Vec::new(),
        }
    }

    pub fn module_of(&self, decl_id: usize) -> usize {
        self.decls[decl_id].mod_id
    }

    pub fn find_type_in(&self, mod_id: usize, name: &str) -> Option<usize> {
        self.type_syms[mod_id]
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    pub fn find_fn_in(&self, mod_id: usize, name: &str) -> Option<usize> {
        self.fn_syms[mod_id]
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    pub fn find_const_in(&self, mod_id: usize, name: &str) -> Option<usize> {
        self.const_syms[mod_id]
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    /// 记录 Result / Option 实例化
    pub fn note_inst(&mut self, t: &Type) {
        match t {
            Type::Array(e) => {
                self.note_inst(e);
                if !self.arrays.iter().any(|x| x == t) {
                    self.arrays.push(t.clone());
                }
            }
            Type::Result(a, b) => {
                self.note_inst(a);
                self.note_inst(b);
                if !self.insts.iter().any(|x| x == t) {
                    self.insts.push(t.clone());
                }
            }
            Type::Option(a) => {
                self.note_inst(a);
                if !self.insts.iter().any(|x| x == t) {
                    self.insts.push(t.clone());
                }
            }
            Type::Ptr(e) => self.note_inst(e),
            _ => {}
        }
    }

    pub fn note_impl(&mut self, class: usize, iface: usize) {
        if !self.impls.iter().any(|(c, i)| *c == class && *i == iface) {
            self.impls.push((class, iface));
        }
    }
}

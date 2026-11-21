//! 后端抽象：前端只认识 `Program`（以及 MIR），由此层决定产物形态。
//!
//! 当前有 AST-C（`backend::c`）、MIR-C（`backend::mir_c`）与 LLVM（`backend::llvm`）
//! 三个后端，默认走 LLVM；新增后端实现同一个 `Backend` trait 即可接入，
//! `driver` 不需要再增加分支。

mod c;
mod ir;
mod mir_c;
mod llvm;

pub use c::CBackend;
pub use ir::IrBackend;
pub use mir_c::MirCBackend;
pub use llvm::LlvmBackend;

/// 产物形态：C 源码、自研 IR 文本或 LLVM IR 文本。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtifactKind {
    CSource,
    Ir,
    LlvmIr,
}

/// 后端产物：文本型（C 源码、IR 文本）或二进制（目标文件）。
#[derive(Clone, Debug)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub text: String,
    /// 后端明确拒绝生成时的错误；`None` 表示可正常编译。
    pub error: Option<String>,
}

/// 导出策略：Windows DLL 下 MSVC 需要 `__declspec(dllexport)`，
/// GNU 用链接器 `--export-all-symbols` 即可。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportMode {
    Default,
    DllExport,
}

pub trait Backend {
    fn name(&self) -> &'static str;

    /// 可执行入口：生成完整程序（含 `main`）。
    fn emit_exe(&self, prog: &crate::ast::Program) -> Artifact;

    /// 库入口：不产生 `main`，改导出 `<stem>_init()`。
    fn emit_lib(&self, prog: &crate::ast::Program, mode: ExportMode) -> Artifact;

    /// 与库配套的头文件文本。
    fn header(&self, prog: &crate::ast::Program) -> String;

    /// 使用已经合并过 MIR 库的程序直接发射可执行产物。
    /// 默认拒绝：只有真正消费 MIR 的后端才能支持 MIR 库链接。
    fn emit_exe_mir(&self, _prog: &crate::ast::Program, _ir: &crate::mir::MirProgram) -> Artifact {
        Artifact {
            kind: ArtifactKind::CSource,
            text: String::new(),
            error: Some(format!("后端 `{}` 不支持 MIR 库链接", self.name())),
        }
    }
}

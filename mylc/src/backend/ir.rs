//! IR 后端：把 `Program` 降到 MIR 并输出文本。
//!
//! 它是 `Backend` trait 的第二个实现，用来证明前端不依赖 C codegen；
//! 当前只产出文本，后续 LLVM 后端会直接消费 `mir::MirProgram`。

use super::{Artifact, ArtifactKind, Backend, ExportMode};
use crate::ast::Program;
use crate::mir;

pub struct IrBackend;

impl Backend for IrBackend {
    fn name(&self) -> &'static str {
        "ir"
    }

    fn emit_exe(&self, prog: &Program) -> Artifact {
        let ir = mir::lower(prog);
        Artifact { kind: ArtifactKind::Ir, text: mir::dump(&ir), error: None }
    }

    fn emit_lib(&self, prog: &Program, _mode: ExportMode) -> Artifact {
        let ir = mir::lower(prog);
        Artifact { kind: ArtifactKind::Ir, text: mir::dump(&ir), error: None }
    }

    fn emit_exe_mir(&self, _prog: &Program, ir: &crate::mir::MirProgram) -> Artifact {
        Artifact { kind: ArtifactKind::Ir, text: crate::mir::dump(ir), error: None }
    }

    fn header(&self, _prog: &Program) -> String {
        String::new()
    }
}

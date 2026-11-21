//! C11 后端：把 `Program` 发射为 C 源码，由外部 cc（gcc/clang/cl）编译链接。

use super::{Artifact, ArtifactKind, Backend, ExportMode};
use crate::ast::Program;
use crate::codegen;

pub struct CBackend {
    pub export: ExportMode,
}

impl Default for CBackend {
    fn default() -> Self {
        CBackend { export: ExportMode::Default }
    }
}

impl Backend for CBackend {
    fn name(&self) -> &'static str {
        "c"
    }

    fn emit_exe(&self, prog: &Program) -> Artifact {
        Artifact { kind: ArtifactKind::CSource, text: codegen::generate(prog), error: None }
    }

    fn emit_lib(&self, prog: &Program, mode: ExportMode) -> Artifact {
        Artifact {
            kind: ArtifactKind::CSource,
            text: codegen::generate_lib_with(prog, mode == ExportMode::DllExport),
            error: None,
        }
    }

    fn header(&self, prog: &Program) -> String {
        codegen::header_text(prog)
    }
}

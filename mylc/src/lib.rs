//! Mylis 编译器库：供 `mylc` 命令行与 `myls` 语言服务器共用。
//!
//! 模块划分与编译器内部一致：lexer → parser → program（模块装载）→ sema → backend（当前为 C）。

pub mod ast;
pub mod backend;
pub mod codegen;
pub mod derive;
pub mod diag;
pub mod generics;
pub mod json;
pub mod lexer;
pub mod manifest;
pub mod lockfile;
pub mod parser;
pub mod mir;
pub mod mir_json;
pub mod mylib;
pub mod program;
pub mod sema;
pub mod semver;
pub mod sha256;
pub mod toolchain;

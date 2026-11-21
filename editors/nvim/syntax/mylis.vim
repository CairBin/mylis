" Mylis 语法高亮（Vim / Neovim）
if exists("b:current_syntax")
  finish
endif

syn keyword mylisKeyword fn class interface enum import pub static const type unsafe new match as
syn keyword mylisControl if else while for return break continue
syn keyword mylisConst true false null this
syn keyword mylisType void bool string i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 Result Option
syn match   mylisAnnotation "@[A-Za-z_][A-Za-z0-9_]*"
syn keyword mylisBuiltinAnnot export header deprecated inline contained
syn match   mylisComment "//.*$" contains=@Spell
syn match   mylisBlockComment "/\*.*\*/" contains=@Spell
syn region  mylisString start=+"+ skip=+\\"+ end=+"+ contains=mylisEscape,mylisInterp
syn match   mylisEscape "\\." contained
syn region  mylisInterp start=+{+ end=+}+ contained
syn match   mylisNumber "\b0x[0-9a-fA-F]\+\b\|\b\d\+\.\d\+\b\|\b\d\+\b"
syn match   mylisTypeName "\<\(class\|interface\|enum\)\s\+\zs[A-Za-z_][A-Za-z0-9_]*"
syn match   mylisAliasName "\<type\s\+\zs[A-Za-z_][A-Za-z0-9_]*"
syn match   mylisFuncName "\<fn\s\+\zs[A-Za-z_][A-Za-z0-9_]*"
syn match   mylisField "\<this\.\zs[A-Za-z_][A-Za-z0-9_]*"

hi def link mylisKeyword Keyword
hi def link mylisControl Conditional
hi def link mylisConst Constant
hi def link mylisType Type
hi def link mylisAnnotation StorageClass
hi def link mylisBuiltinAnnot StorageClass
hi def link mylisComment Comment
hi def link mylisBlockComment Comment
hi def link mylisString String
hi def link mylisEscape SpecialChar
hi def link mylisInterp Special
hi def link mylisNumber Number
hi def link mylisTypeName Type
hi def link mylisAliasName Type
hi def link mylisFuncName Function
hi def link mylisField Identifier

let b:current_syntax = "mylis"

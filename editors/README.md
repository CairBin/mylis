# 编辑器支持（VS Code / Neovim）

两套配置共用同一个语言服务器 `myls`（与编译器同一份 lexer/parser/sema，诊断与 `mylc --check` 同源）。

`myls` 提供：诊断（带 `MYLnnn` 码）、文档符号（大纲）、悬停（签名/字段）、跳转定义、补全（关键字 + 项目符号 + 标准库）。

## 服务器查找顺序

1. 编辑器设置指定（VS Code `mylis.serverPath` / Neovim `setup({ server = ... })`）
2. 环境变量 `MYLIS_SERVER`
3. 工作区/工程目录 `./bin/myls(.exe)`
4. 工作区上级 `../bin`、扩展同级目录
5. `PATH`

`make all` 产出的 `bin/` 里就带着 `myls`，因此把工程工作区指到仓库根即可自动命中。

---

## VS Code

```
editors/vscode/
├── package.json                     # 扩展清单（无 npm 依赖）
├── language-configuration.json      # 括号/注释/缩进
├── syntaxes/mylis.tmLanguage.json   # TextMate 语法（const / type 别名 / 注解 / 插值）
└── src/extension.js                 # 极简 LSP 客户端（Content-Length 分帧）
```

安装方式（不需要 `npm install`）：

```bash
cd editors/vscode
npx vsce package                     # → mylis-vscode-0.1.0.vsix
code --install-extension mylis-vscode-0.1.0.vsix
```

或开发模式：用 VS Code 打开 `editors/vscode`，按 `F5` 启动扩展宿主，再打开任意 `.myl`。

功能与命令：

| 能力 | 触发 |
| --- | --- |
| 诊断（保存/输入即时） | 自动 |
| 悬停签名 | 鼠标悬停 / `Ctrl+K` |
| 跳转定义 | `F12` / `Ctrl+Click` |
| 补全 | 输入 `/` 或 `.` 时，或 `Ctrl+Space` |
| 大纲 | 资源管理器 → Outline |
| `Mylis: 重启语言服务器` | 命令面板（状态栏点击同样触发） |
| `Mylis: 查看语言服务器日志` | 命令面板（输出面板 `Mylis LSP`） |

状态栏左侧会显示 `myls: 就绪 / 已停止 / 启动失败`。

---

## Neovim（0.8+）

```
editors/nvim/
├── ftdetect/mylis.vim
├── syntax/mylis.vim
└── lua/mylis.lua
```

```lua
-- init.lua
vim.opt.rtp:append("~/mylis/editors/nvim")
require("mylis").setup()                       -- 自动找 myls
-- 或显式指定：
-- require("mylis").setup({ server = "/home/me/mylis/bin/myls" })
```

走 Neovim 内置 `vim.lsp.start`，不需要 lspconfig/nvim-lspconfig；同一 buffer 复用同一个 client。
buffer 内自动加两个映射：`gd`（跳转定义）、`K`（悬停签名）；补全用原生的 `Ctrl+Space` / `Ctrl+X Ctrl+O`。

---

## 常见问题

| 现象 | 原因 / 解决 |
| --- | --- |
| 没有诊断 | `myls` 没找到：设置 `mylis.serverPath` 或 `MYLIS_SERVER` 指向 `bin/myls`；看 `Mylis LSP` 输出面板 |
| 诊断和 `mylc` 不一致 | 不可能：两者共用同一份前端；若出现请先 `mylc --check` 对照 |
| 中文注释乱码 | 文件需存为 UTF-8（Mylis 源码按 UTF-8 读取） |
| Windows 下启动失败 | `myls.exe` 与 `mylc.exe` 同目录；把该目录加入 PATH 或写死 `serverPath` |

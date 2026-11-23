-- Mylis 语言服务器接入（Neovim 0.8+ 内置 vim.lsp.start，无需插件管理器）
--
-- 安装：把 editors/nvim/* 拷到 ~/.config/nvim/，然后：
--   require("mylis").setup()                       -- 自动查找 myls
--   require("mylis").setup({ server = "/path/to/bin/myls" })
--
-- 内置快捷键（buffer-local）：
--   gd   跳转定义      gr   查找引用      K    悬停签名
--   <leader>rn  重命名  <C-k> 函数签名    <C-x><C-o> 补全（omnifunc）
local M = {}

local function exe_names()
  if vim.fn.has("win32") == 1 or vim.fn.has("win64") == 1 then
    return { "myls.exe", "myls" }
  end
  return { "myls", "myls.exe" }
end

local uv = vim.uv or vim.loop

local function exists(p)
  return uv.fs_stat(p) ~= nil
end

local function join(a, b)
  local sep = (a:sub(-1) == "/" or a:sub(-1) == "\\") and "" or "/"
  return a .. sep .. b
end

--- 查找顺序：opts.server → $MYLIS_SERVER → 工程 ./bin → 工程上级 ./bin → PATH
function M.find_server(server)
  if server and server ~= "" then return server end
  local env = vim.env.MYLIS_SERVER
  if env and env ~= "" then return env end
  local cwd = vim.fn.getcwd()
  for _, name in ipairs(exe_names()) do
    for _, dir in ipairs({ join(cwd, "bin"), join(cwd, ".."), join(cwd, "../bin") }) do
      local p = join(dir, name)
      if exists(p) then return p end
    end
  end
  return exe_names()[1] -- 交给 PATH
end

local function root_dir(bufnr)
  local name = vim.api.nvim_buf_get_name(bufnr)
  local dir = name ~= "" and vim.fn.fnamemodify(name, ":p:h") or vim.fn.getcwd()
  if dir == "" then dir = vim.fn.getcwd() end
  while true do
    for _, marker in ipairs({ "mylis.toml", ".git", "Makefile" }) do
      if exists(join(dir, marker)) then return dir end
    end
    local parent = vim.fn.fnamemodify(dir, ":h")
    if parent == dir or parent == "" then break end
    dir = parent
  end
  return vim.fn.getcwd()
end

function M.setup(opts)
  opts = opts or {}
  local server = M.find_server(opts.server)
  local client_id = nil

  local function attach(bufnr)
    if client_id then
      local ok = pcall(vim.lsp.buf_attach_client, bufnr, client_id)
      if ok then return end
    end
    client_id = vim.lsp.start({
      name = "myls",
      cmd = { server, "--stdio" },
      root_dir = root_dir(bufnr),
      capabilities = vim.lsp.protocol.make_client_capabilities(),
    }, { bufnr = bufnr })
    if not client_id then
      vim.notify("myls 启动失败：" .. server, vim.log.levels.WARN)
    end
  end

  vim.api.nvim_create_autocmd("FileType", {
    pattern = "mylis",
    callback = function(args)
      attach(args.buf)
      pcall(function()
        if vim.bo[args.buf].omnifunc == "" then
          vim.bo[args.buf].omnifunc = "v:lua.vim.lsp.omnifunc"
        end
      end)
      vim.keymap.set("n", "gd", vim.lsp.buf.definition, { buffer = args.buf, desc = "Mylis: 跳转定义" })
      vim.keymap.set("n", "gr", vim.lsp.buf.references, { buffer = args.buf, desc = "Mylis: 查找引用" })
      vim.keymap.set("n", "K", vim.lsp.buf.hover, { buffer = args.buf, desc = "Mylis: 悬停" })
      vim.keymap.set("n", "<leader>rn", vim.lsp.buf.rename, { buffer = args.buf, desc = "Mylis: 重命名" })
      vim.keymap.set("n", "<C-k>", vim.lsp.buf.signature_help, { buffer = args.buf, desc = "Mylis: 函数签名" })
    end,
  })

  vim.api.nvim_create_autocmd("BufRead,BufNewFile", {
    pattern = "*.myl",
    callback = function() vim.bo.filetype = "mylis" end,
  })

  vim.api.nvim_create_user_command("MylisWorkspaceSymbols", function(args)
    if vim.lsp.buf.workspace_symbol then
      vim.lsp.buf.workspace_symbol(args.args)
    else
      vim.notify("当前 Neovim 不支持 workspace/symbol", vim.log.levels.WARN)
    end
  end, { nargs = "?" })
end

return M

-- Minimal Neovim init for Layer 2 live LSP dogfooding:
--   nvim -u tests/dogfood_init.lua <file>
local root = vim.fn.getcwd()
vim.opt.runtimepath:prepend(root)
vim.cmd("runtime plugin/lineage.lua")

require("lineage").setup({
  lsp = { timeout_ms = 150 },
})

local function start_lsp_if_available(name, cmd, filetypes, root_markers)
  if vim.fn.executable(cmd[1]) ~= 1 then
    return
  end
  vim.api.nvim_create_autocmd("FileType", {
    pattern = filetypes,
    callback = function(ev)
      local fname = vim.api.nvim_buf_get_name(ev.buf)
      if fname == "" then
        return
      end
      local found = vim.fs.find(root_markers, { path = fname, upward = true })[1]
      local root_dir = found and vim.fs.dirname(found) or vim.fn.getcwd()
      vim.lsp.start({
        name = name,
        cmd = cmd,
        root_dir = root_dir,
      })
    end,
  })
end

-- TypeScript / JavaScript: prefer vtsls, fall back to typescript-language-server (ts_ls)
if vim.fn.executable("vtsls") == 1 then
  start_lsp_if_available(
    "vtsls",
    { "vtsls", "--stdio" },
    { "typescript", "typescriptreact", "javascript", "javascriptreact" },
    { "tsconfig.json", "package.json", ".git" }
  )
else
  start_lsp_if_available(
    "ts_ls",
    { "typescript-language-server", "--stdio" },
    { "typescript", "typescriptreact", "javascript", "javascriptreact" },
    { "tsconfig.json", "package.json", ".git" }
  )
end

-- Python: prefer pyright-langserver, fall back to ty
if vim.fn.executable("pyright-langserver") == 1 then
  start_lsp_if_available(
    "pyright",
    { "pyright-langserver", "--stdio" },
    { "python" },
    { "pyproject.toml", "setup.py", ".git" }
  )
else
  start_lsp_if_available("ty", { "ty", "server" }, { "python" }, { "pyproject.toml", ".git" })
end

-- Rust: rust-analyzer
start_lsp_if_available("rust_analyzer", { "rust-analyzer" }, { "rust" }, { "Cargo.toml", ".git" })

-- Go: gopls
start_lsp_if_available("gopls", { "gopls" }, { "go" }, { "go.work", "go.mod", ".git" })

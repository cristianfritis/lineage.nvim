local M = {}

M.defaults = {
  binary_path = nil,
  version = nil,
  build_from_source = false,
  hover = {
    enter_on_open = true,
    border = "rounded",
    max_width = 80,
    max_height = 24,
    icons = {
      unicode = true,
    },
    keymaps = {
      jump_cursor = "<CR>",
      jump_prefix = "g",
      toggle_unverified = "gu",
      export = "ge",
      close = { "q", "<Esc>" },
    },
  },
  lsp = {
    timeout_ms = 150,
  },
  tree = {
    height = 14,
    keymaps = {
      export = "e",
    },
  },
  export = {
    default_format = "mermaid",
    lsp_timeout_ms = 500,
    picker = true,
    html = {
      open = true,
      cache_max_files = 20,
    },
  },
  exclude_tests = false,
}

M.options = vim.deepcopy(M.defaults)

function M.setup(opts)
  M.options = vim.tbl_deep_extend("force", vim.deepcopy(M.defaults), opts or {})
  return M.options
end

function M.get()
  return M.options
end

return M

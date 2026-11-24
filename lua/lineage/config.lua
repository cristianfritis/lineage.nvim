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
      close = { "q", "<Esc>" },
    },
  },
  lsp = {
    timeout_ms = 150,
  },
  tree = {
    height = 14,
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

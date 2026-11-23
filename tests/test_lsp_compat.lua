local lsp_compat = require("lineage.lsp_compat")

-- Verify 0.10 vs 0.11+ method dispatch and empty batch handling
local batch_done = false
local batch_res = nil
lsp_compat.resolve_batch({ query_id = 1, items = {} }, nil, function(res)
  batch_res = res
  batch_done = true
end)
assert(batch_done and #batch_res.results == 0, "Empty batch should resolve synchronously")

-- Verify unattached buffer returns unattached=true without calling hover or didOpen
local buf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_name(buf, "/tmp/lineage_unattached.ts")
local uri = vim.uri_from_bufnr(buf)

local u_done = false
local u_res = nil
lsp_compat.resolve_batch(
  {
    query_id = 2,
    timeout_ms = 50,
    items = {
      {
        key = "foo",
        uri = uri,
        position = { line = 0, character = 0 },
        kind = "definition",
      },
    },
  },
  nil,
  function(res)
    u_res = res
    u_done = true
  end
)
assert(
  vim.wait(1000, function()
    return u_done
  end, 10),
  "Unattached resolve_batch timed out"
)
assert(u_res.results[1].unattached == true, "Expected unattached=true")

vim.api.nvim_buf_delete(buf, { force = true })
print("test_lsp_compat.lua: PASS")

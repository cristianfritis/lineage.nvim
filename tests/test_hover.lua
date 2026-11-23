local hover = require("lineage.ui.hover")
local jump_stack = require("lineage.ui.jump_stack")

-- 1. Verify <leader>l* is not overwritten
local leader_maps = vim.api.nvim_get_keymap("n")
for _, m in ipairs(leader_maps) do
  assert(not vim.startswith(m.lhs, " l"), "Must not overwrite <leader>l* mappings")
end

-- 2. Verify stale delta is ignored cleanly
hover.apply_delta({
  projections = {
    hover_card = {
      symbol = "stale",
      verdict_line = "stale: Unverified",
      status_badge = "Unverified",
      phase1_items = {},
      phase2_items = {},
    },
  },
}, 999999)

-- 3. Verify jump_stack push & pop roundtrip
local buf1 = vim.api.nvim_create_buf(false, true)
local buf2 = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_lines(buf1, 0, -1, false, { "line1", "line2", "line3" })
vim.api.nvim_buf_set_lines(buf2, 0, -1, false, { "target1", "target2" })
vim.api.nvim_buf_set_name(buf2, "/tmp/lineage_jump_target.ts")

vim.api.nvim_set_current_buf(buf1)
vim.api.nvim_win_set_cursor(0, { 2, 3 })
jump_stack.jump_to(0, vim.uri_from_bufnr(buf2), 1, 0)
assert(vim.api.nvim_get_current_buf() == buf2, "Should have jumped to buf2")
assert(jump_stack.pop() == true, "Should pop back to buf1")
assert(vim.api.nvim_get_current_buf() == buf1, "Should have returned to buf1")
local cur = vim.api.nvim_win_get_cursor(0)
assert(cur[1] == 2 and cur[2] == 3, "Should restore exact cursor position")

vim.api.nvim_buf_delete(buf1, { force = true })
vim.api.nvim_buf_delete(buf2, { force = true })
print("test_hover.lua: PASS")

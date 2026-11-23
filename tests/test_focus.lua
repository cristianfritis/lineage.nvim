local focus = require("lineage.ui.focus")

local buf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_name(buf, "/tmp/lineage_focus_test.ts")
vim.api.nvim_buf_set_lines(buf, 0, -1, false, {
  "function f(x: number) {",
  "  if (x === 0) return null;",
  "  const y = x + 1;",
  "  return y;",
  "}",
})
vim.api.nvim_win_set_buf(0, buf)

local uri = vim.uri_from_bufnr(buf)
local spans = {
  {
    file_id = 0,
    uri = uri,
    range = { start = { line = 1, col = 2 }, ["end"] = { line = 1, col = 26 } },
    role = "local_ssi",
    badge = "[NULL]",
    highlight = "LineageStateAbsent",
    label = "null",
  },
  {
    file_id = 0,
    uri = uri,
    range = { start = { line = 3, col = 2 }, ["end"] = { line = 3, col = 10 } },
    role = "local_ssi",
    badge = "[OK]",
    highlight = "LineageStatePresent",
    label = "y",
  },
}

-- Activate state manually and render spans
local st = focus.get_state()
st.active = true
st.pinned = false
st.stale = false
st.bufnr = buf
st.uri = uri
st.symbol = "y"
focus.render_spans(buf, spans, { uri = uri })

local ns = vim.api.nvim_create_namespace("lineage_focus")
local marks = vim.api.nvim_buf_get_extmarks(buf, ns, 0, -1, { details = true })
assert(#marks == 5, "Expected 5 extmarks across 5 lines, got " .. #marks)
assert(marks[1][4].line_hl_group == "LineageFocusDim", "Line 0 should be dimmed")
assert(marks[2][4].line_hl_group == "LineageStateAbsent", "Line 1 should have LineageStateAbsent")
assert(marks[4][4].line_hl_group == "LineageStatePresent", "Line 3 should have LineageStatePresent")

-- Verify [stale] transition on edit
focus.mark_stale()
assert(st.stale == true, "Expected stale=true after mark_stale")
assert(focus.statusline():find("%[stale%]") ~= nil, "Expected [stale] in statusline")

-- Verify pinning persists across buffer switches
st.pinned = true
local buf2 = vim.api.nvim_create_buf(false, true)
vim.api.nvim_win_set_buf(0, buf2)
assert(focus.get_state().active == true and focus.get_state().pinned == true, "Pin should remain active across buffer switch")

-- Verify clear removes all extmarks and resets state
focus.clear()
assert(focus.get_state().active == false, "Expected active=false after clear")
local marks_after = vim.api.nvim_buf_get_extmarks(buf, ns, 0, -1, {})
assert(#marks_after == 0, "Expected 0 extmarks after clear")

vim.api.nvim_buf_delete(buf, { force = true })
vim.api.nvim_buf_delete(buf2, { force = true })
print("test_focus.lua: PASS")

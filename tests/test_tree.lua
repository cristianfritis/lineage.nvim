local tree = require("lineage.ui.tree")

local buf = vim.api.nvim_create_buf(false, true)
local tmpfile = vim.fn.tempname() .. ".ts"
vim.fn.writefile({
  "export function findSession(id: string) {",
  "  if (!id) return null;",
  "  return id;",
  "}",
}, tmpfile)
vim.api.nvim_buf_set_name(buf, tmpfile)
vim.api.nvim_win_set_buf(0, buf)

local uri = vim.uri_from_bufnr(buf)
tree.open("to", { skip_query = true })

local rows = {
  {
    node_id = "n0",
    depth = 0,
    file_id = 0,
    uri = uri,
    range = { start = { line = 0, col = 0 }, ["end"] = { line = 0, col = 10 } },
    label = "findSession",
    badge = "[NULL]",
    is_cycle = false,
    is_frontier = false,
    is_absent = true,
    group_key = "findSession",
    provenance = "local_ssi",
    has_children = true,
  },
  {
    node_id = "n1",
    depth = 1,
    file_id = 0,
    uri = uri,
    range = { start = { line = 1, col = 2 }, ["end"] = { line = 1, col = 20 } },
    label = "return null",
    badge = "[NULL] [cycle]",
    is_cycle = true,
    is_frontier = false,
    is_absent = true,
    group_key = "findSession",
    provenance = "local_ssi",
    has_children = false,
  },
  {
    node_id = "n2",
    depth = 1,
    file_id = 0,
    uri = uri,
    range = { start = { line = 2, col = 2 }, ["end"] = { line = 2, col = 12 } },
    label = "return id",
    badge = "[OK] [+]",
    is_cycle = false,
    is_frontier = true,
    expansion_token = "exp:n2:0:2",
    is_absent = false,
    group_key = "findSession",
    provenance = "local_ssi",
    has_children = false,
  },
}

tree.set_rows(rows, { "MaxDepth" }, { direction = "to" })
local st = tree.get_state()
assert(st.winid and vim.api.nvim_win_is_valid(st.winid), "Tree split should be open")
assert(st.bufnr and vim.api.nvim_buf_is_valid(st.bufnr), "Tree buffer should be valid")

local rendered = vim.api.nvim_buf_get_lines(st.bufnr, 0, -1, false)
assert(rendered[1]:find("%[partial%]") ~= nil, "Header should display [partial] when truncated")

-- Verify floating preview opened without stealing focus
assert(vim.api.nvim_get_current_win() == st.winid, "Preview must not steal focus from tree split")
if st.preview_win and vim.api.nvim_win_is_valid(st.preview_win) then
  local cfg = vim.api.nvim_win_get_config(st.preview_win)
  assert(cfg.focusable == false, "Preview float must be non-focusable")
end

-- Verify null-only filter hides non-absent row n2
st.null_only = true
tree.render()
local filtered_lines = table.concat(vim.api.nvim_buf_get_lines(st.bufnr, 0, -1, false), "\n")
assert(filtered_lines:find("return id") == nil, "Null-only filter should hide non-absent row")

tree.close()
assert(st.winid == nil, "Tree window should be nil after close")
vim.fn.delete(tmpfile)
vim.api.nvim_buf_delete(buf, { force = true })
print("test_tree.lua: PASS")

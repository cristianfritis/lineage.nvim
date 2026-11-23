local M = {}

local stack = {}

function M.push(origin_buf, origin_pos)
  table.insert(stack, {
    bufnr = origin_buf,
    pos = origin_pos,
  })
end

function M.jump_to(origin_win, uri, line0, col0)
  local target_win = vim.api.nvim_win_is_valid(origin_win) and origin_win
    or vim.api.nvim_get_current_win()
  local origin_buf = vim.api.nvim_win_get_buf(target_win)
  local origin_pos = vim.api.nvim_win_get_cursor(target_win)

  vim.api.nvim_set_current_win(target_win)
  vim.cmd("normal! m'")
  M.push(origin_buf, origin_pos)

  local target_buf = vim.uri_to_bufnr(uri)
  if not vim.api.nvim_buf_is_loaded(target_buf) then
    vim.fn.bufload(target_buf)
  end
  vim.api.nvim_win_set_buf(target_win, target_buf)

  local line_count = vim.api.nvim_buf_line_count(target_buf)
  local target_line = math.max(1, math.min(line0 + 1, line_count))
  pcall(vim.api.nvim_win_set_cursor, target_win, { target_line, math.max(0, col0) })
end

function M.pop()
  local entry = table.remove(stack)
  if not entry then
    vim.notify("lineage.nvim: jump stack is empty", vim.log.levels.INFO)
    return false
  end
  if vim.api.nvim_buf_is_valid(entry.bufnr) then
    vim.cmd("normal! m'")
    vim.api.nvim_set_current_buf(entry.bufnr)
    pcall(vim.api.nvim_win_set_cursor, 0, entry.pos)
    return true
  end
  return false
end

function M.depth()
  return #stack
end

function M.clear()
  stack = {}
end

return M

local M = {}

M.MAX_SYNC_BYTES = 1024 * 1024

-- Map of bufnr -> last synced b:changedtick
M.synced_ticks = {}

function M.reset()
  M.synced_ticks = {}
end

function M.buf_uri(bufnr)
  local name = vim.api.nvim_buf_get_name(bufnr)
  if name == "" then
    return nil
  end
  return vim.uri_from_bufnr(bufnr)
end

function M.sync_if_dirty(bufnr, client)
  if not vim.api.nvim_buf_is_valid(bufnr) then
    return true
  end
  local uri = M.buf_uri(bufnr)
  if not uri then
    return false
  end

  local is_modified = vim.bo[bufnr].modified
  if not is_modified and M.synced_ticks[bufnr] == nil then
    return true
  end

  local tick = vim.api.nvim_buf_get_changedtick(bufnr)
  if M.synced_ticks[bufnr] == tick then
    return true
  end

  local lines = vim.api.nvim_buf_get_lines(bufnr, 0, -1, false)
  local text = table.concat(lines, "\n")
  if #text > M.MAX_SYNC_BYTES then
    return false, "file_too_large"
  end

  client.notify("lineage/syncBuffer", {
    uri = uri,
    changedtick = tick,
    text = text,
  })
  M.synced_ticks[bufnr] = tick
  return true
end

function M.on_buf_close(bufnr)
  local had_sync = M.synced_ticks[bufnr] ~= nil
  M.synced_ticks[bufnr] = nil
  if had_sync and vim.api.nvim_buf_is_valid(bufnr) then
    local uri = M.buf_uri(bufnr)
    if uri then
      local client = require("lineage.client")
      if client.is_running() then
        client.notify("lineage/closeBuffer", { uri = uri })
      end
    end
  end
end

return M

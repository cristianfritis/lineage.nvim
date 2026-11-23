local client = require("lineage.client")
local sync = require("lineage.sync")

local M = {}

local function open_scratch(title, lines)
  vim.cmd("botright new")
  local buf = vim.api.nvim_get_current_buf()
  vim.bo[buf].buftype = "nofile"
  vim.bo[buf].bufhidden = "wipe"
  vim.bo[buf].swapfile = false
  vim.api.nvim_buf_set_name(buf, title)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.bo[buf].modifiable = false
end

function M.show_status()
  client.request("lineage/status", {}, function(err, res)
    if err then
      vim.notify(
        "lineage.nvim: status failed: " .. tostring(err.message or err),
        vim.log.levels.ERROR
      )
      return
    end
    local lines = vim.split(vim.json.encode(res), "\n")
    local pretty = {
      "lineage-engine status:",
      "  protocol_version: " .. tostring(res.protocol_version),
      "  engine_version:   " .. tostring(res.engine_version),
      "  cache_bytes:      " .. tostring(res.cache_bytes) .. " / " .. tostring(res.cache_cap_bytes),
      "  dirty_buffers:    "
        .. tostring(res.dirty_buffers)
        .. " ("
        .. tostring(res.dirty_bytes)
        .. " bytes)",
      "  grammars:         " .. table.concat(res.loaded_grammars or {}, ", "),
      "  log_path:         " .. tostring(res.log_path or "none"),
    }
    if #lines == 0 then
      pretty = {}
    end
    open_scratch("lineage://status", pretty)
  end)
end

function M.show_log()
  local tail = client.get_stderr_tail()
  if #tail == 0 then
    tail = { "(stderr ring buffer is empty)" }
  end
  open_scratch("lineage://log", tail)
end

function M.debug_dump()
  local uri = sync.buf_uri(0)
  if not uri then
    vim.notify("lineage.nvim: current buffer has no URI", vim.log.levels.WARN)
    return
  end
  sync.sync_if_dirty(0, client)
  client.request("lineage/debugDump", { uri = uri }, function(err, res)
    if err then
      vim.notify(
        "lineage.nvim: debug-dump failed: " .. tostring(err.message or err),
        vim.log.levels.ERROR
      )
      return
    end
    local ir_text = (res and res.ir) or ""
    open_scratch("lineage://debug-dump", vim.split(ir_text, "\n"))
  end)
end

return M

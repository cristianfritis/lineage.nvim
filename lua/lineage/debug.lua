local client = require("lineage.client")
local sync = require("lineage.sync")

local M = {}

local uv = vim.uv or vim.loop

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

local function format_rss(rss_bytes)
  if type(rss_bytes) ~= "number" then
    return "n/a"
  end
  return string.format("%.1f MB (%d bytes)", rss_bytes / (1024 * 1024), rss_bytes)
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
    local pretty = {
      "lineage-engine status:",
      "  protocol_version: " .. tostring(res.protocol_version),
      "  engine_version:   " .. tostring(res.engine_version),
      "  rss_bytes:        " .. format_rss(res.rss_bytes),
      "  cache_bytes:      " .. tostring(res.cache_bytes) .. " / " .. tostring(res.cache_cap_bytes),
      "  dirty_buffers:    "
        .. tostring(res.dirty_buffers)
        .. " ("
        .. tostring(res.dirty_bytes)
        .. " bytes)",
      "  grammars:         " .. table.concat(res.loaded_grammars or {}, ", "),
      "  log_path:         " .. tostring(res.log_path or "none"),
    }
    local ewma_clients = res.ewma_clients or {}
    table.insert(pretty, "  ewma_clients:     " .. tostring(#ewma_clients))
    for _, ewma in ipairs(ewma_clients) do
      table.insert(
        pretty,
        string.format(
          "    - %s @ %s: %.1fms | circuit=%s | timeouts=%d",
          ewma.client_name,
          ewma.workspace_root,
          ewma.ewma_ms,
          ewma.circuit_state,
          ewma.consecutive_timeouts
        )
      )
    end
    local telemetry = client.get_telemetry()
    table.insert(pretty, "  telemetry_ring:   " .. tostring(#telemetry) .. " queries")
    if #telemetry > 0 then
      local last = telemetry[#telemetry]
      table.insert(
        pretty,
        string.format(
          "    - last qid=%s trace=%s phase1=%sms phase2=%sms rtt=%sms delta=%s",
          tostring(last.query_id),
          tostring(last.trace_id),
          tostring(last.phase1_ms),
          tostring(last.phase2_ms),
          tostring(last.rtt_ms),
          tostring(last.delta_received)
        )
      )
    end
    open_scratch("lineage://status", pretty)
  end)
end

local function find_newest_log_file(base_path)
  if not base_path or base_path == "" then
    return nil
  end
  local matches = vim.fn.glob(base_path .. "*", false, true)
  if #matches == 0 then
    if vim.fn.filereadable(base_path) == 1 then
      return base_path
    end
    return nil
  end
  local best_path = nil
  local best_mtime = -1
  for _, p in ipairs(matches) do
    local st = uv.fs_stat(p)
    if st and st.type == "file" then
      local mtime = (st.mtime and st.mtime.sec) or 0
      if mtime >= best_mtime then
        best_mtime = mtime
        best_path = p
      end
    end
  end
  return best_path or (vim.fn.filereadable(base_path) == 1 and base_path or nil)
end

function M.show_log()
  local lines = {}
  local tail = client.get_stderr_tail()
  table.insert(lines, "=== stderr ring ===")
  if #tail == 0 then
    table.insert(lines, "(stderr ring buffer is empty)")
  else
    for _, l in ipairs(tail) do
      table.insert(lines, l)
    end
  end

  local default_log = (uv.os_tmpdir() or "/tmp") .. "/lineage-engine-logs/lineage-engine.log"
  local function finish_with_log_path(log_path)
    local newest = find_newest_log_file(log_path or default_log)
    table.insert(lines, "")
    table.insert(lines, "=== rotating log: " .. tostring(newest or "none") .. " ===")
    if newest and vim.fn.filereadable(newest) == 1 then
      local file_lines = vim.fn.readfile(newest)
      local start_idx = math.max(1, #file_lines - 199)
      for i = start_idx, #file_lines do
        table.insert(lines, file_lines[i])
      end
    else
      table.insert(lines, "(no rotating log file found)")
    end
    open_scratch("lineage://log", lines)
  end

  if client.is_running() then
    client.request("lineage/status", {}, function(err, res)
      finish_with_log_path((not err and res and res.log_path) or default_log)
    end)
  else
    finish_with_log_path(default_log)
  end
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

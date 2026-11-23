local client = require("lineage.client")
local lsp_compat = require("lineage.lsp_compat")

local M = {}

function M.check()
  vim.health.start("lineage.nvim")

  -- 1. Neovim version (>= 0.10)
  if vim.fn.has("nvim-0.10") == 1 then
    local v = vim.version()
    vim.health.ok(string.format("Neovim %d.%d.%d (>= 0.10)", v.major, v.minor, v.patch))
  else
    vim.health.error("Neovim >= 0.10 is required")
  end

  -- 2. Binary discovery
  local bin = client.find_binary()
  if bin then
    vim.health.ok("lineage-engine binary: " .. bin)
  else
    vim.health.error("lineage-engine binary not found (run `cargo build --release` in engine/)")
  end

  -- 3. Attached LSP clients
  local clients = lsp_compat.get_clients(0)
  if #clients == 0 then
    vim.health.info(
      "No LSP clients attached to current buffer (Phase 1 static analysis still active)"
    )
  else
    for _, c in ipairs(clients) do
      local caps = {}
      for _, m in ipairs({ "definition", "typeDefinition", "implementation" }) do
        if lsp_compat.client_supports(c, "textDocument/" .. m) then
          table.insert(caps, m)
        end
      end
      vim.health.ok(
        string.format(
          "LSP client '%s' (offset_encoding=%s, methods=[%s])",
          c.name,
          tostring(c.offset_encoding or "utf-16"),
          table.concat(caps, ", ")
        )
      )
    end
  end

  -- 4. Sidecar live status ping (if running or binary available)
  if bin then
    local done = false
    local status_res = nil
    local status_err = nil
    local uv = vim.uv or vim.loop
    local t0 = uv.hrtime()
    client.request("lineage/status", {}, function(err, res)
      status_err = err
      status_res = res
      done = true
    end)
    vim.wait(1000, function()
      return done
    end, 10)
    local ping_ms = math.floor((uv.hrtime() - t0) / 1e6)

    if status_res then
      vim.health.ok(
        string.format(
          "Sidecar ping: %dms | protocol_version=%s | engine_version=%s",
          ping_ms,
          tostring(status_res.protocol_version),
          tostring(status_res.engine_version)
        )
      )
      vim.health.info("Loaded grammars: " .. table.concat(status_res.loaded_grammars or {}, ", "))
      vim.health.info(
        string.format(
          "LRU cache: %d / %d bytes | dirty buffers: %d (%d bytes)",
          status_res.cache_bytes or 0,
          status_res.cache_cap_bytes or 0,
          status_res.dirty_buffers or 0,
          status_res.dirty_bytes or 0
        )
      )
      if status_res.log_path then
        vim.health.info("Rotating log path: " .. status_res.log_path)
      end
      for _, ewma in ipairs(status_res.ewma_clients or {}) do
        vim.health.info(
          string.format(
            "EWMA [%s @ %s]: %.1fms | circuit=%s | timeouts=%d",
            ewma.client_name,
            ewma.workspace_root,
            ewma.ewma_ms,
            ewma.circuit_state,
            ewma.consecutive_timeouts
          )
        )
      end
    elseif status_err then
      vim.health.warn("Sidecar status error: " .. tostring(status_err.message or status_err))
    else
      vim.health.warn("Sidecar status timed out")
    end
  end
end

return M

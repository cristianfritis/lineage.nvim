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
  local bin, bin_source = client.find_binary()
  local install = require("lineage.install")
  local pinned = install.resolve_pinned_binary()
  if bin then
    local tag_info = pinned and pinned.engine_tag or "unknown"
    local prov = (pinned and pinned.path == bin) and pinned.provenance or "dev/custom"
    vim.health.ok(
      string.format(
        "lineage-engine binary: %s [%s | engine_tag=%s | provenance=%s]",
        bin,
        bin_source or "unknown source",
        tag_info,
        prov
      )
    )
  else
    vim.health.error(
      "lineage-engine binary not found. Run :Lineage install (or `cargo build --release` in engine/)"
    )
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
      for _, m in ipairs({
        "definition",
        "typeDefinition",
        "implementation",
        "prepareCallHierarchy",
        "references",
      }) do
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
          "LRU cache: %d / %d bytes | dirty buffers: %d (%d bytes) | rss_bytes: %s",
          status_res.cache_bytes or 0,
          status_res.cache_cap_bytes or 0,
          status_res.dirty_buffers or 0,
          status_res.dirty_bytes or 0,
          status_res.rss_bytes and tostring(status_res.rss_bytes) or "n/a"
        )
      )
      if status_res.log_path then
        vim.health.info("Rotating log path: " .. status_res.log_path)
      end
      local caps = client.capabilities()
      if caps and type(caps.export_formats) == "table" then
        local fmts = table.concat(caps.export_formats, ", ")
        if vim.tbl_contains(caps.export_formats, "html") then
          vim.health.ok("Export formats: " .. fmts)
        else
          vim.health.warn(
            "Export formats: " .. fmts .. " (missing 'html'; run :Lineage install! to update)"
          )
        end
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

  -- 5. Export cache & browser open readiness
  local export_cache = require("lineage.export_cache")
  local stats = export_cache.stats()
  if stats.is_symlink then
    vim.health.error("Export cache directory is a symlink (refused): " .. stats.dir)
  elseif stats.exists and stats.is_dir then
    if not export_cache.is_windows() and not stats.mode_ok then
      vim.health.warn(
        string.format(
          "Export cache permissions %s (expected 0700): %s",
          tostring(stats.mode_octal),
          stats.dir
        )
      )
    else
      vim.health.ok(
        string.format(
          "Export cache: %s (%s%d file(s), %d bytes)",
          stats.dir,
          stats.mode_octal and ("mode " .. stats.mode_octal .. ", ") or "",
          stats.file_count,
          stats.total_bytes
        )
      )
    end
  else
    vim.health.info("Export cache: " .. stats.dir .. " (not yet created)")
  end
  if stats.can_open then
    vim.health.ok("Browser open: ready (vim.ui.open)")
  else
    vim.health.info(
      "Browser open: skipped ("
        .. tostring(stats.open_reason)
        .. "; HTML exports will save to file and notify path)"
    )
  end
end

return M

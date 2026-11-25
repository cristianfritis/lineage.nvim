local config = require("lineage.config")
local client = require("lineage.client")
local sync = require("lineage.sync")
local lsp_compat = require("lineage.lsp_compat")
local export_cache = require("lineage.export_cache")

local M = {}

local uv = vim.uv or vim.loop

local LUA_SUPPORTED_FORMATS = {
  mermaid = { label = "Mermaid (clipboard)", ext = "mmd" },
  dot = { label = "Graphviz DOT (clipboard)", ext = "dot" },
  html = { label = "HTML (browser)", ext = "html" },
}

local PICKER_ORDER = { "mermaid", "dot", "html" }

function M.supported_picker_formats(caps)
  if type(caps) ~= "table" or type(caps.export_formats) ~= "table" then
    return {}
  end
  local out = {}
  for _, fmt in ipairs(PICKER_ORDER) do
    if LUA_SUPPORTED_FORMATS[fmt] and vim.tbl_contains(caps.export_formats, fmt) then
      table.insert(out, fmt)
    end
  end
  return out
end

function M.resolve_context()
  local tree_ctx = require("lineage.ui.tree").get_export_context()
  if tree_ctx then
    return "tree", tree_ctx
  end

  local hover_ctx = require("lineage.ui.hover").get_export_context()
  if hover_ctx then
    return "hover", hover_ctx
  end

  local win = vim.api.nvim_get_current_win()
  local bufnr = vim.api.nvim_win_get_buf(win)
  local pos = vim.api.nvim_win_get_cursor(win)
  local uri = sync.buf_uri(bufnr)
  if not uri or vim.bo[bufnr].buftype ~= "" then
    vim.notify("lineage.nvim: current buffer has no file URI", vim.log.levels.WARN)
    return nil, nil
  end
  local cfg = config.get()
  return "cursor",
    {
      origin_buf = bufnr,
      origin_pos = pos,
      origin_uri = uri,
      direction = "to",
      exclude_tests = cfg.exclude_tests or false,
      null_only = false,
    }
end

local function write_file_atomic(path, content)
  local parent = vim.fn.fnamemodify(path, ":h")
  if parent and parent ~= "" and parent ~= "." then
    vim.fn.mkdir(parent, "p")
  end
  local tmp = path .. ".tmp." .. tostring(uv.hrtime())
  local fd, open_err = uv.fs_open(tmp, "w", 420) -- 0644
  if not fd then
    return false, open_err or "failed to open temporary file"
  end
  local ok_w, write_err = uv.fs_write(fd, content, 0)
  uv.fs_close(fd)
  if not ok_w then
    pcall(uv.fs_unlink, tmp)
    return false, write_err or "failed to write file"
  end
  local ok_r, ren_err = uv.fs_rename(tmp, path)
  if not ok_r then
    pcall(uv.fs_unlink, tmp)
    return false, ren_err or "failed to rename temporary file"
  end
  return true, nil
end

local function format_summary(res)
  local parts = {
    string.format("%d nodes, %d edges", res.node_count or 0, res.edge_count or 0),
  }
  if type(res.completeness_badges) == "table" and #res.completeness_badges > 0 then
    table.insert(parts, table.concat(res.completeness_badges, "; "))
  end
  if type(res.warnings) == "table" and #res.warnings > 0 then
    table.insert(parts, table.concat(res.warnings, "; "))
  end
  return table.concat(parts, " | ")
end

local function deliver_result(res, format, opts, used_snippets)
  local summary = format_summary(res)
  local snippet_tag = used_snippets and "snippets ON" or "snippets OFF"
  if opts.path then
    if not opts.force and uv.fs_stat(opts.path) ~= nil then
      local msg = string.format(
        "lineage.nvim: refusing to overwrite existing file '%s' (use :Lineage export! to overwrite)",
        opts.path
      )
      vim.notify(msg, vim.log.levels.ERROR)
      if opts.on_done then
        opts.on_done({ code = -1, message = msg }, nil)
      end
      return
    end
    local ok, write_err = write_file_atomic(opts.path, res.content or "")
    if not ok then
      local msg = "lineage.nvim: failed to write '" .. opts.path .. "': " .. tostring(write_err)
      vim.notify(msg, vim.log.levels.ERROR)
      if opts.on_done then
        opts.on_done({ code = -1, message = msg }, nil)
      end
      return
    end

    local open_status = nil
    if format == "html" then
      if opts.open == true then
        local open_res =
          export_cache.open_file(opts.path, { open_fn = opts.open_fn, env = opts.env })
        open_status = open_res.status
        if open_res.status == "open_failed" then
          vim.notify(
            "lineage.nvim: browser open failed: " .. tostring(open_res.err),
            vim.log.levels.WARN
          )
        end
      else
        open_status = "saved_silent"
      end
      client.annotate_last_export_telemetry({ open_status = open_status, cache_pruned = 0 })
      res._written_path = opts.path
      res._open_status = open_status
      vim.notify(
        string.format(
          "lineage.nvim: exported html to %s (%s | %s)",
          opts.path,
          summary,
          snippet_tag
        ),
        vim.log.levels.INFO
      )
    else
      vim.notify(
        string.format("lineage.nvim: exported %s to %s (%s)", format, opts.path, summary),
        vim.log.levels.INFO
      )
    end
  elseif format == "html" then
    local cfg = config.get()
    local slug = "slice"
    if opts._origin_uri then
      local fname = vim.uri_to_fname(opts._origin_uri)
      slug = vim.fn.fnamemodify(fname, ":t:r")
    end
    local max_files = opts.cache_max_files
      or (cfg.export and cfg.export.html and cfg.export.html.cache_max_files)
      or 20
    local cache_res, cache_err = export_cache.write_cached(res.content or "", slug, {
      cache_dir = opts.cache_dir,
      max_files = max_files,
    })
    if not cache_res then
      local msg = "lineage.nvim: failed to write cached HTML: " .. tostring(cache_err)
      vim.notify(msg, vim.log.levels.ERROR)
      if opts.on_done then
        opts.on_done({ code = -1, message = msg }, nil)
      end
      return
    end

    local cfg_open = (cfg.export and cfg.export.html and cfg.export.html.open) ~= false
    local should_open = (opts.open ~= nil) and opts.open or cfg_open
    local open_status
    if should_open then
      local open_res =
        export_cache.open_file(cache_res.path, { open_fn = opts.open_fn, env = opts.env })
      open_status = open_res.status
      if open_res.status == "open_failed" then
        vim.notify(
          "lineage.nvim: browser open failed: " .. tostring(open_res.err),
          vim.log.levels.WARN
        )
      end
    else
      open_status = "skipped_flag"
    end

    client.annotate_last_export_telemetry({
      open_status = open_status,
      cache_pruned = cache_res.pruned or 0,
    })
    res._written_path = cache_res.path
    res._open_status = open_status
    res._cache_pruned = cache_res.pruned or 0
    vim.notify(
      string.format(
        "lineage.nvim: exported html to %s (%s | %s)",
        cache_res.path,
        summary,
        snippet_tag
      ),
      vim.log.levels.INFO
    )
  else
    if vim.fn.has("clipboard") == 1 then
      vim.fn.setreg("+", res.content or "")
      vim.notify(
        string.format("lineage.nvim: copied %s to + clipboard (%s)", format, summary),
        vim.log.levels.INFO
      )
    else
      vim.fn.setreg('"', res.content or "")
      vim.notify(
        string.format(
          'lineage.nvim: system clipboard unavailable; copied %s to " register (%s)',
          format,
          summary
        ),
        vim.log.levels.WARN
      )
    end
  end
  if opts.on_done then
    opts.on_done(nil, res)
  end
end

local function run_export_with_context(ctx, opts)
  local cfg = config.get()
  local format = opts.format or (cfg.export and cfg.export.default_format) or "mermaid"

  if ctx.origin_buf and vim.api.nvim_buf_is_valid(ctx.origin_buf) then
    sync.sync_if_dirty(ctx.origin_buf, client)
  end

  local clients = (ctx.origin_buf and vim.api.nvim_buf_is_valid(ctx.origin_buf))
      and lsp_compat.get_clients(ctx.origin_buf)
    or {}
  local primary = (ctx.origin_buf and vim.api.nvim_buf_is_valid(ctx.origin_buf))
      and (lsp_compat.pick_client(ctx.origin_buf, "textDocument/definition") or clients[1])
    or nil

  local include_snippets = opts.include_snippets
  if include_snippets == nil and format == "html" and not opts.path then
    include_snippets = true
  end
  local used_snippets = (include_snippets == true)

  local direction = opts.direction or ctx.direction or "to"
  local exclude_tests = (opts.exclude_tests ~= nil) and opts.exclude_tests
    or (ctx.exclude_tests or false)
  local null_only = (opts.null_only ~= nil) and opts.null_only or (ctx.null_only or false)
  local no_lsp = opts.no_lsp or false
  local lsp_timeout_ms = opts.lsp_timeout_ms or (cfg.export and cfg.export.lsp_timeout_ms) or 500

  local pos = nil
  if ctx.origin_pos then
    pos = { line = ctx.origin_pos[1] - 1, character = ctx.origin_pos[2] }
  end

  local params = {
    format = format,
    query_id = ctx.query_id,
    uri = ctx.origin_uri,
    position = pos,
    offset_encoding = "utf-8",
    direction = direction,
    exclude_tests = exclude_tests,
    null_only = null_only,
    include_snippets = include_snippets,
    no_lsp = no_lsp,
    lsp_attached = (not no_lsp) and (#clients > 0),
    lsp_client_name = primary and primary.name or nil,
    lsp_timeout_ms = lsp_timeout_ms,
    workspace_root = opts.workspace_root or vim.fn.getcwd(),
  }

  local finished = false
  local timer = uv.new_timer()
  if timer and not ctx.query_id and not no_lsp and #clients > 0 then
    timer:start(150, 0, function()
      vim.schedule(function()
        if not finished then
          vim.notify("lineage.nvim: settling LSP for graph export...", vim.log.levels.INFO)
        end
      end)
    end)
  end

  client.request("lineage/export", params, function(req_err, res)
    finished = true
    if timer then
      pcall(function()
        timer:stop()
        timer:close()
      end)
    end
    if req_err then
      local msg
      if req_err.code == -32601 then
        msg =
          "lineage.nvim: lineage-engine does not support lineage/export; run :Lineage install! to update"
      else
        msg = "lineage.nvim: export failed: " .. tostring(req_err.message or req_err)
      end
      vim.notify(msg, vim.log.levels.ERROR)
      if opts.on_done then
        opts.on_done(req_err, nil)
      end
      return
    end
    local deliver_opts = vim.tbl_extend("force", opts, { _origin_uri = ctx.origin_uri })
    deliver_result(res, format, deliver_opts, used_snippets)
  end)
end

function M.clean(opts)
  local res, err = export_cache.clean(opts)
  if not res then
    vim.notify("lineage.nvim: clean failed: " .. tostring(err), vim.log.levels.ERROR)
    return nil, err
  end
  if res.removed_files == 0 then
    vim.notify("lineage.nvim: No cached HTML exports found", vim.log.levels.INFO)
  else
    vim.notify(
      string.format(
        "lineage.nvim: removed %d cached HTML export(s) (%d bytes reclaimed)",
        res.removed_files,
        res.freed_bytes
      ),
      vim.log.levels.INFO
    )
  end
  return res, nil
end

local function check_capabilities_for_format(format)
  local caps = client.capabilities()
  if
    not caps
    or type(caps.export_formats) ~= "table"
    or not vim.tbl_contains(caps.export_formats, format)
  then
    return false,
      string.format(
        "lineage.nvim: lineage-engine does not advertise export format '%s'; run :Lineage install! to update",
        tostring(format)
      )
  end
  return true, nil
end

function M.export(opts)
  opts = opts or {}
  local cfg = config.get()
  local format = opts.format or (cfg.export and cfg.export.default_format) or "mermaid"

  if opts.path and not opts.force and uv.fs_stat(opts.path) ~= nil then
    local msg = string.format(
      "lineage.nvim: refusing to overwrite existing file '%s' (use :Lineage export! to overwrite)",
      opts.path
    )
    vim.notify(msg, vim.log.levels.ERROR)
    if opts.on_done then
      opts.on_done({ code = -1, message = msg }, nil)
    end
    return
  end

  local ctx = opts.ctx
  if not ctx then
    local _, resolved = M.resolve_context()
    ctx = resolved
  end
  if not ctx then
    if opts.on_done then
      opts.on_done({ code = -1, message = "no export context" }, nil)
    end
    return
  end

  client.ensure_started(function(err)
    if err then
      vim.notify("lineage.nvim: " .. tostring(err), vim.log.levels.ERROR)
      if opts.on_done then
        opts.on_done({ code = -32000, message = err }, nil)
      end
      return
    end

    local ok, cap_err = check_capabilities_for_format(format)
    if not ok then
      vim.notify(cap_err, vim.log.levels.ERROR)
      if opts.on_done then
        opts.on_done({ code = -32601, message = cap_err }, nil)
      end
      return
    end

    local merged_opts = vim.tbl_extend("force", opts, { format = format })
    run_export_with_context(ctx, merged_opts)
  end)
end

function M.pick_and_export(opts)
  opts = opts or {}
  local cfg = config.get()

  -- Capture context immediately before any async call or picker UI steals focus.
  local _, ctx = M.resolve_context()
  if not ctx then
    return
  end

  client.ensure_started(function(err)
    if err then
      vim.notify("lineage.nvim: " .. tostring(err), vim.log.levels.ERROR)
      return
    end
    local caps = client.capabilities()
    local available = M.supported_picker_formats(caps)
    if #available == 0 then
      vim.notify(
        "lineage.nvim: connected lineage-engine does not support graph export; run :Lineage install! to update",
        vim.log.levels.ERROR
      )
      return
    end

    local use_picker = (cfg.export and cfg.export.picker ~= false)
    if opts.format or not use_picker then
      local fmt = opts.format or (cfg.export and cfg.export.default_format) or available[1]
      local ok, cap_err = check_capabilities_for_format(fmt)
      if not ok then
        vim.notify(cap_err, vim.log.levels.ERROR)
        return
      end
      run_export_with_context(ctx, vim.tbl_extend("force", opts, { format = fmt }))
      return
    end

    vim.ui.select(available, {
      prompt = "Export Lineage Graph:",
      format_item = function(fmt)
        local info = LUA_SUPPORTED_FORMATS[fmt]
        return info and info.label or fmt
      end,
    }, function(choice)
      if not choice then
        return
      end
      run_export_with_context(ctx, vim.tbl_extend("force", opts, { format = choice }))
    end)
  end)
end

return M

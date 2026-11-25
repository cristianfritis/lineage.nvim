local config = require("lineage.config")
local client = require("lineage.client")
local sync = require("lineage.sync")
local lsp_compat = require("lineage.lsp_compat")
local highlights = require("lineage.ui.highlights")
local jump_stack = require("lineage.ui.jump_stack")

local M = {}

local ns = vim.api.nvim_create_namespace("lineage_tree")
local preview_ns = vim.api.nvim_create_namespace("lineage_tree_preview")

local state = {
  winid = nil,
  bufnr = nil,
  preview_win = nil,
  preview_buf = nil,
  origin_win = nil,
  origin_buf = nil,
  origin_pos = nil,
  origin_uri = nil,
  direction = "to",
  grouping = "origin", -- "origin" | "nullability"
  null_only = false,
  exclude_tests = false,
  rows = {},
  collapsed = {},
  expanded_children = {},
  truncation = {},
  query_id = 0,
  line_to_row = {},
  augroup = nil,
}

function M.get_state()
  return state
end

function M.get_export_context()
  if
    not state.bufnr
    or not vim.api.nvim_buf_is_valid(state.bufnr)
    or not state.winid
    or not vim.api.nvim_win_is_valid(state.winid)
    or vim.api.nvim_get_current_buf() ~= state.bufnr
  then
    return nil
  end
  return {
    active_buf = state.bufnr,
    query_id = (state.query_id and state.query_id > 0) and state.query_id or nil,
    origin_buf = state.origin_buf,
    origin_uri = state.origin_uri,
    origin_pos = state.origin_pos,
    direction = state.direction,
    exclude_tests = state.exclude_tests,
    null_only = state.null_only,
  }
end

local function close_preview()
  if state.preview_win and vim.api.nvim_win_is_valid(state.preview_win) then
    pcall(vim.api.nvim_win_close, state.preview_win, true)
  end
  state.preview_win = nil
  state.preview_buf = nil
end

function M.close()
  if state.query_id > 0 then
    client.cancel_query(state.query_id)
  end
  close_preview()
  if state.augroup then
    pcall(vim.api.nvim_del_augroup_by_id, state.augroup)
    state.augroup = nil
  end
  if state.winid and vim.api.nvim_win_is_valid(state.winid) then
    pcall(vim.api.nvim_win_close, state.winid, true)
  end
  state.winid = nil
  state.bufnr = nil
  state.line_to_row = {}
end

local function build_display_rows()
  local merged = {}
  for _, row in ipairs(state.rows) do
    table.insert(merged, row)
    local extra = state.expanded_children[row.node_id]
    if extra then
      for _, child in ipairs(extra) do
        local copy = vim.deepcopy(child)
        copy.depth = (row.depth or 0) + (child.depth or 0) + 1
        table.insert(merged, copy)
      end
    end
  end

  local filtered = {}
  local skip_depth = nil
  for _, row in ipairs(merged) do
    local d = row.depth or 0
    if skip_depth ~= nil then
      if d > skip_depth then
        goto continue
      else
        skip_depth = nil
      end
    end
    if state.collapsed[row.node_id] then
      skip_depth = d
    end
    if state.null_only and not row.is_absent then
      goto continue
    end
    table.insert(filtered, row)
    ::continue::
  end

  if state.grouping == "nullability" then
    table.sort(filtered, function(a, b)
      if a.is_absent ~= b.is_absent then
        return a.is_absent
      end
      return (a.depth or 0) < (b.depth or 0)
    end)
  end

  return filtered
end

local function update_preview()
  if not state.winid or not vim.api.nvim_win_is_valid(state.winid) then
    return
  end
  if not state.bufnr or not vim.api.nvim_buf_is_valid(state.bufnr) then
    return
  end
  local cursor = vim.api.nvim_win_get_cursor(state.winid)
  local row = state.line_to_row[cursor[1]]
  if not row or not row.uri or row.uri == "" then
    close_preview()
    return
  end

  local fname = vim.uri_to_fname(row.uri)
  local lines = {}
  local target_buf = vim.fn.bufnr(fname, false)
  if target_buf ~= -1 and vim.api.nvim_buf_is_loaded(target_buf) then
    lines = vim.api.nvim_buf_get_lines(target_buf, 0, -1, false)
  elseif vim.fn.filereadable(fname) == 1 then
    lines = vim.fn.readfile(fname)
  elseif row.snippet then
    lines = { row.snippet }
  end

  if #lines == 0 then
    close_preview()
    return
  end

  local target_line = (row.range and row.range.start and row.range.start.line or 0)
  local start_l = math.max(0, target_line - 4)
  local end_l = math.min(#lines - 1, target_line + 6)
  local slice_lines = {}
  for i = start_l, end_l do
    table.insert(slice_lines, string.format("%4d | %s", i + 1, lines[i + 1] or ""))
  end

  if not state.preview_buf or not vim.api.nvim_buf_is_valid(state.preview_buf) then
    state.preview_buf = vim.api.nvim_create_buf(false, true)
    vim.bo[state.preview_buf].buftype = "nofile"
    vim.bo[state.preview_buf].bufhidden = "wipe"
    vim.bo[state.preview_buf].modifiable = false
  end

  vim.bo[state.preview_buf].modifiable = true
  vim.api.nvim_buf_set_lines(state.preview_buf, 0, -1, false, slice_lines)
  vim.bo[state.preview_buf].modifiable = false

  vim.api.nvim_buf_clear_namespace(state.preview_buf, preview_ns, 0, -1)
  local hl_row = target_line - start_l
  if hl_row >= 0 and hl_row < #slice_lines then
    pcall(vim.api.nvim_buf_set_extmark, state.preview_buf, preview_ns, hl_row, 0, {
      line_hl_group = row.is_absent and "LineageStateAbsent" or "Visual",
    })
  end

  local width = math.min(72, math.max(36, math.floor(vim.o.columns * 0.45)))
  local height = math.max(1, #slice_lines)
  local col = math.max(0, vim.o.columns - width - 2)
  local win_row = 1

  if state.preview_win and vim.api.nvim_win_is_valid(state.preview_win) then
    pcall(vim.api.nvim_win_set_config, state.preview_win, {
      relative = "editor",
      row = win_row,
      col = col,
      width = width,
      height = height,
    })
  else
    local ok, pwin = pcall(vim.api.nvim_open_win, state.preview_buf, false, {
      relative = "editor",
      row = win_row,
      col = col,
      width = width,
      height = height,
      focusable = false,
      style = "minimal",
      border = "rounded",
      title = " " .. vim.fn.fnamemodify(fname, ":t") .. " ",
      title_pos = "center",
    })
    if ok then
      state.preview_win = pwin
    end
  end
end

function M.render()
  if not state.bufnr or not vim.api.nvim_buf_is_valid(state.bufnr) then
    return
  end
  highlights.setup()

  local partial_tag = (#(state.truncation or {}) > 0) and " [partial]" or ""
  local group_label = state.grouping == "nullability" and "Nullability" or "Root Origin"
  local null_label = state.null_only and "ON" or "OFF"
  local tests_label = state.exclude_tests and "EXCLUDED" or "INCLUDED"
  local header = string.format(
    "Lineage Tree (%s)%s | Group[g]: %s | Null-only[n]: %s | Tests[t]: %s | [<CR>:jump/expand o:fold e:export m:dir q:close]",
    string.upper(state.direction),
    partial_tag,
    group_label,
    null_label,
    tests_label
  )

  local lines = { header, string.rep("-", math.min(80, #header)) }
  local line_to_row = {}
  local hl_specs = {}

  local display_rows = build_display_rows()
  local last_group = nil
  for _, row in ipairs(display_rows) do
    local grp = state.grouping == "nullability"
        and (row.is_absent and "Null / Absent" or "Non-Null / Verified")
      or (row.group_key or "Root")
    if grp ~= last_group then
      last_group = grp
      table.insert(lines, string.format("[%s]", grp))
      table.insert(hl_specs, { line = #lines - 1, hl = "LineageHeader" })
    end

    local indent = string.rep("  ", row.depth or 0)
    local fold_icon = ""
    if row.is_frontier then
      fold_icon = "[+] "
    elseif row.has_children then
      fold_icon = state.collapsed[row.node_id] and "▸ " or "▾ "
    end
    local short_uri = (row.uri and row.uri ~= "")
        and vim.fn.fnamemodify(vim.uri_to_fname(row.uri), ":~:.")
      or ""
    local line1 = (row.range and row.range.start and row.range.start.line or 0) + 1
    local loc = short_uri ~= "" and string.format("%s:%d", short_uri, line1) or ""
    local text =
      string.format("%s%s%s %s  %s", indent, fold_icon, row.badge or "[?]", row.label or "", loc)
    table.insert(lines, text)
    line_to_row[#lines] = row

    local hl = row.is_absent and "LineageStateAbsent"
      or (row.is_cycle and "LineageTreeCycle")
      or (row.is_frontier and "LineageTreeFrontier")
      or "LineageStatePresent"
    table.insert(hl_specs, { line = #lines - 1, hl = hl })
  end

  if #display_rows == 0 then
    table.insert(lines, "  (no matching nodes)")
  end

  state.line_to_row = line_to_row

  local safe_lines = {}
  for _, l in ipairs(lines) do
    local s = tostring(l):gsub("[\r\n]+", " ")
    table.insert(safe_lines, s)
  end

  vim.bo[state.bufnr].modifiable = true
  vim.api.nvim_buf_set_lines(state.bufnr, 0, -1, false, safe_lines)
  vim.bo[state.bufnr].modifiable = false

  vim.api.nvim_buf_clear_namespace(state.bufnr, ns, 0, -1)
  for _, spec in ipairs(hl_specs) do
    pcall(vim.api.nvim_buf_set_extmark, state.bufnr, ns, spec.line, 0, {
      line_hl_group = spec.hl,
    })
  end

  update_preview()
end

function M.set_rows(rows, truncation, opts)
  opts = opts or {}
  state.rows = rows or {}
  state.truncation = truncation or {}
  if opts.direction then
    state.direction = opts.direction
  end
  if opts.exclude_tests ~= nil then
    state.exclude_tests = opts.exclude_tests
  end
  M.render()
end

function M.expand_row(row)
  if not row or not row.expansion_token then
    return
  end
  client.ensure_started(function(err)
    if err then
      return
    end
    local lnum = row.range and row.range.start and row.range.start.line or 0
    local col = row.range and row.range.start and row.range.start.col or 0
    local uri = (row.uri and row.uri ~= "") and row.uri or state.origin_uri
    local clients = lsp_compat.get_clients(state.origin_buf or 0)
    client.request("lineage/expand", {
      expansion_token = row.expansion_token,
      uri = uri,
      position = { line = lnum, character = col },
      offset_encoding = "utf-8",
      direction = state.direction,
      exclude_tests = state.exclude_tests,
      lsp_attached = #clients > 0,
      workspace_root = vim.fn.getcwd(),
      parent_query_id = (state.query_id and state.query_id > 0) and state.query_id or nil,
      parent_node_id = row.node_id,
    }, function(req_err, res)
      if req_err or not res then
        return
      end
      state.expanded_children[row.node_id] = res.tree_rows or {}
      row.is_frontier = false
      row.has_children = #state.expanded_children[row.node_id] > 0
      M.render()
    end)
  end)
end

local function bind_keymaps(bufnr)
  local map_opts = { buffer = bufnr, silent = true, nowait = true }

  vim.keymap.set("n", "<CR>", function()
    local cursor = vim.api.nvim_win_get_cursor(0)
    local row = state.line_to_row[cursor[1]]
    if not row then
      return
    end
    if row.is_frontier and row.expansion_token and not state.expanded_children[row.node_id] then
      M.expand_row(row)
      return
    end
    if row.uri and row.uri ~= "" then
      local lnum = row.range and row.range.start and row.range.start.line or 0
      local col = row.range and row.range.start and row.range.start.col or 0
      jump_stack.jump_to(state.origin_win, row.uri, lnum, col)
    end
  end, map_opts)

  vim.keymap.set("n", "o", function()
    local cursor = vim.api.nvim_win_get_cursor(0)
    local row = state.line_to_row[cursor[1]]
    if not row then
      return
    end
    if row.is_frontier and row.expansion_token and not state.expanded_children[row.node_id] then
      M.expand_row(row)
    else
      state.collapsed[row.node_id] = not state.collapsed[row.node_id]
      M.render()
    end
  end, map_opts)

  vim.keymap.set("n", "g", function()
    state.grouping = (state.grouping == "origin") and "nullability" or "origin"
    M.render()
  end, map_opts)

  vim.keymap.set("n", "n", function()
    state.null_only = not state.null_only
    M.render()
  end, map_opts)

  vim.keymap.set("n", "t", function()
    state.exclude_tests = not state.exclude_tests
    M.refresh()
  end, map_opts)

  vim.keymap.set("n", "m", function()
    if state.direction == "to" then
      state.direction = "from"
    elseif state.direction == "from" then
      state.direction = "both"
    else
      state.direction = "to"
    end
    M.refresh()
  end, map_opts)

  local cfg = config.get()
  local export_key = (cfg.tree and cfg.tree.keymaps and cfg.tree.keymaps.export) or "e"
  vim.keymap.set("n", export_key, function()
    require("lineage.export").pick_and_export()
  end, map_opts)

  for _, key in ipairs({ "q", "<Esc>" }) do
    vim.keymap.set("n", key, function()
      M.close()
    end, map_opts)
  end
end

local function ensure_split()
  if
    state.winid
    and vim.api.nvim_win_is_valid(state.winid)
    and state.bufnr
    and vim.api.nvim_buf_is_valid(state.bufnr)
  then
    return
  end

  local cfg = config.get()
  local height = (cfg.tree and cfg.tree.height) or 14
  vim.cmd("botright " .. tostring(height) .. "split")
  local winid = vim.api.nvim_get_current_win()
  local bufnr = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_win_set_buf(winid, bufnr)

  vim.bo[bufnr].buftype = "nofile"
  vim.bo[bufnr].bufhidden = "wipe"
  vim.bo[bufnr].filetype = "lineage-tree"
  vim.bo[bufnr].modifiable = false
  vim.wo[winid].number = false
  vim.wo[winid].relativenumber = false
  vim.wo[winid].wrap = false

  state.winid = winid
  state.bufnr = bufnr
  bind_keymaps(bufnr)

  if state.augroup then
    pcall(vim.api.nvim_del_augroup_by_id, state.augroup)
  end
  local group = vim.api.nvim_create_augroup("LineageTreePanel", { clear = true })
  state.augroup = group
  vim.api.nvim_create_autocmd("CursorMoved", {
    group = group,
    buffer = bufnr,
    callback = function()
      update_preview()
    end,
  })
  vim.api.nvim_create_autocmd("WinClosed", {
    group = group,
    pattern = tostring(winid),
    callback = function()
      close_preview()
      state.winid = nil
      state.bufnr = nil
    end,
  })
end

function M.refresh()
  if not state.origin_uri or not state.origin_pos then
    return
  end
  if state.query_id > 0 then
    client.cancel_query(state.query_id)
  end
  local qid = client.next_query_id()
  state.query_id = qid

  client.ensure_started(function(err)
    if err or qid ~= state.query_id then
      if err then
        vim.notify("lineage.nvim: " .. tostring(err), vim.log.levels.ERROR)
      end
      return
    end

    if state.origin_buf and vim.api.nvim_buf_is_valid(state.origin_buf) then
      sync.sync_if_dirty(state.origin_buf, client)
    end
    local clients = state.origin_buf and lsp_compat.get_clients(state.origin_buf) or {}
    local primary_client = state.origin_buf
        and (lsp_compat.pick_client(state.origin_buf, "textDocument/prepareCallHierarchy") or lsp_compat.pick_client(
          state.origin_buf,
          "textDocument/definition"
        ))
      or clients[1]
    local cfg = config.get()

    client.on_delta(qid, function(delta_env)
      client.clear_delta(qid)
      if qid ~= state.query_id then
        return
      end
      local rows = (delta_env and delta_env.projections and delta_env.projections.tree_rows) or {}
      local trunc = (delta_env and delta_env.truncation) or {}
      if #rows > 0 then
        ensure_split()
        M.set_rows(
          rows,
          trunc,
          { direction = state.direction, exclude_tests = state.exclude_tests }
        )
      end
    end)

    client.request("lineage/slice", {
      uri = state.origin_uri,
      position = { line = state.origin_pos[1] - 1, character = state.origin_pos[2] },
      offset_encoding = "utf-8",
      direction = state.direction,
      include_graph = false,
      include_focus = false,
      include_tree = true,
      include_snippets = true,
      exclude_tests = state.exclude_tests,
      query_id = qid,
      lsp_attached = #clients > 0,
      lsp_client_name = primary_client and primary_client.name or nil,
      lsp_timeout_ms = cfg.lsp and cfg.lsp.timeout_ms or 150,
      workspace_root = vim.fn.getcwd(),
    }, function(req_err, res)
      if qid ~= state.query_id then
        client.clear_delta(qid)
        return
      end
      if req_err then
        client.clear_delta(qid)
        vim.notify(
          "lineage.nvim: " .. (req_err.message or "tree slice failed"),
          vim.log.levels.ERROR
        )
        return
      end
      state.expanded_children = {}
      state.collapsed = {}
      local rows = (res and res.projections and res.projections.tree_rows) or {}
      local trunc = (res and res.truncation) or {}
      ensure_split()
      M.set_rows(rows, trunc, { direction = state.direction, exclude_tests = state.exclude_tests })
    end)
  end)
end

function M.open(direction, opts)
  opts = opts or {}
  local win = opts.origin_win or vim.api.nvim_get_current_win()
  local bufnr = opts.origin_buf or vim.api.nvim_win_get_buf(win)
  local pos = opts.origin_pos or vim.api.nvim_win_get_cursor(win)
  local uri = opts.origin_uri or sync.buf_uri(bufnr)
  if not uri then
    vim.notify("lineage.nvim: buffer has no file URI", vim.log.levels.WARN)
    return
  end

  state.origin_win = win
  state.origin_buf = bufnr
  state.origin_pos = pos
  state.origin_uri = uri
  state.direction = direction or "to"
  if opts.exclude_tests ~= nil then
    state.exclude_tests = opts.exclude_tests
  else
    state.exclude_tests = config.get().exclude_tests or false
  end

  ensure_split()
  M.set_rows({}, {}, { direction = state.direction, exclude_tests = state.exclude_tests })
  if not opts.skip_query then
    M.refresh()
  end
end

return M

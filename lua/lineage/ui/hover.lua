local config = require("lineage.config")
local client = require("lineage.client")
local sync = require("lineage.sync")
local lsp_compat = require("lineage.lsp_compat")
local highlights = require("lineage.ui.highlights")
local jump_stack = require("lineage.ui.jump_stack")

local M = {}

local active = {
  hover_id = 0,
  query_id = 0,
  bufnr = nil,
  winid = nil,
  origin_win = nil,
  origin_pos = nil,
  anchor = "NW",
  show_unverified = false,
  card = nil,
  line_to_item = {},
  index_to_item = {},
}

function M.get_active_state()
  return active
end

local function close_hover(restore_cursor)
  if active.query_id > 0 then
    client.cancel_query(active.query_id)
  end
  if active.winid and vim.api.nvim_win_is_valid(active.winid) then
    pcall(vim.api.nvim_win_close, active.winid, true)
  end
  if restore_cursor and active.origin_win and vim.api.nvim_win_is_valid(active.origin_win) then
    vim.api.nvim_set_current_win(active.origin_win)
    if active.origin_pos then
      pcall(vim.api.nvim_win_set_cursor, active.origin_win, active.origin_pos)
    end
  end
  active.winid = nil
  active.bufnr = nil
  active.card = nil
end

local function format_item_line(item)
  local short_uri = vim.fn.fnamemodify(vim.uri_to_fname(item.uri), ":~:.")
  local line1 = (item.range and item.range.start and item.range.start.line or 0) + 1
  local tag = item.is_absent_origin and " [absent]" or ""
  return string.format("  [%d] %s:%d  %s%s", item.index, short_uri, line1, item.label or "", tag)
end

local function render_card(hover_id)
  if hover_id ~= active.hover_id then
    return
  end
  if not active.bufnr or not vim.api.nvim_buf_is_valid(active.bufnr) then
    return
  end
  if not active.winid or not vim.api.nvim_win_is_valid(active.winid) then
    return
  end
  local card = active.card
  if not card then
    return
  end

  local opts = config.get()
  local use_unicode = opts.hover and opts.hover.icons and opts.hover.icons.unicode
  local lines = {}
  local line_to_item = {}
  local index_to_item = {}

  if card.is_trivial_peek and card.peek_lines and #card.peek_lines > 0 then
    for _, l in ipairs(card.peek_lines) do
      table.insert(lines, l)
    end
  else
    local verdict = card.verdict_line
      or (card.symbol .. ": " .. (card.status_badge or "Unverified"))
    if use_unicode then
      local prefix = (card.symbol or "") .. ": "
      verdict = prefix
        .. highlights.badge(card.status_badge or "", true)
        .. verdict:sub(#(prefix .. (card.status_badge or "")) + 1)
    end
    table.insert(lines, verdict)

    local p1 = card.phase1_items or {}
    local p2 = card.phase2_items or {}
    if #p1 > 0 or #p2 > 0 then
      table.insert(lines, "")
      table.insert(lines, "Origins:")
      for _, item in ipairs(p1) do
        table.insert(lines, format_item_line(item))
        line_to_item[#lines] = item
        index_to_item[item.index] = item
      end
      for _, item in ipairs(p2) do
        table.insert(lines, format_item_line(item))
        line_to_item[#lines] = item
        index_to_item[item.index] = item
      end
    end

    if card.local_risks and #card.local_risks > 0 then
      table.insert(lines, "")
      table.insert(lines, "Local Risks:")
      for _, r in ipairs(card.local_risks) do
        local lnum = (r.range and r.range.start and r.range.start.line or 0) + 1
        table.insert(lines, string.format("  ! L%d: %s", lnum, r.message))
      end
    end

    if active.show_unverified and card.unknown_boundaries and #card.unknown_boundaries > 0 then
      table.insert(lines, "")
      table.insert(lines, "Unverified Boundaries:")
      for _, b in ipairs(card.unknown_boundaries) do
        local lnum = (b.range and b.range.start and b.range.start.line or 0) + 1
        table.insert(lines, string.format("  ? %s (L%d) — %s", b.symbol, lnum, b.reason))
      end
    end
  end

  active.line_to_item = line_to_item
  active.index_to_item = index_to_item

  local safe_lines = {}
  for _, l in ipairs(lines) do
    local s = tostring(l):gsub("[\r\n]+", " ")
    table.insert(safe_lines, s)
  end

  vim.bo[active.bufnr].modifiable = true
  vim.api.nvim_buf_set_lines(active.bufnr, 0, -1, false, safe_lines)
  vim.bo[active.bufnr].modifiable = false

  local max_w = (opts.hover and opts.hover.max_width) or 80
  local max_h = (opts.hover and opts.hover.max_height) or 24
  local width = 20
  for _, l in ipairs(safe_lines) do
    width = math.max(width, #l + 2)
  end
  width = math.min(width, max_w)
  local height = math.min(#safe_lines, max_h)

  -- Resize without flipping anchor across the cursor line
  pcall(vim.api.nvim_win_set_config, active.winid, {
    width = width,
    height = math.max(1, height),
  })
end

local function jump_to_item(item)
  if not item then
    return
  end
  local origin_win = active.origin_win
  close_hover(false)
  local lnum = item.range and item.range.start and item.range.start.line or 0
  local col = item.range and item.range.start and item.range.start.col or 0
  jump_stack.jump_to(origin_win, item.uri, lnum, col)
end

local function bind_hover_keymaps(bufnr, hover_id)
  local opts = config.get()
  local km = (opts.hover and opts.hover.keymaps) or {}
  local map_opts = { buffer = bufnr, silent = true, nowait = true }

  local jump_cursor = km.jump_cursor or "<CR>"
  vim.keymap.set("n", jump_cursor, function()
    if hover_id ~= active.hover_id then
      return
    end
    local row = vim.api.nvim_win_get_cursor(0)[1]
    local item = active.line_to_item[row]
    if item then
      jump_to_item(item)
    end
  end, map_opts)

  local prefix = km.jump_prefix or "g"
  for i = 1, 9 do
    vim.keymap.set("n", prefix .. tostring(i), function()
      if hover_id ~= active.hover_id then
        return
      end
      local item = active.index_to_item[i]
      if item then
        jump_to_item(item)
      end
    end, map_opts)
  end

  local toggle_unv = km.toggle_unverified or "gu"
  vim.keymap.set("n", toggle_unv, function()
    if hover_id ~= active.hover_id then
      return
    end
    active.show_unverified = not active.show_unverified
    render_card(hover_id)
  end, map_opts)

  vim.keymap.set("n", "gs", function()
    if hover_id ~= active.hover_id then
      return
    end
    local origin_win = active.origin_win
    local origin_pos = active.origin_pos
    close_hover(true)
    local bufnr = origin_win
        and vim.api.nvim_win_is_valid(origin_win)
        and vim.api.nvim_win_get_buf(origin_win)
      or vim.api.nvim_get_current_buf()
    require("lineage.ui.focus").focus({ bufnr = bufnr, position = origin_pos })
  end, map_opts)

  vim.keymap.set("n", "gp", function()
    if hover_id ~= active.hover_id then
      return
    end
    local origin_win = active.origin_win
    local origin_pos = active.origin_pos
    close_hover(true)
    local bufnr = origin_win
        and vim.api.nvim_win_is_valid(origin_win)
        and vim.api.nvim_win_get_buf(origin_win)
      or vim.api.nvim_get_current_buf()
    require("lineage.ui.focus").pin({ bufnr = bufnr, position = origin_pos })
  end, map_opts)

  vim.keymap.set("n", "gt", function()
    if hover_id ~= active.hover_id then
      return
    end
    local origin_win = active.origin_win
    local origin_pos = active.origin_pos
    close_hover(true)
    require("lineage.ui.tree").open("to", { origin_win = origin_win, origin_pos = origin_pos })
  end, map_opts)

  vim.keymap.set("n", "gf", function()
    if hover_id ~= active.hover_id then
      return
    end
    local origin_win = active.origin_win
    local origin_pos = active.origin_pos
    close_hover(true)
    require("lineage.ui.tree").open("from", { origin_win = origin_win, origin_pos = origin_pos })
  end, map_opts)

  vim.keymap.set("n", "gn", function()
    if hover_id ~= active.hover_id then
      return
    end
    local origin_win = active.origin_win
    local origin_pos = active.origin_pos
    close_hover(true)
    require("lineage.ui.tree").open("both", { origin_win = origin_win, origin_pos = origin_pos })
  end, map_opts)

  local closes = km.close or { "q", "<Esc>" }
  if type(closes) == "string" then
    closes = { closes }
  end
  for _, key in ipairs(closes) do
    vim.keymap.set("n", key, function()
      if hover_id == active.hover_id then
        close_hover(true)
      end
    end, map_opts)
  end
end

local function open_float_with_card(envelope, hover_id)
  if hover_id ~= active.hover_id then
    return
  end
  local card = envelope and envelope.projections and envelope.projections.hover_card
  if not card then
    return
  end

  active.card = card
  active.show_unverified = false

  local opts = config.get()
  local bufnr = vim.api.nvim_create_buf(false, true)
  vim.bo[bufnr].buftype = "nofile"
  vim.bo[bufnr].bufhidden = "wipe"
  vim.bo[bufnr].filetype = "lineage-hover"
  vim.bo[bufnr].modifiable = false

  -- Compute single anchor once on open (NW below cursor if space, else SW above)
  local screen_row = vim.fn.screenrow()
  local anchor = (screen_row + 10 <= vim.o.lines) and "NW" or "SW"
  local row_offset = (anchor == "NW") and 1 or 0
  active.anchor = anchor

  local enter_on_open = true
  if opts.hover and opts.hover.enter_on_open ~= nil then
    enter_on_open = opts.hover.enter_on_open
  end

  local winid = vim.api.nvim_open_win(bufnr, enter_on_open, {
    relative = "cursor",
    row = row_offset,
    col = 0,
    width = 40,
    height = 3,
    anchor = anchor,
    style = "minimal",
    border = (opts.hover and opts.hover.border) or "rounded",
  })

  active.bufnr = bufnr
  active.winid = winid
  bind_hover_keymaps(bufnr, hover_id)
  render_card(hover_id)
end

--- Merge Phase 2 items onto Phase 1 without renumbering [1]..[k] or duplicating (file_id, range).
function M.apply_delta(envelope, hover_id)
  if hover_id ~= active.hover_id then
    return
  end
  if not active.winid or not vim.api.nvim_win_is_valid(active.winid) then
    return
  end
  if not active.bufnr or not vim.api.nvim_buf_is_valid(active.bufnr) then
    return
  end
  local new_card = envelope and envelope.projections and envelope.projections.hover_card
  if not new_card or not active.card then
    return
  end

  local seen = {}
  for _, item in ipairs(active.card.phase1_items or {}) do
    local r = item.range and item.range.start or { line = 0, col = 0 }
    seen[string.format("%d:%d:%d", item.file_id or 0, r.line, r.col)] = true
  end

  local merged_p2 = {}
  local next_idx = #(active.card.phase1_items or {}) + 1
  for _, item in ipairs(new_card.phase2_items or {}) do
    if next_idx > 9 then
      break
    end
    local r = item.range and item.range.start or { line = 0, col = 0 }
    local key = string.format("%d:%d:%d", item.file_id or 0, r.line, r.col)
    if not seen[key] then
      seen[key] = true
      local copy = vim.deepcopy(item)
      copy.index = next_idx
      next_idx = next_idx + 1
      table.insert(merged_p2, copy)
    end
  end

  active.card.phase2_items = merged_p2
  active.card.status_badge = new_card.status_badge or active.card.status_badge
  if new_card.is_trivial_peek ~= nil then
    active.card.is_trivial_peek = new_card.is_trivial_peek
  end
  active.card.unknown_boundaries = new_card.unknown_boundaries or active.card.unknown_boundaries
  active.card.unverified_count = new_card.unverified_count or active.card.unverified_count
  active.card.verdict_line = new_card.verdict_line or active.card.verdict_line
  client.clear_delta(active.query_id)
  render_card(hover_id)
end

function M.open()
  close_hover(false)

  local origin_win = vim.api.nvim_get_current_win()
  local origin_buf = vim.api.nvim_win_get_buf(origin_win)
  local origin_pos = vim.api.nvim_win_get_cursor(origin_win)
  local uri = sync.buf_uri(origin_buf)
  if not uri then
    vim.notify("lineage.nvim: buffer has no file URI", vim.log.levels.WARN)
    return
  end

  active.hover_id = active.hover_id + 1
  active.query_id = client.next_query_id()
  active.origin_win = origin_win
  active.origin_pos = origin_pos

  local hover_id = active.hover_id
  local query_id = active.query_id

  client.ensure_started(function(err)
    if err or hover_id ~= active.hover_id then
      if err then
        vim.notify("lineage.nvim: " .. tostring(err), vim.log.levels.ERROR)
      end
      return
    end

    local ok_sync, sync_err = sync.sync_if_dirty(origin_buf, client)
    if not ok_sync then
      vim.notify("lineage.nvim: skipped sync (" .. tostring(sync_err) .. ")", vim.log.levels.WARN)
    end

    local clients = lsp_compat.get_clients(origin_buf)
    local lsp_attached = #clients > 0
    local primary_client = lsp_compat.pick_client(origin_buf, "textDocument/definition")
      or clients[1]
    local cfg = config.get()

    client.on_delta(query_id, function(delta_env)
      M.apply_delta(delta_env, hover_id)
    end)

    client.request("lineage/slice", {
      uri = uri,
      position = { line = origin_pos[1] - 1, character = origin_pos[2] },
      offset_encoding = "utf-8",
      direction = "to",
      include_graph = false,
      include_snippets = true,
      exclude_tests = cfg.exclude_tests or false,
      query_id = query_id,
      lsp_attached = lsp_attached,
      lsp_client_name = primary_client and primary_client.name or nil,
      lsp_timeout_ms = cfg.lsp and cfg.lsp.timeout_ms or 150,
      workspace_root = vim.fn.getcwd(),
    }, function(req_err, res)
      if hover_id ~= active.hover_id then
        client.clear_delta(query_id)
        return
      end
      if req_err then
        client.clear_delta(query_id)
        vim.notify("lineage.nvim: " .. (req_err.message or "slice failed"), vim.log.levels.ERROR)
        return
      end
      open_float_with_card(res, hover_id)
    end)
  end)
end

return M

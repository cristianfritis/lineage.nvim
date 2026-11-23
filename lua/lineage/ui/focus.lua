local config = require("lineage.config")
local client = require("lineage.client")
local sync = require("lineage.sync")
local lsp_compat = require("lineage.lsp_compat")
local highlights = require("lineage.ui.highlights")

local M = {}

local ns = vim.api.nvim_create_namespace("lineage_focus")
local indicator_ns = vim.api.nvim_create_namespace("lineage_focus_indicator")

local state = {
  active = false,
  pinned = false,
  stale = false,
  uri = nil,
  bufnr = nil,
  position = nil,
  symbol = nil,
  spans = {},
  augroup = nil,
  debounce_timer = nil,
  query_id = 0,
}

function M.get_state()
  return state
end

function M.statusline()
  if not state.active then
    return ""
  end
  local mode = state.pinned and "PIN" or "FOCUS"
  local sym = state.symbol and (" " .. state.symbol) or ""
  local stale = state.stale and " [stale]" or ""
  return string.format("[Lineage: %s%s%s]", mode, sym, stale)
end

local function format_badge(badge, hl, use_unicode)
  if not use_unicode then
    return badge
  end
  if hl == "LineageStateAbsent" or badge == "[NULL]" then
    return "○ " .. badge
  elseif hl == "LineageStateUnknown" or badge == "[?]" then
    return "? " .. badge
  else
    return "● " .. badge
  end
end

local function render_indicator(bufnr)
  if not bufnr or not vim.api.nvim_buf_is_valid(bufnr) then
    return
  end
  vim.api.nvim_buf_clear_namespace(bufnr, indicator_ns, 0, -1)
  if not state.active then
    return
  end
  local line_count = vim.api.nvim_buf_line_count(bufnr)
  if line_count == 0 then
    return
  end
  local text = M.statusline()
  pcall(vim.api.nvim_buf_set_extmark, bufnr, indicator_ns, 0, 0, {
    virt_text = { { text, "LineageFocusIndicator" } },
    virt_text_pos = "right_align",
    priority = 200,
  })
end

function M.render_spans(bufnr, spans, opts)
  if not bufnr or not vim.api.nvim_buf_is_valid(bufnr) then
    return
  end
  highlights.setup()
  vim.api.nvim_buf_clear_namespace(bufnr, ns, 0, -1)

  opts = opts or {}
  local cfg = config.get()
  local use_unicode = cfg.hover and cfg.hover.icons and cfg.hover.icons.unicode
  local target_uri = opts.uri or sync.buf_uri(bufnr)

  local relevant_by_line = {}
  for _, span in ipairs(spans or {}) do
    local same_file = (span.file_id == 0 and (not span.uri or span.uri == "" or span.uri == target_uri))
      or (target_uri and span.uri == target_uri)
    if same_file and span.range and span.range.start then
      local lnum = span.range.start.line
      if not relevant_by_line[lnum] then
        relevant_by_line[lnum] = span
      end
    end
  end

  local line_count = vim.api.nvim_buf_line_count(bufnr)
  for lnum = 0, line_count - 1 do
    local span = relevant_by_line[lnum]
    if span then
      local hl = span.highlight or "LineageStateUnknown"
      local badge = format_badge(span.badge or "[?]", hl, use_unicode)
      pcall(vim.api.nvim_buf_set_extmark, bufnr, ns, lnum, 0, {
        line_hl_group = hl,
        virt_text = { { " " .. badge, hl } },
        virt_text_pos = "eol",
        priority = 150,
      })
    else
      pcall(vim.api.nvim_buf_set_extmark, bufnr, ns, lnum, 0, {
        line_hl_group = "LineageFocusDim",
        priority = 100,
      })
    end
  end

  render_indicator(bufnr)
end

local function stop_debounce()
  if state.debounce_timer then
    if not state.debounce_timer:is_closing() then
      state.debounce_timer:stop()
      state.debounce_timer:close()
    end
    state.debounce_timer = nil
  end
end

function M.mark_stale()
  if not state.active then
    return
  end
  state.stale = true
  render_indicator(vim.api.nvim_get_current_buf())

  stop_debounce()
  local timer = vim.uv.new_timer()
  state.debounce_timer = timer
  timer:start(400, 0, function()
    stop_debounce()
    vim.schedule(function()
      if state.active and state.stale then
        M.refresh()
      end
    end)
  end)
end

function M.clear()
  stop_debounce()
  if state.augroup then
    pcall(vim.api.nvim_del_augroup_by_id, state.augroup)
    state.augroup = nil
  end
  for _, bufnr in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_valid(bufnr) then
      vim.api.nvim_buf_clear_namespace(bufnr, ns, 0, -1)
      vim.api.nvim_buf_clear_namespace(bufnr, indicator_ns, 0, -1)
    end
  end
  state.active = false
  state.pinned = false
  state.stale = false
  state.uri = nil
  state.bufnr = nil
  state.position = nil
  state.symbol = nil
  state.spans = {}
end

local function setup_autocmds()
  if state.augroup then
    pcall(vim.api.nvim_del_augroup_by_id, state.augroup)
  end
  local group = vim.api.nvim_create_augroup("LineageFocusMode", { clear = true })
  state.augroup = group

  vim.api.nvim_create_autocmd({ "TextChanged", "TextChangedI" }, {
    group = group,
    callback = function(ev)
      if state.active and ev.buf == state.bufnr then
        M.mark_stale()
      end
    end,
  })

  vim.api.nvim_create_autocmd("BufEnter", {
    group = group,
    callback = function(ev)
      if not state.active then
        return
      end
      if state.pinned then
        render_indicator(ev.buf)
        if #state.spans > 0 then
          local bufnr_uri = sync.buf_uri(ev.buf)
          for _, sp in ipairs(state.spans) do
            if sp.uri == bufnr_uri then
              M.render_spans(ev.buf, state.spans, { uri = bufnr_uri })
              break
            end
          end
        end
      end
    end,
  })

  vim.api.nvim_create_autocmd("BufWipeout", {
    group = group,
    callback = function(ev)
      if state.active and not state.pinned and ev.buf == state.bufnr then
        M.clear()
      end
    end,
  })
end

local function fetch_and_render(bufnr, uri, pos)
  state.query_id = state.query_id + 1
  local qid = state.query_id

  client.ensure_started(function(err)
    if err or qid ~= state.query_id then
      if err then
        vim.notify("lineage.nvim: " .. tostring(err), vim.log.levels.ERROR)
      end
      return
    end

    sync.sync_if_dirty(bufnr, client)
    local clients = lsp_compat.get_clients(bufnr)
    local primary_client = clients[1]

    client.request("lineage/slice", {
      uri = uri,
      position = { line = pos[1] - 1, character = pos[2] },
      offset_encoding = "utf-8",
      direction = "to",
      include_graph = false,
      include_focus = true,
      include_tree = false,
      include_snippets = false,
      exclude_tests = config.get().exclude_tests or false,
      query_id = qid,
      lsp_attached = #clients > 0,
      lsp_client_name = primary_client and primary_client.name or nil,
      workspace_root = vim.fn.getcwd(),
    }, function(req_err, res)
      if qid ~= state.query_id or not state.active then
        return
      end
      if req_err then
        vim.notify("lineage.nvim: " .. (req_err.message or "focus slice failed"), vim.log.levels.ERROR)
        return
      end
      local card = res and res.projections and res.projections.hover_card
      if card and card.symbol then
        state.symbol = card.symbol
      end
      local spans = (res and res.projections and res.projections.focus_spans) or {}
      state.spans = spans
      state.stale = false
      M.render_spans(bufnr, spans, { uri = uri })
    end)
  end)
end

function M.refresh()
  if not state.active or not state.bufnr or not vim.api.nvim_buf_is_valid(state.bufnr) then
    return
  end
  if not state.uri or not state.position then
    return
  end
  fetch_and_render(state.bufnr, state.uri, state.position)
end

function M.focus(opts)
  opts = opts or {}
  local win = vim.api.nvim_get_current_win()
  local bufnr = opts.bufnr or vim.api.nvim_win_get_buf(win)
  local pos = opts.position or vim.api.nvim_win_get_cursor(win)
  local uri = opts.uri or sync.buf_uri(bufnr)
  if not uri then
    vim.notify("lineage.nvim: buffer has no file URI", vim.log.levels.WARN)
    return
  end

  state.active = true
  state.pinned = opts.pinned == true
  state.stale = false
  state.uri = uri
  state.bufnr = bufnr
  state.position = pos
  state.symbol = opts.symbol or vim.fn.expand("<cword>")

  setup_autocmds()
  render_indicator(bufnr)
  fetch_and_render(bufnr, uri, pos)
end

function M.pin(opts)
  opts = opts or {}
  opts.pinned = true
  M.focus(opts)
end

return M

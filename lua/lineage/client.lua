local config = require("lineage.config")
local sync = require("lineage.sync")
local lsp_compat = require("lineage.lsp_compat")

local M = {}

local uv = vim.uv or vim.loop
local PROTOCOL_VERSION = 1
local MAX_RESTARTS = 3
local STDERR_RING_MAX = 50

local state = {
  handle = nil,
  stdin = nil,
  stdout = nil,
  stderr = nil,
  buf = "",
  next_id = 1,
  pending = {},
  delta_listeners = {},
  restarts = 0,
  stderr_ring = {},
  initialized = false,
  protocol_mismatch = false,
  binary_path = nil,
}

local function push_stderr(chunk)
  for line in chunk:gmatch("[^\r\n]+") do
    table.insert(state.stderr_ring, line)
    if #state.stderr_ring > STDERR_RING_MAX then
      table.remove(state.stderr_ring, 1)
    end
  end
end

function M.get_stderr_tail()
  return vim.deepcopy(state.stderr_ring)
end

function M.find_binary(opts)
  opts = opts or config.get()
  if opts.binary_path and vim.fn.executable(opts.binary_path) == 1 then
    return opts.binary_path
  end

  local ver = opts.version or "0.1.0"
  local data_bin = vim.fn.stdpath("data") .. "/lineage/bin/lineage-engine-v" .. ver
  if vim.fn.executable(data_bin) == 1 then
    return data_bin
  end

  -- Local dev binary inside plugin repo
  local source_dir = debug.getinfo(1, "S").source:sub(2)
  local plugin_root = vim.fn.fnamemodify(source_dir, ":h:h:h")
  local release_bin = plugin_root .. "/engine/target/release/lineage-engine"
  if vim.fn.executable(release_bin) == 1 then
    return release_bin
  end
  local debug_bin = plugin_root .. "/engine/target/debug/lineage-engine"
  if vim.fn.executable(debug_bin) == 1 then
    return debug_bin
  end

  local path_bin = vim.fn.exepath("lineage-engine")
  if path_bin ~= "" then
    return path_bin
  end

  return nil
end

function M.is_running()
  return state.handle ~= nil and not state.handle:is_closing()
end

function M.stop()
  if state.handle and not state.handle:is_closing() then
    pcall(function()
      state.handle:kill("sigterm")
    end)
  end
  state.handle = nil
  state.stdin = nil
  state.stdout = nil
  state.stderr = nil
  state.initialized = false
end

local function reject_all_pending(reason)
  local pending = state.pending
  state.pending = {}
  for _, cb in pairs(pending) do
    pcall(cb, { code = -32000, message = reason }, nil)
  end
end

local function send_raw(tbl)
  if not state.stdin or state.stdin:is_closing() then
    return false
  end
  local line = vim.json.encode(tbl) .. "\n"
  state.stdin:write(line)
  return true
end

function M.notify(method, params)
  if not M.is_running() then
    return false
  end
  return send_raw({
    jsonrpc = "2.0",
    method = method,
    params = params or vim.empty_dict(),
  })
end

function M.on_delta(query_id, cb)
  state.delta_listeners[query_id] = cb
end

function M.clear_delta(query_id)
  state.delta_listeners[query_id] = nil
end

function M.cancel_query(query_id)
  M.clear_delta(query_id)
  lsp_compat.cancel_query(query_id)
  if M.is_running() then
    M.notify("$/cancelRequest", { query_id = query_id })
  end
end

local function handle_message(msg)
  -- Response to our c-<n> request
  if msg.id and not msg.method then
    local cb = state.pending[msg.id]
    if cb then
      state.pending[msg.id] = nil
      cb(msg.error, msg.result)
    end
    return
  end

  -- Reverse-RPC request from engine (e-<n>)
  if msg.id and msg.method == "lineage/lspResolveBatch" then
    lsp_compat.resolve_batch(msg.params or {}, nil, function(res)
      send_raw({
        jsonrpc = "2.0",
        id = msg.id,
        result = res,
      })
    end)
    return
  end

  -- Server notification
  if msg.method == "lineage/sliceDelta" and msg.params then
    local qid = msg.params.query_id
    local listener = state.delta_listeners[qid]
    if listener then
      listener(msg.params.envelope)
    end
  end
end

local function on_stdout_chunk(err, data)
  if err or not data then
    return
  end
  state.buf = state.buf .. data
  while true do
    local nl = state.buf:find("\n", 1, true)
    if not nl then
      break
    end
    local line = state.buf:sub(1, nl - 1)
    state.buf = state.buf:sub(nl + 1)
    if line ~= "" then
      local ok, decoded = pcall(vim.json.decode, line)
      if ok and type(decoded) == "table" then
        vim.schedule(function()
          handle_message(decoded)
        end)
      end
    end
  end
end

function M.ensure_started(cb)
  if state.protocol_mismatch then
    cb("lineage-engine protocol_version mismatch; upgrade binary", nil)
    return
  end
  if M.is_running() and state.initialized then
    cb(nil, true)
    return
  end
  if M.is_running() and not state.initialized then
    -- Wait for initialize
    table.insert(state.pending, function(err)
      cb(err, err == nil)
    end)
    return
  end

  if state.restarts >= MAX_RESTARTS then
    cb("lineage-engine exceeded max restarts (" .. MAX_RESTARTS .. ")", nil)
    return
  end

  local bin = M.find_binary()
  if not bin then
    cb("lineage-engine binary not found", nil)
    return
  end
  state.binary_path = bin

  local stdin = uv.new_pipe(false)
  local stdout = uv.new_pipe(false)
  local stderr = uv.new_pipe(false)

  local handle
  handle = uv.spawn(bin, {
    args = { "serve" },
    stdio = { stdin, stdout, stderr },
  }, function(code, signal)
    vim.schedule(function()
      state.handle = nil
      state.initialized = false
      state.restarts = state.restarts + 1
      sync.reset()
      reject_all_pending(
        "lineage-engine exited (code=" .. tostring(code) .. ", signal=" .. tostring(signal) .. ")"
      )
    end)
  end)

  if not handle then
    cb("Failed to spawn lineage-engine: " .. bin, nil)
    return
  end

  state.handle = handle
  state.stdin = stdin
  state.stdout = stdout
  state.stderr = stderr
  state.buf = ""

  stdout:read_start(on_stdout_chunk)
  stderr:read_start(function(err, data)
    if not err and data then
      vim.schedule(function()
        push_stderr(data)
      end)
    end
  end)

  M.request("initialize", {
    client_name = "lineage.nvim",
    protocol_version = PROTOCOL_VERSION,
  }, function(err, res)
    if err then
      M.stop()
      cb(err.message or "initialize failed", nil)
      return
    end
    if not res or res.protocol_version ~= PROTOCOL_VERSION then
      state.protocol_mismatch = true
      local got = res and tostring(res.protocol_version) or "nil"
      M.stop()
      local msg = string.format(
        "lineage-engine protocol_version mismatch: expected %d, got %s. Please rebuild/upgrade lineage-engine.",
        PROTOCOL_VERSION,
        got
      )
      vim.notify(msg, vim.log.levels.ERROR)
      cb(msg, nil)
      return
    end
    state.initialized = true
    state.restarts = 0
    cb(nil, true)
  end)
end

function M.request(method, params, cb)
  if method ~= "initialize" and not M.is_running() then
    M.ensure_started(function(err)
      if err then
        cb({ code = -32000, message = err }, nil)
        return
      end
      M.request(method, params, cb)
    end)
    return
  end

  local id = "c-" .. tostring(state.next_id)
  state.next_id = state.next_id + 1
  state.pending[id] = cb

  local ok = send_raw({
    jsonrpc = "2.0",
    id = id,
    method = method,
    params = params or vim.empty_dict(),
  })
  if not ok then
    state.pending[id] = nil
    cb({ code = -32000, message = "Failed to write to lineage-engine stdin" }, nil)
  end
  return id
end

function M.get_binary_path()
  return state.binary_path or M.find_binary()
end

return M

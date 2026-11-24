local config = require("lineage.config")
local sync = require("lineage.sync")
local lsp_compat = require("lineage.lsp_compat")

local M = {}

local uv = vim.uv or vim.loop
local PROTOCOL_VERSION = 1
local MAX_RESTARTS = 3
local STDERR_RING_MAX = 50
local TELEMETRY_RING_MAX = 100

local state = {
  handle = nil,
  stdin = nil,
  stdout = nil,
  stderr = nil,
  buf = "",
  next_id = 1,
  next_query_id = 0,
  pending = {},
  init_waiters = {},
  delta_listeners = {},
  inflight_telemetry = {},
  telemetry_ring = {},
  parse_errors = 0,
  restarts = 0,
  stderr_ring = {},
  initialized = false,
  protocol_mismatch = false,
  binary_path = nil,
}

local function push_telemetry(entry)
  table.insert(state.telemetry_ring, entry)
  if #state.telemetry_ring > TELEMETRY_RING_MAX then
    table.remove(state.telemetry_ring, 1)
  end
end

function M.next_query_id()
  state.next_query_id = state.next_query_id + 1
  return state.next_query_id
end

function M.get_telemetry()
  return vim.deepcopy(state.telemetry_ring)
end

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
    return opts.binary_path, "custom binary_path"
  end

  local install = require("lineage.install")
  local pinned = install.resolve_pinned_binary(opts)
  if pinned and pinned.path and vim.fn.executable(pinned.path) == 1 then
    local label = pinned.is_source_build
        and ("installed (" .. pinned.engine_tag .. ", source_build)")
      or ("installed (" .. pinned.engine_tag .. ")")
    return pinned.path, label
  end

  local ver = opts.version or (pinned and pinned.version) or "0.1.0"
  local ext = (uv.os_uname().sysname == "Windows_NT") and ".exe" or ""
  local data_bin = vim.fn.stdpath("data") .. "/lineage/bin/lineage-engine-v" .. ver .. ext
  if vim.fn.executable(data_bin) == 1 then
    return data_bin, "installed (v" .. ver .. ")"
  end

  -- Local dev binary inside plugin repo
  local source_dir = debug.getinfo(1, "S").source:sub(2)
  local plugin_root = vim.fn.fnamemodify(source_dir, ":h:h:h")
  local release_bin = plugin_root .. "/engine/target/release/lineage-engine" .. ext
  if vim.fn.executable(release_bin) == 1 then
    return release_bin, "local dev (target/release)"
  end
  local debug_bin = plugin_root .. "/engine/target/debug/lineage-engine" .. ext
  if vim.fn.executable(debug_bin) == 1 then
    return debug_bin, "local dev (target/debug)"
  end

  local path_bin = vim.fn.exepath("lineage-engine")
  if path_bin ~= "" then
    return path_bin, "PATH"
  end

  return nil, nil
end

function M.is_running()
  return state.handle ~= nil and not state.handle:is_closing()
end

local function drain_init_waiters(err, ok)
  local waiters = state.init_waiters
  state.init_waiters = {}
  for _, cb in ipairs(waiters) do
    pcall(cb, err, ok)
  end
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
  drain_init_waiters("lineage-engine stopped", nil)
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
    local p = msg.params or {}
    local open_set = {}
    if type(p.circuit_open_clients) == "table" then
      for _, name in ipairs(p.circuit_open_clients) do
        open_set[name] = true
      end
    end
    lsp_compat.resolve_batch(p, open_set, function(res)
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
    local env = msg.params.envelope
    if qid and env and env.stats then
      for i = #state.telemetry_ring, 1, -1 do
        local entry = state.telemetry_ring[i]
        if entry.query_id == qid then
          entry.delta_received = true
          entry.phase2_ms = env.stats.phase2_ms
          break
        end
      end
    end
    local listener = state.delta_listeners[qid]
    if listener then
      listener(env)
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
      else
        state.parse_errors = state.parse_errors + 1
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
    table.insert(state.init_waiters, cb)
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
      local exit_msg = "lineage-engine exited (code="
        .. tostring(code)
        .. ", signal="
        .. tostring(signal)
        .. ")"
      reject_all_pending(exit_msg)
      drain_init_waiters(exit_msg, nil)
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
  table.insert(state.init_waiters, cb)

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
      local msg = err.message or "initialize failed"
      M.stop()
      drain_init_waiters(msg, nil)
      return
    end
    if not res or res.protocol_version ~= PROTOCOL_VERSION then
      state.protocol_mismatch = true
      local got = res and tostring(res.protocol_version) or "nil"
      local msg = string.format(
        "lineage-engine protocol_version mismatch: expected %d, got %s. Please rebuild/upgrade lineage-engine.",
        PROTOCOL_VERSION,
        got
      )
      M.stop()
      vim.notify(msg, vim.log.levels.ERROR)
      drain_init_waiters(msg, nil)
      return
    end
    state.initialized = true
    state.restarts = 0
    drain_init_waiters(nil, true)
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
  local t0 = uv.hrtime()
  local qid = type(params) == "table" and params.query_id or nil

  state.pending[id] = function(err, result)
    if method == "lineage/slice" then
      local rtt_ms = math.floor((uv.hrtime() - t0) / 1e6)
      local stats = (type(result) == "table" and result.stats) or {}
      push_telemetry({
        query_id = qid,
        trace_id = type(result) == "table" and result.trace_id or nil,
        method = method,
        phase1_ms = stats.phase1_ms,
        phase2_ms = stats.phase2_ms,
        rtt_ms = rtt_ms,
        delta_received = false,
        parse_errors = state.parse_errors,
        restarts = state.restarts,
      })
    end
    cb(err, result)
  end

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

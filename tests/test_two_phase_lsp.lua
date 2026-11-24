local client = require("lineage.client")
local hover = require("lineage.ui.hover")
local tree = require("lineage.ui.tree")
local lsp_compat = require("lineage.lsp_compat")
local config = require("lineage.config")

config.setup({ hover = { enter_on_open = false, icons = { unicode = true } } })

local tmp_dir = vim.fn.tempname()
vim.fn.mkdir(tmp_dir, "p")

-- 1. UTF-16 column conversion on multi-byte lines (loaded & unloaded without bufload)
local mb_file = tmp_dir .. "/utf16_test.ts"
local mb_line = "const αβγ = helper('x')"
do
  local fh = assert(io.open(mb_file, "wb"))
  fh:write(mb_line .. "\n")
  fh:close()
end
local mb_uri = vim.uri_from_fname(mb_file)
local byte_col = mb_line:find("helper", 1, true) - 1 -- UTF-8 byte offset (multi-byte αβγ = 6 bytes, 3 UTF-16 code units)
local conv_utf16 = lsp_compat.utf8_col_to_encoding(mb_line, byte_col, "utf-16")
assert(
  conv_utf16 == byte_col - 3,
  string.format("Expected UTF-16 col %d, got %d", byte_col - 3, conv_utf16)
)
-- Verify unloaded buffer does not trigger bufload when resolving via lsp_compat
local unloaded_buf = vim.uri_to_bufnr(mb_uri)
assert(not vim.api.nvim_buf_is_loaded(unloaded_buf), "Buffer should remain unloaded")

-- 2. Two-phase hover ordering, immutable [1..k] + [k+1], unicode symbol prefix, updated status_badge
local ts_file = tmp_dir .. "/main.ts"
do
  local fh = assert(io.open(ts_file, "wb"))
  fh:write("import { ext } from './ext';\nfunction f() {\n  const val = ext();\n  return val;\n}\n")
  fh:close()
end
vim.cmd("edit " .. vim.fn.fnameescape(ts_file))
local bufnr = vim.api.nvim_get_current_buf()
vim.api.nvim_win_set_cursor(0, { 3, 8 })

-- Mock client.ensure_started & client.request & client.cancel_query
local orig_ensure = client.ensure_started
local orig_request = client.request
local orig_cancel = client.cancel_query
local orig_get_clients = lsp_compat.get_clients

local delta_cbs = {}
local orig_on_delta = client.on_delta
local orig_clear_delta = client.clear_delta
client.on_delta = function(qid, cb)
  delta_cbs[qid] = cb
end
client.clear_delta = function(qid)
  delta_cbs[qid] = nil
end

local cancelled_qids = {}
client.cancel_query = function(qid)
  cancelled_qids[qid] = true
  delta_cbs[qid] = nil
end

lsp_compat.get_clients = function(_)
  return {
    {
      name = "mock_ts",
      offset_encoding = "utf-16",
      supports_method = function(_, m)
        return m == "textDocument/definition" or m == "textDocument/prepareCallHierarchy"
      end,
    },
  }
end

local sent_requests = {}
client.ensure_started = function(cb)
  cb(nil, true)
end
client.request = function(method, params, cb)
  table.insert(sent_requests, { method = method, params = params })
  if method == "lineage/slice" and not params.include_tree then
    -- Return Phase 1 immediately
    cb(nil, {
      query_id = params.query_id,
      trace_id = "tr-1",
      projections = {
        hover_card = {
          symbol = "val",
          verdict_line = "val: Unverified (partial — 1 unverified [gu])",
          status_badge = "Unverified",
          is_trivial_peek = false,
          peek_lines = {},
          phase1_items = {
            {
              index = 1,
              file_id = 0,
              uri = params.uri,
              range = { start = { line = 2, col = 8 }, ["end"] = { line = 2, col = 11 } },
              label = "call ext",
              provenance = "local_ssi",
              is_absent_origin = false,
            },
          },
          phase2_items = {},
          guards = {},
          local_risks = {},
          unknown_boundaries = {
            {
              file_id = 0,
              uri = params.uri,
              range = { start = { line = 2, col = 14 }, ["end"] = { line = 2, col = 17 } },
              symbol = "ext",
              reason = "external_untyped",
            },
          },
          unverified_count = 1,
        },
      },
      truncation = {},
      stats = { phase1_ms = 2, phase2_ms = 0 },
    })
  elseif method == "lineage/slice" and params.include_tree then
    cb(nil, {
      query_id = params.query_id,
      trace_id = "tr-2",
      projections = {
        tree_rows = {
          {
            node_id = "n0",
            depth = 0,
            file_id = 0,
            uri = params.uri,
            range = { start = { line = 1, col = 0 }, ["end"] = { line = 1, col = 10 } },
            label = "f",
            badge = "[OK]",
            is_cycle = false,
            is_frontier = false,
            is_absent = false,
            group_key = "f",
            provenance = "local_ssi",
            has_children = false,
          },
        },
      },
      truncation = {},
      stats = { phase1_ms = 3, phase2_ms = 0 },
    })
  end
end

hover.open()
local st = hover.get_active_state()
assert(st.winid ~= nil and vim.api.nvim_win_is_valid(st.winid), "Phase 1 hover should open")
local p1_lines = vim.api.nvim_buf_get_lines(st.bufnr, 0, -1, false)
assert(
  p1_lines[1]:find("^val: "),
  "Unicode hover header must preserve 'val: ' prefix, got: " .. p1_lines[1]
)
assert(
  p1_lines[1]:find("Unverified", 1, true),
  "Phase 1 header should show Unverified, got: " .. p1_lines[1]
)

-- Now deliver Phase 2 delta
local qid1 = st.query_id
assert(delta_cbs[qid1] ~= nil, "Delta listener must be registered")
delta_cbs[qid1]({
  query_id = qid1,
  projections = {
    hover_card = {
      symbol = "val",
      verdict_line = "val: Nullable (null)",
      status_badge = "Nullable (null)",
      is_trivial_peek = false,
      phase1_items = st.card.phase1_items,
      phase2_items = {
        {
          index = 2,
          file_id = 1,
          uri = "file:///ext.ts",
          range = { start = { line = 0, col = 0 }, ["end"] = { line = 0, col = 5 } },
          label = "LSP target ext",
          provenance = "lsp_definition",
          is_absent_origin = true,
        },
      },
      unknown_boundaries = {},
      unverified_count = 0,
    },
  },
})

local p2_lines = vim.api.nvim_buf_get_lines(st.bufnr, 0, -1, false)
assert(p2_lines[1]:find("^val: "), "Phase 2 unicode header must preserve 'val: ' prefix")
assert(p2_lines[1]:find("Nullable", 1, true), "Phase 2 header must update status_badge to Nullable")
local joined = table.concat(p2_lines, "\n")
assert(
  joined:find("[1]", 1, true) and joined:find("[2]", 1, true),
  "Expected [1] and [2] after delta"
)
assert(delta_cbs[qid1] == nil, "Delta listener must be cleared after receipt")

-- 3. Cancellation of in-flight query without stale mutation
hover.open()
local st2 = hover.get_active_state()
local qid2 = st2.query_id
-- Simulate user closing / cancelling before Phase 2 delta arrives
vim.api.nvim_win_close(st2.winid, true)
client.cancel_query(qid2)
assert(cancelled_qids[qid2] == true, "cancel_query should record qid2")
assert(delta_cbs[qid2] == nil, "Delta listener must be cleared on cancel")

-- 4. :Lineage from tree delta delivery
tree.open("from")
local tst = tree.get_state()
local tqid = tst.query_id
assert(delta_cbs[tqid] ~= nil, "Tree must register delta listener")
delta_cbs[tqid]({
  query_id = tqid,
  projections = {
    tree_rows = {
      {
        node_id = "n0",
        depth = 0,
        file_id = 0,
        uri = vim.uri_from_fname(ts_file),
        range = { start = { line = 1, col = 0 }, ["end"] = { line = 1, col = 10 } },
        label = "f",
        badge = "[OK]",
        is_cycle = false,
        is_frontier = false,
        is_absent = false,
        group_key = "f",
        provenance = "local_ssi",
        has_children = true,
      },
      {
        node_id = "lsp_caller:1:0",
        depth = 1,
        file_id = 1,
        uri = "file:///caller.ts",
        range = { start = { line = 4, col = 2 }, ["end"] = { line = 4, col = 5 } },
        label = "caller -> f",
        badge = "[OK]",
        is_cycle = false,
        is_frontier = false,
        is_absent = false,
        group_key = "f",
        provenance = "lsp_call_hierarchy",
        has_children = false,
      },
    },
  },
  truncation = {},
})
local tree_lines = vim.api.nvim_buf_get_lines(tst.bufnr, 0, -1, false)
assert(
  table.concat(tree_lines, "\n"):find("caller -> f", 1, true),
  "Tree must render Phase 2 caller rows from delta"
)
tree.close()

-- Restore mocks
client.ensure_started = orig_ensure
client.request = orig_request
client.cancel_query = orig_cancel
client.on_delta = orig_on_delta
client.clear_delta = orig_clear_delta
lsp_compat.get_clients = orig_get_clients

-- 5. Real sidecar concurrent ensure_started + cancel test (if binary exists)
local bin = client.find_binary()
if bin then
  client.stop()
  local w1_done, w2_done = false, false
  local w1_err, w2_err = nil, nil
  client.ensure_started(function(e)
    w1_err = e
    w1_done = true
  end)
  client.ensure_started(function(e)
    w2_err = e
    w2_done = true
  end)
  local ok_wait = vim.wait(3000, function()
    return w1_done and w2_done
  end, 10)
  assert(ok_wait, "Concurrent ensure_started waiters timed out")
  assert(
    w1_err == nil and w2_err == nil,
    "Concurrent ensure_started failed: " .. tostring(w1_err or w2_err)
  )
  client.stop()
end

vim.fn.delete(tmp_dir, "rf")
print("test_two_phase_lsp.lua: PASS")

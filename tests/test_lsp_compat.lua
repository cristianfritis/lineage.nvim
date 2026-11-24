local lsp_compat = require("lineage.lsp_compat")

-- Verify 0.10 vs 0.11+ method dispatch and empty batch handling
local batch_done = false
local batch_res = nil
lsp_compat.resolve_batch({ query_id = 1, items = {} }, nil, function(res)
  batch_res = res
  batch_done = true
end)
assert(batch_done and #batch_res.results == 0, "Empty batch should resolve synchronously")

-- Verify unattached buffer returns unattached=true without calling hover or didOpen
local buf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_name(buf, "/tmp/lineage_unattached.ts")
local uri = vim.uri_from_bufnr(buf)

local u_done = false
local u_res = nil
lsp_compat.resolve_batch(
  {
    query_id = 2,
    timeout_ms = 50,
    items = {
      {
        key = "foo",
        uri = uri,
        position = { line = 0, character = 0 },
        kind = "definition",
      },
    },
  },
  nil,
  function(res)
    u_res = res
    u_done = true
  end
)
assert(
  vim.wait(1000, function()
    return u_done
  end, 10),
  "Unattached resolve_batch timed out"
)
assert(u_res.results[1].unattached == true, "Expected unattached=true")

-- Verify callHierarchyIncoming falls back to textDocument/references when prepareCallHierarchy returns null/empty
local orig_get_clients = lsp_compat.get_clients
local orig_supports = lsp_compat.client_supports
local orig_request = lsp_compat.client_request

local methods_called = {}
local fake_client = { name = "vtsls", offset_encoding = "utf-16" }
lsp_compat.get_clients = function()
  return { fake_client }
end
lsp_compat.client_supports = function(_, method)
  return method == "textDocument/prepareCallHierarchy" or method == "textDocument/references"
end
lsp_compat.client_request = function(_, method, _, handler)
  table.insert(methods_called, method)
  if method == "textDocument/prepareCallHierarchy" then
    vim.schedule(function()
      handler(nil, vim.NIL) -- vtsls unopened-file null behavior
    end)
    return true, 1
  elseif method == "textDocument/references" then
    vim.schedule(function()
      handler(nil, {
        {
          uri = "file:///tmp/caller.ts",
          range = { start = { line = 4, character = 2 }, ["end"] = { line = 4, character = 12 } },
        },
      })
    end)
    return true, 2
  end
  return false, nil
end

local fb_done = false
local fb_res = nil
lsp_compat.resolve_batch(
  {
    query_id = 3,
    timeout_ms = 200,
    items = {
      {
        key = "findSession",
        uri = uri,
        position = { line = 0, character = 16 },
        kind = "callHierarchyIncoming",
      },
    },
  },
  nil,
  function(res)
    fb_res = res
    fb_done = true
  end
)

assert(
  vim.wait(1000, function()
    return fb_done
  end, 10),
  "callHierarchyIncoming fallback timed out"
)
assert(
  methods_called[1] == "textDocument/prepareCallHierarchy",
  "Expected prepareCallHierarchy first"
)
assert(
  methods_called[2] == "textDocument/references",
  "Expected fallback to textDocument/references"
)
assert(#fb_res.results[1].locations == 1, "Expected 1 fallback location")
assert(fb_res.results[1].locations[1].uri == "file:///tmp/caller.ts")

lsp_compat.get_clients = orig_get_clients
lsp_compat.client_supports = orig_supports
lsp_compat.client_request = orig_request

vim.api.nvim_buf_delete(buf, { force = true })
print("test_lsp_compat.lua: PASS")

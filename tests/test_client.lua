local client = require("lineage.client")
local sync = require("lineage.sync")

-- 1. Binary discovery finds local release or debug binary
local bin = client.find_binary()
assert(bin ~= nil and bin ~= "", "Expected lineage-engine binary to be discovered")

-- 2. Initialize + lineage/status roundtrip
local done = false
local status_res = nil
local status_err = nil
client.request("lineage/status", {}, function(err, res)
  status_err = err
  status_res = res
  done = true
end)
assert(
  vim.wait(3000, function()
    return done
  end, 10),
  "Timed out waiting for lineage/status"
)
assert(status_err == nil, "Unexpected status error: " .. vim.inspect(status_err))
assert(status_res.protocol_version == 1, "Expected protocol_version == 1")
assert(status_res.schema_version == 1, "Expected schema_version == 1")

-- 3. Sync > 1 MB buffer is skipped on Lua side
local buf = vim.api.nvim_create_buf(false, true)
vim.bo[buf].buftype = ""
vim.api.nvim_buf_set_name(buf, "/tmp/lineage_large_test.ts")
local big_line = string.rep("a", 2048)
local lines = {}
for i = 1, 600 do
  lines[i] = big_line
end
vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
vim.bo[buf].modified = true
local ok, reason = sync.sync_if_dirty(buf, client)
assert(ok == false and reason == "file_too_large", "Expected >1MB buffer to be skipped on Lua side")
vim.api.nvim_buf_delete(buf, { force = true })

client.stop()
print("test_client.lua: PASS")

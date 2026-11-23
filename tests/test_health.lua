local health = require("lineage.health")
local client = require("lineage.client")

-- Capture vim.health calls
local reports = {}
vim.health = {
  start = function(msg)
    table.insert(reports, { "start", msg })
  end,
  ok = function(msg)
    table.insert(reports, { "ok", msg })
  end,
  info = function(msg)
    table.insert(reports, { "info", msg })
  end,
  warn = function(msg)
    table.insert(reports, { "warn", msg })
  end,
  error = function(msg)
    table.insert(reports, { "error", msg })
  end,
}

health.check()
assert(#reports >= 3, "Expected health check to report Neovim version, binary, and sidecar status")
assert(reports[1][1] == "start" and reports[1][2] == "lineage.nvim")

client.stop()
print("test_health.lua: PASS")

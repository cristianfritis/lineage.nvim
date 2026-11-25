local config = require("lineage.config")
local highlights = require("lineage.ui.highlights")

local M = {}
local initialized = false

function M.setup(opts)
  config.setup(opts)
  highlights.setup()
  if not initialized then
    initialized = true
    local group = vim.api.nvim_create_augroup("LineageNvim", { clear = true })
    vim.api.nvim_create_autocmd({ "BufDelete", "BufUnload" }, {
      group = group,
      callback = function(ev)
        require("lineage.sync").on_buf_close(ev.buf)
      end,
    })
  end
end

function M.hover()
  if not initialized then
    M.setup()
  end
  require("lineage.ui.hover").open()
end

function M.back()
  require("lineage.ui.jump_stack").pop()
end

function M.focus()
  if not initialized then
    M.setup()
  end
  require("lineage.ui.focus").focus()
end

function M.pin()
  if not initialized then
    M.setup()
  end
  require("lineage.ui.focus").pin()
end

function M.clear()
  require("lineage.ui.focus").clear()
end

function M.to()
  if not initialized then
    M.setup()
  end
  require("lineage.ui.tree").open("to")
end

function M.from()
  if not initialized then
    M.setup()
  end
  require("lineage.ui.tree").open("from")
end

function M.both()
  if not initialized then
    M.setup()
  end
  require("lineage.ui.tree").open("both")
end

function M.status()
  if not initialized then
    M.setup()
  end
  require("lineage.debug").show_status()
end

function M.log()
  if not initialized then
    M.setup()
  end
  require("lineage.debug").show_log()
end

function M.debug_dump()
  if not initialized then
    M.setup()
  end
  require("lineage.debug").debug_dump()
end

function M.install(opts)
  if not initialized then
    M.setup()
  end
  return require("lineage.install").install(opts)
end

function M.export(opts)
  if not initialized then
    M.setup()
  end
  return require("lineage.export").export(opts)
end

function M.export_clean(opts)
  if not initialized then
    M.setup()
  end
  return require("lineage.export").clean(opts)
end

return M

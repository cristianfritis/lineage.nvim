if vim.g.loaded_lineage then
  return
end
vim.g.loaded_lineage = 1

vim.api.nvim_create_user_command("Lineage", function(opts)
  local sub = opts.fargs[1] or "hover"
  local lineage = require("lineage")
  if sub == "hover" then
    lineage.hover()
  elseif sub == "focus" then
    lineage.focus()
  elseif sub == "pin" then
    lineage.pin()
  elseif sub == "clear" then
    lineage.clear()
  elseif sub == "to" then
    lineage.to()
  elseif sub == "from" then
    lineage.from()
  elseif sub == "both" then
    lineage.both()
  elseif sub == "back" then
    lineage.back()
  elseif sub == "install!" or (sub == "install" and opts.bang) then
    lineage.install({ force = true })
  elseif sub == "install" then
    lineage.install()
  elseif sub == "status" then
    lineage.status()
  elseif sub == "log" then
    lineage.log()
  elseif sub == "debug-dump" then
    lineage.debug_dump()
  else
    vim.notify("lineage.nvim: unknown subcommand '" .. sub .. "'", vim.log.levels.WARN)
  end
end, {
  nargs = "?",
  bang = true,
  complete = function(arg_lead)
    local cmds = {
      "hover",
      "focus",
      "pin",
      "clear",
      "to",
      "from",
      "both",
      "back",
      "install",
      "status",
      "log",
      "debug-dump",
    }
    return vim.tbl_filter(function(c)
      return vim.startswith(c, arg_lead)
    end, cmds)
  end,
  desc = "lineage.nvim commands",
})

vim.keymap.set("n", "<Plug>(LineageHover)", function()
  require("lineage").hover()
end, { silent = true, desc = "Lineage: Open dataflow hover" })

vim.keymap.set("n", "<Plug>(LineageFocus)", function()
  require("lineage").focus()
end, { silent = true, desc = "Lineage: Toggle inline focus mode" })

vim.keymap.set("n", "<Plug>(LineagePin)", function()
  require("lineage").pin()
end, { silent = true, desc = "Lineage: Pin symbol across buffers" })

vim.keymap.set("n", "<Plug>(LineageTo)", function()
  require("lineage").to()
end, { silent = true, desc = "Lineage: Open Data Flow To Here tree" })

vim.keymap.set("n", "<Plug>(LineageFrom)", function()
  require("lineage").from()
end, { silent = true, desc = "Lineage: Open Data Flow From Here tree" })

vim.keymap.set("n", "<Plug>(LineageBack)", function()
  require("lineage").back()
end, { silent = true, desc = "Lineage: Jump back in Lineage stack" })

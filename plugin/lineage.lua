if vim.g.loaded_lineage then
  return
end
vim.g.loaded_lineage = 1

vim.api.nvim_create_user_command("Lineage", function(opts)
  local sub = opts.fargs[1] or "hover"
  local lineage = require("lineage")
  if sub == "hover" then
    lineage.hover()
  elseif sub == "back" then
    lineage.back()
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
  complete = function(arg_lead)
    local cmds = { "hover", "back", "status", "log", "debug-dump" }
    return vim.tbl_filter(function(c)
      return vim.startswith(c, arg_lead)
    end, cmds)
  end,
  desc = "lineage.nvim commands",
})

vim.keymap.set("n", "<Plug>(LineageHover)", function()
  require("lineage").hover()
end, { silent = true, desc = "Lineage: Open dataflow hover" })

vim.keymap.set("n", "<Plug>(LineageBack)", function()
  require("lineage").back()
end, { silent = true, desc = "Lineage: Jump back in Lineage stack" })

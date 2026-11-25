if vim.g.loaded_lineage then
  return
end
vim.g.loaded_lineage = 1

vim.api.nvim_create_user_command("Lineage", function(opts)
  require("lineage.commands").dispatch(opts)
end, {
  nargs = "*",
  bang = true,
  complete = function(arg_lead, cmd_line, cursor_pos)
    return require("lineage.commands").complete(arg_lead, cmd_line, cursor_pos)
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

vim.keymap.set("n", "<Plug>(LineageExport)", function()
  require("lineage.export").pick_and_export()
end, { silent = true, desc = "Lineage: Export active slice graph" })

vim.keymap.set("n", "<Plug>(LineageExportMermaid)", function()
  require("lineage").export({ format = "mermaid" })
end, { silent = true, desc = "Lineage: Export active slice graph as Mermaid" })

vim.keymap.set("n", "<Plug>(LineageExportDot)", function()
  require("lineage").export({ format = "dot" })
end, { silent = true, desc = "Lineage: Export active slice graph as Graphviz DOT" })

vim.keymap.set("n", "<Plug>(LineageExportHtml)", function()
  require("lineage").export({ format = "html" })
end, { silent = true, desc = "Lineage: Export active slice graph as offline HTML" })

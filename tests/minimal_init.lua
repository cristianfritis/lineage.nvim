local root = vim.fn.getcwd()
vim.opt.runtimepath:prepend(root)
vim.cmd("runtime plugin/lineage.lua")
require("lineage").setup()

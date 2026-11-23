local M = {}

function M.setup()
  vim.api.nvim_set_hl(0, "LineageNonNull", { link = "DiagnosticOk", default = true })
  vim.api.nvim_set_hl(0, "LineageNullable", { link = "DiagnosticWarn", default = true })
  vim.api.nvim_set_hl(0, "LineageAbsent", { link = "DiagnosticError", default = true })
  vim.api.nvim_set_hl(0, "LineageUnverified", { link = "DiagnosticInfo", default = true })
  vim.api.nvim_set_hl(0, "LineageIndex", { link = "Number", default = true })
  vim.api.nvim_set_hl(0, "LineageRisk", { link = "DiagnosticError", default = true })
  vim.api.nvim_set_hl(0, "LineageHeader", { link = "Title", default = true })
  vim.api.nvim_set_hl(0, "LineageDim", { link = "Comment", default = true })
  vim.api.nvim_set_hl(0, "LineageFocusDim", { link = "Comment", default = true })
  vim.api.nvim_set_hl(0, "LineageStateAbsent", { link = "DiagnosticError", default = true })
  vim.api.nvim_set_hl(0, "LineageStatePresent", { link = "DiagnosticOk", default = true })
  vim.api.nvim_set_hl(0, "LineageStateUnknown", { link = "DiagnosticInfo", default = true })
  vim.api.nvim_set_hl(0, "LineageFocusIndicator", { link = "DiagnosticInfo", default = true })
  vim.api.nvim_set_hl(0, "LineageTreeCycle", { link = "DiagnosticWarn", default = true })
  vim.api.nvim_set_hl(0, "LineageTreeFrontier", { link = "Special", default = true })
end

function M.badge(status_badge, use_unicode)
  if use_unicode then
    if vim.startswith(status_badge, "NonNull") then
      return "● " .. status_badge
    elseif vim.startswith(status_badge, "Nullable") then
      return "◐ " .. status_badge
    elseif vim.startswith(status_badge, "Absent") then
      return "○ " .. status_badge
    else
      return "? " .. status_badge
    end
  end
  return status_badge
end

return M

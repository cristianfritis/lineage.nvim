local M = {}

local VALID_DIRECTIONS = { to = true, from = true, both = true }
local VALID_FORMATS = { mermaid = true, dot = true, html = true }

local SUBCOMMANDS = {
  "hover",
  "focus",
  "pin",
  "clear",
  "to",
  "from",
  "both",
  "back",
  "export",
  "install",
  "status",
  "log",
  "debug-dump",
}

local EXPORT_FLAGS = {
  "--direction",
  "--exclude-tests",
  "--null-only",
  "--no-lsp",
  "--snippets",
  "--no-snippets",
  "--open",
  "--no-open",
  "--out",
}

function M.parse_export_args(args, bang)
  if args[1] == "clean" then
    if #args > 1 or bang == true then
      return nil, "clean does not accept export flags or extra arguments"
    end
    return { action = "clean" }, nil
  end

  local parsed = {
    format = nil,
    path = nil,
    direction = nil,
    exclude_tests = nil,
    null_only = nil,
    no_lsp = false,
    include_snippets = nil,
    open = nil,
    force = bang == true,
  }

  local positional_path = nil
  local out_path = nil
  local i = 1
  while i <= #args do
    local tok = args[i]
    if tok == "clean" then
      return nil, "clean must be used as ':Lineage export clean' without export flags"
    elseif tok == "--exclude-tests" then
      parsed.exclude_tests = true
    elseif tok == "--null-only" then
      parsed.null_only = true
    elseif tok == "--no-lsp" then
      parsed.no_lsp = true
    elseif tok == "--snippets" then
      if parsed.include_snippets == false then
        return nil, "Cannot combine --snippets and --no-snippets"
      end
      parsed.include_snippets = true
    elseif tok == "--no-snippets" then
      if parsed.include_snippets == true then
        return nil, "Cannot combine --snippets and --no-snippets"
      end
      parsed.include_snippets = false
    elseif tok == "--open" then
      if parsed.open == false then
        return nil, "Cannot combine --open and --no-open"
      end
      parsed.open = true
    elseif tok == "--no-open" then
      if parsed.open == true then
        return nil, "Cannot combine --open and --no-open"
      end
      parsed.open = false
    elseif tok == "--direction" or vim.startswith(tok, "--direction=") then
      local val
      if tok == "--direction" then
        i = i + 1
        val = args[i]
      else
        val = tok:sub(#"--direction=" + 1)
      end
      if not val or not VALID_DIRECTIONS[val] then
        return nil, "Invalid --direction (expected to, from, or both)"
      end
      parsed.direction = val
    elseif tok == "--out" or vim.startswith(tok, "--out=") then
      local val
      if tok == "--out" then
        i = i + 1
        val = args[i]
      else
        val = tok:sub(#"--out=" + 1)
      end
      if not val or val == "" then
        return nil, "--out requires a file path"
      end
      if positional_path then
        return nil, "Cannot specify both positional [path] and --out"
      end
      out_path = val
    elseif vim.startswith(tok, "-") then
      return nil, "Unknown export flag: " .. tok
    else
      if not parsed.format and VALID_FORMATS[tok] then
        parsed.format = tok
      elseif not parsed.format and not tok:find("[/.]") then
        return nil, "Unknown export format: " .. tok
      else
        if out_path then
          return nil, "Cannot specify both positional [path] and --out"
        end
        if positional_path then
          return nil, "Unexpected extra positional argument: " .. tok
        end
        positional_path = tok
      end
    end
    i = i + 1
  end

  parsed.path = out_path or positional_path
  return parsed, nil
end

function M.dispatch(opts)
  local fargs = opts.fargs or {}
  local sub = fargs[1] or "hover"
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
  elseif sub == "export!" or sub == "export" then
    local rest = {}
    for idx = 2, #fargs do
      table.insert(rest, fargs[idx])
    end
    local parsed, err = M.parse_export_args(rest, opts.bang or (sub == "export!"))
    if err then
      vim.notify("lineage.nvim: " .. err, vim.log.levels.ERROR)
      return
    end
    if parsed.action == "clean" then
      lineage.export_clean()
      return
    end
    local cfg = require("lineage.config").get()
    local effective_fmt = parsed.format or (cfg.export and cfg.export.default_format) or "mermaid"
    if parsed.open ~= nil and effective_fmt ~= "html" then
      vim.notify("lineage.nvim: --open/--no-open only applies to html exports", vim.log.levels.WARN)
    end
    lineage.export(parsed)
  else
    vim.notify("lineage.nvim: unknown subcommand '" .. sub .. "'", vim.log.levels.WARN)
  end
end

function M.complete(arg_lead, cmd_line, cursor_pos)
  local before = (cmd_line or ""):sub(1, cursor_pos or #(cmd_line or ""))
  local ends_space = before:match("%s$") ~= nil
  local tokens = vim.split(vim.trim(before), "%s+", { trimempty = true })

  -- tokens[1] is `:Lineage` (or `Lineage!`)
  local arg_idx = #tokens - 1
  if ends_space then
    arg_idx = arg_idx + 1
  end

  if arg_idx <= 1 then
    return vim.tbl_filter(function(c)
      return vim.startswith(c, arg_lead or "")
    end, SUBCOMMANDS)
  end

  local sub = tokens[2]
  if sub ~= "export" and sub ~= "export!" then
    return {}
  end

  if tokens[3] == "clean" then
    return {}
  end

  local prev = ends_space and tokens[#tokens] or tokens[#tokens - 1]
  if prev == "--direction" then
    return vim.tbl_filter(function(d)
      return vim.startswith(d, arg_lead or "")
    end, { "to", "from", "both" })
  end
  if prev == "--out" then
    return vim.fn.getcompletion(arg_lead or "", "file")
  end

  local lead = arg_lead or ""
  if vim.startswith(lead, "-") then
    return vim.tbl_filter(function(f)
      return vim.startswith(f, lead)
    end, EXPORT_FLAGS)
  end

  local has_format = false
  for idx = 3, #tokens do
    if VALID_FORMATS[tokens[idx]] then
      has_format = true
      break
    end
  end

  local candidates = {}
  if arg_idx == 2 and vim.startswith("clean", lead) then
    table.insert(candidates, "clean")
  end
  if not has_format then
    for _, fmt in ipairs({ "mermaid", "dot", "html" }) do
      if vim.startswith(fmt, lead) then
        table.insert(candidates, fmt)
      end
    end
  end
  for _, flag in ipairs(EXPORT_FLAGS) do
    if vim.startswith(flag, lead) then
      table.insert(candidates, flag)
    end
  end
  for _, path in ipairs(vim.fn.getcompletion(lead, "file")) do
    table.insert(candidates, path)
  end
  return candidates
end

return M

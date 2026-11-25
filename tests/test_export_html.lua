local commands = require("lineage.commands")
local client = require("lineage.client")
local debug_mod = require("lineage.debug")
local export = require("lineage.export")
local export_cache = require("lineage.export_cache")
local health = require("lineage.health")
local uv = vim.uv or vim.loop

-- 1. Command parsing, clean dispatch, --open / --no-open, completion, and <Plug> mappings
local clean_cmd, clean_err = commands.parse_export_args({ "clean" }, false)
assert(clean_err == nil and clean_cmd.action == "clean", "Expected clean action")

local _, clean_flag_err = commands.parse_export_args({ "clean", "--no-lsp" }, false)
assert(clean_flag_err ~= nil, "clean must reject export flags")

local _, clean_bang_err = commands.parse_export_args({ "clean" }, true)
assert(clean_bang_err ~= nil, "clean must reject bang")

local _, open_conflict = commands.parse_export_args({ "html", "--open", "--no-open" }, false)
assert(open_conflict ~= nil, "Conflicting --open and --no-open must fail")

local html_parsed, html_err =
  commands.parse_export_args({ "html", "--no-open", "--snippets", "out.html" }, true)
assert(
  html_err == nil
    and html_parsed.format == "html"
    and html_parsed.open == false
    and html_parsed.include_snippets == true
    and html_parsed.path == "out.html"
    and html_parsed.force == true
)

local comp_exp = commands.complete("", ":Lineage export ", 16)
assert(
  vim.tbl_contains(comp_exp, "clean") and vim.tbl_contains(comp_exp, "html"),
  "Completion must include clean and html"
)
local comp_clean = commands.complete("", ":Lineage export clean ", 22)
assert(#comp_clean == 0, "Completion after clean must be empty")

-- Verify <Plug>(LineageExportHtml) and :Lineage install! / :Lineage! install
local found_html_plug = false
for _, m in ipairs(vim.api.nvim_get_keymap("n")) do
  if m.lhs == "<Plug>(LineageExportHtml)" then
    found_html_plug = true
  end
end
assert(found_html_plug, "Expected <Plug>(LineageExportHtml) mapping")

-- 2. Private cache 0700/0600, dedup, symlink refusal, 20-file lazy prune, and clean()
local tmp_root = vim.fn.tempname()
vim.fn.mkdir(tmp_root, "p")
local cache_dir = tmp_root .. "/exports"

local c1, err1 = export_cache.write_cached(
  "<html>A</html>",
  "auth/flow",
  { cache_dir = cache_dir, max_files = 20 }
)
assert(err1 == nil and c1 ~= nil and c1.reused == false, "First cache write should succeed")
local c2, err2 = export_cache.write_cached(
  "<html>A</html>",
  "auth/flow",
  { cache_dir = cache_dir, max_files = 20 }
)
assert(
  err2 == nil and c2.path == c1.path and c2.reused == true,
  "Identical content must deduplicate"
)

if not export_cache.is_windows() then
  local dst = uv.fs_lstat(cache_dir)
  assert(dst and (dst.mode % 512) == 448, "Cache dir must be 0700")
  local fst = uv.fs_lstat(c1.path)
  assert(fst and (fst.mode % 512) == 384, "Cache file must be 0600")
end

-- Symlink refusal on cache directory
local sym_cache = tmp_root .. "/sym_exports"
local ok_sym = uv.fs_symlink(cache_dir, sym_cache)
if ok_sym then
  local _, sym_err = export_cache.write_cached("<html>B</html>", "x", { cache_dir = sym_cache })
  assert(sym_err ~= nil and sym_err:find("symlink") ~= nil, "Symlink cache dir must be refused")
end

-- 20-file lazy pruning (write 22 distinct files with max_files=20)
for idx = 1, 22 do
  local content = string.format("<html>item-%02d</html>", idx)
  local wr, werr =
    export_cache.write_cached(content, "item", { cache_dir = cache_dir, max_files = 20 })
  assert(werr == nil and wr ~= nil)
end
local stats = export_cache.stats({ cache_dir = cache_dir })
assert(
  stats.file_count == 20,
  "Cache pruning must retain 20 files, got " .. tostring(stats.file_count)
)

-- Clean removes only lineage-*.html regular files
local unrelated = cache_dir .. "/keep-me.txt"
vim.fn.writefile({ "keep" }, unrelated)
local clean_res, clean_e = export_cache.clean({ cache_dir = cache_dir })
assert(clean_e == nil and clean_res.removed_files == 20 and clean_res.freed_bytes > 0)
assert(vim.fn.filereadable(unrelated) == 1, "clean() must leave non-matching files untouched")
local missing_clean, missing_e = export_cache.clean({ cache_dir = tmp_root .. "/does_not_exist" })
assert(missing_e == nil and missing_clean.removed_files == 0)

-- 3. Headless / SSH skip vs. WSL allowance
local open_calls = {}
local ssh_res = export_cache.open_file(unrelated, {
  env = { ui_count = 1, sysname = "Linux", is_wsl = false, display = "", wayland = "" },
  open_fn = function(p)
    table.insert(open_calls, p)
  end,
})
assert(
  ssh_res.status == "skipped_headless" and #open_calls == 0,
  "SSH Linux without DISPLAY must skip vim.ui.open"
)

local wsl_res = export_cache.open_file(unrelated, {
  env = { ui_count = 1, sysname = "Linux", is_wsl = true, display = "", wayland = "" },
  open_fn = function(p)
    table.insert(open_calls, p)
    return {}
  end,
})
assert(wsl_res.status == "opened" and #open_calls == 1, "WSL must allow vim.ui.open")

-- 4. Live RPC tests: bare HTML (snippets ON) vs explicit path (snippets OFF + silent) + telemetry + capability gate
local src_file = tmp_root .. "/sample.ts"
vim.fn.writefile({
  "function helper(v: string | null) {",
  "  return v;",
  "}",
  "export function run(flag: boolean) {",
  "  const x = flag ? null : 'ok';",
  "  return helper(x);",
  "}",
}, src_file)

local buf = vim.api.nvim_create_buf(true, false)
vim.api.nvim_buf_set_name(buf, src_file)
vim.api.nvim_buf_set_lines(buf, 0, -1, false, vim.fn.readfile(src_file))
vim.bo[buf].filetype = "typescript"
vim.api.nvim_set_current_buf(buf)
vim.api.nvim_win_set_cursor(0, { 6, 9 })

local bare_cache_dir = tmp_root .. "/bare_exports"
local bare_done = false
local bare_res = nil
local opened_paths = {}
export.export({
  format = "html",
  no_lsp = true,
  workspace_root = tmp_root,
  cache_dir = bare_cache_dir,
  env = { ui_count = 1, sysname = "Linux", is_wsl = true, display = "", wayland = "" },
  open_fn = function(p)
    table.insert(opened_paths, p)
    return {}
  end,
  on_done = function(e, r)
    assert(e == nil and r ~= nil)
    bare_res = r
    bare_done = true
  end,
})
assert(
  vim.wait(4000, function()
    return bare_done
  end, 10),
  "Timed out waiting for bare HTML export"
)
assert(#opened_paths == 1 and vim.fn.filereadable(bare_res._written_path) == 1)
assert(
  bare_res.content:find("Snippets: ON (local)", 1, true) ~= nil,
  "Bare HTML export must default snippets ON"
)

local explicit_out = tmp_root .. "/explicit.html"
local exp_done = false
local exp_res = nil
export.export({
  format = "html",
  path = explicit_out,
  no_lsp = true,
  workspace_root = tmp_root,
  open_fn = function(p)
    table.insert(opened_paths, p)
    return {}
  end,
  on_done = function(e, r)
    assert(e == nil and r ~= nil)
    exp_res = r
    exp_done = true
  end,
})
assert(
  vim.wait(4000, function()
    return exp_done
  end, 10),
  "Timed out waiting for explicit HTML export"
)
assert(#opened_paths == 1, "Explicit path save must be silent without --open")
assert(exp_res._open_status == "saved_silent")
assert(
  exp_res.content:find("Snippets: OFF", 1, true) ~= nil,
  "Explicit HTML export must default snippets OFF"
)

-- Verify telemetry line formatting includes open= and pruned=
local telemetry = client.get_telemetry()
local last_t = telemetry[#telemetry]
local t_line = debug_mod.format_telemetry_entry(last_t)
assert(t_line:find("open=saved_silent", 1, true) ~= nil and t_line:find("pruned=0", 1, true) ~= nil)

-- Capability mismatch rejection for Stage C.1 binary missing "html"
local orig_caps = client.capabilities
client.capabilities = function()
  return { languages = { "typescript" }, export_formats = { "mermaid", "dot" } }
end
local cap_err = nil
export.export({
  format = "html",
  ctx = { origin_uri = vim.uri_from_bufnr(buf), origin_pos = { 6, 9 } },
  on_done = function(e, _)
    cap_err = e
  end,
})
assert(vim.wait(2000, function()
  return cap_err ~= nil
end, 10))
assert(cap_err.message:find("Lineage install!", 1, true) ~= nil)
client.capabilities = orig_caps

-- Run health.check() to ensure Stage C.2 diagnostics do not error
health.check()

client.stop()
vim.api.nvim_buf_delete(buf, { force = true })
vim.fn.delete(tmp_root, "rf")
print("test_export_html.lua: PASS")

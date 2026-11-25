local commands = require("lineage.commands")
local client = require("lineage.client")
local debug_mod = require("lineage.debug")
local export = require("lineage.export")
local tree = require("lineage.ui.tree")
local hover = require("lineage.ui.hover")
local focus = require("lineage.ui.focus")

-- 1. Multi-arg :Lineage export parsing & completion
local parsed, err = commands.parse_export_args({
  "--no-lsp",
  "--direction",
  "both",
  "--null-only",
  "--exclude-tests",
  "mermaid",
  "--snippets",
  "out/flow.mmd",
}, true)
assert(
  err == nil and parsed ~= nil,
  "Expected order-independent parse to succeed: " .. tostring(err)
)
assert(parsed.format == "mermaid")
assert(parsed.direction == "both")
assert(parsed.null_only == true)
assert(parsed.exclude_tests == true)
assert(parsed.no_lsp == true)
assert(parsed.include_snippets == true)
assert(parsed.path == "out/flow.mmd")
assert(parsed.force == true)

local _, conflict_err = commands.parse_export_args({ "mermaid", "a.mmd", "--out", "b.mmd" }, false)
assert(conflict_err ~= nil, "Simultaneous positional path and --out must fail")

local _, snip_err = commands.parse_export_args({ "mermaid", "--snippets", "--no-snippets" }, false)
assert(snip_err ~= nil, "Conflicting --snippets and --no-snippets must fail")

local _, bad_fmt = commands.parse_export_args({ "json" }, false)
assert(bad_fmt ~= nil, "Unsupported format keyword must fail")

local comp_sub = commands.complete("ex", ":Lineage ex", 11)
assert(vim.tbl_contains(comp_sub, "export"), "Completion should suggest export subcommand")

local comp_dir = commands.complete("", ":Lineage export --direction ", 28)
assert(
  vim.tbl_contains(comp_dir, "both")
    and vim.tbl_contains(comp_dir, "to")
    and vim.tbl_contains(comp_dir, "from"),
  "Completion should suggest directions after --direction"
)

-- Verify <Plug>(LineageExport*) mappings registered
local plug_maps = vim.api.nvim_get_keymap("n")
local found_plugs = {}
for _, m in ipairs(plug_maps) do
  if vim.startswith(m.lhs, "<Plug>(LineageExport") then
    found_plugs[m.lhs] = true
  end
end
assert(
  found_plugs["<Plug>(LineageExport)"]
    and found_plugs["<Plug>(LineageExportMermaid)"]
    and found_plugs["<Plug>(LineageExportDot)"],
  "Expected all three <Plug>(LineageExport*) mappings"
)

-- 2. Transport hardening: chunk-array NDJSON buffering + decode-error rejection
local chunk_res = nil
local chunk_err = nil
client._register_pending_for_test("c-chunk-1", function(e, r)
  chunk_err = e
  chunk_res = r
end)
client._feed_stdout_chunk('{"jsonrpc":"2.0","id":"c-chunk-1",')
assert(chunk_res == nil, "Partial chunk without newline must not fire callback")
client._feed_stdout_chunk('"result":{"format":"mermaid",')
client._feed_stdout_chunk('"content":"flowchart TD\\n  N0[\\"ok\\"]"}}\n')
assert(
  vim.wait(1000, function()
    return chunk_res ~= nil or chunk_err ~= nil
  end, 10),
  "Timed out waiting for multi-chunk NDJSON response"
)
assert(chunk_err == nil and chunk_res.format == "mermaid")

local dec_err = nil
client._register_pending_for_test("c-bad-1", function(e, _)
  dec_err = e
end)
client._feed_stdout_chunk('{"id":"c-bad-1","result":CORRUPT_JSON\n')
assert(
  vim.wait(1000, function()
    return dec_err ~= nil
  end, 10),
  "Corrupted JSON frame must reject pending callback cleanly instead of hanging"
)

-- 3. Telemetry formatting for lineage/export in debug.lua
local fmt_line = debug_mod.format_telemetry_entry({
  method = "lineage/export",
  format = "mermaid",
  source_mode = "live_snapshot",
  phase2_status = "complete",
  rtt_ms = 12,
  output_bytes = 256,
})
assert(fmt_line:find("lineage/export") ~= nil and fmt_line:find("phase1=nil") == nil)

-- 4. Picker intersection with Lua-supported formats
local intersected = export.supported_picker_formats({
  export_formats = { "mermaid", "dot", "html", "unknown_fmt" },
})
assert(
  vim.deep_equal(intersected, { "mermaid", "dot", "html" }),
  "Picker must keep supported formats and intersect out unknown formats"
)

-- 5. Live engine tests: Cursor export, overwrite guard, clipboard fallback, Tree e / Hover ge, Focus non-hijack
local tmpdir = vim.fn.tempname()
vim.fn.mkdir(tmpdir, "p")
local src_file = tmpdir .. "/sample.ts"
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
vim.bo[buf].modified = false
vim.api.nvim_set_current_buf(buf)
vim.api.nvim_win_set_cursor(0, { 6, 9 })

-- Focus/Pin must NOT hijack cursor export
focus.pin({ bufnr = buf, position = { 1, 10 } })
local kind, ctx = export.resolve_context()
assert(kind == "cursor", "Focus/Pin must not hijack cursor export, got " .. tostring(kind))
assert(ctx.origin_pos[1] == 6, "Cursor export must preserve current cursor line")
focus.clear()

-- Fresh cursor export to explicit file + overwrite protection
local out_dot = tmpdir .. "/diagram.dot"
local exp_done = false
local exp_err = nil
export.export({
  format = "dot",
  path = out_dot,
  no_lsp = true,
  workspace_root = tmpdir,
  on_done = function(e, r)
    exp_err = e
    exp_done = r ~= nil
  end,
})
assert(
  vim.wait(4000, function()
    return exp_done or exp_err ~= nil
  end, 10),
  "Timed out waiting for fresh cursor DOT export"
)
assert(exp_err == nil and vim.fn.filereadable(out_dot) == 1, "Expected DOT file to be written")
local dot_txt = table.concat(vim.fn.readfile(out_dot), "\n")
assert(dot_txt:find("digraph lineage") ~= nil)
assert(dot_txt:find("Static only (--no-lsp)", 1, true) ~= nil)

-- Without force, overwriting existing file must refuse
local refuse_err = nil
export.export({
  format = "dot",
  path = out_dot,
  force = false,
  no_lsp = true,
  on_done = function(e, _)
    refuse_err = e
  end,
})
assert(refuse_err ~= nil and refuse_err.message:find("Lineage export!") ~= nil)

-- With force = true (:Lineage export!), overwrite succeeds
local force_done = false
export.export({
  format = "dot",
  path = out_dot,
  force = true,
  no_lsp = true,
  workspace_root = tmpdir,
  on_done = function(e, r)
    assert(e == nil and r ~= nil)
    force_done = true
  end,
})
assert(
  vim.wait(3000, function()
    return force_done
  end, 10),
  "Timed out waiting for forced overwrite export"
)

-- 6. Clipboard '+' -> '"' WARN fallback when has("clipboard") == 0
local orig_has = vim.fn.has
local warn_messages = {}
local orig_notify = vim.notify
vim.fn.has = function(feat)
  if feat == "clipboard" then
    return 0
  end
  return orig_has(feat)
end
vim.notify = function(msg, level)
  if level == vim.log.levels.WARN then
    table.insert(warn_messages, msg)
  end
end

local clip_done = false
export.export({
  format = "mermaid",
  no_lsp = true,
  workspace_root = tmpdir,
  on_done = function(e, r)
    assert(e == nil and r ~= nil)
    clip_done = true
  end,
})
assert(
  vim.wait(3000, function()
    return clip_done
  end, 10),
  "Timed out waiting for clipboard fallback export"
)
vim.fn.has = orig_has
vim.notify = orig_notify
assert(
  vim.fn.getreg('"'):find("flowchart TD") ~= nil,
  'Expected Mermaid in unnamed " register on clipboard fallback'
)
assert(#warn_messages > 0, "Expected WARN notification on clipboard fallback")

-- 7. Tree Panel live export & zero collision with `g` grouping
tree.open("to")
assert(
  vim.wait(4000, function()
    local st = tree.get_state()
    return st.bufnr ~= nil and #(st.rows or {}) > 0
  end, 10),
  "Timed out waiting for Tree Panel rows"
)
local tst = tree.get_state()
local hdr = vim.api.nvim_buf_get_lines(tst.bufnr, 0, 1, false)[1]
assert(hdr:find("e:export") ~= nil, "Tree header must advertise e:export")
local t_kind, t_ctx = export.resolve_context()
assert(t_kind == "tree" and t_ctx.query_id == tst.query_id, "Focused Tree must take precedence")

-- Verify `g` keymap still toggles grouping cleanly without hanging for `ge`
local tree_maps = vim.api.nvim_buf_get_keymap(tst.bufnr, "n")
local has_g, has_e = false, false
for _, m in ipairs(tree_maps) do
  if m.lhs == "g" then
    has_g = true
    assert(m.nowait == 1, "Tree g mapping must keep nowait")
  elseif m.lhs == "e" then
    has_e = true
  end
end
assert(has_g and has_e, "Expected both g and e mappings in Tree Panel")

local tree_exp_done = false
local tree_exp_res = nil
export.export({
  format = "mermaid",
  workspace_root = tmpdir,
  on_done = function(e, r)
    assert(e == nil)
    tree_exp_res = r
    tree_exp_done = true
  end,
})
assert(
  vim.wait(3000, function()
    return tree_exp_done
  end, 10),
  "Timed out waiting for live Tree export"
)
assert(tree_exp_res.source_mode == "live_snapshot", "Tree export must hit live_snapshot LRU")
tree.close()

-- 8. Hover export context & `ge` keymap
vim.api.nvim_set_current_buf(buf)
vim.api.nvim_win_set_cursor(0, { 6, 9 })
hover.open()
assert(
  vim.wait(4000, function()
    local hst = hover.get_active_state()
    return hst.winid ~= nil and vim.api.nvim_win_is_valid(hst.winid)
  end, 10),
  "Timed out waiting for Hover float"
)
local hst = hover.get_active_state()
local hover_maps = vim.api.nvim_buf_get_keymap(hst.bufnr, "n")
local has_ge = false
for _, m in ipairs(hover_maps) do
  if m.lhs == "ge" then
    has_ge = true
  end
end
assert(has_ge, "Expected ge export keymap in Hover float")
local h_kind, h_ctx = export.resolve_context()
assert(
  h_kind == "hover" and h_ctx.query_id == hst.query_id,
  "Focused Hover must resolve hover context"
)
vim.api.nvim_win_close(hst.winid, true)

-- 9. Capability mismatch fail-fast on pre-Stage-C engine binaries
local orig_caps = client.capabilities
client.capabilities = function()
  return { languages = { "typescript" }, reverse_rpc = true, delta_notifications = true }
end
local cap_err = nil
export.export({
  format = "mermaid",
  ctx = { origin_uri = vim.uri_from_bufnr(buf), origin_pos = { 6, 9 } },
  on_done = function(e, _)
    cap_err = e
  end,
})
assert(
  vim.wait(2000, function()
    return cap_err ~= nil
  end, 10),
  "Expected capability mismatch error"
)
assert(
  cap_err.message:find("Lineage install!") ~= nil,
  "Capability mismatch must guide user to :Lineage install!"
)
client.capabilities = orig_caps

client.stop()
vim.api.nvim_buf_delete(buf, { force = true })
vim.fn.delete(tmpdir, "rf")
print("test_export.lua: PASS")

local install = require("lineage.install")
local client = require("lineage.client")
local health = require("lineage.health")

local function make_temp_dir()
  local dir = vim.fn.tempname()
  vim.fn.mkdir(dir, "p")
  return dir
end

-- 1. Pure LuaJIT `bit` SHA-256 NIST vectors
assert(
  install.sha256_pure("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "SHA-256 empty string mismatch"
)
assert(
  install.sha256_pure("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
  "SHA-256 'abc' mismatch"
)
assert(
  install.sha256_pure("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
    == "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
  "SHA-256 448-bit NIST vector mismatch"
)

-- 2. Binary-safe hashing with embedded NUL bytes (0x00)
local tmp_dir = make_temp_dir()
local bin_payload = "\0\1\2\0\255\127\128lineage\0engine\0\10\13\0"
local bin_file = tmp_dir .. "/nul_payload.bin"
do
  local fh = assert(io.open(bin_file, "wb"))
  fh:write(bin_payload)
  fh:close()
end

local pure_digest = assert(install.sha256_file(bin_file, { force_pure = true }))
local cli_digest = assert(install.sha256_file(bin_file))
assert(
  pure_digest == cli_digest,
  string.format("Pure SHA-256 (%s) != CLI SHA-256 (%s)", pure_digest, cli_digest)
)
assert(
  pure_digest == install.sha256_pure(bin_payload),
  "File streaming pure SHA-256 must match in-memory pure SHA-256"
)

-- 3. Repo checksums.json structure
local cs, cs_err = install.read_checksums()
assert(cs ~= nil, "Failed to read repo checksums.json: " .. tostring(cs_err))
assert(cs.version == "0.1.0", "Expected version 0.1.0")
assert(
  cs.repo == "cristianfritis/lineage.nvim",
  "Expected repo cristianfritis/lineage.nvim, got " .. tostring(cs.repo)
)
assert(
  type(cs.engine_tag) == "string" and cs.engine_tag ~= "",
  "Expected engine_tag in checksums.json"
)
for _, triple in ipairs({
  "x86_64-unknown-linux-musl",
  "aarch64-unknown-linux-musl",
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
}) do
  assert(cs.targets[triple] ~= nil, "Missing target in checksums.json: " .. triple)
  assert(type(cs.targets[triple].asset) == "string", "Missing asset for " .. triple)
end

-- 4. Target detection
assert(install.detect_target("Linux", "x86_64") == "x86_64-unknown-linux-musl")
assert(install.detect_target("Linux", "aarch64") == "aarch64-unknown-linux-musl")
assert(install.detect_target("Darwin", "arm64") == "aarch64-apple-darwin")
assert(install.detect_target("Darwin", "x86_64") == "x86_64-apple-darwin")
local win_target = install.detect_target("Windows_NT", "AMD64")
assert(win_target == nil, "Expected Windows target to fall back to Cargo")

-- 5. Fixture-isolated checksum mismatch rejection & engine_tag URL routing
local test_cs_path = tmp_dir .. "/checksums_test.json"
local good_bytes = "#!/bin/sh\necho ok\n\0"
local good_sha = install.sha256_pure(good_bytes)
do
  local fh = assert(io.open(test_cs_path, "wb"))
  fh:write(vim.json.encode({
    version = "0.1.0",
    engine_tag = "engine-v0.1.0",
    repo = "cristianfritis/lineage.nvim",
    targets = {
      ["x86_64-unknown-linux-musl"] = {
        asset = "lineage-engine-x86_64-unknown-linux-musl",
        sha256 = good_sha,
      },
    },
  }))
  fh:close()
end

local data_dir = tmp_dir .. "/data"
local expected_dest = install.dest_path("engine-v0.1.0", { data_dir = data_dir })
local bad_ok, bad_err = install.install({
  silent = true,
  force = true,
  checksums_path = test_cs_path,
  data_dir = data_dir,
  target = "x86_64-unknown-linux-musl",
  download_fn = function(url, tmp_path)
    assert(
      url
        == "https://github.com/cristianfritis/lineage.nvim/releases/download/engine-v0.1.0/lineage-engine-x86_64-unknown-linux-musl",
      "Unexpected release URL: " .. tostring(url)
    )
    local fh = assert(io.open(tmp_path, "wb"))
    fh:write("corrupted-binary-payload")
    fh:close()
    return true, nil
  end,
})
assert(bad_ok == false, "Expected checksum mismatch to fail install without Cargo fallback")
assert(
  tostring(bad_err):find("mismatch", 1, true),
  "Expected mismatch error, got: " .. tostring(bad_err)
)
assert(vim.fn.filereadable(expected_dest) == 0, "Destination must not exist after failed verify")
local leftover = vim.fn.glob(expected_dest .. ".tmp.*", false, true)
assert(#leftover == 0, "Temp file must be deleted on checksum mismatch")

-- 6. Sidecar stop + chmod 0755 + atomic rename on valid download + already_installed SHA verify
local stop_called = 0
local orig_stop = client.stop
client.stop = function()
  stop_called = stop_called + 1
  orig_stop()
end

local ok_inst, res_inst = install.install({
  silent = true,
  force = true,
  force_pure = true,
  checksums_path = test_cs_path,
  data_dir = data_dir,
  target = "x86_64-unknown-linux-musl",
  download_fn = function(_, tmp_path)
    local fh = assert(io.open(tmp_path, "wb"))
    fh:write(good_bytes)
    fh:close()
    return true, nil
  end,
})
client.stop = orig_stop
assert(ok_inst == true, "Expected verified install to succeed: " .. tostring(res_inst))
assert(res_inst.status == "installed_prebuilt")
assert(stop_called == 1, "Expected client.stop() to be called before atomic rename")
assert(vim.fn.executable(expected_dest) == 1, "Installed binary must be executable (0755)")

-- Already installed with matching SHA returns already_installed without downloading
local dl_called_again = false
local ok_already, res_already = install.install({
  silent = true,
  force = false,
  force_pure = true,
  checksums_path = test_cs_path,
  data_dir = data_dir,
  target = "x86_64-unknown-linux-musl",
  download_fn = function()
    dl_called_again = true
    return false, "should not download"
  end,
})
assert(ok_already == true and res_already.status == "already_installed")
assert(dl_called_again == false, "Should not re-download when existing SHA-256 matches")

-- Corrupting existing binary without .source_build forces re-download
do
  local fh = assert(io.open(expected_dest, "wb"))
  fh:write("stale-corrupted-binary")
  fh:close()
end
local dl_on_corrupt = false
local ok_reinst, res_reinst = install.install({
  silent = true,
  force = false,
  force_pure = true,
  checksums_path = test_cs_path,
  data_dir = data_dir,
  target = "x86_64-unknown-linux-musl",
  download_fn = function(_, tmp_path)
    dl_on_corrupt = true
    local fh = assert(io.open(tmp_path, "wb"))
    fh:write(good_bytes)
    fh:close()
    return true, nil
  end,
})
assert(ok_reinst == true and res_reinst.status == "installed_prebuilt")
assert(dl_on_corrupt == true, "Corrupted existing binary must trigger re-download")

-- 7. Fixture-isolated "TBD" Cargo fallback + network-failure Cargo fallback + .source_build marker
local tbd_cs_path = tmp_dir .. "/checksums_tbd.json"
do
  local fh = assert(io.open(tbd_cs_path, "wb"))
  fh:write(vim.json.encode({
    version = "0.1.0",
    engine_tag = "engine-v0.1.0",
    repo = "cristianfritis/lineage.nvim",
    targets = {
      ["x86_64-unknown-linux-musl"] = {
        asset = "lineage-engine-x86_64-unknown-linux-musl",
        sha256 = "TBD",
      },
    },
  }))
  fh:close()
end

local fallback_data = tmp_dir .. "/data_cargo"
local fake_built = tmp_dir .. "/fake_built_engine"
do
  local fh = assert(io.open(fake_built, "wb"))
  fh:write("#!/bin/sh\nexit 0\n")
  fh:close()
end
local cargo_called = false
local ok_cargo, res_cargo = install.install({
  silent = true,
  force = true,
  checksums_path = tbd_cs_path,
  data_dir = fallback_data,
  target = "x86_64-unknown-linux-musl",
  built_binary = fake_built,
  cargo_fn = function()
    cargo_called = true
    return true, nil
  end,
})
assert(ok_cargo == true, "Expected Cargo fallback install to succeed: " .. tostring(res_cargo))
assert(cargo_called == true, "Expected cargo_fn to be invoked for TBD checksums")
assert(res_cargo.status == "installed_cargo")
assert(vim.fn.executable(res_cargo.path) == 1, "Cargo fallback binary must be executable")
assert(
  vim.fn.filereadable(install.source_build_marker(res_cargo.path)) == 1,
  "Cargo fallback must write .source_build marker"
)

-- Network download failure falls back to Cargo and writes .source_build marker
local net_data = tmp_dir .. "/data_net_fallback"
local net_cargo_called = false
local ok_net, res_net = install.install({
  silent = true,
  force = true,
  checksums_path = test_cs_path,
  data_dir = net_data,
  target = "x86_64-unknown-linux-musl",
  built_binary = fake_built,
  download_fn = function()
    return false, "curl failed (6): Could not resolve host"
  end,
  cargo_fn = function()
    net_cargo_called = true
    return true, nil
  end,
})
assert(ok_net == true and res_net.status == "installed_cargo")
assert(net_cargo_called == true, "Network failure should fall back to Cargo")
assert(
  vim.fn.filereadable(install.source_build_marker(res_net.path)) == 1,
  "Network Cargo fallback must write .source_build marker"
)

-- Verified release install cleans up .source_build marker
local ok_clean_rel, res_clean_rel = install.install({
  silent = true,
  force = true,
  force_pure = true,
  checksums_path = test_cs_path,
  data_dir = net_data,
  target = "x86_64-unknown-linux-musl",
  download_fn = function(_, tmp_path)
    local fh = assert(io.open(tmp_path, "wb"))
    fh:write(good_bytes)
    fh:close()
    return true, nil
  end,
})
assert(ok_clean_rel == true and res_clean_rel.status == "installed_prebuilt")
assert(
  vim.fn.filereadable(install.source_build_marker(res_clean_rel.path)) == 0,
  "Verified release install must remove .source_build marker"
)

-- 8. :Lineage install! and :Lineage! install pass { force = true }
local lineage = require("lineage")
local orig_install = lineage.install
local install_calls = {}
lineage.install = function(o)
  table.insert(install_calls, o or {})
end
vim.cmd("Lineage install!")
vim.cmd("Lineage! install")
lineage.install = orig_install
assert(#install_calls == 2, "Expected 2 install calls from :Lineage install! and :Lineage! install")
assert(install_calls[1].force == true and install_calls[2].force == true, "Expected force=true")

-- 9. :checkhealth lineage surfaces :Lineage install when binary is missing
local orig_find = client.find_binary
local orig_health = vim.health
local health_errors = {}
client.find_binary = function()
  return nil, nil
end
vim.health = {
  start = function() end,
  ok = function() end,
  info = function() end,
  warn = function() end,
  error = function(msg)
    table.insert(health_errors, msg)
  end,
}
health.check()
client.find_binary = orig_find
vim.health = orig_health

local found_install_hint = false
for _, msg in ipairs(health_errors) do
  if tostring(msg):find(":Lineage install", 1, true) then
    found_install_hint = true
  end
end
assert(
  found_install_hint,
  "Expected :checkhealth lineage missing binary error to mention :Lineage install"
)

-- Cleanup
vim.fn.delete(tmp_dir, "rf")
print("test_install.lua: PASS")

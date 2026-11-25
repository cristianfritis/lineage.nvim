local bit = require("bit")
local config = require("lineage.config")

local M = {}

local uv = vim.uv or vim.loop

-- FIPS 180-4 SHA-256 initial hash values and round constants
local H_INIT = {
  0x6a09e667,
  0xbb67ae85,
  0x3c6ef372,
  0xa54ff53a,
  0x510e527f,
  0x9b05688c,
  0x1f83d9ab,
  0x5be0cd19,
}

local K = {
  0x428a2f98,
  0x71374491,
  0xb5c0fbcf,
  0xe9b5dba5,
  0x3956c25b,
  0x59f111f1,
  0x923f82a4,
  0xab1c5ed5,
  0xd807aa98,
  0x12835b01,
  0x243185be,
  0x550c7dc3,
  0x72be5d74,
  0x80deb1fe,
  0x9bdc06a7,
  0xc19bf174,
  0xe49b69c1,
  0xefbe4786,
  0x0fc19dc6,
  0x240ca1cc,
  0x2de92c6f,
  0x4a7484aa,
  0x5cb0a9dc,
  0x76f988da,
  0x983e5152,
  0xa831c66d,
  0xb00327c8,
  0xbf597fc7,
  0xc6e00bf3,
  0xd5a79147,
  0x06ca6351,
  0x14292967,
  0x27b70a85,
  0x2e1b2138,
  0x4d2c6dfc,
  0x53380d13,
  0x650a7354,
  0x766a0abb,
  0x81c2c92e,
  0x92722c85,
  0xa2bfe8a1,
  0xa81a664b,
  0xc24b8b70,
  0xc76c51a3,
  0xd192e819,
  0xd6990624,
  0xf40e3585,
  0x106aa070,
  0x19a4c116,
  0x1e376c08,
  0x2748774c,
  0x34b0bcb5,
  0x391c0cb3,
  0x4ed8aa4a,
  0x5b9cca4f,
  0x682e6ff3,
  0x748f82ee,
  0x78a5636f,
  0x84c87814,
  0x8cc70208,
  0x90befffa,
  0xa4506ceb,
  0xbef9a3f7,
  0xc67178f2,
}

local function sha256_compress(h, block, offset)
  local w = {}
  for i = 1, 16 do
    local p = offset + (i - 1) * 4
    local b1, b2, b3, b4 = string.byte(block, p, p + 3)
    w[i] = bit.bor(bit.lshift(b1, 24), bit.lshift(b2, 16), bit.lshift(b3, 8), b4)
  end
  for i = 17, 64 do
    local s0 = bit.bxor(bit.ror(w[i - 15], 7), bit.ror(w[i - 15], 18), bit.rshift(w[i - 15], 3))
    local s1 = bit.bxor(bit.ror(w[i - 2], 17), bit.ror(w[i - 2], 19), bit.rshift(w[i - 2], 10))
    w[i] = bit.tobit(w[i - 16] + s0 + w[i - 7] + s1)
  end

  local a, b, c, d, e, f, g, hh = h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8]

  for i = 1, 64 do
    local S1 = bit.bxor(bit.ror(e, 6), bit.ror(e, 11), bit.ror(e, 25))
    local ch = bit.bxor(bit.band(e, f), bit.band(bit.bnot(e), g))
    local temp1 = bit.tobit(hh + S1 + ch + K[i] + w[i])
    local S0 = bit.bxor(bit.ror(a, 2), bit.ror(a, 13), bit.ror(a, 22))
    local maj = bit.bxor(bit.band(a, b), bit.band(a, c), bit.band(b, c))
    local temp2 = bit.tobit(S0 + maj)

    hh = g
    g = f
    f = e
    e = bit.tobit(d + temp1)
    d = c
    c = b
    b = a
    a = bit.tobit(temp1 + temp2)
  end

  h[1] = bit.tobit(h[1] + a)
  h[2] = bit.tobit(h[2] + b)
  h[3] = bit.tobit(h[3] + c)
  h[4] = bit.tobit(h[4] + d)
  h[5] = bit.tobit(h[5] + e)
  h[6] = bit.tobit(h[6] + f)
  h[7] = bit.tobit(h[7] + g)
  h[8] = bit.tobit(h[8] + hh)
end

local function sha256_init()
  local h = {}
  for i = 1, 8 do
    h[i] = bit.tobit(H_INIT[i])
  end
  return { h = h, buf = "", total_bytes = 0 }
end

local function sha256_update(ctx, chunk)
  if #chunk == 0 then
    return
  end
  ctx.total_bytes = ctx.total_bytes + #chunk
  ctx.buf = ctx.buf .. chunk
  local pos = 1
  local len = #ctx.buf
  while pos + 63 <= len do
    sha256_compress(ctx.h, ctx.buf, pos)
    pos = pos + 64
  end
  if pos > 1 then
    ctx.buf = ctx.buf:sub(pos)
  end
end

local function sha256_final(ctx)
  local rem = ctx.buf
  local pad_len = (55 - #rem) % 64
  local bit_hi = math.floor(ctx.total_bytes / 0x20000000)
  local bit_lo = (ctx.total_bytes * 8) % 0x100000000

  local tail = rem
    .. "\128"
    .. string.rep("\0", pad_len)
    .. string.char(
      bit.band(bit.rshift(bit_hi, 24), 0xff),
      bit.band(bit.rshift(bit_hi, 16), 0xff),
      bit.band(bit.rshift(bit_hi, 8), 0xff),
      bit.band(bit_hi, 0xff),
      bit.band(bit.rshift(bit_lo, 24), 0xff),
      bit.band(bit.rshift(bit_lo, 16), 0xff),
      bit.band(bit.rshift(bit_lo, 8), 0xff),
      bit.band(bit_lo, 0xff)
    )

  for pos = 1, #tail, 64 do
    sha256_compress(ctx.h, tail, pos)
  end

  local out = {}
  for i = 1, 8 do
    out[i] = bit.tohex(ctx.h[i], 8)
  end
  return table.concat(out)
end

--- Compute binary-safe SHA-256 hex digest of an in-memory string using pure LuaJIT `bit`.
function M.sha256_pure(data)
  local ctx = sha256_init()
  sha256_update(ctx, data or "")
  return sha256_final(ctx)
end

--- Stream a file in binary mode and compute SHA-256 via pure LuaJIT `bit` (handles NUL bytes).
function M.sha256_file_pure(path)
  local fh, err = io.open(path, "rb")
  if not fh then
    return nil, "Failed to open file for hashing: " .. tostring(err)
  end
  local ctx = sha256_init()
  while true do
    local chunk = fh:read(65536)
    if not chunk then
      break
    end
    sha256_update(ctx, chunk)
  end
  fh:close()
  return sha256_final(ctx), nil
end

--- Compute SHA-256 digest of a file using platform CLI tools with pure LuaJIT fallback.
function M.sha256_file(path, opts)
  opts = opts or {}
  if not opts.force_pure then
    if vim.fn.executable("sha256sum") == 1 then
      local res = vim.system({ "sha256sum", path }, { text = true }):wait()
      if res.code == 0 and res.stdout then
        local digest = res.stdout:match("^([0-9a-fA-F]+)")
        if digest and #digest == 64 then
          return digest:lower(), nil
        end
      end
    elseif vim.fn.executable("shasum") == 1 then
      local res = vim.system({ "shasum", "-a", "256", path }, { text = true }):wait()
      if res.code == 0 and res.stdout then
        local digest = res.stdout:match("^([0-9a-fA-F]+)")
        if digest and #digest == 64 then
          return digest:lower(), nil
        end
      end
    elseif vim.fn.has("win32") == 1 and vim.fn.executable("powershell") == 1 then
      local ps_cmd = string.format(
        "(Get-FileHash -Algorithm SHA256 -LiteralPath '%s').Hash",
        path:gsub("'", "''")
      )
      local res = vim
        .system({ "powershell", "-NoProfile", "-Command", ps_cmd }, { text = true })
        :wait()
      if res.code == 0 and res.stdout then
        local digest = res.stdout:match("([0-9a-fA-F]+)")
        if digest and #digest == 64 then
          return digest:lower(), nil
        end
      end
    end
  end

  return M.sha256_file_pure(path)
end

function M.plugin_root()
  local source_dir = debug.getinfo(1, "S").source:sub(2)
  return vim.fn.fnamemodify(source_dir, ":h:h:h")
end

function M.read_checksums(path)
  local p = path or (M.plugin_root() .. "/checksums.json")
  local fh, err = io.open(p, "rb")
  if not fh then
    return nil, "Failed to read checksums.json (" .. p .. "): " .. tostring(err)
  end
  local raw = fh:read("*a")
  fh:close()

  local ok, decoded = pcall(vim.json.decode, raw)
  if not ok or type(decoded) ~= "table" then
    return nil, "Invalid checksums.json: " .. tostring(decoded)
  end
  if type(decoded.version) ~= "string" or decoded.version == "" then
    return nil, "checksums.json missing 'version'"
  end
  if type(decoded.repo) ~= "string" or decoded.repo == "" then
    return nil, "checksums.json missing 'repo'"
  end
  if type(decoded.targets) ~= "table" then
    return nil, "checksums.json missing 'targets'"
  end
  if type(decoded.engine_tag) ~= "string" or decoded.engine_tag == "" then
    decoded.engine_tag = "v" .. tostring(decoded.version)
  end
  return decoded, nil
end

function M.detect_target(sysname, machine)
  if not sysname or not machine then
    local uname = uv.os_uname()
    sysname = sysname or uname.sysname
    machine = machine or uname.machine
  end

  if sysname == "Linux" then
    if machine == "x86_64" or machine == "amd64" then
      return "x86_64-unknown-linux-musl", nil
    elseif machine == "aarch64" or machine == "arm64" then
      return "aarch64-unknown-linux-musl", nil
    end
  elseif sysname == "Darwin" then
    if machine == "arm64" or machine == "aarch64" then
      return "aarch64-apple-darwin", nil
    elseif machine == "x86_64" or machine == "amd64" then
      return "x86_64-apple-darwin", nil
    end
  end

  return nil,
    string.format("Unsupported prebuilt target (%s / %s)", tostring(sysname), tostring(machine))
end

function M.dest_path(tag_or_version, opts)
  opts = opts or config.get()
  local data_dir = opts.data_dir or vim.fn.stdpath("data")
  local ext = (uv.os_uname().sysname == "Windows_NT") and ".exe" or ""
  local tag = tostring(tag_or_version)
  if not tag:match("^engine%-") and not tag:match("^v") then
    tag = "v" .. tag
  end
  return data_dir .. "/lineage/bin/lineage-engine-" .. tag .. ext
end

function M.source_build_marker(dest_path)
  return dest_path .. ".source_build"
end

function M.resolve_pinned_binary(opts)
  opts = opts or {}
  local checksums, cs_err = M.read_checksums(opts.checksums_path)
  if not checksums then
    return nil, cs_err
  end
  local version = checksums.version or config.get().version or "0.2.0"
  local engine_tag = checksums.engine_tag or ("v" .. tostring(version))
  local dest = M.dest_path(engine_tag, opts)
  local target, target_err = M.detect_target(opts.sysname, opts.machine)
  local entry = target and checksums.targets and checksums.targets[target] or nil
  local is_source_build = vim.fn.filereadable(M.source_build_marker(dest)) == 1
  return {
    path = dest,
    version = version,
    engine_tag = engine_tag,
    repo = checksums.repo,
    target = target,
    target_err = target_err,
    entry = entry,
    checksums = checksums,
    is_source_build = is_source_build,
    provenance = is_source_build and "source_build" or "release",
  },
    nil
end

function M.is_placeholder_sha(sha)
  return sha == nil or sha == "" or sha == "TBD"
end

function M.verify_file(path, expected_sha256, opts)
  if type(expected_sha256) ~= "string" or M.is_placeholder_sha(expected_sha256) then
    return false, nil, "Missing or placeholder SHA-256 digest"
  end
  local actual, err = M.sha256_file(path, opts)
  if not actual then
    return false, nil, err
  end
  if actual:lower() ~= expected_sha256:lower() then
    return false,
      actual,
      string.format("SHA-256 mismatch: expected %s, got %s", expected_sha256:lower(), actual)
  end
  return true, actual, nil
end

local function download_file(url, dest_tmp)
  if vim.fn.executable("curl") == 1 then
    local res = vim
      .system(
        { "curl", "-fsSL", "--connect-timeout", "15", "--max-time", "120", "-o", dest_tmp, url },
        { text = true }
      )
      :wait()
    if res.code == 0 then
      return true, nil
    end
    return false, "curl failed (" .. tostring(res.code) .. "): " .. (res.stderr or "")
  elseif vim.fn.executable("wget") == 1 then
    local res = vim
      .system({ "wget", "-q", "--timeout=15", "-O", dest_tmp, url }, { text = true })
      :wait()
    if res.code == 0 then
      return true, nil
    end
    return false, "wget failed (" .. tostring(res.code) .. "): " .. (res.stderr or "")
  elseif vim.fn.has("win32") == 1 and vim.fn.executable("powershell") == 1 then
    local ps_cmd = string.format(
      "Invoke-WebRequest -Uri '%s' -OutFile '%s'",
      url:gsub("'", "''"),
      dest_tmp:gsub("'", "''")
    )
    local res = vim
      .system({ "powershell", "-NoProfile", "-Command", ps_cmd }, { text = true })
      :wait()
    if res.code == 0 then
      return true, nil
    end
    return false, "Invoke-WebRequest failed: " .. (res.stderr or "")
  end
  return false, "Neither curl nor wget is available to download prebuilt binary"
end

function M.atomic_install_file(tmp_path, dest_path, opts)
  opts = opts or {}
  local parent = vim.fn.fnamemodify(dest_path, ":h")
  vim.fn.mkdir(parent, "p")

  -- Stop any active sidecar before swapping the binary into place.
  local client = opts.client or require("lineage.client")
  if client and client.stop then
    pcall(client.stop)
  end

  local is_windows = (opts.sysname or uv.os_uname().sysname) == "Windows_NT"
  if not is_windows then
    local ok_chmod, chmod_err = uv.fs_chmod(tmp_path, tonumber("755", 8))
    if not ok_chmod then
      pcall(uv.fs_unlink, tmp_path)
      return false, "Failed to chmod binary: " .. tostring(chmod_err)
    end
  end

  local ok_ren, ren_err = uv.fs_rename(tmp_path, dest_path)
  if not ok_ren then
    pcall(uv.fs_unlink, tmp_path)
    return false, "Failed to rename binary into place: " .. tostring(ren_err)
  end

  return true, nil
end

function M.cargo_build_install(dest_path, opts)
  opts = opts or {}
  if vim.fn.executable("cargo") ~= 1 then
    return false, "Prebuilt binary unavailable and 'cargo' not found in PATH"
  end

  local root = opts.plugin_root or M.plugin_root()
  local manifest = opts.manifest_path or (root .. "/engine/Cargo.toml")
  local run_cargo = opts.cargo_fn
    or function()
      local res = vim
        .system({ "cargo", "build", "--release", "--manifest-path", manifest }, { text = true })
        :wait()
      if res.code ~= 0 then
        return false, "cargo build --release failed: " .. (res.stderr or "")
      end
      return true, nil
    end

  local ok_build, build_err = run_cargo()
  if not ok_build then
    return false, build_err
  end

  local is_windows = (opts.sysname or uv.os_uname().sysname) == "Windows_NT"
  local ext = is_windows and ".exe" or ""
  local built_bin = opts.built_binary or (root .. "/engine/target/release/lineage-engine" .. ext)
  if vim.fn.filereadable(built_bin) ~= 1 then
    return false, "Cargo build succeeded but binary missing at " .. built_bin
  end

  local parent = vim.fn.fnamemodify(dest_path, ":h")
  vim.fn.mkdir(parent, "p")
  local tmp_path = dest_path .. ".tmp." .. tostring(vim.fn.getpid())
  local ok_copy, copy_err = uv.fs_copyfile(built_bin, tmp_path)
  if not ok_copy then
    pcall(uv.fs_unlink, tmp_path)
    return false, "Failed to copy built binary: " .. tostring(copy_err)
  end

  local ok_swap, swap_err = M.atomic_install_file(tmp_path, dest_path, opts)
  if not ok_swap then
    return false, swap_err
  end

  local marker = io.open(M.source_build_marker(dest_path), "wb")
  if marker then
    marker:close()
  end

  return true, nil
end

local function notify(opts, msg, level)
  if opts and opts.silent then
    return
  end
  vim.notify("lineage.nvim: " .. msg, level or vim.log.levels.INFO)
end

function M.install(opts)
  opts = opts or {}
  local pinned, pin_err = M.resolve_pinned_binary(opts)
  if not pinned then
    notify(opts, pin_err, vim.log.levels.ERROR)
    return false, pin_err
  end

  local version = pinned.version
  local engine_tag = pinned.engine_tag
  local dest = pinned.path
  local target = opts.target or pinned.target
  local entry = target and pinned.checksums.targets and pinned.checksums.targets[target] or nil
  local cfg = config.get()
  local build_from_source = opts.build_from_source
  if build_from_source == nil then
    build_from_source = cfg.build_from_source
  end

  if not opts.force and vim.fn.executable(dest) == 1 then
    if pinned.is_source_build then
      notify(
        opts,
        "lineage-engine " .. engine_tag .. " (source_build) already installed at " .. dest
      )
      return true,
        {
          status = "already_installed",
          path = dest,
          version = version,
          engine_tag = engine_tag,
          provenance = "source_build",
        }
    elseif entry and not M.is_placeholder_sha(entry.sha256) then
      local ok_existing = M.verify_file(dest, entry.sha256, opts)
      if ok_existing then
        notify(opts, "lineage-engine " .. engine_tag .. " already installed at " .. dest)
        return true,
          {
            status = "already_installed",
            path = dest,
            version = version,
            engine_tag = engine_tag,
            provenance = "release",
          }
      end
      notify(opts, "existing binary failed SHA-256 verification; reinstalling", vim.log.levels.WARN)
    end
  end

  if not build_from_source and entry and entry.asset and not M.is_placeholder_sha(entry.sha256) then
    local url = string.format(
      "https://github.com/%s/releases/download/%s/%s",
      pinned.repo,
      engine_tag,
      entry.asset
    )
    local parent = vim.fn.fnamemodify(dest, ":h")
    vim.fn.mkdir(parent, "p")
    local tmp_path = dest .. ".tmp." .. tostring(vim.fn.getpid())

    local dl = opts.download_fn or download_file
    local ok_dl, dl_err = dl(url, tmp_path)
    if ok_dl then
      local ok_sha, _, sha_err = M.verify_file(tmp_path, entry.sha256, opts)
      if not ok_sha then
        pcall(uv.fs_unlink, tmp_path)
        notify(opts, sha_err, vim.log.levels.ERROR)
        return false, sha_err
      end

      local ok_swap, swap_err = M.atomic_install_file(tmp_path, dest, opts)
      if not ok_swap then
        notify(opts, swap_err, vim.log.levels.ERROR)
        return false, swap_err
      end

      pcall(uv.fs_unlink, M.source_build_marker(dest))
      notify(opts, "installed lineage-engine " .. engine_tag .. " (" .. target .. ") to " .. dest)
      return true,
        {
          status = "installed_prebuilt",
          path = dest,
          version = version,
          engine_tag = engine_tag,
          target = target,
          provenance = "release",
        }
    end

    pcall(uv.fs_unlink, tmp_path)
    if vim.fn.executable("cargo") ~= 1 and not opts.cargo_fn then
      notify(opts, "download failed: " .. tostring(dl_err), vim.log.levels.ERROR)
      return false, dl_err
    end
    notify(
      opts,
      "download failed (" .. tostring(dl_err) .. "); falling back to `cargo build --release`",
      vim.log.levels.WARN
    )
  elseif not build_from_source then
    notify(
      opts,
      "No published prebuilt binary for "
        .. tostring(target or "current platform")
        .. "; falling back to `cargo build --release`"
    )
  end

  local ok_cargo, cargo_err = M.cargo_build_install(dest, opts)
  if not ok_cargo then
    notify(opts, cargo_err, vim.log.levels.ERROR)
    return false, cargo_err
  end

  notify(opts, "built and installed lineage-engine " .. engine_tag .. " to " .. dest)
  return true,
    {
      status = "installed_cargo",
      path = dest,
      version = version,
      engine_tag = engine_tag,
      provenance = "source_build",
    }
end

return M

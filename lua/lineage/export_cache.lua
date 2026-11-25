local M = {}

local uv = vim.uv or vim.loop

local MODE_0700 = 448 -- tonumber("700", 8)
local MODE_0600 = 384 -- tonumber("600", 8)
local MAX_ORPHAN_TMP_AGE_SECS = 3600

function M.is_windows()
  local uname = uv.os_uname and uv.os_uname() or {}
  return uname.sysname == "Windows_NT"
end

function M.default_cache_dir()
  return vim.fn.stdpath("cache") .. "/lineage/exports"
end

function M.sanitize_slug(slug)
  local s = tostring(slug or "slice"):gsub("[^A-Za-z0-9._-]", "-")
  s = s:gsub("%-+", "-"):gsub("^[.-]+", ""):gsub("[.-]+$", "")
  if s == "" then
    s = "slice"
  end
  if #s > 40 then
    s = s:sub(1, 40):gsub("[.-]+$", "")
    if s == "" then
      s = "slice"
    end
  end
  return s
end

local function ensure_single_dir(dir)
  local st = uv.fs_lstat(dir)
  if st then
    if st.type == "link" then
      return false, "refusing symlink cache directory: " .. dir
    end
    if st.type ~= "directory" then
      return false, "cache path is not a directory: " .. dir
    end
    return true, nil
  end

  local parent = vim.fn.fnamemodify(dir, ":h")
  if parent and parent ~= "" and parent ~= dir then
    local ok_p, err_p = ensure_single_dir(parent)
    if not ok_p then
      return false, err_p
    end
  end

  local ok_mk, mk_err = uv.fs_mkdir(dir, MODE_0700)
  if not ok_mk then
    local re_st = uv.fs_lstat(dir)
    if not re_st or re_st.type ~= "directory" then
      return false, mk_err or ("failed to create directory: " .. dir)
    end
  end
  local post = uv.fs_lstat(dir)
  if not post or post.type == "link" or post.type ~= "directory" then
    return false, "refusing symlink or non-directory cache path: " .. dir
  end
  return true, nil
end

function M.ensure_dir(dir)
  local target = dir or M.default_cache_dir()
  local ok, err = ensure_single_dir(target)
  if not ok then
    return false, err
  end
  if not M.is_windows() then
    local ok_ch, ch_err = uv.fs_chmod(target, MODE_0700)
    if not ok_ch then
      return false, ch_err or ("failed to chmod 0700 on " .. target)
    end
  end
  return true, nil
end

local function mtime_key(st)
  local m = st and st.mtime or {}
  return m.sec or 0, m.nsec or 0
end

function M.prune(dir, max_files, keep_path)
  local limit = max_files or 20
  if limit < 1 then
    limit = 1
  end
  local scanner = uv.fs_scandir(dir)
  if not scanner then
    return 0
  end

  local now_sec = os.time()
  local entries = {}
  while true do
    local name = uv.fs_scandir_next(scanner)
    if not name then
      break
    end
    local full = dir .. "/" .. name
    local st = uv.fs_lstat(full)
    if st and st.type == "file" then
      if name:match("^lineage%-.*%.html$") then
        local sec, nsec = mtime_key(st)
        table.insert(entries, {
          name = name,
          path = full,
          sec = sec,
          nsec = nsec,
        })
      elseif name:match("%.tmp$") then
        local sec = mtime_key(st)
        if (now_sec - sec) > MAX_ORPHAN_TMP_AGE_SECS then
          pcall(uv.fs_unlink, full)
        end
      end
    end
  end

  table.sort(entries, function(a, b)
    if a.sec ~= b.sec then
      return a.sec > b.sec
    end
    if a.nsec ~= b.nsec then
      return a.nsec > b.nsec
    end
    if keep_path then
      if a.path == keep_path and b.path ~= keep_path then
        return true
      end
      if b.path == keep_path and a.path ~= keep_path then
        return false
      end
    end
    return a.name > b.name
  end)

  local pruned = 0
  local kept = 0
  for _, entry in ipairs(entries) do
    if entry.path == keep_path then
      kept = kept + 1
    elseif kept < limit then
      kept = kept + 1
    else
      local ok_u = uv.fs_unlink(entry.path)
      if ok_u then
        pruned = pruned + 1
      end
    end
  end
  return pruned
end

function M.write_cached(content, slug, opts)
  opts = opts or {}
  local dir = opts.cache_dir or M.default_cache_dir()
  local ok_dir, dir_err = M.ensure_dir(dir)
  if not ok_dir then
    return nil, dir_err
  end

  local sha12 = vim.fn.sha256(content):sub(1, 12)
  local clean_slug = M.sanitize_slug(slug)
  local final_path = string.format("%s/lineage-%s-%s.html", dir, clean_slug, sha12)

  local existing = uv.fs_lstat(final_path)
  if existing then
    if existing.type == "link" or existing.type ~= "file" then
      return nil, "refusing symlink or non-regular cache entry: " .. final_path
    end
    local now = os.time()
    pcall(uv.fs_utime, final_path, now, now)
    if not M.is_windows() then
      pcall(uv.fs_chmod, final_path, MODE_0600)
    end
    local pruned = M.prune(dir, opts.max_files, final_path)
    return { path = final_path, pruned = pruned, reused = true }, nil
  end

  local tmp = string.format("%s/.lineage-%s-%s.tmp", dir, sha12, tostring(uv.hrtime()))
  local fd, open_err = uv.fs_open(tmp, "wx", MODE_0600)
  if not fd then
    return nil, open_err or ("failed to create temp cache file: " .. tmp)
  end

  local ok_w, write_err = uv.fs_write(fd, content, 0)
  uv.fs_close(fd)
  if not ok_w then
    pcall(uv.fs_unlink, tmp)
    return nil, write_err or "failed to write cached HTML"
  end

  local ok_ren, ren_err = uv.fs_rename(tmp, final_path)
  if not ok_ren then
    pcall(uv.fs_unlink, tmp)
    return nil, ren_err or "failed to rename cached HTML"
  end

  local pruned = M.prune(dir, opts.max_files, final_path)
  return { path = final_path, pruned = pruned, reused = false }, nil
end

function M.clean(opts)
  opts = opts or {}
  local dir = opts.cache_dir or M.default_cache_dir()
  local st = uv.fs_lstat(dir)
  if not st then
    return { removed_files = 0, freed_bytes = 0 }, nil
  end
  if st.type == "link" or st.type ~= "directory" then
    return nil, "refusing symlink or non-directory cache path: " .. dir
  end

  local scanner = uv.fs_scandir(dir)
  if not scanner then
    return { removed_files = 0, freed_bytes = 0 }, nil
  end

  local removed = 0
  local freed = 0
  while true do
    local name = uv.fs_scandir_next(scanner)
    if not name then
      break
    end
    if name:match("^lineage%-.*%.html$") then
      local full = dir .. "/" .. name
      local est = uv.fs_lstat(full)
      if est and est.type == "file" then
        local ok_u = uv.fs_unlink(full)
        if ok_u then
          removed = removed + 1
          freed = freed + (est.size or 0)
        end
      end
    end
  end

  return { removed_files = removed, freed_bytes = freed }, nil
end

function M.can_open_browser(env_override)
  local ui_count = (env_override and env_override.ui_count ~= nil) and env_override.ui_count
    or #vim.api.nvim_list_uis()
  if ui_count == 0 then
    return false, "headless"
  end
  local sysname = (env_override and env_override.sysname)
    or ((uv.os_uname and uv.os_uname().sysname) or "")
  local is_wsl = (env_override and env_override.is_wsl ~= nil) and env_override.is_wsl
    or (vim.fn.has("wsl") == 1)
  if sysname ~= "Darwin" and sysname ~= "Windows_NT" and not is_wsl then
    local display = (env_override and env_override.display ~= nil) and env_override.display
      or vim.env.DISPLAY
    local wayland = (env_override and env_override.wayland ~= nil) and env_override.wayland
      or vim.env.WAYLAND_DISPLAY
    if (not display or display == "") and (not wayland or wayland == "") then
      return false, "no_display"
    end
  end
  return true, nil
end

function M.open_file(path, opts)
  opts = opts or {}
  local can_open, reason = M.can_open_browser(opts.env)
  if not can_open then
    return { status = "skipped_headless", reason = reason, err = nil }
  end
  local opener = opts.open_fn or vim.ui.open
  if type(opener) ~= "function" then
    return { status = "open_failed", err = "vim.ui.open unavailable" }
  end
  local ok, sys_obj, open_err = pcall(opener, path)
  if not ok then
    return { status = "open_failed", err = tostring(sys_obj) }
  end
  if sys_obj == nil and open_err then
    return { status = "open_failed", err = tostring(open_err) }
  end
  return { status = "opened", err = nil }
end

function M.stats(opts)
  opts = opts or {}
  local dir = opts.cache_dir or M.default_cache_dir()
  local st = uv.fs_lstat(dir)
  local can_open, open_reason = M.can_open_browser(opts.env)
  local res = {
    dir = dir,
    exists = st ~= nil,
    is_symlink = st ~= nil and st.type == "link",
    is_dir = st ~= nil and st.type == "directory",
    mode_ok = true,
    mode_octal = nil,
    file_count = 0,
    total_bytes = 0,
    can_open = can_open,
    open_reason = open_reason,
  }
  if not st or st.type ~= "directory" then
    return res
  end
  if not M.is_windows() and type(st.mode) == "number" then
    local perm = st.mode % 512
    res.mode_ok = (perm == MODE_0700)
    res.mode_octal = string.format("%04o", perm)
  end
  local scanner = uv.fs_scandir(dir)
  if scanner then
    while true do
      local name = uv.fs_scandir_next(scanner)
      if not name then
        break
      end
      if name:match("^lineage%-.*%.html$") then
        local est = uv.fs_lstat(dir .. "/" .. name)
        if est and est.type == "file" then
          res.file_count = res.file_count + 1
          res.total_bytes = res.total_bytes + (est.size or 0)
        end
      end
    end
  end
  return res
end

return M

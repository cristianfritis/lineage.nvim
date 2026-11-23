local M = {}

local is_011 = vim.fn.has("nvim-0.11") == 1

local METHOD_MAP = {
  definition = "textDocument/definition",
  typeDefinition = "textDocument/typeDefinition",
  implementation = "textDocument/implementation",
  references = "textDocument/references",
  callHierarchyIncoming = "textDocument/prepareCallHierarchy",
}

local active_lsp_requests = {}

function M.is_nvim_011()
  return is_011
end

function M.get_clients(bufnr)
  return vim.lsp.get_clients({ bufnr = bufnr })
end

function M.client_supports(client, method)
  if is_011 then
    return client:supports_method(method)
  end
  return client.supports_method(method)
end

function M.client_request(client, method, params, handler, bufnr)
  if is_011 then
    return client:request(method, params, handler, bufnr)
  end
  return client.request(method, params, handler, bufnr)
end

function M.cancel_client_request(client, req_id)
  if not req_id then
    return
  end
  if is_011 then
    pcall(function()
      client:cancel_request(req_id)
    end)
  else
    pcall(function()
      client.cancel_request(req_id)
    end)
  end
end

function M.cancel_query(query_id)
  local list = active_lsp_requests[query_id]
  if not list then
    return
  end
  for _, entry in ipairs(list) do
    M.cancel_client_request(entry.client, entry.req_id)
  end
  active_lsp_requests[query_id] = nil
end

local function normalize_locations(result)
  local out = {}
  if not result then
    return out
  end
  local items = vim.islist(result) and result or { result }
  for _, loc in ipairs(items) do
    local uri = loc.uri or loc.targetUri
    local range = loc.range or loc.targetSelectionRange or loc.targetRange
    if uri and range and range.start and range["end"] then
      table.insert(out, {
        uri = uri,
        start = { line = range.start.line, character = range.start.character },
        ["end"] = { line = range["end"].line, character = range["end"].character },
      })
    end
  end
  return out
end

local function normalize_call_hierarchy_incoming(result)
  local out = {}
  if not result or not vim.islist(result) then
    return out
  end
  for _, entry in ipairs(result) do
    local from = entry.from
    if from and from.uri then
      local range = (entry.fromRanges and entry.fromRanges[1])
        or from.selectionRange
        or from.range
      if range and range.start and range["end"] then
        table.insert(out, {
          uri = from.uri,
          start = { line = range.start.line, character = range.start.character },
          ["end"] = { line = range["end"].line, character = range["end"].character },
        })
      end
    end
  end
  return out
end

function M.pick_client(bufnr, lsp_method, circuit_open_clients)
  local clients = M.get_clients(bufnr)
  for _, client in ipairs(clients) do
    local broken = circuit_open_clients and circuit_open_clients[client.name]
    if not broken and M.client_supports(client, lsp_method) then
      return client
    end
  end
  return nil
end

function M.resolve_batch(params, circuit_open_clients, callback)
  local items = params.items or {}
  if #items == 0 then
    callback({ results = {} })
    return
  end

  local query_id = params.query_id
  local timeout_ms = params.timeout_ms or 150
  local pending = #items
  local results = {}
  local finished = false

  active_lsp_requests[query_id] = active_lsp_requests[query_id] or {}

  local timer = vim.uv.new_timer()
  timer:start(timeout_ms, 0, function()
    timer:stop()
    timer:close()
    vim.schedule(function()
      if finished then
        return
      end
      finished = true
      for i, item in ipairs(items) do
        if not results[i] then
          results[i] = {
            key = item.key,
            locations = {},
            timed_out = true,
            unattached = false,
            circuit_open = false,
            latency_ms = timeout_ms,
          }
        end
      end
      M.cancel_query(query_id)
      callback({ results = results })
    end)
  end)

  local function finish_one()
    pending = pending - 1
    if pending == 0 and not finished then
      finished = true
      if not timer:is_closing() then
        timer:stop()
        timer:close()
      end
      active_lsp_requests[query_id] = nil
      callback({ results = results })
    end
  end

  local uv = vim.uv or vim.loop

  local function record_result(idx, item, client, locs, elapsed_ms)
    results[idx] = {
      key = item.key,
      locations = locs,
      timed_out = false,
      unattached = false,
      circuit_open = false,
      client_name = client and client.name or nil,
      offset_encoding = (client and client.offset_encoding) or "utf-16",
      latency_ms = elapsed_ms,
    }
    finish_one()
  end

  local function send_references(idx, item, bufnr, t0)
    local ref_client = M.pick_client(bufnr, "textDocument/references", circuit_open_clients)
    if not ref_client then
      results[idx] = {
        key = item.key,
        locations = {},
        timed_out = false,
        unattached = true,
        circuit_open = false,
        latency_ms = 0,
      }
      finish_one()
      return
    end
    local ref_params = {
      textDocument = { uri = item.uri },
      position = { line = item.position.line, character = item.position.character },
      context = { includeDeclaration = false },
    }
    local ok, req_id = M.client_request(ref_client, "textDocument/references", ref_params, function(err, res)
      if finished then
        return
      end
      local elapsed_ms = math.floor((uv.hrtime() - t0) / 1e6)
      local locs = (not err and res) and normalize_locations(res) or {}
      record_result(idx, item, ref_client, locs, elapsed_ms)
    end, bufnr)
    if ok and req_id and active_lsp_requests[query_id] then
      table.insert(active_lsp_requests[query_id], { client = ref_client, req_id = req_id })
    end
  end

  for idx, item in ipairs(items) do
    local bufnr = vim.uri_to_bufnr(item.uri)
    local t0 = uv.hrtime()

    if item.kind == "callHierarchyIncoming" then
      local ch_client = M.pick_client(bufnr, "textDocument/prepareCallHierarchy", circuit_open_clients)
      if not ch_client then
        send_references(idx, item, bufnr, t0)
      else
        local prep_params = {
          textDocument = { uri = item.uri },
          position = { line = item.position.line, character = item.position.character },
        }
        local ok, req_id = M.client_request(ch_client, "textDocument/prepareCallHierarchy", prep_params, function(err, prep_res)
          if finished then
            return
          end
          local ch_items = (not err and prep_res and vim.islist(prep_res)) and prep_res or {}
          if #ch_items == 0 then
            send_references(idx, item, bufnr, t0)
            return
          end
          local ok2, req_id2 = M.client_request(ch_client, "callHierarchy/incomingCalls", { item = ch_items[1] }, function(err2, inc_res)
            if finished then
              return
            end
            local locs = (not err2 and inc_res) and normalize_call_hierarchy_incoming(inc_res) or {}
            if #locs == 0 then
              send_references(idx, item, bufnr, t0)
              return
            end
            local elapsed_ms = math.floor((uv.hrtime() - t0) / 1e6)
            record_result(idx, item, ch_client, locs, elapsed_ms)
          end, bufnr)
          if ok2 and req_id2 and active_lsp_requests[query_id] then
            table.insert(active_lsp_requests[query_id], { client = ch_client, req_id = req_id2 })
          end
        end, bufnr)
        if ok and req_id and active_lsp_requests[query_id] then
          table.insert(active_lsp_requests[query_id], { client = ch_client, req_id = req_id })
        end
      end
    else
      local lsp_method = METHOD_MAP[item.kind] or "textDocument/definition"
      local client = M.pick_client(bufnr, lsp_method, circuit_open_clients)
      if not client then
        results[idx] = {
          key = item.key,
          locations = {},
          timed_out = false,
          unattached = true,
          circuit_open = false,
          latency_ms = 0,
        }
        finish_one()
      else
        local req_params = {
          textDocument = { uri = item.uri },
          position = { line = item.position.line, character = item.position.character },
        }
        if item.kind == "references" then
          req_params.context = { includeDeclaration = false }
        end
        local ok, req_id = M.client_request(client, lsp_method, req_params, function(err, res)
          if finished then
            return
          end
          local elapsed_ms = math.floor((uv.hrtime() - t0) / 1e6)
          local locs = (not err and res) and normalize_locations(res) or {}
          record_result(idx, item, client, locs, elapsed_ms)
        end, bufnr)
        if ok and req_id and active_lsp_requests[query_id] then
          table.insert(active_lsp_requests[query_id], { client = client, req_id = req_id })
        end
      end
    end
  end
end

return M

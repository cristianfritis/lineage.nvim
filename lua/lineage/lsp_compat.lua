local M = {}

local is_011 = vim.fn.has("nvim-0.11") == 1

local METHOD_MAP = {
  definition = "textDocument/definition",
  typeDefinition = "textDocument/typeDefinition",
  implementation = "textDocument/implementation",
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
  for idx, item in ipairs(items) do
    local bufnr = vim.uri_to_bufnr(item.uri)
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
      local t0 = uv.hrtime()
      local req_params = {
        textDocument = { uri = item.uri },
        position = { line = item.position.line, character = item.position.character },
      }
      local ok, req_id = M.client_request(client, lsp_method, req_params, function(err, res)
        if finished then
          return
        end
        local elapsed_ms = math.floor((uv.hrtime() - t0) / 1e6)
        if err then
          results[idx] = {
            key = item.key,
            locations = {},
            timed_out = false,
            unattached = false,
            circuit_open = false,
            client_name = client.name,
            offset_encoding = client.offset_encoding or "utf-16",
            latency_ms = elapsed_ms,
          }
        else
          results[idx] = {
            key = item.key,
            locations = normalize_locations(res),
            timed_out = false,
            unattached = false,
            circuit_open = false,
            client_name = client.name,
            offset_encoding = client.offset_encoding or "utf-16",
            latency_ms = elapsed_ms,
          }
        end
        finish_one()
      end, bufnr)
      if ok and req_id then
        table.insert(active_lsp_requests[query_id], { client = client, req_id = req_id })
      end
    end
  end
end

return M

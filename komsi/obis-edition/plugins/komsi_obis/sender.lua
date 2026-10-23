-- OBIS 송신 (K-OMSI OBIS 판): OBIS Client(v1.1.0, openOMSI 호환판)가 /api/ingest 로 보내는
-- snapshot 을 게임 안에서 같은 모양으로 만듦. 보내기는 omsi.obis(토큰은 K-OMSI 가 쥠, 플러그인은 모름).
-- 원본 대비: bridge/state.py AppState.snapshot + _route_key_for_trip_file + _update_obis_first_stop_gate_locked,
--           bridge/route_projection.position_on_route, 호환판 omsi_memory._route_frame_xy
-- 노선 데이터 = obis_routes/ (tools/build_obis_routes.py 가 OBIS Client data/routes 에서 만듦)

local M = {}

local NULL = {}                -- JSON null 표시
local idx = nil                -- obis_routes/index.json
local route_cache = {}         -- routeKey → { raw = 원문, t = 표, xy = {id → {x,y}} }

-- ------------------------------------------------------------ JSON (키 순서·null 유지, route 는 원문 그대로)
local function enc(v)
  if v == NULL or v == nil then return "null" end
  local t = type(v)
  if t == "table" then
    if v.__raw then return v.__raw end
    if v.__keys then
      local parts = {}
      for _, k in ipairs(v.__keys) do parts[#parts + 1] = omsi.json.encode(k) .. ":" .. enc(v[k]) end
      return "{" .. table.concat(parts, ",") .. "}"
    end
    local parts = {}
    for _, x in ipairs(v) do parts[#parts + 1] = enc(x) end
    return "[" .. table.concat(parts, ",") .. "]"
  end
  return omsi.json.encode(v)
end

local function obj(keys, vals)
  vals.__keys = keys
  return vals
end

-- ------------------------------------------------------------ 노선 자료
local function norm_trip(name)
  local s = tostring(name or ""):gsub("^%s+", ""):gsub("%s+$", ""):gsub("\\", "/")
  s = (s:match("([^/]*)$") or s):lower()
  if s:sub(-4) == ".ttp" then s = s:sub(1, -5) end
  return s
end

local function load_index()
  if idx then return idx end
  local text = omsi.plugin.read("obis_routes/index.json")
  idx = text and omsi.json.decode(text) or { routes = {}, trips = {} }
  return idx
end

local function route(key)
  if not key then return nil end
  local r = route_cache[key]
  if r then return r end
  local info = load_index().routes[key]
  if not info then return nil end
  local raw = omsi.plugin.read("obis_routes/" .. info.file)
  if not raw then return nil end
  local t = omsi.json.decode(raw)
  if type(t) ~= "table" then return nil end
  r = { raw = raw:gsub("^%s+", ""):gsub("%s+$", ""), t = t, xy = {} }
  for _, s in ipairs(t.stations or {}) do
    local id = math.floor(tonumber(s.id) or 0)
    if not r.xy[id] then r.xy[id] = { tonumber(s.x) or 0, tonumber(s.y) or 0 } end
  end
  route_cache[key] = r
  return r
end

local function has_stop(key, sid)
  local info = load_index().routes[key]
  if not info then return false end
  for _, id in ipairs(info.stops) do if id == sid then return true end end
  return false
end

local function stop_order(key, sid)
  local info = load_index().routes[key]
  for i, id in ipairs(info and info.stops or {}) do if id == sid then return i - 1 end end
  return nil
end

-- _route_key_for_trip_file: trip 파일명 일치가 최종 권위, 여러 맵에 같은 이름이면 다음 정류장 id 로 좁힘
local function key_for_trip(trip, sid)
  local cands = load_index().trips[norm_trip(trip)] or {}
  if #cands == 1 then return cands[1] end
  if #cands > 1 and sid and sid ~= 0 then
    local narrowed = {}
    for _, k in ipairs(cands) do if has_stop(k, sid) then narrowed[#narrowed + 1] = k end end
    if #narrowed == 1 then return narrowed[1] end
  end
  return nil
end

-- 위치로 노선 정하기(폴백): 노선번호 일치 → 다음 정류장 순번이 가장 맞는 노선
local function base_line(s) return (tostring(s or ""):gsub("_[abc]$", "")) end

local function key_by_position(line, sid, sidx)
  if not sid or sid == 0 then return nil end
  local best, bestd = nil, math.huge
  for key, info in pairs(load_index().routes) do
    if base_line(info.line) == base_line(line) and has_stop(key, sid) then
      local d = math.abs((stop_order(key, sid) or 0) - (sidx or 0))
      if d < bestd or (d == bestd and best and key < best) then best, bestd = key, d end
    end
  end
  return best
end

-- position_on_route: 노선 꺾은선 위 가장 가까운 점 → (진행률 0..1, 구간 번호, 거리)
local function position_on_route(stations, x, y)
  local pts = {}
  for _, s in ipairs(stations) do pts[#pts + 1] = { tonumber(s.x) or 0, tonumber(s.y) or 0 } end
  if #pts == 0 then return 0.0, 0, 0.0 end
  if #pts == 1 then return 0.0, 0, math.sqrt((x - pts[1][1]) ^ 2 + (y - pts[1][2]) ^ 2) end
  local lens, total = {}, 0
  for i = 1, #pts - 1 do
    lens[i] = math.sqrt((pts[i + 1][1] - pts[i][1]) ^ 2 + (pts[i + 1][2] - pts[i][2]) ^ 2)
    total = total + lens[i]
  end
  if total <= 0 then total = 1.0 end
  local best_d, best_seg, best_along, trav = math.huge, 0, 0.0, 0.0
  for i, len in ipairs(lens) do
    local ax, ay, bx, by = pts[i][1], pts[i][2], pts[i + 1][1], pts[i + 1][2]
    if len < 1e-6 then
      trav = trav + len
    else
      local t = math.max(0.0, math.min(1.0, ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / (len * len)))
      local px, py = ax + (bx - ax) * t, ay + (by - ay) * t
      local d = math.sqrt((x - px) ^ 2 + (y - py) ^ 2)
      if d < best_d then best_d, best_seg, best_along = d, i - 1, trav + t * len end
      trav = trav + len
    end
  end
  return math.max(0.0, math.min(1.0, best_along / total)), best_seg, best_d
end

-- ------------------------------------------------------------ 상태 (AppState 와 같은 이름)
local st = {
  active = false, started_at = nil, inactive_at = nil, phase = "idle", key = nil,
  gate_key = nil, gate_origin = nil, gate_min = nil, gate_departed = false,
}

local function gate_reset()
  st.gate_key, st.gate_origin, st.gate_min, st.gate_departed = nil, nil, nil, false
end

-- _update_obis_first_stop_gate_locked: 첫 정류장 5m 안 방문 → 40m 이상(최소보다 15m 더) 떠나면 출발
local function gate_update(r, lp)
  if not st.active or st.phase ~= "running" or not r then gate_reset(); return end
  if st.key ~= st.gate_key then gate_reset(); st.gate_key = st.key end
  local stations = r.t.stations or {}
  if #stations == 0 then return end
  local first = math.floor(tonumber(stations[1].id) or 0)
  local later = {}
  for i = 2, #stations do later[math.floor(tonumber(stations[i].id) or 0)] = true end
  local wanted = lp and lp.schedNextStopId or 0
  if wanted ~= 0 and wanted ~= first and not later[wanted] then return end
  if wanted ~= 0 and later[wanted] and wanted ~= first and st.gate_origin then
    st.gate_departed = true
    return
  end
  local dist = lp and lp.schedNextStopDist
  if type(dist) ~= "number" or dist < 0 or dist >= 999999 then return end
  if wanted == first then
    if st.gate_min == nil or dist < st.gate_min then st.gate_min = dist end
    if st.gate_origin == nil and dist <= 5.0 then st.gate_origin = wanted end
  end
  if not st.gate_departed and st.gate_origin and st.gate_min and st.gate_min <= 5.0
      and dist >= 40.0 and dist - st.gate_min > 15.0 then
    st.gate_departed = true
  end
end

-- ------------------------------------------------------------ 한 번의 snapshot
-- game: { duty, stops, x, y, z, passengers, next_dist, has_bus }
function M.snapshot(g)
  local now = omsi.util.now()
  local duty = g.duty
  local lp, r, vehicles = nil, nil, {}

  if duty and duty.next_stop then
    if not st.active then st.active, st.started_at, st.inactive_at, st.phase = true, now, nil, "running" end
    local ns, ps = duty.next_stop, duty.previous_stop
    local sid = math.floor(tonumber(ns.id) or 0)
    local sidx = math.max(0, (ns.number or 1) - 1)
    local key = key_for_trip(duty.trip_name, sid) or key_by_position(duty.line, sid, sidx)
    if key ~= st.key then st.key = key end
    r = route(key)
    -- 노선 화면 좌표: 이전·다음 정류장 사이를 남은 거리 비율만큼 (호환판 _route_frame_xy, 단 이 노선의 정류장 좌표로)
    local x, y = g.x, g.y
    if r then
      local n = r.xy[sid]
      if n then
        x, y = n[1], n[2]
        local pid = ps and math.floor(tonumber(ps.id) or 0) or 0
        local q = r.xy[pid]
        -- (원본처럼 정차 중이어도 남은 거리로 보간. 단 정류장 좌표는 이 노선 것: 원본 호환판은 모든 노선을
        --  합친 표에서 먼저 나온 것을 써서, 여러 노선이 같이 쓰는 정류장에서 엉뚱한 위치가 나왔음)
        if q and ps and ps.x and ns.x then
          local seg = math.sqrt((ns.x - ps.x) ^ 2 + (ns.y - ps.y) ^ 2)
          if seg >= 1.0 then
            local f = math.max(0.0, math.min(1.0, 1.0 - g.next_dist / seg))
            x, y = q[1] + (n[1] - q[1]) * f, q[2] + (n[2] - q[2]) * f
          end
        end
      end
    end
    local pid = ps and math.floor(tonumber(ps.id) or 0) or 0
    lp = obj({ "posTs", "x", "y", "z", "rawAbs", "kachel", "schedValid", "schedLine", "schedTour", "schedTrip",
      "schedNextStopId", "schedNextStopDist", "schedNextStopDistRaw", "schedNextStopIndex", "schedNextBusstopPtr",
      "schedPrevBusstopPtr", "schedNextStopState", "schedAtStation", "schedActiveVar", "humansCount",
      "schedLineNumber", "schedTripFile" }, {
      posTs = now, x = x, y = y, z = g.z or 0.0, rawAbs = { x, g.z or 0.0, y }, kachel = 0,
      schedValid = true, schedLine = 0, schedTour = math.floor(tonumber(duty.tour) or 0),
      schedTrip = math.max(0, (tonumber(duty.trip) or 1) - 1),
      schedNextStopId = sid, schedNextStopDist = g.next_dist, schedNextStopDistRaw = g.next_dist,
      schedNextStopIndex = sidx, schedNextBusstopPtr = sid ~= 0 and 1 or 0, schedPrevBusstopPtr = pid ~= 0 and 1 or 0,
      schedNextStopState = duty.at_stop and 1 or 2, schedAtStation = duty.at_stop and 1.0 or 0.0, schedActiveVar = 1.0,
      humansCount = (g.passengers or 0) + 0.0,
      schedLineNumber = tostring(duty.line or "") ~= "" and tostring(duty.line) or NULL,
      schedTripFile = tostring(duty.trip_name or "") ~= "" and tostring(duty.trip_name) or NULL,
    })
    if r then
      local prog, seg = position_on_route(r.t.stations or {}, x, y)
      vehicles[1] = obj({ "id", "label", "progress", "segmentIndex", "x", "y", "source" }, {
        id = "me", label = "내 버스", progress = math.floor(prog * 10000 + 0.5) / 10000, segmentIndex = seg,
        x = x, y = y, source = "memory" })
    end
  else
    if st.active then st.active, st.inactive_at, st.phase = false, now, "idle" end
    st.key = nil
  end
  gate_update(r, lp)

  local snap = obj({ "mapId", "routeKey", "route", "vehicles", "vehicleCount", "connectedOmsi", "lastError",
    "positionError", "positionHint", "timetableActive", "timetableInactiveAt", "timetableStartedAt", "tripPhase",
    "tripCancelled", "lastPosition", "obisFirstStopGateRouteKey", "obisFirstStopOriginId", "obisFirstStopVisited",
    "obisFirstStopDeparted" }, {
    mapId = load_index().default_map or NULL,
    routeKey = st.key or NULL,
    route = r and { __raw = r.raw } or NULL,
    vehicles = vehicles, vehicleCount = #vehicles,
    connectedOmsi = g.has_bus and true or false,
    lastError = NULL, positionError = NULL, positionHint = NULL,
    timetableActive = st.active, timetableInactiveAt = st.inactive_at or NULL, timetableStartedAt = st.started_at or NULL,
    tripPhase = st.phase, tripCancelled = st.phase == "cancelled",
    lastPosition = lp or NULL,
    obisFirstStopGateRouteKey = st.gate_key or NULL, obisFirstStopOriginId = st.gate_origin or NULL,
    obisFirstStopVisited = st.gate_min ~= nil and st.gate_min <= 5.0, obisFirstStopDeparted = st.gate_departed,
  })
  return enc(snap), st.key
end

M.NULL = NULL
M.position_on_route = position_on_route
M.key_for_trip = key_for_trip
return M

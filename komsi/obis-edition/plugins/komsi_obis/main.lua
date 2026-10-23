-- K-OMSI OBIS 플러그인: OBIS Client 창을 게임 안으로
-- · 오른쪽 위 패널: 서버 연결·운행 상태·이번 정류장·내 버스 표시·접속자 목록
--   OBIS 판 K-OMSI(omsi.obis)면 게임이 직접 내 버스를 보냄(sender.lua, OBIS Client 불필요),
--   "송신 중"은 보내는 연결이 열려 있고 OBIS 서버 실시간 피드에 내 버스가 보일 때 켜짐
-- · 내비게이터 보드(노선 띠, 지도 (가)안): 내 trip 정류장 세로 레일 위에 OBIS 유저·AI 시간표 버스 위치
--   위치 = 다음 정류장 + 남은 거리 비율 (좌표계가 달라도 같은 방식, project_omsi_bis 와 동일)
-- 필요: omsi.ws (K-OMSI patch/lua-websocket). 없으면 AI 버스만 보임.
-- 단축키: Ctrl+Shift+O 패널, Ctrl+Shift+B 노선 띠

local OBIS_URL = "wss://obis.kr/api/ws"
local PANEL, BOARD = "obis", "line"
local GREEN, GREY, RED, AMBER, BLUE = "#34B26E", "#5F6368", "#E5392F", "#F2A33A", "#3D7BF5"
local MAX_USERS = 8

local settings = omsi.plugin.settings({
  { key = "show_ai", type = "bool", label = "노선 띠에 AI 시간표 버스도 표시", default = true },
  { key = "board", type = "bool", label = "노선 띠 켜기", default = true },
  { key = "send", type = "bool", label = "게임이 직접 내 버스를 OBIS 로 보냄 (켜면 OBIS Client 는 끄세요)", default = true },
  { key = "in_nav", type = "bool", label = "노선 띠를 내비게이터 안에 (끄면 왼쪽 패널)", default = false },
}, "OBIS")

local has_ws = omsi.ws ~= nil
local has_send = omsi.obis ~= nil      -- K-OMSI OBIS 판
local sender = has_send and require("sender") or nil
local send_on = false
local shown = true
local log = {}                 -- 활동 로그 (최근 것 위)

-- ------------------------------------------------------------ 한글 정류장명 (eb_nearby 와 같은 규칙)
local ko, obis_maps, obis_use, obis_base, obis_picked, obis_ko = {}, {}, {}, {}, "", {}

local function load_json(path)
  local text = omsi.plugin.read(path)
  if not text then return nil end
  local v = omsi.json.decode(text)
  return type(v) == "table" and v or nil
end

local function has_hangul(s)
  if type(s) ~= "string" then return false end
  local ok, res = pcall(function()
    for _, c in utf8.codes(s, true) do if c >= 0xAC00 and c <= 0xD7A3 then return true end end
    return false
  end)
  return ok and res
end

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

local function base_name(n)
  n = trim(n)
  n = n:gsub("_Start_?R?%d*$", ""):gsub("_End_?R?%d*$", ""):gsub("_R?%d*$", "")
  local r, c = n:gsub("%s+%u$", "")
  if c == 0 then r, c = n:gsub("%s+%u?%d%d?%d?%d?$", "") end
  if c == 0 then r, c = n:gsub("%s+[NSEW][NSEW]?$", "") end
  return r:lower()
end

local function pick_obis_names(names, trip)
  if obis_picked == trip then return end
  obis_picked = trip
  obis_use, obis_base = {}, {}
  local ranked, top = {}, 0
  for _, m in pairs(obis_maps) do
    local hit = 0
    for _, n in ipairs(names) do if m[n] then hit = hit + 1 end end
    if hit > 0 then ranked[#ranked + 1] = { m = m, n = hit }; if hit > top then top = hit end end
  end
  for _, r in ipairs(ranked) do
    if r.n >= math.min(30, top) then for k, v in pairs(r.m) do obis_use[k] = v end end
  end
  for k, v in pairs(obis_use) do
    local b = base_name(k)
    if not obis_base[b] then obis_base[b] = v end
  end
end

local function stop_name(id, name)
  name = name or ""
  local k = ko[name]
  if k and k:match("%S") then return k end
  if obis_ko[id] then return obis_ko[id] end
  return obis_use[trim(name)] or obis_base[base_name(name)] or name
end

-- ------------------------------------------------------------ OBIS 피드
local buses = {}               -- 최근 스냅샷의 버스
local msgs, last_msg_ms = 0, nil
local my_id, my_seen = "", nil

local function truthy(v) return v == true or (type(v) == "number" and v >= 0.5) end

local function add_log(text)
  table.insert(log, 1, os.date("%H:%M") .. "  " .. text)
  while #log > 4 do table.remove(log) end
end

local function take(text)
  local doc = omsi.json.decode(text)
  if type(doc) ~= "table" or type(doc.buses) ~= "table" then return end
  local list = {}
  for _, b in ipairs(doc.buses) do
    local lp = b.lastPosition
    if type(lp) == "table" then
      local prefs = type(b.prefs) == "table" and b.prefs or {}
      local rt = type(b.route) == "table" and b.route or {}
      list[#list + 1] = {
        id = tostring(b.id or ""),
        nick = trim(prefs.nickname or ""),
        line = trim(rt.lineNumber or rt.name or b.routeKey or ""),
        trip = trim(lp.schedTripFile or ""):lower(),
        next_id = math.floor(tonumber(lp.schedNextStopId) or 0),
        dist = tonumber(lp.schedNextStopDist) or 0,
        at = truthy(lp.schedAtStation),
        age = tonumber(b.ageMs) or 0,
        got = omsi.util.ms(),
      }
      if type(rt.stations) == "table" then
        for _, s in ipairs(rt.stations) do
          if has_hangul(s.name_ko) then
            obis_ko[math.floor(tonumber(s.id) or 0)] = s.name_ko
            list[#list].names = list[#list].names or {}
            list[#list].names[math.floor(tonumber(s.id) or 0)] = s.name_ko
          end
        end
      end
    end
  end
  buses = list
end

local function fresh(b) return b.age + (omsi.util.ms() - b.got) <= 60000 end

-- ------------------------------------------------------------ 내 운행·위치
local function pos_of(stops, nxt, dist, at)       -- nxt: 1부터, 결과 0 = 첫 정류장
  nxt = math.max(1, math.min(nxt, #stops))
  if at or nxt == 1 then return nxt - 1 end
  local a, b = stops[nxt], stops[nxt - 1]
  if not (a.x and b.x) then return nxt - 1.5 end
  local seg = math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
  local frac = seg > 1 and math.max(0, math.min(1, 1 - dist / seg)) or 1
  return nxt - 2 + frac
end

local function my_state()
  local duty = omsi.duty.get()
  if not duty or not duty.next_stop then return nil end
  local stops = omsi.duty.stops()
  if type(stops) ~= "table" or #stops < 2 then return nil end
  local names = {}
  for i, s in ipairs(stops) do names[i] = s.name end
  pick_obis_names(names, duty.trip_name or "")
  local bx, by = omsi.position()
  local ns = duty.next_stop
  local dist = (bx and ns.x) and math.sqrt((bx - ns.x) ^ 2 + (by - ns.y) ^ 2) or 0
  return { duty = duty, stops = stops, trip = trim(duty.trip_name):lower(), next = ns.number, next_id = ns.id,
    dist = dist, at = duty.at_stop, p = pos_of(stops, ns.number, dist, duty.at_stop) }
end

-- 피드에서 내 버스 찾기: 같은 trip·같은 다음 정류장·거리 차 5m 이내 (eb_nearby 와 같음)
local function find_me(me)
  local now = omsi.util.ms()
  if me then
    for _, b in ipairs(buses) do
      if fresh(b) and ((my_id ~= "" and b.id == my_id)
          or (b.trip == me.trip and b.next_id == me.next_id and math.abs(b.dist - me.dist) < 5)) then
        if b.id ~= my_id then my_id = b.id; add_log("서버에서 내 버스 확인 (" .. (b.nick ~= "" and b.nick or b.id) .. ")") end
        my_seen = now
        return b
      end
    end
  end
  if my_id ~= "" and my_seen and now - my_seen > 15000 then my_id = ""; add_log("서버에 내 버스가 안 보임") end
  return nil
end

-- ------------------------------------------------------------ 패널
local function chip(id, text) return { type = "badge", id = id, text = text, color = "#2A2D33" } end
local panel_hint = nil         -- 지금 패널에 보이는 안내 (바뀌면 패널을 다시 만듦)

local function build_panel(hint)
  panel_hint = hint
  omsi.ui.set(PANEL, {
    anchor = "top_right", x = 16, y = 80, width = 380, draggable = true, grab = true, gap = 8,
    children = {
      { type = "row", children = {
        { type = "icon", name = "directions_bus", size = 26, color = BLUE },
        { type = "text", text = "OBIS", size = 18, weight = "bold", wrap = false },
        { type = "text", text = "실시간 위치", size = 12, color = "#9AA0A6", grow = true, wrap = false },
        { type = "badge", id = "state", text = "연결 중", color = GREY },
      } },
      { type = "row", gap = 6, children = { chip("c_srv", "서버 obis.kr"), chip("c_omsi", "OMSI 대기"), chip("c_users", "접속 0명") } },
      { type = "box", color = BLUE, gap = 4, children = {
        { type = "row", children = {
          { type = "badge", id = "line", text = "-", color = BLUE, weight = "bold" },
          { type = "text", id = "run", text = "대기 중", weight = "bold", size = 15, grow = true, wrap = false },
          { type = "text", id = "term", text = "운행 정보 없음", size = 12, color = "#9AA0A6", align = "right", wrap = false },
        } },
        { type = "row", children = {
          { type = "icon", name = "location_on", size = 18, color = BLUE },
          { type = "text", id = "stop", text = "이번 정류장  -", grow = true, wrap = false },
          { type = "text", id = "stop_t", text = "-", color = "#9AA0A6", wrap = false },
        } },
      } },
      { type = "row", gap = 6, children = {
        { type = "text", text = "내 버스 표시", size = 12, color = "#9AA0A6" },
        { type = "badge", id = "nick", text = "-", color = "#2A2D33", text_color = "#D0D4DA" },
        { type = "badge", id = "num", text = "-", color = "#2A2D33", text_color = "#D0D4DA" },
      } },
      { type = "box", color = AMBER, visible = hint ~= nil, children = {
        { type = "text", size = 12, text = hint or "" },
      } },
      { type = "table", id = "users", columns = { "닉네임", "노선", "다음 정류장" }, widths = { 0.28, 0.22, 0.5 }, size = 12, rows = {} },
      { type = "text", id = "log", text = "", size = 11, color = "#9AA0A6" },
      { type = "row", gap = 6, children = {
        { type = "button", id = "b_board", text = "노선 띠", icon = "route", grow = true },
        { type = "button", id = "b_set", icon = "settings" },
        { type = "button", id = "b_hide", icon = "close" },
      } },
    },
  })
end

local function refresh_panel(me, mine)
  local ws = has_ws and omsi.ws.state() or "none"
  local sst = send_on and omsi.obis.state() or nil
  local n = 0
  for _, b in ipairs(buses) do if fresh(b) then n = n + 1 end end
  local state, scol
  if not has_ws then state, scol = "OBIS 꺼짐", GREY
  elseif ws ~= "open" then state, scol = ws == "connecting" and "연결 중" or "재연결 대기", AMBER
  elseif mine then state, scol = "송신 중", GREEN
  elseif sst == "open" then state, scol = "송신 연결됨", BLUE
  else state, scol = "수신만", GREY end
  -- 안내: 운행 중인데 서버에 안 보일 때
  local hint
  if not has_ws then hint = "이 게임 버전엔 OBIS 연결이 없습니다. K-OMSI OBIS 판이 필요해요."
  elseif sst == "no_token" then hint = "OBIS 기기 등록에 실패했어요. 잠시 뒤 다시 시도합니다."
  elseif me and ws == "open" and not mine then
    hint = send_on and "보내는 중인데 서버에 아직 안 보여요. 노선이 OBIS 에 없는 노선일 수 있어요."
      or "서버에 내 버스가 안 보여요. 설정에서 '직접 보냄'을 켜거나 OBIS Client 를 켜 두세요."
  end
  if hint ~= panel_hint then build_panel(hint) end

  omsi.ui.update(PANEL, "state", { text = state, color = scol })
  omsi.ui.update(PANEL, "c_srv", { color = ws == "open" and "#1F5136" or "#2A2D33" })
  omsi.ui.update(PANEL, "c_omsi", { text = me and "운행 중" or "OMSI 대기", color = me and "#1F5136" or "#2A2D33" })
  omsi.ui.update(PANEL, "c_users", { text = "접속 " .. n .. "명" })

  if me then
    local d = me.duty
    local s = me.stops[math.max(1, math.min(me.next, #me.stops))]
    omsi.ui.update(PANEL, "line", { text = tostring(d.line or "-") })
    omsi.ui.update(PANEL, "run", { text = me.at and "정차 중" or "운행 중" })
    omsi.ui.update(PANEL, "term", { text = (d.terminus or "") ~= "" and (stop_name(nil, d.terminus) .. " 행") or "" })
    omsi.ui.update(PANEL, "stop", { text = "이번 정류장  " .. stop_name(s.id, s.name) })
    omsi.ui.update(PANEL, "stop_t", { text = s.arrival and omsi.fmt.clock(s.arrival) or "-" })
  else
    omsi.ui.update(PANEL, "line", { text = "-" })
    omsi.ui.update(PANEL, "run", { text = "대기 중" })
    omsi.ui.update(PANEL, "term", { text = "운행 정보 없음" })
    omsi.ui.update(PANEL, "stop", { text = "이번 정류장  -" })
    omsi.ui.update(PANEL, "stop_t", { text = "-" })
  end
  omsi.ui.update(PANEL, "nick", { text = mine and mine.nick ~= "" and mine.nick or "-" })
  omsi.ui.update(PANEL, "num", { text = omsi.bus.number() or "-" })

  local rows = {}
  for _, b in ipairs(buses) do
    if fresh(b) and #rows < MAX_USERS then
      local nm = b.names and b.names[b.next_id] or obis_ko[b.next_id] or tostring(b.next_id)
      local me_row = b.id == my_id
      rows[#rows + 1] = {
        { text = (b.nick ~= "" and b.nick or "-") .. (me_row and " (나)" or ""), color = me_row and GREEN or nil },
        b.line ~= "" and b.line or "-", nm,
      }
    end
  end
  if #rows == 0 then rows[1] = { "-", "-", "접속한 유저 없음" } end
  omsi.ui.update(PANEL, "users", { rows = rows })
  omsi.ui.update(PANEL, "log", { text = table.concat(log, "\n") })
end

-- ------------------------------------------------------------ 노선 띠 (내비게이터 보드)
local function short(nick) local d = tostring(nick or ""):gsub("%D", ""); return #d > 4 and d:sub(-4) or (d ~= "" and d or tostring(nick or "")) end

local function board_frame(children)
  if settings.in_nav then
    return { anchor = "navigator", padding = 10, gap = 6, children = children }
  end
  return { anchor = "left", x = 16, y = -60, width = 300, gap = 6, draggable = true, children = children }
end

local function refresh_board(me)
  if not settings.board then omsi.ui.remove(BOARD); return end
  if not me then
    omsi.ui.set(BOARD, board_frame({
      { type = "text", text = "노선 띠: 운행(시간표)을 시작하면 보여요", size = 13, color = "#9AA0A6" } }))
    return
  end
  -- 다른 버스: 같은 trip, 위치 p → 그 버스가 막 지난 정류장 줄(floor(p)+1)에 표시
  local idx = {}
  for i, s in ipairs(me.stops) do if not idx[s.id] then idx[s.id] = i end end
  local marks = {}           -- 줄 번호 → { 글자들 }, 색
  local function mark(p, label, color)
    local row = math.max(1, math.min(#me.stops, math.floor(p + 0.001) + 1))
    local m = marks[row] or { t = {}, c = color }
    m.t[#m.t + 1] = label
    if color == GREEN then m.c = GREEN end
    marks[row] = m
  end
  local n_obis, n_ai = 0, 0
  for _, b in ipairs(buses) do
    if fresh(b) and b.id ~= my_id and b.trip == me.trip and idx[b.next_id] then
      mark(pos_of(me.stops, idx[b.next_id], b.dist, b.at), "▶" .. short(b.nick ~= "" and b.nick or b.id), GREEN)
      n_obis = n_obis + 1
    end
  end
  if settings.show_ai then
    for _, a in ipairs(omsi.timetable.buses() or {}) do
      local ni = idx[a.next_stop_id]
      if ni and a.x and not a.trip_done and trim(a.trip):lower() == me.trip then
        local s = me.stops[ni]
        local d = s.x and math.sqrt((a.x - s.x) ^ 2 + (a.y - s.y) ^ 2) or 0
        mark(pos_of(me.stops, ni, d, a.at_stop), "▷" .. short(a.number ~= nil and a.number ~= "" and a.number or a.tour), "#9AA0A6")
        n_ai = n_ai + 1
      end
    end
  end
  local myrow = math.max(1, math.min(#me.stops, math.floor(me.p + 0.001) + 1))
  local list = {}
  local first, last = 1, #me.stops
  if not settings.in_nav then
    first = math.max(1, myrow - 3)
    last = math.min(#me.stops, first + 15)
  end
  for i = first, last do
    local s = me.stops[i]
    if #list >= 128 then break end
    local m = marks[i]
    local st = i < me.next and "done" or (i == me.next and "now" or "ahead")   -- now = 향하는 정류장
    list[#list + 1] = {
      name = stop_name(s.id, s.name) .. (i == myrow and "  ◀ 내 버스" or ""),
      time = s.arrival and omsi.fmt.clock(s.arrival) or "",
      expected = m and table.concat(m.t, " ") or nil, expected_color = m and m.c or nil,
      state = st, tag = i == #me.stops and "terminus" or nil,
    }
  end
  omsi.ui.set(BOARD, board_frame({
    { type = "row", children = {
      { type = "badge", text = tostring(me.duty.line or "-"), color = BLUE, weight = "bold" },
      { type = "text", text = stop_name(nil, me.duty.terminus or "") .. " 행", weight = "bold", grow = true, wrap = false },
      { type = "badge", text = "유저 " .. n_obis, color = n_obis > 0 and GREEN or "#2A2D33" },
      { type = "badge", text = "AI " .. n_ai, color = "#2A2D33" },
    } },
    { type = "stops", stops = list, size = 13, color = BLUE },
    { type = "text", text = string.format("%d / %d 정류장", math.min(me.next, #me.stops), #me.stops), size = 11, color = "#9AA0A6", align = "right" },
  }))
end

-- ------------------------------------------------------------ 주기
local function tick()
  if has_ws then
    local m = omsi.ws.poll()
    if m and #m > 0 then
      msgs = msgs + #m
      if not last_msg_ms then add_log("서버에 연결됨") end
      last_msg_ms = omsi.util.ms()
      take(m[#m])
    end
  end
  local me = my_state()
  -- 직접 송신 (OBIS 판)
  if has_send then
    if settings.send and not send_on then
      local ok, why = omsi.obis.start()
      send_on = ok and true or false
      add_log(ok and "직접 송신 시작" or ("송신 시작 못 함: " .. tostring(why)))
    elseif not settings.send and send_on then
      omsi.obis.stop(); send_on = false; add_log("직접 송신 끔")
    end
    if send_on then
      local bx, by, bz = omsi.position()
      local text = sender.snapshot({ duty = me and me.duty or nil, x = bx or 0, y = by or 0, z = bz or 0,
        next_dist = me and me.dist or 0, passengers = (omsi.bus.passengers()), has_bus = bx ~= nil })
      local ok, why = omsi.obis.send(text)
      if not ok then omsi.log("obis: 보내기 못 함: " .. tostring(why)) end
    end
  end
  local mine = find_me(me)
  if shown then refresh_panel(me, mine) end
  refresh_board(me)
end

function on_start()
  ko = load_json("stop_names_ko.json") or {}
  obis_maps = load_json("obis_names_ko.json") or {}
  if has_ws then
    local ok, why = omsi.ws.open(OBIS_URL)
    if not ok then add_log("OBIS 연결 못 함: " .. tostring(why)); has_ws = false end
  else
    omsi.log("obis: omsi.ws 없음 (K-OMSI patch/lua-websocket 필요) — AI 버스만 표시")
  end
  build_panel()
  omsi.every(1, tick)
  omsi.input.hotkey("Ctrl+Shift+KeyO", function()
    shown = not shown
    if shown then build_panel() else omsi.ui.remove(PANEL) end
  end)
  omsi.input.hotkey("Ctrl+Shift+KeyB", function()
    omsi.plugin.set_setting("board", not settings.board)
    settings.board = not settings.board
  end)
end

omsi.on("ui_click", function(panel, el)
  if panel ~= PANEL then return end
  if el == "b_board" then omsi.plugin.set_setting("board", not settings.board); settings.board = not settings.board
  elseif el == "b_set" then omsi.plugin.show_settings(true); omsi.ui.focus(true)
  elseif el == "b_hide" then shown = false; omsi.ui.remove(PANEL)
  end
end)

omsi.on("setting", function(key, value) settings[key] = value end)

function on_stop()
  if has_ws then pcall(omsi.ws.close) end
  if send_on then pcall(omsi.obis.stop) end
end

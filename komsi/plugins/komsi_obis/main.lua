-- K-OMSI OBIS 플러그인: OBIS Client 창을 게임 안으로
-- · 오른쪽 위 패널: 서버 연결·운행 상태·이번 정류장·내 버스 표시·접속자 목록
--   OBIS 판 K-OMSI(omsi.obis)면 게임이 직접 내 버스를 보냄(sender.lua, OBIS Client 불필요),
--   "송신 중"은 보내는 연결이 열려 있고 OBIS 서버 실시간 피드에 내 버스가 보일 때 켜짐
-- · 노선 띠(지도 (가)안): 세로형(왼쪽/내비게이터) 또는 가로형(위쪽), 같은 trip 의 OBIS 유저·AI 버스 위치와 앞차·뒷차
--   위치 = 다음 정류장 + 남은 거리 비율 (좌표계가 달라도 같은 방식, project_omsi_bis 와 동일)
-- · 차량 설정(OBIS Client 의 "차량 설정"): 닉네임·좌석형/입석형·좌석 수·저상·막차·입석 혼잡도 (omsi.obis.prefs)
-- OBIS Client(OMSI 2\OBIS-client\OBIS-client.exe)가 설치돼 있을 때만 작동 (omsi.komsi.original)
-- 단축키: Ctrl+Shift+O OBIS 켜기/끄기(받기·보내기·패널 전부, 다음 실행에도 유지), Ctrl+Shift+B 노선 띠,
--         Ctrl+Shift+V 패널 숨기기/보이기(받기·보내기·노선 띠는 그대로, 다음 실행에도 유지)

local OBIS_URL = "wss://obis.kr/api/ws"
local PANEL, BOARD = "obis", "line"
local GREEN, GREY, RED, AMBER, BLUE = "#34B26E", "#5F6368", "#E5392F", "#F2A33A", "#3D7BF5"
local MAX_USERS = 8

local settings = omsi.plugin.settings({
  { key = "show_ai", type = "bool", label = "노선 띠에 AI 시간표 버스도 표시", default = true },
  { key = "board", type = "bool", label = "노선 띠 켜기", default = true },
  { key = "send", type = "bool", label = "게임이 직접 내 버스를 OBIS 로 보냄 (켜면 OBIS Client 는 끄세요)", default = true },
  { key = "board_style", type = "choice", label = "노선 띠 모양", choices = { "세로", "가로" }, default = "세로" },
  { key = "in_nav", type = "bool", label = "세로형 노선 띠를 내비게이터 안에 (끄면 왼쪽 패널)", default = false },
  { key = "auto_nick", type = "bool", label = "OBIS 닉네임을 게임 차량번호로 맞춤", default = true },
  { key = "enabled", type = "bool", label = "OBIS 켜기 (Ctrl+Shift+O)", default = true },
  { key = "panel", type = "bool", label = "OBIS 패널 보이기 (Ctrl+Shift+V, 숨겨도 받기·보내기는 계속)", default = true },
}, "OBIS")

-- OBIS Client 오리지널이 있어야 켜짐 (없으면 받기·보내기 둘 다 못 함)
local owns = omsi.komsi ~= nil and omsi.komsi.original("obis")
local has_ws = owns and omsi.ws ~= nil
local has_send = owns and omsi.obis ~= nil      -- K-OMSI OBIS 판
local sender = has_send and require("sender") or nil
local send_on = false
-- 단말기가 운행 중이 아님 = 안 보냄: 신형 EB(EBterm_vis_model 1, EBterm_run 0) / 티머니 B800(vis_Tmoney800_Driver 1, B800C_Run 0).
-- 단말기 없는 차량·운행 변수 없는 옛 설치는 늘 false
local term_idle_was = nil
local function term_idle()
  local vm = omsi.var("EBterm_vis_model")
  if vm ~= nil and vm >= 0.5 and (omsi.var("EBterm_run") or 0) < 0.5 then return true end
  local b8, br = omsi.var("vis_Tmoney800_Driver"), omsi.var("B800C_Run")
  return b8 ~= nil and b8 >= 0.5 and br ~= nil and br < 0.5
end
-- 단말기 운행 중인지: nil = 운행 변수 있는 단말기가 아님 / true = 운행 중 / false = 운행 시작 전·종료 뒤
local function term_running()
  local vm = omsi.var("EBterm_vis_model")
  if vm ~= nil and vm >= 0.5 and omsi.var("EBterm_run") ~= nil then return omsi.var("EBterm_run") >= 0.5 end
  local b8, br = omsi.var("vis_Tmoney800_Driver"), omsi.var("B800C_Run")
  if b8 ~= nil and b8 >= 0.5 and br ~= nil then return br >= 0.5 end
  return nil
end
local term_running_was = nil
local set_enabled               -- 아래에서 정의 (tick 이 단말기 운행 시작·종료 때 부름)
local shown = true
local log = {}                 -- 활동 로그 (최근 것 위)

-- ------------------------------------------------------------ 한글 정류장명 (eb_nearby 와 같은 규칙)
local map_tabs, obis_use, obis_base, obis_picked, obis_ko = {}, {}, {}, "", {}

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

-- 한글 정류장명·맵 = 게임이 불러온 맵 폴더(info.map_path) 하나로 (2026-10-27, eb_nearby 와 같음).
-- 예전 trip 파일명 판별은 trip 이름이 맵끼리 겹쳐서(101 (to ...), 120 A) OBIS 에 없는 맵(청남 등)·미등록 trip 에
-- 다른 맵·개인 맵(NCC(A)) 이름이 들어갔음. 표 = map_names_ko.json (StopNames_DB/build_eb_names.py):
-- {맵 폴더: {obis = mapId|nil, names}}. 표에 없는 맵 = 영문 그대로. 이름 = Client 의 그 맵 이름 > 표 > 영문
local my_map = nil       -- 지금 맵의 OBIS mapId (OBIS 에 없는 맵·모르는 맵 = nil)

local function map_folder_of(path)
  path = tostring(path or ""):gsub("\\", "/")
  return path:match("([^/]+)/[^/]+$")
end

local function pick_map()
  local info = omsi.info and omsi.info() or nil
  local f = info and map_folder_of(info.map_path) or nil
  if f == obis_picked then return end
  obis_picked = f
  local e = f and map_tabs[f] or nil
  my_map = e and e.obis or nil
  -- 보내기 노선도 이 맵 것만: mapId / false = 이 PC 표에 있는 OBIS 미등록 맵(노선 안 붙임) / nil = 모르는 맵(원래 규칙)
  if sender and sender.set_map then sender.set_map(e and (e.obis or false) or nil) end
  obis_use, obis_base = {}, {}
  if my_map then
    local client = omsi.obis and omsi.obis.map_names and omsi.obis.map_names(my_map)
    if type(client) == "table" then for k, v in pairs(client) do obis_use[k] = v end end
  end
  for k, v in pairs(e and e.names or {}) do if obis_use[k] == nil then obis_use[k] = v end end
  for k, v in pairs(obis_use) do
    local b = base_name(k)
    if not obis_base[b] then obis_base[b] = v end
  end
  omsi.log(string.format("obis: 맵 %s -> 한글표 %s, OBIS %s", tostring(f), e and "있음" or "없음(영문 그대로)", tostring(my_map)))
end

local function stop_name(id, name)
  name = name or ""
  local live = my_map and obis_ko[my_map]
  if live and live[id] then return live[id] end
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
        map = rt.mapId and tostring(rt.mapId) or nil,
      }
      if type(rt.stations) == "table" then
        local names = {}
        list[#list].stnames = names
        for _, s in ipairs(rt.stations) do
          local sid = math.floor(tonumber(s.id) or 0)
          names[sid] = has_hangul(s.name_ko) and s.name_ko or s.name
          if has_hangul(s.name_ko) and rt.mapId then
            obis_ko[rt.mapId] = obis_ko[rt.mapId] or {}
            obis_ko[rt.mapId][sid] = s.name_ko
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
  pick_map()
  local duty = omsi.duty.get()
  if not duty or not duty.next_stop then return nil end
  local stops = omsi.duty.stops()
  if type(stops) ~= "table" or #stops < 2 then return nil end
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
local PREFS = "obis_prefs"
local draft = nil              -- 차량 설정 창에서 고치는 중인 값

-- OBIS 차량 설정(prefs) → 칩 글자 (Client 의 "내 버스 표시 설정"과 같은 순서)
local function prefs_chips(p)
  if type(p) ~= "table" then return "-" end
  local t = { (p.nickname and p.nickname ~= "") and p.nickname or "닉네임 없음" }
  t[#t + 1] = p.crowdType == "stand" and "입석형" or "좌석형"
  if p.seatCapacity then t[#t + 1] = tostring(math.floor(p.seatCapacity)) .. "석" end
  if p.lowFloor then t[#t + 1] = "저상" end
  t[#t + 1] = p.vehicleBody == "night" and "막차" or "일반"
  return table.concat(t, " · ")
end

local function build_panel(hint)
  panel_hint = hint
  omsi.ui.set(PANEL, {
    anchor = "top_right", x = 16, y = 80, width = 400, draggable = true, grab = true, gap = 8,
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
        { type = "text", id = "prefs", text = "-", size = 12, weight = "medium", grow = true, wrap = false },
        { type = "button", id = "b_prefs", text = "설정 변경", icon = "tune", color = BLUE },
      } },
      { type = "box", color = AMBER, visible = hint ~= nil, children = {
        { type = "text", size = 12, text = hint or "" },
      } },
      { type = "table", id = "users", columns = { "닉네임", "노선", "다음 정류장", "남은 거리" },
        widths = { 0.2, 0.17, 0.43, 0.2 }, aligns = { "left", "left", "left", "right" }, size = 12, rows = {} },
      { type = "text", id = "log", text = "", size = 11, color = "#9AA0A6" },
      { type = "row", gap = 6, children = {
        { type = "button", id = "b_board", text = "노선 띠", icon = "route", grow = true },
        { type = "button", id = "b_set", icon = "settings" },
        { type = "button", id = "b_min", icon = "remove" },
        { type = "button", id = "b_hide", icon = "power_settings_new" },
      } },
    },
  })
end

-- 차량 설정 창 (OBIS Client 의 "차량 설정" 모달과 같은 항목)
local function build_prefs()
  local p = draft
  omsi.ui.set(PREFS, {
    anchor = "center", width = 380, draggable = true, gap = 10, accent = BLUE,
    children = {
      { type = "row", children = {
        { type = "icon", name = "tune", color = BLUE },
        { type = "text", text = "차량 설정", size = 17, weight = "bold", grow = true },
        { type = "button", id = "p_close", icon = "close" },
      } },
      { type = "text", text = "닉네임", size = 12, color = "#9AA0A6" },
      { type = "input", id = "p_nick", text = p.nickname or "", placeholder = "닉네임", max = 20 },
      { type = "checkbox", id = "p_auto", text = "게임 차량번호를 닉네임으로 (" .. (omsi.bus.number() or "-") .. ")", checked = settings.auto_nick },
      { type = "text", text = "차량 종류", size = 12, color = "#9AA0A6" },
      { type = "tabs", id = "p_crowd", tabs = { "좌석형", "입석형" }, selected = p.crowdType == "stand" and 2 or 1 },
      { type = "text", text = "총 좌석 수 (1~80)", size = 12, color = "#9AA0A6" },
      { type = "input", id = "p_seats", text = tostring(math.floor(p.seatCapacity or 0)), max = 2 },
      { type = "checkbox", id = "p_low", text = "저상 차량", checked = p.lowFloor and true or false },
      { type = "checkbox", id = "p_night", text = "막차", checked = p.vehicleBody == "night" },
      { type = "checkbox", id = "p_stand", text = "입석 혼잡도 표시", checked = p.showStandCrowd and true or false },
      { type = "text", id = "p_msg", text = "", size = 12, color = AMBER },
      { type = "row", gap = 8, children = {
        { type = "button", id = "p_save", text = "저장", icon = "save", color = BLUE, grow = true },
        { type = "button", id = "p_cancel", text = "닫기", grow = true },
      } },
    },
  })
  omsi.ui.focus(true)
end

local function open_prefs()
  local p = has_send and send_on and omsi.obis.prefs() or nil
  if type(p) ~= "table" then
    omsi.ui.toast(has_send and "OBIS 차량 설정을 아직 못 읽었어요. 직접 송신이 켜져 있어야 합니다." or "이 게임 버전엔 OBIS 송신이 없습니다.",
      { title = "OBIS", icon = "warning", color = AMBER })
    return
  end
  draft = {}
  for k, v in pairs(p) do draft[k] = v end
  build_prefs()
end

local function save_prefs()
  local n = tonumber(draft.seatCapacity)
  if not n or n < 1 or n > 80 or n % 1 ~= 0 then
    omsi.ui.update(PREFS, "p_msg", { text = "좌석 수는 1~80 사이 숫자여야 합니다." })
    return
  end
  local ok, why = omsi.obis.set_prefs({ nickname = draft.nickname or "", crowdType = draft.crowdType or "seat",
    seatCapacity = math.floor(n), lowFloor = draft.lowFloor and true or false,
    vehicleBody = draft.vehicleBody == "night" and "night" or "normal", showStandCrowd = draft.showStandCrowd and true or false })
  if not ok then omsi.ui.update(PREFS, "p_msg", { text = "저장 실패: " .. tostring(why) }); return end
  add_log("차량 설정 저장")
  omsi.ui.remove(PREFS)
  draft = nil
end

-- 닉네임을 게임 차량번호로 맞춤 (켜 두면, 번호가 바뀔 때만 저장)
local nick_try_at = -1e9
local function auto_nickname()
  if not (settings.auto_nick and send_on) or draft then return end
  local num = omsi.bus.number()
  local p = omsi.obis.prefs()
  if not num or num == "" or type(p) ~= "table" or p.nickname == num then return end
  local t = omsi.util.ms()
  if t - nick_try_at < 30000 then return end
  nick_try_at = t
  if omsi.obis.set_prefs({ nickname = num }) then add_log("닉네임을 차량번호로: " .. num) end
end

local function km(d)
  if type(d) ~= "number" or d >= 999999 then return "-" end
  return d >= 1000 and string.format("%.1f km", d / 1000) or string.format("%d m", math.floor(d + 0.5))
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
  elseif settings.send and not send_on and term_idle() then hint = "단말기 운행 시작 전(운행종료)이라 내 버스는 보내지 않아요."
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
  -- 내 버스 표시: OBIS 에 저장된 차량 설정 (직접 송신이 아니면 서버 피드에 보이는 닉네임)
  local p = send_on and omsi.obis.prefs() or nil
  if type(p) == "table" then
    omsi.ui.update(PANEL, "prefs", { text = prefs_chips(p) })
  else
    omsi.ui.update(PANEL, "prefs", { text = mine and mine.nick ~= "" and mine.nick or (omsi.bus.number() or "-") })
  end

  local rows = {}
  for _, b in ipairs(buses) do
    if fresh(b) and #rows < MAX_USERS then
      local me_row = b.id == my_id
      local raw = b.stnames and b.stnames[b.next_id]
      -- (그 버스 노선의 한글명 → 같은 맵이면 내 맵 이름표 → 영문)
      local nm = (b.names and b.names[b.next_id]) or (raw and b.map == my_map and stop_name(b.next_id, raw)) or raw or tostring(b.next_id)
      rows[#rows + 1] = {
        { text = (b.nick ~= "" and b.nick or "-") .. (me_row and " (나)" or ""), color = me_row and GREEN or nil },
        b.line ~= "" and b.line or "-", nm, b.at and "정차 중" or km(b.dist),
      }
    end
  end
  if #rows == 0 then rows[1] = { "-", "-", "접속한 유저 없음", "" } end
  omsi.ui.update(PANEL, "users", { rows = rows })
  omsi.ui.update(PANEL, "log", { text = table.concat(log, "\n") })
end

-- ------------------------------------------------------------ 노선 띠 (세로형 / 가로형)
local function short(nick) local d = tostring(nick or ""):gsub("%D", ""); return #d > 4 and d:sub(-4) or (d ~= "" and d or tostring(nick or "")) end

-- 시간표 시각 보간: 위치 p(0 = 첫 정류장)의 예정 시각(초)
local function time_at(stops, p)
  local i = math.max(0, math.min(math.floor(p), #stops - 1))
  if i >= #stops - 1 then return stops[#stops].arrival or 0 end
  local a, b = stops[i + 1].arrival or 0, stops[i + 2].arrival or 0
  return a + (b - a) * (p - i)
end

-- 같은 trip 의 다른 버스들: { p, label, user(OBIS 유저면 true) }
local function others_of(me)
  local idx, out = {}, {}
  for i, s in ipairs(me.stops) do if not idx[s.id] then idx[s.id] = i end end
  for _, b in ipairs(buses) do
    if fresh(b) and b.id ~= my_id and my_map and b.map == my_map and b.trip == me.trip and idx[b.next_id] then   -- 같은 맵 유저만
      out[#out + 1] = { p = pos_of(me.stops, idx[b.next_id], b.dist, b.at), label = short(b.nick ~= "" and b.nick or b.id), user = true }
    end
  end
  if settings.show_ai then
    for _, a in ipairs(omsi.timetable.buses() or {}) do
      local ni = idx[a.next_stop_id]
      if ni and a.x and not a.trip_done and trim(a.trip):lower() == me.trip then
        local s = me.stops[ni]
        local d = s.x and math.sqrt((a.x - s.x) ^ 2 + (a.y - s.y) ^ 2) or 0
        out[#out + 1] = { p = pos_of(me.stops, ni, d, a.at_stop), label = short(a.number ~= nil and a.number ~= "" and a.number or a.tour), user = false }
      end
    end
  end
  return out
end

-- 앞차·뒷차: 유저가 있으면 유저 중에서, 없으면 AI 중에서 가장 가까운 것
local function neighbours(me, others)
  local function pick(users_only)
    local ahead, behind
    for _, o in ipairs(others) do
      if o.user or not users_only then
        if o.p > me.p + 0.001 and (not ahead or o.p < ahead.p) then ahead = o end
        if o.p < me.p - 0.001 and (not behind or o.p > behind.p) then behind = o end
      end
    end
    return ahead, behind
  end
  local a, b = pick(true)
  local a2, b2 = pick(false)
  return a or a2, b or b2
end

local function gap_text(me, o, word)
  if not o then return word .. " 없음" end
  local stops = math.floor(math.abs(o.p - me.p) + 0.5)
  local mins = math.floor(math.abs(time_at(me.stops, o.p) - time_at(me.stops, me.p)) / 60 + 0.5)
  return string.format("%s %s%s · %d정류장 · %d분", word, o.label, o.user and "" or "(AI)", stops, mins)
end

local function header(me, others, n_obis, n_ai)
  local ahead, behind = neighbours(me, others)
  return {
    { type = "row", children = {
      { type = "badge", text = tostring(me.duty.line or "-"), color = BLUE, weight = "bold" },
      { type = "space", grow = true },   -- (행선지 글자는 빼 달라고 함, 2026-10-26)
      { type = "badge", text = "유저 " .. n_obis, color = n_obis > 0 and GREEN or "#2A2D33" },
      { type = "badge", text = "AI " .. n_ai, color = "#2A2D33" },
      { type = "button", id = "bd_style", icon = "sync_alt" },
    } },
    { type = "row", gap = 6, children = {
      { type = "badge", text = "▲ " .. gap_text(me, ahead, "앞차"), color = ahead and (ahead.user and "#1F5136" or "#2A2D33") or "#2A2D33", grow = true },
      { type = "badge", text = "▼ " .. gap_text(me, behind, "뒷차"), color = behind and (behind.user and "#1F5136" or "#2A2D33") or "#2A2D33", grow = true },
    } },
  }
end

local function marks_of(me, others)
  local marks = {}           -- 정류장 줄 → { t = 글자들, c = 색 }
  for _, o in ipairs(others) do
    local row = math.max(1, math.min(#me.stops, math.floor(o.p + 0.001) + 1))
    local m = marks[row] or { t = {}, c = "#9AA0A6" }
    m.t[#m.t + 1] = (o.user and "▶" or "▷") .. o.label
    if o.user then m.c = GREEN end
    marks[row] = m
  end
  return marks
end

local function frame_vertical(children)
  if settings.in_nav then
    return { anchor = "navigator", padding = 10, gap = 6, children = children }
  end
  return { anchor = "left", x = 16, y = -60, width = 320, gap = 6, draggable = true, grab = true, children = children }
end

local function board_vertical(me, others, n_obis, n_ai)
  local marks = marks_of(me, others)
  local myrow = math.max(1, math.min(#me.stops, math.floor(me.p + 0.001) + 1))
  local first, last = 1, #me.stops
  if not settings.in_nav then
    first = math.max(1, myrow - 3)
    last = math.min(#me.stops, first + 15)
  end
  local list = {}
  for i = first, last do
    if #list >= 128 then break end
    local s, m = me.stops[i], marks[i]
    list[#list + 1] = {
      name = stop_name(s.id, s.name) .. (i == myrow and "  ◀ 내 버스" or ""),
      time = s.arrival and omsi.fmt.clock(s.arrival) or "",
      expected = m and table.concat(m.t, " ") or nil, expected_color = m and m.c or nil,
      state = i < me.next and "done" or (i == me.next and "now" or "ahead"),   -- now = 향하는 정류장
      tag = i == #me.stops and "terminus" or nil,
    }
  end
  local ch = header(me, others, n_obis, n_ai)
  ch[#ch + 1] = { type = "stops", stops = list, size = 13, color = BLUE }
  ch[#ch + 1] = { type = "text", text = string.format("%d / %d 정류장", math.min(me.next, #me.stops), #me.stops), size = 11, color = "#9AA0A6", align = "right" }
  omsi.ui.set(BOARD, frame_vertical(ch))
end

-- 가로형: 위에 노선 전체 막대(내 버스·다른 버스 위치), 아래에 내 앞뒤 정류장 7칸
local function board_horizontal(me, others, n_obis, n_ai)
  local L = math.max(1, #me.stops - 1)
  local w = L * 0.006                    -- 표시 하나의 폭
  local pts = { { p = me.p, c = "#FFFFFF" } }
  local shown_n = 0
  for _, o in ipairs(others) do
    if shown_n < 6 then pts[#pts + 1] = { p = o.p, c = o.user and GREEN or "#9AA0A6" }; shown_n = shown_n + 1 end
  end
  table.sort(pts, function(a, b) return a.p < b.p end)
  local segs, at = {}, 0
  for _, q in ipairs(pts) do
    local s = math.max(at, math.min(L - w, q.p - w / 2))
    if s > at then segs[#segs + 1] = { value = s - at, color = at < me.p and BLUE or "#3A3F47" } end
    segs[#segs + 1] = { value = w, color = q.c }
    at = s + w
  end
  if at < L then segs[#segs + 1] = { value = L - at, color = at < me.p and BLUE or "#3A3F47" } end

  local marks = marks_of(me, others)
  local myrow = math.max(1, math.min(#me.stops, math.floor(me.p + 0.001) + 1))
  local first = math.max(1, math.min(myrow - 2, #me.stops - 6))
  local cols = {}
  for i = first, math.min(#me.stops, first + 6) do
    local s, m = me.stops[i], marks[i]
    local st = i < me.next and "done" or (i == me.next and "now" or "ahead")
    cols[#cols + 1] = { type = "box", grow = true, gap = 2,
      color = st == "now" and BLUE or (st == "done" and "#4A4F57" or "#2A2D33"),
      children = {
        { type = "text", text = stop_name(s.id, s.name), size = 12, weight = st == "now" and "bold" or "regular", wrap = true },
        { type = "text", text = (s.arrival and omsi.fmt.clock(s.arrival) or "") .. (i == #me.stops and "  종점" or ""), size = 11, color = "#9AA0A6" },
        { type = "text", text = (i == myrow and "◀ 내 버스 " or "") .. (m and table.concat(m.t, " ") or ""), size = 11,
          color = (m and m.c == GREEN) and GREEN or "#FFFFFF" },
      } }
  end
  local ch = header(me, others, n_obis, n_ai)
  ch[#ch + 1] = { type = "bar", segments = segs, height = 8 }
  ch[#ch + 1] = { type = "row", gap = 4, children = cols }
  ch[#ch + 1] = { type = "text", text = string.format("%d / %d 정류장", math.min(me.next, #me.stops), #me.stops), size = 11, color = "#9AA0A6", align = "right" }
  omsi.ui.set(BOARD, { anchor = "top", x = 0, y = 70, width = 1040, gap = 6, draggable = true, grab = true, children = ch })
end

local function refresh_board(me)
  if not settings.board then omsi.ui.remove(BOARD); return end
  if not me then
    local msg = { { type = "text", text = "노선 띠: 운행(시간표)을 시작하면 보여요", size = 13, color = "#9AA0A6" } }
    omsi.ui.set(BOARD, settings.board_style == "가로" and { anchor = "top", x = 0, y = 70, width = 420, children = msg } or frame_vertical(msg))
    return
  end
  local others = others_of(me)
  local n_obis, n_ai = 0, 0
  for _, o in ipairs(others) do if o.user then n_obis = n_obis + 1 else n_ai = n_ai + 1 end end
  if settings.board_style == "가로" then board_horizontal(me, others, n_obis, n_ai)
  else board_vertical(me, others, n_obis, n_ai) end
end

-- ------------------------------------------------------------ 주기
local function tick()
  -- 단말기 운행 시작 → OBIS 켬, 운행 종료 → 끔 (바뀌는 순간만: 그 사이 손으로 켜고 끈 건 그대로 둠)
  local run = term_running()
  if run ~= nil and term_running_was ~= nil and run ~= term_running_was and run ~= settings.enabled then
    omsi.log("obis: 단말기 " .. (run and "운행 시작 → OBIS 켬" or "운행 종료 → OBIS 끔"))
    set_enabled(run)
  end
  term_running_was = run
  if not settings.enabled then return end
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
  -- 직접 송신 (OBIS 판). 신형 EB 단말기 차량은 단말기 운행 시작~운행 종료 동안만 (EBterm_run)
  if has_send then
    local idle = term_idle()
    local want = settings.send and not idle
    if want and not send_on then
      local ok, why = omsi.obis.start()
      send_on = ok and true or false
      add_log(ok and (term_idle_was and "단말기 운행 시작 → 직접 송신 시작" or "직접 송신 시작")
        or ("송신 시작 못 함: " .. tostring(why)))
    elseif not want and send_on then
      omsi.obis.stop(); send_on = false; add_log(idle and "단말기 운행 종료 → 송신 멈춤" or "직접 송신 끔")
    end
    if term_idle_was ~= nil and idle ~= term_idle_was then omsi.log("obis: 단말기 " .. (idle and "운행 종료 → 송신 멈춤" or "운행 시작 → 송신")) end
    term_idle_was = idle
    if send_on then
      local bx, by, bz = omsi.position()
      local text = sender.snapshot({ duty = me and me.duty or nil, x = bx or 0, y = by or 0, z = bz or 0,
        next_dist = me and me.dist or 0, passengers = (omsi.bus.passengers()), has_bus = bx ~= nil })
      local ok, why = omsi.obis.send(text)
      if not ok then omsi.log("obis: 보내기 못 함: " .. tostring(why)) end
      auto_nickname()
    end
  end
  local mine = find_me(me)
  if shown then refresh_panel(me, mine) end
  refresh_board(me)
end

-- 켜기: 받기 연결 + 패널 (보내기는 tick 이 설정 따라 시작) / 끄기: 받기·보내기 끊고 패널 모두 내림
local function switch_on()
  if has_ws then
    local ok, why = omsi.ws.open(OBIS_URL)
    if not ok then add_log("OBIS 연결 못 함: " .. tostring(why)) end
  end
  shown = settings.panel
  if shown then build_panel() end
end

local function switch_off()
  if has_ws then pcall(omsi.ws.close) end
  if send_on then pcall(omsi.obis.stop); send_on = false end
  buses, my_id, last_msg_ms = {}, "", nil
  for _, id in ipairs({ PANEL, BOARD, PREFS }) do omsi.ui.remove(id) end
  draft = nil
end

function set_enabled(on)
  omsi.plugin.set_setting("enabled", on)
  settings.enabled = on
  if on then switch_on() else switch_off() end
  omsi.ui.toast(on and "OBIS 켬" or "OBIS 끔 (받기·보내기 멈춤)", { title = "OBIS", icon = "directions_bus", seconds = 3 })
end

-- 패널만 숨기기/보이기: 받기·보내기·노선 띠는 tick 이 그대로 돌림
local function set_panel(on)
  omsi.plugin.set_setting("panel", on)
  settings.panel = on
  if not settings.enabled then return end
  shown = on
  if on then build_panel(panel_hint) else omsi.ui.remove(PANEL) end
  omsi.ui.toast(on and "OBIS 패널 보임" or "OBIS 패널 숨김 (받기·보내기는 계속, Ctrl+Shift+V 로 다시 보기)",
    { title = "OBIS", icon = "directions_bus", seconds = 3 })
end

function on_start()
  if not owns then
    omsi.log("obis: OBIS Client(OBIS-client/OBIS-client.exe)가 없어 OBIS 기능을 끔")
    return
  end
  local mt = load_json("map_names_ko.json")
  map_tabs = mt and mt.maps or {}
  if settings.enabled then switch_on() end
  omsi.every(1, tick)
  omsi.input.hotkey("Ctrl+Shift+KeyO", function() set_enabled(not settings.enabled) end)
  omsi.input.hotkey("Ctrl+Shift+KeyB", function()
    if not settings.enabled then return end
    omsi.plugin.set_setting("board", not settings.board)
    settings.board = not settings.board
  end)
  omsi.input.hotkey("Ctrl+Shift+KeyV", function()
    if settings.enabled then set_panel(not settings.panel) end
  end)
end

omsi.on("ui_click", function(panel, el)
  if panel == BOARD and el == "bd_style" then
    local v = settings.board_style == "가로" and "세로" or "가로"
    omsi.plugin.set_setting("board_style", v); settings.board_style = v
    return
  end
  if panel == PREFS then
    if el == "p_save" then save_prefs()
    elseif el == "p_close" or el == "p_cancel" then omsi.ui.remove(PREFS); draft = nil end
    return
  end
  if panel ~= PANEL then return end
  if el == "b_prefs" then open_prefs(); return end
  if el == "b_board" then omsi.plugin.set_setting("board", not settings.board); settings.board = not settings.board
  elseif el == "b_set" then omsi.plugin.show_settings(true); omsi.ui.focus(true)
  elseif el == "b_min" then set_panel(false)
  elseif el == "b_hide" then set_enabled(false)
  end
end)

omsi.on("setting", function(key, value)
  if key == "enabled" and owns and value ~= settings.enabled then set_enabled(value); return end
  if key == "panel" and owns and value ~= settings.panel then set_panel(value); return end
  settings[key] = value
end)

omsi.on("ui_change", function(panel, el, value)
  if panel ~= PREFS or not draft then return end
  if el == "p_nick" then draft.nickname = value
  elseif el == "p_auto" then omsi.plugin.set_setting("auto_nick", value); settings.auto_nick = value
  elseif el == "p_crowd" then draft.crowdType = value == 2 and "stand" or "seat"
  elseif el == "p_seats" then draft.seatCapacity = tonumber(value) or value
  elseif el == "p_low" then draft.lowFloor = value
  elseif el == "p_night" then draft.vehicleBody = value and "night" or "normal"
  elseif el == "p_stand" then draft.showStandCrowd = value
  end
end)

function on_stop()
  if not owns then return end
  if has_ws then pcall(omsi.ws.close) end
  if send_on then pcall(omsi.obis.stop) end
end

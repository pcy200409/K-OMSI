-- K-OMSI 디지털 운행기록계(타코) 플러그인
-- · 게임 화면에 OMSI 2 Digital Tachograph 모양 패널: 상태·속도·RPM·페달·주행거리·방향·G, 이벤트 램프 8개
-- · 녹화(REC)는 게임 안에서: 플러그인 data\rec\*.jsonl 에 샘플을 적음 (TACO 분석기 실시간 저장과 같은 줄 형식)
-- · 채점·리포트는 하지 않음 → 외부 TACO 분석기(taco_gui.py / taco_score.py replay)가 이 파일을 읽어 채점
--   (램프는 운전 중 알림일 뿐, 횟수·점수는 화면에도 파일에도 남기지 않음)
-- 단축키: Ctrl+Shift+T 패널 보이기/숨기기. 버튼은 마우스로 바로 누름(grab), 설정은 패널의 톱니바퀴.

local PANEL = "dtg"
local DETECT_DT = 0.1          -- 램프 판정 간격(초)
local FLUSH_EVERY = 2          -- 녹화 버퍼를 파일에 쓰는 간격(초)

local LCD = "#9DB25E"        -- 원본 화면의 올리브색 LCD
local INK = "#E4F0BE"          -- 상자 칠이 반투명이라 밝은 글자가 잘 보임
local LAMP_OFF, LAMP_TXT, LAMP_ON = "#2A2D2A", "#C4C9C4", "#E5392F"

local LAMPS = {
  { id = "l_start", ko = "급출발", ev = { "sudden_start" } },
  { id = "l_acc", ko = "급가속", ev = { "rapid_acc" } },
  { id = "l_dec", ko = "급감속", ev = { "rapid_dec" } },
  { id = "l_stop", ko = "급정지", ev = { "sudden_stop" } },
  { id = "l_lane", ko = "급차로", ev = { "lane_change" } },
  { id = "l_over", ko = "급추월", ev = { "overtaking" } },
  { id = "l_turn", ko = "급회전", ev = { "sudden_left_turn", "sudden_right_turn" } },
  { id = "l_uturn", ko = "급유턴", ev = { "sudden_uturn" } },
}
local LAMP_OF = {}
for _, l in ipairs(LAMPS) do for _, e in ipairs(l.ev) do LAMP_OF[e] = l end end

local settings = omsi.plugin.settings({
  { key = "driver", type = "text", label = "기사 이름", default = "" },
  { key = "plate", type = "text", label = "차량번호", default = "" },
  { key = "route", type = "text", label = "노선", default = "" },
  { key = "dir", type = "choice", label = "방향", choices = { "상행", "하행" }, default = "상행" },
  { key = "period", type = "number", label = "샘플링 주기(초)", min = 0.1, max = 1.0, step = 0.1, default = 0.2 },
}, "디지털 운행기록계")

local shown = true
local rec = nil                -- 녹화 중: { file, lines = {}, n, t0 }
local trip_km, last_odo, last_x, last_y = 0, nil, nil, nil
local odo_km = 0
local lit_until = {}           -- 램프 id → 꺼질 시각(omsi.time)
local snap = {}                -- 패널에 보일 값

-- ------------------------------------------------------------ 이벤트 판정 (taco_score.Scorer 와 같은 기준, 램프 전용)
local CD = { sudden_start = 3, rapid_acc = 3, rapid_dec = 3, sudden_stop = 3, lane_change = 5,
  overtaking = 5, sudden_left_turn = 5, sudden_right_turn = 5, sudden_uturn = 8 }
local last_fire, hist, cand = {}, {}, {}

local function hd(a, b) return (a - b + 180) % 360 - 180 end

local function fire(k, t)
  if t - (last_fire[k] or -1e9) < CD[k] then return end
  last_fire[k] = t
  local l = LAMP_OF[k]
  if l then lit_until[l.id] = t + 3 end
end

local function sample_at(t, off)
  local best, bd = nil, 0.5
  for i = #hist, 1, -1 do
    local s = hist[i]
    local d = math.abs(t - s[1] - off)
    if d < bd then best, bd = s, d end
    if t - s[1] > off + 0.5 then break end
  end
  return best
end

local function detect(t, v, h)
  hist[#hist + 1] = { t, v, h }
  while #hist > 0 and t - hist[1][1] > 10 do table.remove(hist, 1) end
  local s1 = sample_at(t, 1.0)
  local a, om, v1 = 0, 0, nil
  if s1 then v1 = s1[2]; a = v - v1; om = hd(h, s1[3]) end
  if v1 and v1 <= 5 and a >= 8 then fire("sudden_start", t) end
  if a >= 0 then
    if (v >= 6 and v <= 10 and a >= 8) or (v > 10 and v <= 20 and a >= 7) or (v > 20 and a >= 6) then fire("rapid_acc", t) end
  end
  if v >= 6 then
    if (v <= 30 and a <= -9) or (v > 30 and v <= 50 and a <= -10) or (v > 50 and a <= -12) then fire("rapid_dec", t) end
  end
  if a <= -9 and v <= 5 then fire("sudden_stop", t) end
  if v >= 30 and math.abs(om) >= 8 then
    local near = false
    for _, c in ipairs(cand) do if math.abs(t - c.t) < 3 then near = true end end
    if not near then cand[#cand + 1] = { t = t, h = h, acc = {} } end
  end
  for i = #cand, 1, -1 do
    local c = cand[i]
    c.acc[#c.acc + 1] = a
    if t - c.t >= 5 then
      table.remove(cand, i)
      if math.abs(hd(h, c.h)) <= 2 and #c.acc > 0 then
        local calm, push = true, false
        for _, x in ipairs(c.acc) do
          if x < -2 or x > 2 then calm = false end
          if x >= 3 then push = true end
        end
        if calm then fire("lane_change", t) end
        if push then fire("overtaking", t) end
      end
    end
  end
  local s3 = sample_at(t, 3.0)
  if s3 and v >= 25 and s3[2] >= 25 then
    local dd = hd(h, s3[3])
    if math.abs(dd) >= 60 and math.abs(dd) < 160 then fire(dd < 0 and "sudden_left_turn" or "sudden_right_turn", t) end
  end
  local s6 = sample_at(t, 6.0)
  if s6 and v >= 20 and s6[2] >= 20 and math.abs(hd(h, s6[3])) >= 160 then fire("sudden_uturn", t) end
end

-- ------------------------------------------------------------ 값 읽기
local function pct(x) return type(x) == "number" and math.max(0, math.min(100, x * 100)) or 0 end

local function read()
  local st = omsi.bus.state()
  local x, y, _, heading = omsi.position()
  if not st or not x then return nil end
  local v = math.abs(st.speed or omsi.speed() or 0)
  if v < 0.05 then v = 0 end
  -- 누적거리: 버스 주행계가 있으면 그것, 없으면 좌표 이동량
  if last_x then
    local step = math.sqrt((x - last_x) ^ 2 + (y - last_y) ^ 2)
    if step < 200 then
      if not st.odometer then odo_km = odo_km + step / 1000 end
      trip_km = trip_km + step / 1000
    end
  end
  last_x, last_y = x, y
  local km = st.odometer or odo_km
  local across, along, up = omsi.bus.acceleration()
  local g = 9.80665
  local ax, ay, az = (across or 0) / g, (along or 0) / g, (up or 0) / g
  return {
    vel_kmh = v, vel_ms = v / 3.6, km_stand = km,
    throttle = pct(st.throttle), brake = pct(st.brake), clutch = pct(st.clutch), pedals = st.brake ~= nil,
    rpm = st.rpm or "N/A", gear = st.gear or 0, heading = (heading or 0) % 360,
    acc_x_g = ax, acc_y_g = ay, acc_z_g = az, acc_total_g = math.sqrt(ax * ax + ay * ay),
    x = x, y = y,
  }
end

-- ------------------------------------------------------------ 녹화
local function safe(s) return (tostring(s or ""):gsub("[\\/:*?\"<>|%s]", "")) end

local function flush()
  if rec and #rec.lines > 0 then
    omsi.files.append(rec.file, table.concat(rec.lines, "\n") .. "\n")
    rec.lines = {}
  end
end

local function rec_start()
  local d = settings.dir == "하행" and "L" or "H"
  local base = os.date("%y%m%d_%H%M%S") .. "_" .. safe(settings.driver ~= "" and settings.driver or "기사")
    .. "_" .. safe(settings.plate ~= "" and settings.plate or (omsi.bus.number() or "0000")) .. "_" .. d .. safe(settings.route)
  omsi.files.mkdir("rec")
  local info = omsi.info() or {}
  local meta = { meta = {
    format = "komsi_taco/1", driver = settings.driver, plate = settings.plate ~= "" and settings.plate or omsi.bus.number(),
    route = settings.route, dir = settings.dir, start = os.date("%Y-%m-%d %H:%M:%S"),
    vehicle = omsi.vehicle(), bus_file = omsi.bus.file(), map = info.map, period = settings.period,
    game_version = omsi.game.version(),
  } }
  -- 시각은 게임 시간으로 흐름(일시정지·배속에도 채점기의 1초·3초 판정이 게임과 같게)
  rec = { file = "rec/" .. base .. ".jsonl", lines = { omsi.json.encode(meta) }, n = 0,
    t0 = omsi.time(), ts0 = omsi.util.now(), next_at = 0 }
  flush()
  omsi.ui.toast("녹화 시작: " .. base, { title = "운행기록계", icon = "videocam", color = LAMP_ON, seconds = 4 })
end

local function rec_stop()
  if not rec then return end
  rec.lines[#rec.lines + 1] = omsi.json.encode({ ["end"] = { stop = os.date("%Y-%m-%d %H:%M:%S"), samples = rec.n } })
  flush()
  omsi.ui.toast("녹화 저장: " .. rec.file .. "\n채점은 TACO 분석기에서", { title = "운행기록계", icon = "save", seconds = 6 })
  omsi.log("taco: 녹화 끝 " .. omsi.files.dir() .. "/" .. rec.file .. " (" .. rec.n .. "줄)")
  rec = nil
end

local function record(d)
  local now = rec.ts0 + (omsi.time() - rec.t0)
  local wt = omsi.world.time()
  local date = omsi.world.date()
  rec.lines[#rec.lines + 1] = omsi.json.encode({
    connected = true, source = "komsi_taco", timestamp = now,
    vel_kmh = d.vel_kmh, vel_ms = d.vel_ms, km_stand = d.km_stand,
    throttle = d.throttle, brake = d.brake, clutch = d.clutch, pedals = d.pedals,
    rpm = d.rpm, gear = d.gear, heading = d.heading,
    acc_x_g = d.acc_x_g, acc_y_g = d.acc_y_g, acc_z_g = d.acc_z_g, acc_total_g = d.acc_total_g,
    x = d.x, y = d.y, game_time_sec = wt or 0,
    game_date = date and { date.year, date.month, date.day } or nil,
  })
  rec.n = rec.n + 1
end

-- ------------------------------------------------------------ 패널
local function lamp(l)
  return { type = "badge", id = l.id, text = l.ko, color = LAMP_OFF, text_color = LAMP_TXT, weight = "bold", grow = true }
end

local function build()
  local rows = {}
  for i = 1, #LAMPS, 2 do
    rows[#rows + 1] = { type = "row", gap = 6, children = { lamp(LAMPS[i]), lamp(LAMPS[i + 1]) } }
  end
  omsi.ui.set(PANEL, {
    anchor = "bottom", x = 0, y = 16, width = 640, draggable = true, grab = true,   -- 왼쪽 아래는 내비게이터 자리
    background = "#1D1F1EF2", gap = 8,
    children = {
      { type = "row", children = {
        { type = "text", id = "status", text = "OMSI: -  REC: OFF (○)", size = 13, weight = "bold", color = "#C9D6A3", grow = true, wrap = false },
        { type = "text", id = "veh", text = "NO VEHICLE", size = 13, weight = "bold", color = "#C9D6A3", align = "right", wrap = false },
      } },
      { type = "row", gap = 10, children = {
        { type = "box", color = LCD, gap = 6, grow = true, children = {
          { type = "divider", color = "#3E4A2A" },
          { type = "text", id = "spd", text = "SPD: 0.0 km/h | RPM: 0", size = 17, weight = "bold", align = "center", color = INK },
          { type = "text", id = "ped", text = "ACC: 0% | BRK: 0%", size = 17, weight = "bold", align = "center", color = INK },
          { type = "divider", color = "#3E4A2A" },
          { type = "text", id = "odo", text = "ODO: 0.0 km | TRIP: 0.000 km | HDG: 0.0° | G-FORCE: 0.00 G", size = 12, align = "center", color = INK, wrap = false },
          { type = "text", id = "clk", text = "----- --:--:--", size = 12, align = "center", color = INK },
        } },
        { type = "box", color = "#2A2D2A", gap = 6, width = 170, children = rows },
      } },
      { type = "row", gap = 8, children = {
        { type = "text", text = "샘플링 주기", size = 13, color = "#C9D6A3" },
        { type = "badge", id = "period", text = string.format("%.1f 초", settings.period), color = "#2A2D2A", text_color = "#E8E8E8" },
        { type = "button", id = "p_up", icon = "keyboard_arrow_up" },
        { type = "button", id = "p_dn", icon = "keyboard_arrow_down" },
        { type = "space", grow = true },
        { type = "button", id = "rec", text = "녹화", icon = "videocam", color = "#3A3D3A" },
        { type = "button", id = "trip", text = "TRIP 초기화", icon = "restart_alt" },
        { type = "button", id = "set", icon = "settings" },
      } },
    },
  })
end

local function refresh(d)
  local t = omsi.time()
  local veh = omsi.vehicle()
  omsi.ui.update(PANEL, "status", { text = string.format("OMSI: %s  REC: %s",
    d and "CONNECTED" or "DISCONNECTED", rec and "ON (●)" or "OFF (○)") })
  omsi.ui.update(PANEL, "veh", { text = veh or "NO VEHICLE" })
  if d then
    omsi.ui.update(PANEL, "spd", { text = string.format("SPD: %.1f km/h | RPM: %s", d.vel_kmh,
      type(d.rpm) == "number" and string.format("%.0f", d.rpm) or "N/A") })
    omsi.ui.update(PANEL, "ped", { text = string.format("ACC: %.0f%% | BRK: %.0f%%", d.throttle, d.brake) })
    omsi.ui.update(PANEL, "odo", { text = string.format("ODO: %.1f km | TRIP: %.3f km | HDG: %.1f° | G-FORCE: %.2f G",
      d.km_stand, trip_km, d.heading, d.acc_total_g) })
  end
  local date, wt = omsi.world.date(), omsi.world.time()
  omsi.ui.update(PANEL, "clk", { text = (date and string.format("%04d-%02d-%02d", date.year, date.month, date.day) or "-----")
    .. " " .. (wt and omsi.fmt.clock(wt, true) or "--:--:--") })
  for _, l in ipairs(LAMPS) do
    local on = (lit_until[l.id] or 0) > t
    omsi.ui.update(PANEL, l.id, { color = on and LAMP_ON or LAMP_OFF })
  end
  omsi.ui.update(PANEL, "rec", { text = rec and "녹화 중지" or "녹화", color = rec and LAMP_ON or "#3A3D3A" })
end

local function set_period(p)
  p = math.floor(math.max(0.1, math.min(1.0, p)) * 10 + 0.5) / 10
  omsi.plugin.set_setting("period", p)
  settings.period = p
  omsi.ui.update(PANEL, "period", { text = string.format("%.1f 초", p) })
end

-- ------------------------------------------------------------ 이벤트
local cur = nil

function on_start()
  build()
  omsi.every(DETECT_DT, function()
    cur = read()
    if not cur then hist, cand = {}, {}; last_x = nil; return end
    detect(omsi.time(), cur.vel_kmh, cur.heading)
    if rec and omsi.time() >= rec.next_at then
      rec.next_at = omsi.time() + settings.period - 0.001
      record(cur)
    end
  end)
  omsi.every(0.25, function() if shown then refresh(cur) end end)
  omsi.every(FLUSH_EVERY, flush)
  omsi.input.hotkey("Ctrl+Shift+KeyT", function()
    shown = not shown
    if shown then build(); refresh(cur) else omsi.ui.remove(PANEL) end
  end)
end

omsi.on("ui_click", function(panel, el)
  if panel ~= PANEL then return end
  if el == "rec" then
    if rec then rec_stop() else rec_start() end
    refresh(cur)
  elseif el == "trip" then
    trip_km = 0
    refresh(cur)
  elseif el == "p_up" then set_period(settings.period + 0.1)
  elseif el == "p_dn" then set_period(settings.period - 0.1)
  elseif el == "set" then omsi.plugin.show_settings(true); omsi.ui.focus(true)
  end
end)

omsi.on("setting", function(key, value)
  settings[key] = value
  if key == "period" then omsi.ui.update(PANEL, "period", { text = string.format("%.1f 초", value) }) end
end)

function on_stop()
  pcall(rec_stop)
end

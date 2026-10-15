//! Isolated Chromium display: loopback page + CDP screenshots and native pointer events.
//! Latest-frame queues keep the simulation thread independent from browser/network stalls.
use super::*;
use base64::Engine;
use omsi_sim::vehicle_api::ApiValue;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static NAVIGATION: std::sync::OnceLock<Mutex<Value>> = std::sync::OnceLock::new();
static NAVIGATION_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CLIENTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub fn navigation_requested() -> bool {
    CLIENTS.load(Ordering::Relaxed) > 0
}
pub fn publish_navigation(value: Value) {
    *NAVIGATION
        .get_or_init(|| Mutex::new(Value::Null))
        .lock()
        .unwrap() = value;
    NAVIGATION_REVISION.fetch_add(1, Ordering::Release);
}

// Double-resolution canonical vehicle displays retain the logical UI size.
fn display_scale(w: u32, h: u32) -> u32 {
    if w >= 1496 && h >= 900 && w % 2 == 0 && h % 2 == 0 {
        2
    } else {
        1
    }
}

fn display_rect(w: u32, h: u32, html: &str) -> [u32; 4] {
    let full = [0, 0, w, h];
    let Some(s) = html.split("name=\"komsi-display-uv\" content=\"").nth(1) else {
        return full;
    };
    let v: Vec<f64> = s
        .split('"')
        .next()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect();
    if v.len() != 4
        || v.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || v[2] <= v[0]
        || v[3] <= v[1]
    {
        return full;
    }
    // Align both boundaries to the device scale so screenshots have exact dimensions.
    let scale = display_scale(w, h);
    let x = (v[0] * w as f64).ceil() as u32;
    let y = (v[1] * h as f64).ceil() as u32;
    let rw = ((v[2] * w as f64).floor() as u32).saturating_sub(x) / scale * scale;
    let rh = ((v[3] * h as f64).floor() as u32).saturating_sub(y) / scale * scale;
    if rw < 64 || rh < 64 {
        full
    } else {
        [x, y, rw, rh]
    }
}

pub fn executable() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("K_OMSI_AUTO_BROWSER") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}
fn value(a: &ApiValue) -> Value {
    match a {
        ApiValue::Null => Value::Null,
        ApiValue::Bool(b) => json!(b),
        ApiValue::Num(n) => json!(n),
        ApiValue::Str(s) => json!(s),
        ApiValue::List(v) => Value::Array(v.iter().map(value).collect()),
        ApiValue::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), value(v))).collect()),
    }
}
#[derive(Default)]
struct Shared {
    vehicle: Value,
    env: Value,
    num: serde_json::Map<String, Value>,
    strs: serde_json::Map<String, Value>,
    pointers: Vec<(f32, f32, PointerKind)>,
    events: Vec<(String, f32)>,
    frame: Option<Vec<u8>>,
    error: Option<String>,
    ready: bool,
    finished: bool,
    #[cfg(test)]
    produced: u64,
    #[cfg(test)]
    commands: Vec<String>,
    #[cfg(test)]
    results: Vec<Value>,
}
pub struct BrowserRenderer {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    fallback: EngineRenderer,
    #[cfg(test)]
    rect: [u32; 4],
}
impl BrowserRenderer {
    pub fn new(exe: PathBuf, w: u32, h: u32, fallback: &str) -> Self {
        let rect = display_rect(w, h, fallback);
        CLIENTS.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, quit) = (shared.clone(), stop.clone());
        std::thread::spawn(move || {
            for attempt in 0..3 {
                if quit.load(Ordering::Relaxed) {
                    break;
                }
                match run(exe.clone(), w, h, rect, &s, &quit) {
                    Ok(()) => break,
                    Err(e) => {
                        log::warn!("Android Auto browser attempt {}: {e}", attempt + 1);
                        if attempt == 2 {
                            s.lock().unwrap().error = Some(e);
                        } else {
                            for _ in 0..10 {
                                if quit.load(Ordering::Relaxed) {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        }
                    }
                }
            }
            s.lock().unwrap().finished = true;
        });
        Self {
            shared,
            stop,
            fallback: EngineRenderer::new(w, h, fallback),
            #[cfg(test)]
            rect,
        }
    }
}
impl Drop for BrowserRenderer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        CLIENTS.fetch_sub(1, Ordering::Relaxed);
    }
}
impl HtmlRenderer for BrowserRenderer {
    fn set_vars(&mut self, n: &[(String, f32)], s: &[(String, String)]) {
        self.fallback.set_vars(n, s);
        let mut state = self.shared.lock().unwrap();
        for (k, v) in n {
            state.num.insert(k.to_ascii_lowercase(), json!(v));
        }
        for (k, v) in s {
            state.strs.insert(k.to_ascii_lowercase(), json!(v));
        }
    }
    fn set_vehicle(&mut self, a: &ApiValue) {
        self.fallback.set_vehicle(a);
        self.shared.lock().unwrap().vehicle = value(a);
    }
    fn set_env(&mut self, a: &ApiValue) {
        self.fallback.set_env(a);
        self.shared.lock().unwrap().env = value(a);
    }
    fn poll_frame(&mut self) -> Option<Vec<u8>> {
        let mut s = self.shared.lock().unwrap();
        if s.ready {
            s.frame.take()
        } else {
            if s.error.is_some() {
                self.fallback.set_vars(
                    &[],
                    &[("k_omsi_browser_error".into(), "Browser unavailable".into())],
                );
            }
            self.fallback.poll_frame()
        }
    }
    fn pointer(&mut self, x: f32, y: f32, k: PointerKind) {
        if self.shared.lock().unwrap().error.is_some() {
            self.fallback.pointer(x, y, k);
            return;
        }
        let mut s = self.shared.lock().unwrap();
        if k == PointerKind::Move && s.pointers.last().is_some_and(|p| p.2 == PointerKind::Move) {
            s.pointers.pop();
        }
        if s.pointers.len() >= 64 && k == PointerKind::Up {
            s.pointers.clear();
        }
        if s.pointers.len() < 64 {
            s.pointers.push((x, y, k));
        }
    }
    fn take_events(&mut self) -> Vec<(String, f32)> {
        let mut e = std::mem::take(&mut self.shared.lock().unwrap().events);
        e.extend(self.fallback.take_events());
        e
    }
}
struct Session {
    child: Child,
    profile: PathBuf,
    stop: Arc<AtomicBool>,
}
struct ServerStop(Arc<AtomicBool>);
impl Drop for ServerStop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
#[cfg(windows)]
struct BrowserJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl BrowserJob {
    fn new(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        unsafe {
            let handle =
                CreateJobObjectW(None, windows::core::PCWSTR::null()).map_err(|e| e.to_string())?;
            let job = Self(handle);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            )
            .map_err(|e| e.to_string())?;
            AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))
                .map_err(|e| e.to_string())?;
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for BrowserJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Only the unique directory created by this session is removed.
        if self.profile.parent() == Some(std::env::temp_dir().as_path())
            && self
                .profile
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("k-omsi-auto-"))
        {
            let _ = std::fs::remove_dir_all(&self.profile);
        }
    }
}
struct Cdp {
    socket: tungstenite::WebSocket<TcpStream>,
    id: u64,
    screencast: Option<Value>,
}
impl Cdp {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.socket
            .send(tungstenite::Message::Text(
                json!({"id":id,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .map_err(|e| e.to_string())?;
        loop {
            let msg = self.socket.read().map_err(|e| e.to_string())?;
            if let tungstenite::Message::Text(t) = msg {
                let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
                if v["method"] == "Page.screencastFrame" {
                    self.screencast = Some(v["params"].clone());
                    self.id += 1;
                    self.socket.send(tungstenite::Message::Text(json!({"id":self.id,"method":"Page.screencastFrameAck","params":{"sessionId":v["params"]["sessionId"]}}).to_string().into())).map_err(|e|e.to_string())?;
                    continue;
                }
                if v["id"].as_u64() == Some(id) {
                    if v.get("error").is_some() {
                        return Err(v.to_string());
                    }
                    return Ok(v["result"].clone());
                }
            }
        }
    }
}
fn run(
    exe: PathBuf,
    w: u32,
    h: u32,
    rect: [u32; 4],
    shared: &Arc<Mutex<Shared>>,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let page_port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let server_stop = Arc::new(AtomicBool::new(false));
    let server_flag = server_stop.clone();
    std::thread::spawn(move || {
        while !server_flag.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let html = include_str!("../../../docs/examples/android-auto/browser.html");
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    html.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(html.as_bytes());
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    });
    let _server_guard = ServerStop(server_stop.clone());
    let profile = std::env::temp_dir().join(format!(
        "k-omsi-auto-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&profile).map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command
        .args([
            "--headless=new",
            "--remote-debugging-port=0",
            "--remote-debugging-address=127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            server_stop.store(true, Ordering::Relaxed);
            let _ = std::fs::remove_dir(&profile);
            return Err(e.to_string());
        }
    };
    let _session = Session {
        child,
        profile: profile.clone(),
        stop: server_stop,
    };
    #[cfg(windows)]
    let _job = BrowserJob::new(&_session.child)?;
    let mut port = None;
    for _ in 0..300 {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Ok(t) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
            port = t.lines().next().and_then(|s| s.parse::<u16>().ok());
            if port.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let port = port.ok_or("browser did not expose its debugging endpoint")?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let body = agent
        .get(&format!("http://127.0.0.1:{port}/json/list"))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let tabs: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let ws = tabs
        .as_array()
        .and_then(|a| a.iter().find(|t| t["type"] == "page"))
        .and_then(|t| t["webSocketDebuggerUrl"].as_str())
        .ok_or("browser page missing")?;
    let tcp = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    let (socket, _) = tungstenite::client(ws, tcp).map_err(|e| e.to_string())?;
    let mut cdp = Cdp {
        socket,
        id: 0,
        screencast: None,
    };
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        json!({"width":rect[2] / display_scale(w,h),"height":rect[3] / display_scale(w,h),"deviceScaleFactor":display_scale(w,h),"mobile":false}),
    )?;
    cdp.call(
        "Page.navigate",
        json!({"url":format!("http://127.0.0.1:{page_port}/auto.html")}),
    )?;
    cdp.call("Page.enable", json!({}))?;
    let ready_until = Instant::now() + Duration::from_secs(30);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if cdp
            .call(
                "Runtime.evaluate",
                json!({"expression":"typeof window.kOmsiUpdate==='function'","returnByValue":true}),
            )
            .is_ok_and(|v| v["result"]["value"] == true)
        {
            break;
        }
        if Instant::now() >= ready_until {
            return Err("browser page did not become ready".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // The material UV bounds include pixels hidden beneath the physical bezel.
    // Keep the interactive page inside that bezel without changing CCTV UVs.
    if rect != [0, 0, w, h] {
        cdp.call("Runtime.evaluate", json!({"expression":"(()=>{const style=document.createElement('style');style.textContent='body{position:absolute;inset:8px 32px 24px 8px}';document.head.appendChild(style)})()"}))?;
    }
    let mut pressed = false;
    let mut navigation_revision = u64::MAX;
    let png = std::env::var("K_OMSI_AUTO_CAPTURE").is_ok_and(|v| v.eq_ignore_ascii_case("png"));
    let fps = std::env::var("K_OMSI_AUTO_FPS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(30)
        .clamp(1, 60);
    let interval = Duration::from_secs_f64(1.0 / fps as f64);
    cdp.call("Page.startScreencast",json!({"format":if png {"png"} else {"jpeg"},"quality":90,"maxWidth":rect[2],"maxHeight":rect[3],"everyNthFrame":1}))?;

    let mut stats_start = Instant::now();
    let (mut stats_frames, mut stats_bytes, mut stats_work) = (0u64, 0usize, Duration::ZERO);
    while !stop.load(Ordering::Relaxed) {
        let frame_start = Instant::now();
        let (mut snapshot, pointers) = {
            let mut s = shared.lock().unwrap();
            (
                json!({"vehicle":s.vehicle,"env":s.env,"num":s.num,"str":s.strs}),
                std::mem::take(&mut s.pointers),
            )
        };
        let revision = NAVIGATION_REVISION.load(Ordering::Acquire);
        if revision != navigation_revision {
            snapshot["navigation"] = NAVIGATION
                .get()
                .map(|n| n.lock().unwrap().clone())
                .unwrap_or(Value::Null);
            navigation_revision = revision;
        }
        cdp.call(
            "Runtime.evaluate",
            json!({"expression":format!("window.kOmsiUpdate && window.kOmsiUpdate({snapshot})" )}),
        )?;
        #[cfg(test)]
        let commands = std::mem::take(&mut shared.lock().unwrap().commands);
        #[cfg(test)]
        for expression in commands {
            let result = cdp.call(
                "Runtime.evaluate",
                json!({"expression":expression,"returnByValue":true}),
            )?;
            shared.lock().unwrap().results.push(result);
        }
        for (x, y, kind) in pointers {
            let typ = match kind {
                PointerKind::Down => {
                    pressed = true;
                    "mousePressed"
                }
                PointerKind::Up => {
                    pressed = false;
                    "mouseReleased"
                }
                PointerKind::Move => "mouseMoved",
            };
            cdp.call("Input.dispatchMouseEvent",json!({"type":typ,"x":(x-rect[0] as f32) / display_scale(w,h) as f32,"y":(y-rect[1] as f32) / display_scale(w,h) as f32,"button":if kind==PointerKind::Move{"none"}else{"left"},"buttons":if pressed{1}else{0},"clickCount":if kind==PointerKind::Move{0}else{1}}))?;
        }
        let response=cdp.call("Runtime.evaluate",json!({"expression":"JSON.stringify(window.kOmsiDrain ? window.kOmsiDrain() : [])","returnByValue":true}))?;
        if let Some(s) = response["result"]["value"].as_str() {
            if let Ok(events) = serde_json::from_str::<Vec<(String, f32)>>(s) {
                for (name, v) in events.into_iter().take(32) {
                    if name == "k_omsi_youtube_paste" {
                        #[cfg(not(any(target_os = "android", target_os = "ios")))]
                        let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).map_err(|_| ());
                        #[cfg(any(target_os = "android", target_os = "ios"))]
                        let text: Result<String, ()> = Err(());
                        let expression=match text { Ok(t) if t.len()<=4096=>format!("window.loadYouTube({})",json!(t)),_=>"document.getElementById('videoStatus').textContent='클립보드의 영상 링크를 읽을 수 없습니다'".into() };
                        cdp.call("Runtime.evaluate", json!({"expression":expression}))?;
                    } else if name == "k_omsi_android_auto" && v.is_finite() {
                        let mut state = shared.lock().unwrap();
                        if state.events.len() < 32 {
                            state.events.push((name, v));
                        }
                    }
                }
            }
        }
        let Some(shot) = cdp.screencast.take() else {
            if let Some(rest) = interval.checked_sub(frame_start.elapsed()) {
                std::thread::sleep(rest);
            }
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().ok_or("missing screenshot")?)
            .map_err(|e| e.to_string())?;
        let mut image = omsi_texture::decode_bytes(
            &bytes,
            std::path::Path::new(if png { "frame.png" } else { "frame.jpg" }),
        )
        .map_err(|e| e.to_string())?;
        // Chromium screencast uses logical viewport pixels rather than the HiDPI
        // screenshot dimensions. Expand once into the unchanged vehicle UV window.
        if image.width > 0
            && image.height > 0
            && (image.width != rect[2] || image.height != rect[3])
        {
            image.rgba = if image.width * 2 == rect[2] && image.height * 2 == rect[3] {
                let mut expanded = vec![0u8; (rect[2] * rect[3] * 4) as usize];
                let row = (rect[2] * 4) as usize;
                for y in 0..image.height as usize {
                    let src = &image.rgba
                        [y * image.width as usize * 4..(y + 1) * image.width as usize * 4];
                    let dst = &mut expanded[y * 2 * row..(y * 2 + 1) * row];
                    for (pixel, out) in src.chunks_exact(4).zip(dst.chunks_exact_mut(8)) {
                        out[..4].copy_from_slice(pixel);
                        out[4..].copy_from_slice(pixel);
                    }
                    expanded.copy_within(y * 2 * row..(y * 2 + 1) * row, (y * 2 + 1) * row);
                }
                expanded
            } else {
                omsi_texture::bc::resize(&image.rgba, image.width, image.height, rect[2], rect[3])
            };
            image.width = rect[2];
            image.height = rect[3];
        }
        if image.width == rect[2] && image.height == rect[3] {
            let mut frame = vec![0u8; (w * h * 4) as usize];
            for y in 0..rect[3] {
                let dst = (((y + rect[1]) * w + rect[0]) * 4) as usize;
                let src = (y * rect[2] * 4) as usize;
                frame[dst..dst + (rect[2] * 4) as usize]
                    .copy_from_slice(&image.rgba[src..src + (rect[2] * 4) as usize]);
            }
            let mut s = shared.lock().unwrap();
            s.frame = Some(frame);
            s.ready = true;
            #[cfg(test)]
            {
                s.produced += 1;
            }
            stats_frames += 1;
            stats_bytes += bytes.len();
        }
        stats_work += frame_start.elapsed();
        if stats_start.elapsed() >= Duration::from_secs(5) && stats_frames > 0 {
            log::info!(
                "Auto capture: {:.1} fps / target {}, {} {}x{}, {:.1} ms work/frame, {:.0} KB/frame",
                stats_frames as f64 / stats_start.elapsed().as_secs_f64(),
                fps,
                if png { "PNG" } else { "JPEG90" },
                rect[2],
                rect[3],
                stats_work.as_secs_f64() * 1000.0 / stats_frames as f64,
                stats_bytes as f64 / stats_frames as f64 / 1024.0
            );
            stats_start = Instant::now();
            stats_frames = 0;
            stats_bytes = 0;
            stats_work = Duration::ZERO;
        }
        if let Some(rest) = interval.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait_until(mut condition: impl FnMut() -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(25);
        while !condition() {
            assert!(
                std::time::Instant::now() < end,
                "browser operation timed out"
            );
            std::thread::sleep(Duration::from_millis(80));
        }
    }
    fn eval(r: &mut BrowserRenderer, script: &str) -> Value {
        r.shared.lock().unwrap().commands.push(script.into());
        wait_until(|| !r.shared.lock().unwrap().results.is_empty());
        let result = r.shared.lock().unwrap().results.remove(0);
        assert!(result.get("exceptionDetails").is_none(), "{result}");
        result["result"]["value"].clone()
    }
    fn click_id(r: &mut BrowserRenderer, id: &str) {
        let at = eval(
            r,
            &format!(
                "(()=>{{const r=document.getElementById({}).getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]}})()",
                json!(id)
            ),
        );
        let (x, y) = (
            at[0].as_f64().unwrap() as f32,
            at[1].as_f64().unwrap() as f32,
        );
        r.pointer(
            x * display_scale(r.fallback.width, r.fallback.height) as f32 + r.rect[0] as f32,
            y * display_scale(r.fallback.width, r.fallback.height) as f32 + r.rect[1] as f32,
            PointerKind::Down,
        );
        r.pointer(
            x * display_scale(r.fallback.width, r.fallback.height) as f32 + r.rect[0] as f32,
            y * display_scale(r.fallback.width, r.fallback.height) as f32 + r.rect[1] as f32,
            PointerKind::Up,
        );
        std::thread::sleep(Duration::from_millis(240));
    }
    fn capture(r: &mut BrowserRenderer, w: u32, h: u32, page: &str) {
        r.shared.lock().unwrap().frame.take();
        wait_until(|| r.shared.lock().unwrap().frame.is_some());
        let frame = r.poll_frame().unwrap();
        assert_eq!(frame.len(), (w * h * 4) as usize);
        if let Ok(dir) = std::env::var("K_OMSI_AA_PREVIEW_DIR") {
            std::fs::write(
                PathBuf::from(dir).join(format!("auto-kit-{w}x{h}-{page}.rgba")),
                frame,
            )
            .unwrap();
        }
    }
    fn fixture(w: u32, h: u32) -> BrowserRenderer {
        fixture_html(
            w,
            h,
            include_str!("../../../docs/examples/android-auto/index.html"),
        )
    }
    fn fixture_html(w: u32, h: u32, html: &str) -> BrowserRenderer {
        let mut roads = Vec::new();
        for i in -5..=5 {
            let p = i as f64 * 55.;
            roads.push(json!({"width":5,"points":[[p,-400],[p,400]]}));
            roads.push(json!({"width":5,"points":[[-400,p],[400,p]]}));
        }
        roads.push(json!({"width":10,"points":[[-320,-300],[-110,-80],[80,180],[240,400]]}));
        publish_navigation(
            json!({"origin":[0,0],"roads":roads,"route":[[[0,-140],[0,110],[165,110],[165,275]]],"turn":{"direction":1,"distance":110,"street":"중앙로 방면"},"nextDistance":720,"stops":[{"position":[165,110]},{"position":[165,275]}]}),
        );
        let mut r = BrowserRenderer::new(executable().expect("Edge/Chrome required"), w, h, html);
        {
            let mut s = r.shared.lock().unwrap();
            s.vehicle = json!({"motion":{"speedKmh":42,"heading":0,"x":0,"y":0},"doors":{"anyOpen":false},"route":{"line":"1611","destination":"중앙터미널 방면","current":{"name":"시청 · 중앙로","arrival":"14:32"}}});
            s.env = json!({"time":{"asString":"14:30:12"}});
            s.num.insert("k_omsi_android_auto".into(), json!(1));
        }
        wait_until(|| {
            let s = r.shared.lock().unwrap();
            assert!(s.error.is_none(), "{:?}", s.error);
            s.ready
        });
        wait_until(|| eval(&mut r, "!!document.getElementById('workspace')") == json!(true));
        r
    }
    fn finish(r: BrowserRenderer) {
        let s = r.shared.clone();
        drop(r);
        wait_until(|| s.lock().unwrap().finished);
    }
    #[test]
    #[ignore = "requires K_OMSI_CARTOGRAPHY_JSON extracted from an installed map and Edge"]
    fn browser_cartography_from_installed_map() {
        let path = std::env::var("K_OMSI_CARTOGRAPHY_JSON").unwrap();
        let data: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let labels = data["labels"].as_array().unwrap();
        assert!(!labels.is_empty());
        let origin = &data["origin"];
        let label = labels
            .iter()
            .min_by(|a, b| {
                let d = |l: &Value| {
                    (l["position"][0].as_f64().unwrap() - origin[0].as_f64().unwrap()).powi(2)
                        + (l["position"][1].as_f64().unwrap() - origin[1].as_f64().unwrap()).powi(2)
                };
                d(a).total_cmp(&d(b))
            })
            .unwrap();
        let x = label["position"][0].as_f64().unwrap() - 110.;
        let y = label["position"][1].as_f64().unwrap() + 40.;
        let mut r = fixture(800, 480);
        {
            r.shared.lock().unwrap().vehicle = json!({"motion":{"x":x,"y":y,"heading":0,"speedKmh":0},"route":{"line":data["stats"]["map"],"current":{"name":"실제 맵 데이터 검증"}}});
        }
        let mut before = data.clone();
        before["roads"] = data["rawRoads"].clone();
        before["labels"] = json!([]);
        before["stops"] = json!([]);
        publish_navigation(before);
        eval(&mut r, "zoom=600;show('nav');true");
        std::thread::sleep(Duration::from_millis(600));
        capture(&mut r, 800, 480, "map-before");
        publish_navigation(data.clone());
        wait_until(|| {
            eval(&mut r, "window.kOmsiMapMetrics?.roads")
                == json!(data["roads"].as_array().unwrap().len())
        });
        assert!(
            eval(&mut r, "window.kOmsiMapMetrics.labels")
                .as_u64()
                .unwrap()
                > 0
        );
        capture(&mut r, 800, 480, "map-after");
        // Zoom-dependent labels and shortest-path bearing interpolation.
        eval(&mut r, "zoom=850;mapCamera.zoom=850;drawMap();true");
        assert_eq!(eval(&mut r, "window.kOmsiMapMetrics.labels"), json!(0));
        eval(&mut r, "mapCamera.heading=359;true");
        {
            r.shared.lock().unwrap().vehicle["motion"]["heading"] = json!(1);
        }
        std::thread::sleep(Duration::from_millis(600));
        let heading = eval(&mut r, "mapCamera.heading").as_f64().unwrap();
        assert!(heading < 5. || heading > 355.);
        finish(r);
    }
    #[test]
    #[ignore = "requires installed Edge; synthetic driving telemetry"]
    fn browser_driving_route_and_next_stop_updates() {
        let mut r = fixture(800, 480);
        eval(&mut r, "show('nav');true");
        // Move without republishing geometry: incremental snapshots must retain the map.
        {
            let mut s = r.shared.lock().unwrap();
            s.vehicle["motion"] = json!({"x":90,"y":60,"heading":45,"speedKmh":30});
            s.vehicle["route"]["current"] = json!({"name":"다음 정류장","arrival":"14:35"});
        }
        wait_until(|| {
            eval(&mut r, "document.getElementById('next').textContent") == json!("다음 정류장")
        });
        wait_until(|| eval(&mut r, "Math.abs(mapCamera.x-90)<1&&Math.abs(mapCamera.y-60)<1&&Math.abs(mapCamera.heading-45)<1") == json!(true));
        assert_eq!(eval(&mut r, "document.getElementById('arrival').textContent"), json!("예정 14:35"));
        assert_eq!(eval(&mut r, "navigation.route[0][2]"), json!([165,110]));
        assert!(eval(&mut r, "window.kOmsiMapMetrics.roads").as_u64().unwrap() > 0);
        // Route recalculation and next-stop distance arrive independently of motion.
        publish_navigation(json!({"origin":[90,60],"roads":[{"width":5,"points":[[0,60],[250,60]]}],"route":[[[90,60],[250,60]]],"turn":{"direction":-1,"distance":80,"street":"갱신 도로"},"nextDistance":180,"rerouting":true}));
        wait_until(|| eval(&mut r, "document.getElementById('street').textContent") == json!("갱신 도로"));
        assert_eq!(eval(&mut r, "navigation.route[0][1]"), json!([250,60]));
        assert_eq!(eval(&mut r, "document.getElementById('turnDistance').textContent"), json!("80 m"));
        assert_eq!(eval(&mut r, "document.getElementById('routeInfo').textContent"), json!("180 m · 1611 · 경로 재탐색"));
        {
            let mut s = r.shared.lock().unwrap();
            s.vehicle["route"]["stops"] = json!([{"name":"지난 정류장"},{"name":"다음 정류장","arrival":"14:35"},{"name":"종점","arrival":"14:40"}]);
            s.vehicle["route"]["nextIndex"] = json!(1);
            s.vehicle["motion"]["speedKmh"] = json!(0);
            s.vehicle["doors"]["anyOpen"] = json!(true);
        }
        publish_navigation(json!({"origin":[90,60],"roads":[],"nextDistance":20}));
        wait_until(|| eval(&mut r, "document.getElementById('stopGuide').textContent") == json!("정류장 부근 · 문 열림"));
        assert_eq!(eval(&mut r, "Array.from(document.querySelectorAll('#stopList .stopRow span')).map(e=>e.textContent)"), json!(["다음 정류장","종점"]));
        {
            let mut s = r.shared.lock().unwrap();
            s.vehicle["route"]["nextIndex"] = json!(2);
            s.vehicle["route"]["current"] = json!({"name":"종점","arrival":"14:40"});
        }
        wait_until(|| eval(&mut r, "document.querySelector('#stopList .stopRow span')?.textContent") == json!("종점"));
        assert_eq!(eval(&mut r, "document.querySelectorAll('#stopList .stopRow').length"), json!(1));
        {
            r.shared.lock().unwrap().vehicle["route"] = json!({});
        }
        wait_until(|| eval(&mut r, "document.getElementById('stopGuide').textContent") == json!("운행 정보 대기"));
        eval(&mut r, "followHeading=false;drawMap();true");
        wait_until(|| eval(&mut r, "Math.min(mapCamera.heading,360-mapCamera.heading)<1") == json!(true));
        finish(r);
    }
    #[test]
    #[ignore = "measures installed Edge capture throughput"]
    fn browser_capture_throughput() {
        let html = concat!(
            "<meta name=\"komsi-display-uv\" content=\"0,0.20368588,0.9999935,0.79631495\">",
            include_str!("../../../docs/examples/android-auto/index.html")
        );
        let mut r = fixture_html(1600, 1600, html);
        eval(
            &mut r,
            "const badge=document.createElement('div');badge.style='position:fixed;top:8px;right:8px;background:red;color:white;z-index:999';document.body.append(badge);setInterval(()=>badge.textContent=performance.now().toFixed(0),16);true",
        );
        let start = Instant::now();
        let first = r.shared.lock().unwrap().produced;
        std::thread::sleep(Duration::from_secs(6));
        let frames = r.shared.lock().unwrap().produced - first;
        let measured = frames as f64 / start.elapsed().as_secs_f64();
        let result = json!({"format":std::env::var("K_OMSI_AUTO_CAPTURE").unwrap_or("jpeg".into()),"targetFps":std::env::var("K_OMSI_AUTO_FPS").unwrap_or("30".into()),"measuredFps":measured,"frames":frames,"texture":[1600,1600],"visible":[1598,948],"scope":"synthetic browser capture only; not game FPS or YouTube playback"});
        println!("{result}");
        if let Ok(dir) = std::env::var("K_OMSI_AA_PREVIEW_DIR") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!(
                    "capture-performance-{}.json",
                    result["format"].as_str().unwrap()
                )),
                result.to_string(),
            )
            .unwrap();
        }
        assert!(frames > 0);
        click_id(&mut r, "railNav");
        assert_eq!(
            eval(&mut r, "document.getElementById('workspace').dataset.page"),
            json!("nav")
        );
        r.shared
            .lock()
            .unwrap()
            .num
            .insert("k_omsi_android_auto".into(), json!(0));
        std::thread::sleep(Duration::from_millis(500));
        let paused = r.shared.lock().unwrap().produced;
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            r.shared.lock().unwrap().produced > paused,
            "CCTV must retain the browser stream"
        );
        eval(
            &mut r,
            "show('youtube');window.keptPlayer=document.createElement('iframe');keptPlayer.id='video';keptPlayer.src='about:blank';document.getElementById('player').appendChild(keptPlayer);true",
        );
        click_id(&mut r, "camera");
        assert_eq!(
            eval(
                &mut r,
                "document.getElementById('video')===window.keptPlayer"
            ),
            json!(true)
        );
        eval(&mut r, "stopVideo();true");
        assert_eq!(
            eval(&mut r, "document.getElementById('video')===null"),
            json!(true)
        );
        finish(r);
    }
    #[test]
    #[ignore = "requires installed Edge"]
    fn browser_higer_uv_window_and_pointer() {
        let html = concat!(
            "<meta name=\"komsi-display-uv\" content=\"0,0.20368588,0.9999935,0.79631495\">",
            include_str!("../../../docs/examples/android-auto/index.html")
        );
        let mut r = fixture_html(1600, 1600, html);
        assert_eq!(r.rect, [0, 326, 1598, 948]);
        assert_eq!(eval(&mut r, "[innerWidth,innerHeight]"), json!([799, 474]));
        assert_eq!(
            eval(
                &mut r,
                "(()=>{const r=document.getElementById('camera').getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight})()"
            ),
            json!(true)
        );
        capture(&mut r, 1600, 1600, "higer-uv-home");
        click_id(&mut r, "railYoutube");
        assert_eq!(eval(&mut r, "(()=>{const ids=['camera','railHome','paste'];const buttons=[...ids.map(id=>document.getElementById(id)),document.querySelector('#youtube .closeButton')];return buttons.every(b=>{const r=b.getBoundingClientRect();return r.left>=8&&r.right<=innerWidth-32&&r.top>=8&&r.bottom<=innerHeight-24})})()"), json!(true));
        capture(&mut r, 1600, 1600, "higer-uv-youtube");
        click_id(&mut r, "railNav");
        assert_eq!(
            eval(&mut r, "document.getElementById('workspace').dataset.page"),
            json!("nav")
        );
        click_id(&mut r, "camera");
        wait_until(|| !r.shared.lock().unwrap().events.is_empty());
        finish(r);
    }
    #[test]
    #[ignore = "requires installed Edge"]
    fn browser_youtube_playback_diagnostic() {
        let url = std::env::var("K_OMSI_AUTO_TEST_YOUTUBE_URL").unwrap();
        let mut r = fixture(800, 480);
        assert_eq!(eval(&mut r, &format!("loadYouTube({})", json!(url))), json!(true));
        if std::env::var("K_OMSI_AUTO_TEST_STANDARD_EMBED").is_ok() {
            eval(&mut r, "const frame=document.getElementById('video');frame.src=frame.src.replace('www.youtube-nocookie.com','www.youtube.com');true");
        }
        wait_until(|| eval(&mut r, "!!youtubePlayer?.getVideoData?.().video_id") == json!(true));
        click_id(&mut r, "player");
        std::thread::sleep(Duration::from_secs(8));
        let result = eval(&mut r, "({status:document.getElementById('videoStatus').textContent,state:youtubePlayer?.getPlayerState?.(),title:youtubePlayer?.getVideoData?.().title,error:youtubePlayer?.getVideoData?.().errorCode})");
        eprintln!("YouTube diagnostic: {result}");
        if let Ok(code) = std::env::var("K_OMSI_AUTO_TEST_EXPECT_ERROR") {
            assert!(result["status"].as_str().unwrap().contains(&format!("({code})")));
        } else {
            assert_eq!(result["state"], json!(1), "{result}");
        }
        capture(&mut r, 800, 480, "youtube-diagnostic");
        finish(r);
    }
    #[test]
    #[ignore = "requires installed Edge"]
    fn browser_youtube_playlist_links() {
        let mut r = fixture(800, 480);
        for (link, path, list, index) in [
            ("https://www.youtube.com/watch?v=M7lc1UVf-VE&list=PLtest_123&index=3", "/embed/M7lc1UVf-VE", "PLtest_123", "3"),
            ("https://youtu.be/M7lc1UVf-VE?list=PLtest_123&index=2", "/embed/M7lc1UVf-VE", "PLtest_123", "2"),
            ("https://www.youtube.com/playlist?list=PLtest_123", "/embed/videoseries", "PLtest_123", ""),
            ("https://www.youtube.com/watch?v=M7lc1UVf-VE", "/embed/M7lc1UVf-VE", "", ""),
        ] {
            assert_eq!(eval(&mut r, &format!("loadYouTube({})", json!(link))), json!(true));
            assert_eq!(eval(&mut r, "(()=>{const u=new URL(document.getElementById('video').src);return [u.pathname,u.searchParams.get('list')||'',u.searchParams.get('index')||'']})()"), json!([path,list,index]));
        }
        for link in ["https://example.com/playlist?list=PLtest_123", "https://youtube.com/playlist?list=bad%26list", "https://youtube.com/watch?v=invalid"] {
            assert_eq!(eval(&mut r, &format!("loadYouTube({})", json!(link))), json!(false));
        }
        eval(&mut r, "stopVideo();true");
        assert_eq!(eval(&mut r, "document.getElementById('video')===null"), json!(true));
        finish(r);
    }
    #[test]
    #[ignore = "requires installed Edge"]
    fn browser_hidpi_frame_and_pointer() {
        let mut r = fixture(1600, 960);
        assert_eq!(
            eval(&mut r, "[innerWidth,innerHeight,devicePixelRatio]"),
            json!([800, 480, 2])
        );
        capture(&mut r, 1600, 960, "hidpi-home");
        click_id(&mut r, "railNav");
        assert_eq!(
            eval(&mut r, "document.getElementById('workspace').dataset.page"),
            json!("nav")
        );
        click_id(&mut r, "camera");
        wait_until(|| !r.shared.lock().unwrap().events.is_empty());
        assert_eq!(
            r.shared.lock().unwrap().events.last().unwrap().0,
            "k_omsi_android_auto"
        );
        finish(r);
    }
    #[test]
    #[ignore = "launches dedicated headless Edge profiles at all reference resolutions"]
    fn browser_launcher_map_pointer_and_cctv_smoke() {
        for (w, h) in [
            (748, 450),
            (800, 480),
            (880, 480),
            (1120, 480),
            (1280, 480),
            (790, 686),
            (800, 880),
            (1920, 584),
            (1600, 960),
        ] {
            let mut r = fixture(w, h);
            let state = eval(
                &mut r,
                "({page:document.getElementById('workspace').dataset.page,rail:getComputedStyle(document.getElementById('rail')).flexDirection,overflow:document.documentElement.scrollWidth>innerWidth||document.documentElement.scrollHeight>innerHeight})",
            );
            assert_eq!(state["page"], json!("home"));
            assert_eq!(state["overflow"], json!(false));
            assert_eq!(
                state["rail"],
                json!(
                    if w / display_scale(w, h) < 880 || (w as f64 / h as f64) <= 1.3 {
                        "row"
                    } else {
                        "column"
                    }
                )
            );
            capture(&mut r, w, h, "home");
            for (id, page) in [
                ("railApps", "apps"),
                ("railNav", "nav"),
                ("railSettings", "settings"),
                ("railYoutube", "youtube"),
            ] {
                click_id(&mut r, id);
                assert_eq!(
                    eval(&mut r, "document.getElementById('workspace').dataset.page"),
                    json!(page)
                );
                let map = eval(
                    &mut r,
                    "getComputedStyle(document.getElementById('mapPanel')).display!=='none'",
                );
                assert_eq!(
                    map,
                    json!(
                        page == "nav"
                            || ((page == "apps" || page == "settings")
                                && w / display_scale(w, h) >= 1280)
                    )
                );
                assert_eq!(
                    eval(
                        &mut r,
                        "(()=>{const r=document.getElementById('camera').getBoundingClientRect();return r.left>=0&&r.top>=0&&r.right<=innerWidth&&r.bottom<=innerHeight})()"
                    ),
                    json!(true)
                );
                capture(&mut r, w, h, page);
                if page == "settings" && w == 748 {
                    r.pointer(150.0, 250.0, PointerKind::Down);
                    r.pointer(150.0, 130.0, PointerKind::Move);
                    r.pointer(150.0, 130.0, PointerKind::Up);
                    std::thread::sleep(Duration::from_millis(300));
                    assert_eq!(
                        eval(
                            &mut r,
                            "document.querySelector('.settingsContent').scrollTop>0"
                        ),
                        json!(true)
                    );
                    assert_eq!(
                        eval(&mut r, "document.getElementById('headingToggle').checked"),
                        json!(true)
                    );
                }
            }
            assert_eq!(
                eval(&mut r, "loadYouTube('https://example.com/not-a-video')"),
                json!(false)
            );
            // Link handling and player teardown without contacting YouTube.
            assert_eq!(eval(&mut r, "loadYouTube('not a link')"), json!(false));
            if w == 800 && h == 480 {
                click_id(&mut r, "railSettings");
                click_id(&mut r, "headingToggle");
                assert_eq!(eval(&mut r, "followHeading"), json!(false));
                click_id(&mut r, "themeToggle");
                assert_eq!(eval(&mut r, "darkMap"), json!(false));
                click_id(&mut r, "railNav");
                capture(&mut r, w, h, "light-map");
                if let Ok(url) = std::env::var("K_OMSI_AUTO_TEST_YOUTUBE_URL") {
                    assert_eq!(
                        eval(&mut r, &format!("loadYouTube({})", json!(url))),
                        json!(true)
                    );
                    std::thread::sleep(Duration::from_secs(8));
                    capture(&mut r, w, h, "youtube-embed");
                    click_id(&mut r, "railHome");
                    assert_eq!(
                        eval(&mut r, "document.getElementById('video')===null"),
                        json!(true)
                    );
                }
            }
            let page_before_camera =
                eval(&mut r, "document.getElementById('workspace').dataset.page");
            click_id(&mut r, "camera");
            wait_until(|| {
                r.shared
                    .lock()
                    .unwrap()
                    .events
                    .iter()
                    .any(|(n, v)| n == "k_omsi_android_auto" && *v == 0.)
            });
            assert_eq!(
                eval(&mut r, "document.getElementById('workspace').dataset.page"),
                page_before_camera
            );
            finish(r);
        }
    }
}

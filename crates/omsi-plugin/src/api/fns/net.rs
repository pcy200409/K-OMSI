//! `omsi.ws`: a read-only WebSocket client for the public live feeds a plugin may listen to
//! (today OBIS, `wss://obis.kr/...`: the buses of the other players). It never sends
//! anything, only the hosts of [`ALLOWED_HOSTS`] are reachable and a plugin holds one
//! connection. The connection lives on its own thread, so the game never waits for the
//! network; a plugin drains the messages that came in with `omsi.ws.poll()`.
//!
//! `omsi.obis` (K-OMSI): the player's own bus to OBIS, as the OBIS client
//! does - `wss://obis.kr/api/ingest` with the device's token. The token never reaches the
//! plugin: it is read here from the OBIS client's `config.local.json` (next to the game) or
//! from `~/.openomsi/obis_device.json`, or the device is registered once and kept there.
//! Only this one address, only the latest snapshot the plugin gave, at most twice a second.
//!
//! These parts work only for a player who has the originals: OBIS (`omsi.ws`, `omsi.obis`)
//! with the OBIS client installed, the tachograph plugin with TACO (`omsi.komsi.original`).

use super::{def, multi, p};
use crate::api::fns::core::ok_or_reason;
use crate::api::{ApiError, ApiFn, Value};
use std::collections::VecDeque;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::{Error as WsError, Message, WebSocket};

const NEW: &str = crate::api::VERSION;

/// The only hosts `omsi.ws.open` connects to (wss, port 443).
const ALLOWED_HOSTS: &[&str] = &["obis.kr"];
/// Longest message taken in; a longer one ends the connection (it is opened again).
const MAX_MESSAGE: usize = 8 * 1024 * 1024;
/// Messages kept for `poll`; the oldest goes first (the feeds send whole snapshots).
const QUEUE_MAX: usize = 4;
/// Wait before a broken connection is opened again.
const RETRY: Duration = Duration::from_secs(5);

const IDLE: u8 = 0;
const CONNECTING: u8 = 1;
const OPEN: u8 = 2;
const WAITING: u8 = 3;

struct Shared {
    stop: AtomicBool,
    state: AtomicU8,
    queue: Mutex<VecDeque<String>>,
}

/// A plugin's `omsi.ws` connection.
#[derive(Default)]
pub struct WsClient {
    shared: Option<Arc<Shared>>,
}

impl Drop for WsClient {
    fn drop(&mut self) {
        self.close();
    }
}

impl WsClient {
    fn close(&mut self) {
        if let Some(s) = self.shared.take() {
            s.stop.store(true, Ordering::Relaxed);
        }
    }

    fn open(&mut self, url: &str) -> Result<(), String> {
        if !has_original("obis") {
            return Err(NO_OBIS.into());
        }
        check_url(url)?;
        self.close();
        let shared = Arc::new(Shared { stop: AtomicBool::new(false), state: AtomicU8::new(CONNECTING), queue: Mutex::new(VecDeque::new()) });
        let (s, u) = (shared.clone(), url.to_string());
        std::thread::Builder::new().name("plugin ws".into()).spawn(move || run(s, u)).map_err(|e| e.to_string())?;
        self.shared = Some(shared);
        Ok(())
    }

    fn poll(&self) -> Vec<Value> {
        let Some(s) = &self.shared else { return Vec::new() };
        let mut q = s.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.drain(..).map(Value::Str).collect()
    }

    fn state(&self) -> &'static str {
        match self.shared.as_ref().map_or(IDLE, |s| s.state.load(Ordering::Relaxed)) {
            CONNECTING => "connecting",
            OPEN => "open",
            WAITING => "waiting",
            _ => "closed",
        }
    }
}

// ------------------------------------------------------------------ the originals

/// An original program: OBIS's client (relative to an OMSI folder); TACO's programs - Light-TACO's
/// scheduler (`OmsiLightTaco`) or the digital tachograph, either will do - by their file names,
/// in the OMSI folder, in `OMSI Addon Patch` or one folder below it (where their packages put them).
fn original_files(name: &str) -> &'static [&'static str] {
    match name {
        "obis" => &["OBIS-client/OBIS-client.exe"],
        "taco" => &["OMSI2_Scheduler.exe", "OmsiDTG_Personal.exe"],
        _ => &[],
    }
}

/// The OMSI folders to look in: the one `--root` names, and the game's folder and those above
/// it (the game is `<OMSI>/OMSI Addon Patch/openOMSI/openomsi.exe`).
fn omsi_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let args: Vec<String> = std::env::args().collect();
    if let Some(r) = args.iter().position(|a| a == "--root").and_then(|i| args.get(i + 1)) {
        roots.push(PathBuf::from(r));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
        roots.extend(dir.ancestors().take(4).map(|p| p.to_path_buf()));
    }
    roots
}

fn find_original(name: &str) -> bool {
    let files = original_files(name);
    for root in omsi_roots() {
        if files.iter().any(|f| root.join(f).is_file()) {
            return true;
        }
        if name == "taco" {
            let addons = root.join("OMSI Addon Patch");
            let mut dirs = vec![addons.clone()];
            if let Ok(rd) = std::fs::read_dir(&addons) {
                dirs.extend(rd.flatten().map(|d| d.path()).filter(|p| p.is_dir()));
            }
            if dirs.iter().any(|d| files.iter().any(|f| d.join(f).is_file())) {
                return true;
            }
        }
    }
    false
}

/// Whether the player has the original (`"obis"`, `"taco"`); looked for once a session.
pub fn has_original(name: &str) -> bool {
    static FOUND: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
    let mut f = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, v)) = f.iter().find(|(n, _)| n == name) {
        return *v;
    }
    let v = std::env::var("OMSI_ORIGINALS").map(|s| s.split(',').any(|x| x.trim() == name)).unwrap_or_else(|_| find_original(name));
    log::info!("K-OMSI: the original {name} is {}", if v { "installed: its plugin parts are on" } else { "not installed: its plugin parts are off" });
    f.push((name.to_string(), v));
    v
}

// ------------------------------------------------------------ the OBIS client's routes

/// The OBIS client's folder (the one with `OBIS-client.exe`).
fn obis_client_dir() -> Option<PathBuf> {
    // (tests: OMSI_OBIS_CLIENT_DIR names it)
    if let Some(d) = std::env::var_os("OMSI_OBIS_CLIENT_DIR").map(PathBuf::from).filter(|d| d.is_dir()) {
        return Some(d);
    }
    omsi_roots().into_iter().map(|r| r.join("OBIS-client")).find(|d| d.join("OBIS-client.exe").is_file())
}

/// `routes/<file>` as the OBIS client keeps them: the name alone, a `.json` file.
fn route_file_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 200 && name.to_ascii_lowercase().ends_with(".json") && !name.contains(['/', '\\', ':']) && !name.contains("..")
}

/// The trip file a route is for, as the OBIS client matches it: its name, lower case, no `.ttp`.
fn norm_trip(name: &str) -> String {
    let s = name.trim().replace('\\', "/");
    let s = s.rsplit('/').next().unwrap_or("").to_lowercase();
    s.strip_suffix(".ttp").map(str::to_string).unwrap_or(s)
}

fn field<'a>(m: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    m.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// The index of the client's routes: `{default_map, routes = {key = {file, line, map, stops}},
/// trips = {trip file = {keys}}}` (what the plugin needs to match a trip to a route).
fn build_route_index(dir: &std::path::Path) -> Value {
    let mut maps: Vec<String> = std::fs::read_dir(dir.join("maps")).map(|rd| rd.flatten().filter(|d| d.path().is_dir()).map(|d| d.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    maps.sort();
    let mut files: Vec<String> = std::fs::read_dir(dir.join("data/routes")).map(|rd| rd.flatten().map(|d| d.file_name().to_string_lossy().into_owned()).filter(|n| route_file_ok(n)).collect()).unwrap_or_default();
    files.sort();
    let mut routes: Vec<(String, Value)> = Vec::new();
    let mut trips: Vec<(String, Value)> = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(dir.join("data/routes").join(&f)) else { continue };
        let Ok(Value::Map(r)) = crate::api::json::decode(&text, false) else { continue };
        let s = |k: &str| match field(&r, k) {
            Some(Value::Str(t)) => t.clone(),
            Some(Value::Int(n)) => n.to_string(),
            Some(Value::Num(n)) => n.to_string(),
            _ => String::new(),
        };
        let key = Some(s("routeKey")).filter(|k| !k.is_empty()).unwrap_or_else(|| f.trim_end_matches(".json").to_string());
        let stops: Vec<Value> = match field(&r, "stations") {
            Some(Value::List(l)) => l.iter().filter_map(|st| match st {
                Value::Map(m) => match field(m, "id") {
                    Some(Value::Int(n)) => Some(Value::Int(*n)),
                    Some(Value::Num(n)) => Some(Value::Int(*n as i64)),
                    _ => None,
                },
                _ => None,
            }).collect(),
            _ => Vec::new(),
        };
        let trip = norm_trip(&s("fileName"));
        if !trip.is_empty() {
            match trips.iter_mut().find(|(t, _)| *t == trip) {
                Some((_, Value::List(l))) => l.push(Value::Str(key.clone())),
                _ => trips.push((trip, Value::List(vec![Value::Str(key.clone())]))),
            }
        }
        routes.push((key, Value::Map(vec![
            ("file".into(), Value::Str(f)),
            ("line".into(), Value::Str(s("lineNumber"))),
            ("map".into(), Value::Str(s("mapId"))),
            ("stops".into(), Value::List(stops)),
        ])));
    }
    log::info!("plugin obis: {} routes of the OBIS client read ({})", routes.len(), dir.display());
    Value::Map(vec![
        ("default_map".into(), maps.first().map(|m| Value::Str(m.clone())).unwrap_or(Value::Nil)),
        ("routes".into(), Value::Map(routes)),
        ("trips".into(), Value::Map(trips)),
    ])
}

/// A map's stop names as the OBIS client has them (`maps/<map>/map.json`, its `stations`:
/// the map's name of a stop -> the Korean one): the folder named so, or the one whose
/// `mapId` it is.
fn map_names(map: &str) -> Value {
    let Some(dir) = obis_client_dir() else { return Value::Nil };
    let read = |p: &std::path::Path| -> Option<Vec<(String, Value)>> {
        match crate::api::json::decode(&std::fs::read_to_string(p).ok()?, false).ok()? {
            Value::Map(m) => Some(m),
            _ => None,
        }
    };
    let maps = dir.join("maps");
    let mut found = None;
    if !map.contains(['/', '\\', ':']) && !map.contains("..") {
        found = read(&maps.join(map).join("map.json"));
    }
    if found.is_none() {
        found = std::fs::read_dir(&maps).ok().and_then(|rd| {
            rd.flatten().filter_map(|d| read(&d.path().join("map.json"))).find(|m| matches!(field(m, "mapId"), Some(Value::Str(id)) if id == map))
        });
    }
    match found.and_then(|m| field(&m, "stations").cloned()) {
        Some(v @ Value::Map(_)) => v,
        _ => Value::Nil,
    }
}

/// The index, read on a worker the first time it is asked for (`nil` until it is there).
fn route_index() -> Value {
    static INDEX: Mutex<Option<Option<Value>>> = Mutex::new(None);
    let mut g = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    match &*g {
        Some(Some(v)) => return v.clone(),
        Some(None) => return Value::Nil,
        None => {}
    }
    let Some(dir) = obis_client_dir() else { return Value::Nil };
    *g = Some(None);
    drop(g);
    let _ = std::thread::Builder::new().name("plugin obis routes".into()).spawn(move || {
        let v = build_route_index(&dir);
        *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(v));
    });
    Value::Nil
}

const NO_OBIS: &str = "the OBIS client (OBIS-client\\OBIS-client.exe in the OMSI folder) is not installed";

/// `wss://<allowed host>/<path>`: no other scheme, no port, no user info.
fn check_url(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("wss://").ok_or("only wss:// addresses are allowed")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains(['@', ':']) {
        return Err("the address must not have user info or a port".into());
    }
    if !ALLOWED_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        return Err(format!("{host} is not an allowed host (allowed: {})", ALLOWED_HOSTS.join(", ")));
    }
    Ok(())
}

fn connect(url: &str, token: Option<&str>) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let cfg = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    if let Some(t) = token {
        req.headers_mut().insert("X-Device-Token", HeaderValue::from_str(t).map_err(|e| e.to_string())?);
    }
    // (no redirects: the host must stay the checked one)
    let (mut ws, _) = tungstenite::client::connect_with_config(req, Some(cfg), 0).map_err(|e| e.to_string())?;
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(Duration::from_millis(200))),
        MaybeTlsStream::Rustls(s) => s.get_mut().set_read_timeout(Some(Duration::from_millis(200))),
        _ => Ok(()),
    }
    .map_err(|e| e.to_string())?;
    Ok(ws)
}

fn sleep_unless_stopped(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::Relaxed) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run(s: Arc<Shared>, url: String) {
    while !s.stop.load(Ordering::Relaxed) {
        s.state.store(CONNECTING, Ordering::Relaxed);
        match connect(&url, None) {
            Ok(mut ws) => {
                log::info!("plugin ws: connected to {url}");
                s.state.store(OPEN, Ordering::Relaxed);
                while !s.stop.load(Ordering::Relaxed) {
                    match ws.read() {
                        Ok(Message::Text(t)) => {
                            let mut q = s.queue.lock().unwrap_or_else(|e| e.into_inner());
                            if q.len() >= QUEUE_MAX {
                                q.pop_front();
                            }
                            q.push_back(t.as_str().to_string());
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(WsError::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                        Err(e) => {
                            log::warn!("plugin ws: {url}: {e}");
                            break;
                        }
                    }
                    // answers pings the read queued
                    let _ = ws.flush();
                }
                let _ = ws.close(None);
            }
            Err(e) => log::warn!("plugin ws: {url}: {e}"),
        }
        s.state.store(WAITING, Ordering::Relaxed);
        sleep_unless_stopped(&s.stop, RETRY);
    }
    s.state.store(IDLE, Ordering::Relaxed);
}

// ------------------------------------------------------------------ omsi.obis (sending)

/// Where the player's bus goes, and where a new device registers.
const INGEST_URL: &str = "wss://obis.kr/api/ingest";
const REGISTER_URL: &str = "https://obis.kr/api/register";
/// The device's vehicle settings on OBIS (what the OBIS client's "차량 설정" edits).
const PREFS_URL: &str = "https://obis.kr/api/me/prefs";
/// Largest snapshot taken; the OBIS client's are a few kB (the route's stations).
const MAX_SNAPSHOT: usize = 256 * 1024;
/// Least time between two snapshots sent.
const SEND_GAP: Duration = Duration::from_millis(500);

const NO_TOKEN: u8 = 4;

struct SendShared {
    stop: AtomicBool,
    state: AtomicU8,
    /// The latest snapshot not sent yet (a newer one replaces it).
    latest: Mutex<Option<String>>,
    sent: AtomicU64,
    source: Mutex<&'static str>,
    /// The vehicle settings as OBIS has them (JSON object text), once read.
    prefs: Mutex<Option<String>>,
    /// Settings to save (`{"prefs": {...}}` text), saved by the sending thread.
    save: Mutex<Option<String>>,
}

/// A plugin's `omsi.obis` connection.
#[derive(Default)]
pub struct ObisSender {
    shared: Option<Arc<SendShared>>,
}

impl Drop for ObisSender {
    fn drop(&mut self) {
        self.stop();
    }
}

impl ObisSender {
    fn stop(&mut self) {
        if let Some(s) = self.shared.take() {
            s.stop.store(true, Ordering::Relaxed);
        }
    }

    fn start(&mut self) -> Result<(), String> {
        if !has_original("obis") {
            return Err(NO_OBIS.into());
        }
        if self.shared.is_some() {
            return Ok(());
        }
        let shared = Arc::new(SendShared { stop: AtomicBool::new(false), state: AtomicU8::new(CONNECTING), latest: Mutex::new(None), sent: AtomicU64::new(0), source: Mutex::new(""), prefs: Mutex::new(None), save: Mutex::new(None) });
        let s = shared.clone();
        std::thread::Builder::new().name("plugin obis".into()).spawn(move || run_sender(s)).map_err(|e| e.to_string())?;
        self.shared = Some(shared);
        Ok(())
    }

    fn send(&self, text: String) -> Result<(), String> {
        let Some(s) = &self.shared else { return Err("not started: call omsi.obis.start() first".into()) };
        if text.len() > MAX_SNAPSHOT {
            return Err(format!("a snapshot is at most {MAX_SNAPSHOT} bytes"));
        }
        *s.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
        Ok(())
    }

    fn state(&self) -> &'static str {
        match self.shared.as_ref().map_or(IDLE, |s| s.state.load(Ordering::Relaxed)) {
            CONNECTING => "connecting",
            OPEN => "open",
            WAITING => "waiting",
            NO_TOKEN => "no_token",
            _ => "closed",
        }
    }

    fn prefs(&self) -> Value {
        let Some(s) = &self.shared else { return Value::Nil };
        let text = s.prefs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        text.and_then(|t| crate::api::json::decode(&t, false).ok()).unwrap_or(Value::Nil)
    }

    fn set_prefs(&self, v: &Value) -> Result<(), String> {
        let Some(s) = &self.shared else { return Err("not started: call omsi.obis.start() first".into()) };
        let Value::Map(m) = v else { return Err("the settings are a table".into()) };
        let mut out: Vec<(String, Value)> = Vec::new();
        for (k, v) in m {
            let ok = match (k.as_str(), v) {
                ("nickname", Value::Str(t)) => t.chars().count() <= 20,
                ("crowdType", Value::Str(t)) => t == "seat" || t == "stand",
                ("vehicleBody", Value::Str(t)) => t == "normal" || t == "night",
                ("seatCapacity", Value::Int(n)) => (1..=80).contains(n),
                ("seatCapacity", Value::Num(n)) => n.fract() == 0.0 && (1.0..=80.0).contains(n),
                ("showStandCrowd" | "lowFloor", Value::Bool(_)) => true,
                _ => return Err(format!("{k}: not a setting OBIS takes, or a wrong value")),
            };
            if !ok {
                return Err(format!("{k}: a wrong value"));
            }
            let v = match v {
                Value::Num(n) => Value::Int(*n as i64),
                v => v.clone(),
            };
            out.push((k.clone(), v));
        }
        let body = crate::api::json::encode(&Value::Map(vec![("prefs".into(), Value::Map(out))]));
        *s.save.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
        Ok(())
    }

    fn info(&self) -> (f64, &'static str) {
        self.shared.as_ref().map_or((0.0, ""), |s| (s.sent.load(Ordering::Relaxed) as f64, *s.source.lock().unwrap_or_else(|e| e.into_inner())))
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// A text field of a small JSON object (the token files are flat).
fn json_str(text: &str, key: &str) -> Option<String> {
    match crate::api::json::decode(text, false).ok()? {
        Value::Map(m) => m.into_iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
            Value::Str(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        }),
        _ => None,
    }
}

/// The OBIS client's own config, `OBIS-client/config.local.json` of the OMSI folder: the one
/// `--root` names, or a folder above the game (`<OMSI>/OMSI Addon Patch/openOMSI/openomsi.exe`).
fn obis_client_configs() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let args: Vec<String> = std::env::args().collect();
    if let Some(r) = args.iter().position(|a| a == "--root").and_then(|i| args.get(i + 1)) {
        roots.push(PathBuf::from(r));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
        roots.extend(dir.ancestors().take(4).map(|p| p.to_path_buf()));
    }
    roots.into_iter().flat_map(|r| [r.join("OBIS-client/config.local.json"), r.join("OBIS-client/_internal/client/config.local.json")]).collect()
}

fn own_device_file() -> Option<PathBuf> {
    home().map(|h| h.join(".openomsi/obis_device.json"))
}

/// The device token and where it came from; registers a new device when there is none.
fn device_token() -> Result<(String, &'static str), String> {
    for f in obis_client_configs() {
        if let Some(t) = std::fs::read_to_string(&f).ok().and_then(|t| json_str(&t, "deviceToken")) {
            return Ok((t, "obis_client"));
        }
    }
    let own = own_device_file().ok_or("no home folder")?;
    let kept = std::fs::read_to_string(&own).unwrap_or_default();
    if let Some(t) = json_str(&kept, "deviceToken") {
        return Ok((t, "komsi"));
    }
    // a new device, registered as the OBIS client does: POST /api/register {deviceId}
    let id = json_str(&kept, "deviceId").unwrap_or_else(new_device_id);
    let body = crate::api::json::encode(&Value::Map(vec![("deviceId".into(), Value::Str(id.clone()))]));
    let answer = ureq::post(REGISTER_URL)
        .timeout(Duration::from_secs(10))
        .set("X-Device-Id", &id)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| format!("register: {e}"))?
        .into_string()
        .map_err(|e| format!("register: {e}"))?;
    let token = json_str(&answer, "deviceToken").ok_or("register: the answer has no deviceToken")?;
    let keep = Value::Map(vec![
        ("deviceId".into(), Value::Str(id)),
        ("deviceToken".into(), Value::Str(token.clone())),
        ("recoveryCode".into(), Value::Str(json_str(&answer, "recoveryCode").unwrap_or_default())),
    ]);
    if let Some(dir) = own.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&own, crate::api::json::encode_pretty(&keep)).map_err(|e| format!("{}: {e}", own.display()))?;
    log::info!("plugin obis: registered a new OBIS device ({})", own.display());
    Ok((token, "komsi_registered"))
}

/// A random UUID (version 4), from the randomly keyed hasher of the standard library.
fn new_device_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut b = [0u8; 16];
    for (i, chunk) in b.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) ^ i as u128);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// The settings OBIS has for the device (GET), or saves them (PUT `{"prefs": ...}`); the
/// answer's `prefs` object as JSON text.
fn prefs_call(token: &str, save: Option<&str>) -> Result<String, String> {
    let url = std::env::var("OMSI_OBIS_PREFS_URL").unwrap_or_else(|_| PREFS_URL.to_string());
    let req = |m: &str| ureq::request(m, &url).timeout(Duration::from_secs(10)).set("X-Device-Token", token);
    let r = match save {
        Some(body) => req("PUT").set("Content-Type", "application/json").send_string(body),
        None => req("GET").call(),
    };
    let text = r.map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
    match crate::api::json::decode(&text, false).map_err(|e| e.to_string())? {
        Value::Map(m) => m.into_iter().find(|(k, _)| k == "prefs").map(|(_, v)| crate::api::json::encode(&v)).ok_or_else(|| "no prefs in the answer".into()),
        _ => Err("not a JSON object".into()),
    }
}

fn run_sender(s: Arc<SendShared>) {
    // (tests and debugging only: another server and token, e.g. a local stand-in of OBIS)
    let url = std::env::var("OMSI_OBIS_INGEST_URL").unwrap_or_else(|_| INGEST_URL.to_string());
    let test_token = std::env::var("OMSI_OBIS_TOKEN").ok();
    let token = loop {
        if s.stop.load(Ordering::Relaxed) {
            s.state.store(IDLE, Ordering::Relaxed);
            return;
        }
        match test_token.clone().map(|t| Ok((t, "env"))).unwrap_or_else(device_token) {
            Ok((t, src)) => {
                *s.source.lock().unwrap_or_else(|e| e.into_inner()) = src;
                break t;
            }
            Err(e) => {
                log::warn!("plugin obis: no device token: {e}");
                s.state.store(NO_TOKEN, Ordering::Relaxed);
                sleep_unless_stopped(&s.stop, Duration::from_secs(30));
            }
        }
    };
    match prefs_call(&token, None) {
        Ok(p) => *s.prefs.lock().unwrap_or_else(|e| e.into_inner()) = Some(p),
        Err(e) => log::warn!("plugin obis: reading the vehicle settings: {e}"),
    }
    while !s.stop.load(Ordering::Relaxed) {
        s.state.store(CONNECTING, Ordering::Relaxed);
        match connect(&url, Some(&token)) {
            Ok(mut ws) => {
                log::info!("plugin obis: sending to {url}");
                s.state.store(OPEN, Ordering::Relaxed);
                let mut last: Option<Instant> = None;
                while !s.stop.load(Ordering::Relaxed) {
                    let save = s.save.lock().unwrap_or_else(|e| e.into_inner()).take();
                    if let Some(body) = save {
                        match prefs_call(&token, Some(&body)) {
                            Ok(p) => {
                                log::info!("plugin obis: vehicle settings saved");
                                *s.prefs.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
                            }
                            Err(e) => log::warn!("plugin obis: saving the vehicle settings: {e}"),
                        }
                    }
                    if last.is_none_or(|t| t.elapsed() >= SEND_GAP) {
                        let next = s.latest.lock().unwrap_or_else(|e| e.into_inner()).take();
                        if let Some(text) = next {
                            if let Err(e) = ws.send(Message::text(text)) {
                                log::warn!("plugin obis: {e}");
                                break;
                            }
                            s.sent.fetch_add(1, Ordering::Relaxed);
                            last = Some(Instant::now());
                        }
                    }
                    // (the server's pings, and its close)
                    match ws.read() {
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(WsError::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                        Err(e) => {
                            log::warn!("plugin obis: {e}");
                            break;
                        }
                    }
                    let _ = ws.flush();
                }
                let _ = ws.close(None);
            }
            Err(e) => log::warn!("plugin obis: {url}: {e}"),
        }
        s.state.store(WAITING, Ordering::Relaxed);
        sleep_unless_stopped(&s.stop, RETRY);
    }
    s.state.store(IDLE, Ordering::Relaxed);
}

pub static FNS: &[ApiFn] = &[
    def!("komsi.original", "network", [p("name", "string")], "boolean", "Whether the player has the original program a part of K-OMSI's OBIS edition stands in for: `\"obis\"` (the OBIS client, `OBIS-client\\OBIS-client.exe` in the OMSI folder: without it `omsi.ws` and `omsi.obis` do nothing) or `\"taco\"` (Light-TACO's `OMSI2_Scheduler.exe` or the tachograph's `OmsiDTG_Personal.exe`, in or under `OMSI Addon Patch`: the tachograph plugin works only with one of them).", NEW, None, false, |c, a| {
        let name = a.str(0)?;
        Ok::<_, ApiError>(Value::Bool(has_original(&name)))
    }),
    def!("ws.open", "network", [p("url", "string")], "true, or false and the reason", "Starts listening to a public live feed over a WebSocket, read-only: the plugin never sends. Only `wss://obis.kr/...` (OBIS, the buses of the other players) is allowed, no port and no other host. The connection runs in the background and is opened again every 5 seconds after a break; opening again replaces the old one. Read what came in with `omsi.ws.poll`.", NEW, NetworkRemote, true, |c, a| {
        let url = a.str(0)?;
        Ok::<_, ApiError>(ok_or_reason(c.state().ws.open(&url)))
    }),
    def!("ws.poll", "network", [], "list of strings", "The text messages that came in since the last call, oldest first (at most the last 4: a feed sends whole snapshots). Empty without a connection.", NEW, NetworkRemote, false, |c, a| {
        Value::List(c.state().ws.poll())
    }),
    def!("ws.state", "network", [], "string", "`\"connecting\"`, `\"open\"`, `\"waiting\"` (broken, opened again soon) or `\"closed\"` (never opened, or `omsi.ws.close`).", NEW, NetworkRemote, false, |c, a| {
        c.state().ws.state()
    }),
    def!("ws.close", "network", [], "nil", "Ends the connection.", NEW, NetworkRemote, false, |c, a| {
        c.state().ws.close();
    }),
    def!("obis.start", "network", [], "true, or false and the reason", "Starts sending the player's bus to OBIS (`wss://obis.kr/api/ingest`) in the background, as the OBIS client does. The device's token is the OBIS client's (its `config.local.json` next to the game) or K-OMSI's own (`~/.openomsi/obis_device.json`, registered once when there is none); the plugin never sees it. Give the snapshots with `omsi.obis.send`.", NEW, NetworkRemote, true, |c, a| {
        Ok::<_, ApiError>(ok_or_reason(c.state().obis.start()))
    }),
    def!("obis.send", "network", [p("snapshot", "string")], "true, or false and the reason", "Hands OBIS the state of the player's bus: the OBIS client's snapshot as JSON text, at most 256 kB. Only the newest is sent, at most twice a second; one not sent yet is replaced.", NEW, NetworkRemote, true, |c, a| {
        let text = a.str(0)?;
        Ok::<_, ApiError>(ok_or_reason(c.state().obis.send(text)))
    }),
    def!("obis.state", "network", [], "state, sent, token", "The sending: `\"connecting\"`, `\"open\"`, `\"waiting\"` (broken, opened again soon), `\"no_token\"` (no token and registering failed: tried again soon) or `\"closed\"`; how many snapshots went; where the token came from (`\"obis_client\"`, `\"komsi\"`, `\"komsi_registered\"`; empty before it is known).", NEW, NetworkRemote, false, |c, a| {
        let st = c.state().obis.state();
        let (sent, src) = c.state().obis.info();
        multi(Some([Value::Str(st.into()), Value::Num(sent), Value::Str(src.into())]))
    }),
    def!("obis.stop", "network", [], "nil", "Stops sending (OBIS shows the bus as gone after a while).", NEW, NetworkRemote, false, |c, a| {
        c.state().obis.stop();
    }),
    def!("obis.routes", "network", [], "table or nil", "The OBIS client's routes, to match the player's trip to one: `{default_map, routes = {[key] = {file, line, map, stops}}, trips = {[trip file] = {keys}}}` (`stops` the stop object ids in order; a trip file lower case, without `.ttp`). Read from the client's `data\\routes` on a worker the first time: `nil` until then, and without the OBIS client.", NEW, NetworkRemote, false, |c, a| {
        if has_original("obis") { route_index() } else { Value::Nil }
    }),
    def!("obis.route_text", "network", [p("file", "string")], "string or nil", "One route of the OBIS client (a `file` of `obis.routes`) as its JSON text.", NEW, NetworkRemote, false, |c, a| {
        let name = a.str(0)?;
        let text = (has_original("obis") && route_file_ok(&name)).then(obis_client_dir).flatten().and_then(|d| std::fs::read_to_string(d.join("data/routes").join(&name)).ok());
        Ok::<_, ApiError>(text.map(Value::Str).unwrap_or(Value::Nil))
    }),
    def!("obis.map_names", "network", [p("map", "string")], "table or nil", "A map's stop names as the OBIS client has them: the map's name of a stop -> the Korean one (its `maps\\<map>\\map.json`). `map` is a route's `map` of `obis.routes`. `nil` without the OBIS client or the map.", NEW, NetworkRemote, false, |c, a| {
        let map = a.str(0)?;
        Ok::<_, ApiError>(if has_original("obis") { map_names(&map) } else { Value::Nil })
    }),
    def!("obis.prefs", "network", [], "table or nil", "The vehicle settings OBIS has for this device, as the OBIS client's \"차량 설정\" shows them: `nickname`, `crowdType` (`\"seat\"`, `\"stand\"`), `seatCapacity` (1 to 80), `showStandCrowd`, `lowFloor`, `vehicleBody` (`\"normal\"`, `\"night\"`: the last bus). `nil` before they are read (after `obis.start`).", NEW, NetworkRemote, false, |c, a| {
        c.state().obis.prefs()
    }),
    def!("obis.set_prefs", "network", [p("prefs", "table")], "true, or false and the reason", "Saves vehicle settings on OBIS (the keys of `obis.prefs`, any of them), in the background; `obis.prefs` has them once OBIS took them.", NEW, NetworkRemote, false, |c, a| {
        let v = a.get(0).clone();
        Ok::<_, ApiError>(ok_or_reason(c.state().obis.set_prefs(&v)))
    }),
];

#[cfg(test)]
mod tests {
    use super::check_url;

    #[test]
    fn only_the_listed_host_over_wss() {
        assert!(check_url("wss://obis.kr/api/ws").is_ok());
        assert!(check_url("wss://OBIS.kr/api/ws").is_ok());
        assert!(check_url("ws://obis.kr/api/ws").is_err());
        assert!(check_url("wss://obis.kr:8443/api/ws").is_err());
        assert!(check_url("wss://obis.kr@evil.example/api/ws").is_err());
        assert!(check_url("wss://evil.example/obis.kr").is_err());
        assert!(check_url("wss://obis.kr.evil.example/ws").is_err());
        assert!(check_url("https://obis.kr/").is_err());
    }
}

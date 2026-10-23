//! `omsi.ws`: a read-only WebSocket client for the public live feeds a plugin may listen to
//! (today OBIS, `wss://obis.kr/...`: the buses of the other players). It never sends
//! anything, only the hosts of [`ALLOWED_HOSTS`] are reachable and a plugin holds one
//! connection. The connection lives on its own thread, so the game never waits for the
//! network; a plugin drains the messages that came in with `omsi.ws.poll()`.
//!
//! `omsi.obis` (K-OMSI's OBIS edition): the player's own bus to OBIS, as the OBIS client
//! does - `wss://obis.kr/api/ingest` with the device's token. The token never reaches the
//! plugin: it is read here from the OBIS client's `config.local.json` (next to the game) or
//! from `~/.openomsi/obis_device.json`, or the device is registered once and kept there.
//! Only this one address, only the latest snapshot the plugin gave, at most twice a second.

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

//! `omsi.ws`: a read-only WebSocket client for the public live feeds a plugin may listen to
//! (today OBIS, `wss://obis.kr/...`: the buses of the other players). It never sends
//! anything, only the hosts of [`ALLOWED_HOSTS`] are reachable and a plugin holds one
//! connection. The connection lives on its own thread, so the game never waits for the
//! network; a plugin drains the messages that came in with `omsi.ws.poll()`.
//!
//! `omsi.obis` (K-OMSI): the player's own bus to OBIS, as the OBIS client
//! does - `wss://obis.kr/api/ingest` with the device's token. The token never reaches the
//! plugin: it is read here from the OBIS client's `config.local.json` (next to the game), and
//! K-OMSI never registers a device of its own: without the client's token nothing is sent.
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

/// The folders of routes, newest first: OBIS's pack as K-OMSI fetched it, the client's
/// `data/routes` (as fresh as the client's last run).
fn route_dirs(client: &std::path::Path, pack: Option<&std::path::Path>) -> Vec<PathBuf> {
    pack.map(|p| p.join("routes")).into_iter().chain([client.join("data/routes")]).collect()
}

/// The folders of maps' data (`<map>/map.json`), newest first, as [`route_dirs`].
fn map_dirs(client: &std::path::Path, pack: Option<&std::path::Path>) -> Vec<PathBuf> {
    pack.map(|p| p.join("maps")).into_iter().chain([client.join("maps")]).collect()
}

/// The index of the client's routes: `{default_map, routes = {key = {file, line, map, stops}},
/// trips = {trip file = {keys}}}` (what the plugin needs to match a trip to a route).
fn build_route_index(dir: &std::path::Path, pack: Option<&std::path::Path>) -> Value {
    let mut maps: Vec<String> = map_dirs(dir, pack).iter().flat_map(|m| std::fs::read_dir(m).map(|rd| rd.flatten().filter(|d| d.path().is_dir()).map(|d| d.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>()).unwrap_or_default()).collect();
    maps.sort();
    maps.dedup();
    let names = |d: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d).map(|rd| rd.flatten().map(|d| d.file_name().to_string_lossy().into_owned()).filter(|n| route_file_ok(n)).collect()).unwrap_or_default();
        v.sort();
        v
    };
    // OBIS's pack first, the client's own next, then those fetched from OBIS for a line neither has
    let mut files: Vec<(PathBuf, String)> = route_dirs(dir, pack).into_iter().flat_map(|d| names(&d).into_iter().map(move |f| (d.clone(), f))).collect();
    if let Some(fetched) = fetched_dir() {
        files.extend(names(&fetched).into_iter().map(|f| (fetched.clone(), f)));
    }
    let mut routes: Vec<(String, Value)> = Vec::new();
    let mut trips: Vec<(String, Value)> = Vec::new();
    for (folder, f) in files {
        let Ok(text) = std::fs::read_to_string(folder.join(&f)) else { continue };
        let Ok(Value::Map(r)) = crate::api::json::decode(&text, false) else { continue };
        let s = |k: &str| match field(&r, k) {
            Some(Value::Str(t)) => t.clone(),
            Some(Value::Int(n)) => n.to_string(),
            Some(Value::Num(n)) => n.to_string(),
            _ => String::new(),
        };
        let key = Some(s("routeKey")).filter(|k| !k.is_empty()).unwrap_or_else(|| f.trim_end_matches(".json").to_string());
        if routes.iter().any(|(k, _)| *k == key) {
            continue; // (an older copy of a route already read)
        }
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
    log::info!("plugin obis: {} routes read (OBIS's pack, the OBIS client's {}, those fetched from OBIS)", routes.len(), dir.display());
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
    let mut found = None;
    for maps in map_dirs(&dir, pack_dir().as_deref()) {
        if !map.contains(['/', '\\', ':']) && !map.contains("..") {
            found = read(&maps.join(map).join("map.json"));
        }
        if found.is_none() {
            found = std::fs::read_dir(&maps).ok().and_then(|rd| {
                rd.flatten().filter_map(|d| read(&d.path().join("map.json"))).find(|m| matches!(field(m, "mapId"), Some(Value::Str(id)) if id == map))
            });
        }
        if found.is_some() {
            break;
        }
    }
    match found.and_then(|m| field(&m, "stations").cloned()) {
        Some(v @ Value::Map(_)) => v,
        _ => Value::Nil,
    }
}

/// The index: not read yet (`None`), being read (`Some(None)`), read.
static INDEX: Mutex<Option<Option<Value>>> = Mutex::new(None);

/// The index, read on a worker the first time it is asked for (`nil` until it is there).
fn route_index() -> Value {
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
        sync_pack();
        let v = build_route_index(&dir, pack_dir().as_deref());
        *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(v));
    });
    Value::Nil
}

// ------------------------------------------------------------ OBIS's route pack

/// OBIS hands its clients all its routes as one pack (OBIS client 1.2+): `/version` its hash,
/// `/pack` the pack (gzip, `{hash, files = {"routes/<key>.json" | "maps/<map>/map.json" = text}}`).
const PACK_URL: &str = "https://obis.kr/api/client/routes";
/// Largest pack taken, unpacked (today's is about 2.5 MB).
const PACK_MAX: u64 = 64 * 1024 * 1024;
/// The pack's own hash next to its folders, as the OBIS client keeps it.
const PACK_META: &str = "routes_pack.json";

/// Where K-OMSI keeps OBIS's pack: `~/.openomsi/obis_pack` (`routes/`, `maps/`, its hash).
fn pack_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("OMSI_OBIS_PACK_DIR") {
        return Some(PathBuf::from(d)); // (tests)
    }
    home().map(|h| h.join(".openomsi/obis_pack"))
}

/// The hash of the pack kept in `dir`.
fn pack_hash(dir: &std::path::Path) -> Option<String> {
    json_str(&std::fs::read_to_string(dir.join(PACK_META)).ok()?, "hash")
}

/// A file of the pack as the OBIS client takes it: under `routes/` or `maps/`, a `.json`, no
/// way out of the folder.
fn pack_path_ok(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts.len() >= 2
        && ["routes", "maps"].contains(&parts[0])
        && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != "..")
        && !rel.contains(['\\', ':'])
        && rel.ends_with(".json")
}

/// The pack's files, once its paths and hash are checked: sha256 over the paths in order,
/// each `path \0 text \0` (as the OBIS client checks it).
fn parse_pack(body: &[u8], expected: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut text = Vec::new();
    flate2::read::GzDecoder::new(body).take(PACK_MAX + 1).read_to_end(&mut text).map_err(|e| format!("the pack is not gzip: {e}"))?;
    if text.len() as u64 > PACK_MAX {
        return Err("the pack is too large".into());
    }
    let v: serde_json::Value = serde_json::from_slice(&text).map_err(|e| format!("the pack is not JSON: {e}"))?;
    let files: std::collections::BTreeMap<String, String> = match v.get("files") {
        Some(serde_json::Value::Object(m)) => m.iter().map(|(k, v)| v.as_str().map(|t| (k.clone(), t.to_string())).ok_or_else(|| format!("{k} is not text"))).collect::<Result<_, _>>()?,
        _ => return Err("the pack has no files".into()),
    };
    if !files.keys().any(|k| k.starts_with("routes/")) {
        return Err("the pack has no routes".into());
    }
    let mut h = Sha256::new();
    for (rel, t) in &files {
        if !pack_path_ok(rel) {
            return Err(format!("a path not allowed in the pack: {rel:?}"));
        }
        h.update(rel.as_bytes());
        h.update(b"\0");
        h.update(t.as_bytes());
        h.update(b"\0");
    }
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !got.eq_ignore_ascii_case(expected.trim()) {
        return Err("the pack's hash does not match".into());
    }
    Ok(files)
}

/// Puts the pack in `dir`: all written next to it first, then the folders swapped, so a pack
/// cut short leaves the one before.
fn install_pack(dir: &std::path::Path, files: &std::collections::BTreeMap<String, String>, hash: &str) -> Result<(), String> {
    let name = dir.file_name().ok_or("no pack folder")?.to_string_lossy().into_owned();
    let stage = dir.with_file_name(format!("{name}.stage"));
    let old = dir.with_file_name(format!("{name}.old"));
    let _ = std::fs::remove_dir_all(&stage);
    for (rel, text) in files {
        let p = stage.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
    }
    std::fs::write(stage.join(PACK_META), serde_json::json!({ "hash": hash }).to_string()).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    let had = dir.exists();
    if had {
        std::fs::rename(dir, &old).map_err(|e| format!("the old pack not moved: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&stage, dir) {
        if had {
            let _ = std::fs::rename(&old, dir);
        }
        return Err(format!("the new pack not put in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Brings `dir` to OBIS's pack (`base` its address): asks the hash, fetches only a new one.
/// Whether it was replaced.
fn sync_pack_from(base: &str, token: &str, dir: &std::path::Path) -> Result<bool, String> {
    let get = |path: &str| {
        ureq::get(&format!("{base}/{path}")).timeout(Duration::from_secs(20)).set("X-Device-Token", token).call().map_err(|e| match e {
            ureq::Error::Status(401 | 403, _) => "OBIS did not take the OBIS client's token".to_string(),
            ureq::Error::Status(404, _) => "OBIS hands out no route pack (an older server)".to_string(),
            e => e.to_string(),
        })
    };
    let version = get("version")?.into_string().map_err(|e| e.to_string())?;
    let hash = json_str(&version, "hash").ok_or("OBIS gave no pack hash")?;
    if pack_hash(dir).is_some_and(|h| h.eq_ignore_ascii_case(&hash)) {
        return Ok(false);
    }
    let r = get("pack")?;
    let expected = r.header("X-Pack-Hash").map(str::to_string).unwrap_or_else(|| hash.clone());
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(r.into_reader(), PACK_MAX + 1), &mut body).map_err(|e| e.to_string())?;
    let files = parse_pack(&body, &expected)?;
    install_pack(dir, &files, &expected)?;
    Ok(true)
}

/// Fetches OBIS's pack once a session (on the index's worker), so the routes are OBIS's latest
/// without the OBIS client being run; on any failure the pack kept and the client's routes stay.
fn sync_pack() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(dir) = pack_dir() else { return };
    // (tests: OMSI_OBIS_PACK_URL a local server, OMSI_OBIS_TOKEN its token)
    let base = std::env::var("OMSI_OBIS_PACK_URL").unwrap_or_else(|_| PACK_URL.to_string());
    let token = match std::env::var("OMSI_OBIS_TOKEN").map_err(|_| ()).or_else(|_| device_token().map(|(t, _)| t)) {
        Ok(t) => t,
        Err(e) => {
            log::info!("plugin obis: OBIS's route pack not fetched: {e}");
            return;
        }
    };
    match sync_pack_from(&base, &token, &dir) {
        Ok(true) => log::info!("plugin obis: OBIS's route pack fetched ({})", dir.display()),
        Ok(false) => log::info!("plugin obis: OBIS's route pack is up to date"),
        Err(e) => log::warn!("plugin obis: OBIS's route pack not fetched ({e}): the routes kept are used"),
    }
}

// ------------------------------------------------------------ routes the client lacks

const ROUTES_URL: &str = "https://obis.kr/api/routes";
/// Between two requests to OBIS (it answers too many with 429).
const FETCH_GAP: Duration = Duration::from_millis(600);
/// A fetched route older than this is fetched again.
const FETCH_KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
/// At most this many routes of one line number (its directions, on each map that has one so numbered).
const FETCH_MAX: usize = 12;

/// Where the routes fetched from OBIS are kept: `~/.openomsi/obis_routes`.
fn fetched_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("OMSI_OBIS_FETCHED_DIR") {
        return Some(PathBuf::from(d)); // (tests)
    }
    home().map(|h| h.join(".openomsi/obis_routes"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// A route's file: OBIS's pack, the client's `data/routes`, else the fetched one.
fn route_file(name: &str) -> Option<PathBuf> {
    if !route_file_ok(name) {
        return None;
    }
    let own = obis_client_dir().and_then(|d| route_dirs(&d, pack_dir().as_deref()).into_iter().map(|r| r.join(name)).find(|p| p.is_file()));
    own.or_else(|| fetched_dir().map(|d| d.join(name)).filter(|p| p.is_file()))
}

/// A route key as it may go into the address and a file name.
fn route_key_ok(key: &str) -> bool {
    !key.is_empty() && key.len() <= 120 && !key.contains("..") && key.chars().all(|c| c.is_alphanumeric() || " -_.()&,'".contains(c))
}

fn url_part(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The keys of the one-direction routes OBIS has for this line number (on any map).
fn line_routes(list: &str, line: &str) -> Vec<String> {
    let Ok(Value::Map(m)) = crate::api::json::decode(list, false) else { return Vec::new() };
    let Some(Value::List(all)) = field(&m, "routes") else { return Vec::new() };
    let mut keys: Vec<String> = Vec::new();
    for r in all {
        let Value::Map(r) = r else { continue };
        if !matches!(field(r, "lineNumber"), Some(Value::Str(n)) if n.trim() == line) {
            continue;
        }
        match field(r, "subRoutes") {
            Some(Value::List(subs)) if !subs.is_empty() => keys.extend(subs.iter().filter_map(|k| match k {
                Value::Str(k) => Some(k.clone()),
                _ => None,
            })),
            _ => {
                if let Some(Value::Str(k)) = field(r, "routeKey") {
                    keys.push(k.clone());
                }
            }
        }
    }
    keys.retain(|k| route_key_ok(k));
    keys.dedup();
    keys.truncate(FETCH_MAX);
    keys
}

/// Whether a route need not be fetched: the pack or the client has it, or it was fetched lately.
fn have_route(key: &str) -> bool {
    let name = format!("{key}.json");
    if obis_client_dir().is_some_and(|d| route_dirs(&d, pack_dir().as_deref()).iter().any(|r| r.join(&name).is_file())) {
        return true;
    }
    let fresh = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < FETCH_KEEP);
    fetched_dir().is_some_and(|d| fresh(d.join(&name)))
}

/// Fetches the routes of a line number the client lacks; how many were added.
fn fetch_line(base: &str, line: &str) -> Result<usize, String> {
    let get = |url: &str| -> Result<String, String> {
        ureq::get(url).timeout(Duration::from_secs(15)).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())
    };
    let keys: Vec<String> = line_routes(&get(base)?, line).into_iter().filter(|k| !have_route(k)).collect();
    if keys.is_empty() {
        return Ok(0);
    }
    let dir = fetched_dir().ok_or("no home folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut added = 0;
    for key in keys {
        std::thread::sleep(FETCH_GAP);
        let text = match get(&format!("{base}/{}", url_part(&key))) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("plugin obis: route {key} not fetched: {e}");
                continue;
            }
        };
        // only a route as the client keeps them: one direction with its stations
        let Ok(Value::Map(r)) = crate::api::json::decode(&text, false) else { continue };
        if !matches!(field(&r, "stations"), Some(Value::List(l)) if !l.is_empty()) {
            continue;
        }
        if std::fs::write(dir.join(format!("{key}.json")), &text).is_ok() {
            added += 1;
        }
    }
    Ok(added)
}

/// `obis.fetch_routes`: `nil` while fetching (started on the first call for a line), then how
/// many routes were added. Once a session per line.
fn fetch_routes(line: &str) -> Value {
    static FETCHES: Mutex<Vec<(String, Option<usize>)>> = Mutex::new(Vec::new());
    let line = line.trim().to_string();
    if line.is_empty() || line.len() > 32 || obis_client_dir().is_none() {
        return Value::Nil;
    }
    let mut g = FETCHES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, done)) = g.iter().find(|(l, _)| *l == line) {
        return done.map(|n| Value::Int(n as i64)).unwrap_or(Value::Nil);
    }
    g.push((line.clone(), None));
    drop(g);
    let _ = std::thread::Builder::new().name("plugin obis fetch".into()).spawn(move || {
        // (tests: OMSI_OBIS_ROUTES_URL names a local server)
        let base = std::env::var("OMSI_OBIS_ROUTES_URL").unwrap_or_else(|_| ROUTES_URL.to_string());
        let n = match fetch_line(&base, &line) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("plugin obis: the routes of line {line} not fetched: {e}");
                0
            }
        };
        log::info!("plugin obis: line {line}: {n} routes fetched from OBIS");
        if n > 0 {
            // read again, with them
            *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let mut g = FETCHES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = g.iter_mut().find(|(l, _)| *l == line) {
            e.1 = Some(n);
        }
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

/// Where the player's bus goes.
const INGEST_URL: &str = "wss://obis.kr/api/ingest";
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

/// The OBIS client's device token, the only one used: K-OMSI never registers a device itself
/// (one player, one device on OBIS), so a client that never registered sends nothing yet.
fn device_token() -> Result<(String, &'static str), String> {
    for f in obis_client_configs() {
        if let Some(t) = std::fs::read_to_string(&f).ok().and_then(|t| json_str(&t, "deviceToken")) {
            return Ok((t, "obis_client"));
        }
    }
    Err("the OBIS client has no device token yet (run OBIS-client.exe once)".into())
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
    def!("obis.start", "network", [], "true, or false and the reason", "Starts sending the player's bus to OBIS (`wss://obis.kr/api/ingest`) in the background, as the OBIS client does. The device's token is the OBIS client's (its `config.local.json` next to the game; K-OMSI never registers a device of its own); the plugin never sees it. Give the snapshots with `omsi.obis.send`.", NEW, NetworkRemote, true, |c, a| {
        Ok::<_, ApiError>(ok_or_reason(c.state().obis.start()))
    }),
    def!("obis.send", "network", [p("snapshot", "string")], "true, or false and the reason", "Hands OBIS the state of the player's bus: the OBIS client's snapshot as JSON text, at most 256 kB. Only the newest is sent, at most twice a second; one not sent yet is replaced.", NEW, NetworkRemote, true, |c, a| {
        let text = a.str(0)?;
        Ok::<_, ApiError>(ok_or_reason(c.state().obis.send(text)))
    }),
    def!("obis.state", "network", [], "state, sent, token", "The sending: `\"connecting\"`, `\"open\"`, `\"waiting\"` (broken, opened again soon), `\"no_token\"` (the OBIS client has no token yet, as before it is first run: looked for again soon) or `\"closed\"`; how many snapshots went; where the token came from (`\"obis_client\"`; empty before it is known).", NEW, NetworkRemote, false, |c, a| {
        let st = c.state().obis.state();
        let (sent, src) = c.state().obis.info();
        multi(Some([Value::Str(st.into()), Value::Num(sent), Value::Str(src.into())]))
    }),
    def!("obis.stop", "network", [], "nil", "Stops sending (OBIS shows the bus as gone after a while).", NEW, NetworkRemote, false, |c, a| {
        c.state().obis.stop();
    }),
    def!("obis.routes", "network", [], "table or nil", "The OBIS client's routes, to match the player's trip to one: `{default_map, routes = {[key] = {file, line, map, stops}}, trips = {[trip file] = {keys}}}` (`stops` the stop object ids in order; a trip file lower case, without `.ttp`). Read on a worker the first time - OBIS's route pack first (fetched then with the OBIS client's token, only when OBIS has a newer one; kept in `~/.openomsi/obis_pack`), then the client's `data\\routes`: `nil` until then, and without the OBIS client.", NEW, NetworkRemote, false, |c, a| {
        if has_original("obis") { route_index() } else { Value::Nil }
    }),
    def!("obis.fetch_routes", "network", [p("line", "string")], "number or nil", "Fetches from OBIS (`https://obis.kr/api/routes`) the routes of a line number the OBIS client's `data\\routes` lacks (an old or empty one), in the background, keeps them in `~/.openomsi/obis_routes` (fetched again after a week) and has `obis.routes` read again with them. `nil` while fetching, then how many routes were added (0: none to add, or OBIS did not answer). Once a session per line number; a request at most every 0.6 s.", NEW, NetworkRemote, false, |c, a| {
        let line = a.str(0)?;
        Ok::<_, ApiError>(if has_original("obis") { fetch_routes(&line) } else { Value::Nil })
    }),
    def!("obis.route_text", "network", [p("file", "string")], "string or nil", "One route (a `file` of `obis.routes`) as its JSON text: OBIS's pack, else the OBIS client's, else one fetched.", NEW, NetworkRemote, false, |c, a| {
        let name = a.str(0)?;
        let text = has_original("obis").then(|| route_file(&name)).flatten().and_then(|p| std::fs::read_to_string(p).ok());
        Ok::<_, ApiError>(text.map(Value::Str).unwrap_or(Value::Nil))
    }),
    def!("obis.map_names", "network", [p("map", "string")], "table or nil", "A map's stop names as OBIS has them: the map's name of a stop -> the Korean one (its `maps\\<map>\\map.json` of OBIS's pack, else of the OBIS client). `map` is a route's `map` of `obis.routes`. `nil` without the OBIS client or the map.", NEW, NetworkRemote, false, |c, a| {
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

    /// The real OBIS (three requests): `OMSI_OBIS_CLIENT_DIR` an empty folder, `OMSI_OBIS_FETCHED_DIR`
    /// where they go; `cargo test -p omsi-plugin --lib fetches_a_line_from_obis -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_a_line_from_obis() {
        let n = super::fetch_line(super::ROUTES_URL, "173-2").unwrap();
        assert!(n >= 1, "{n}");
        let dir = super::fetched_dir().unwrap();
        let text = std::fs::read_to_string(dir.join("NCC_173-2_a.json")).unwrap();
        assert!(text.contains("173-2 (to Hwasang-ri).ttp"));
        assert_eq!(super::fetch_line(super::ROUTES_URL, "173-2").unwrap(), 0, "fetched lately: not again");
    }

    #[test]
    fn a_missing_line_is_its_directions_on_every_map() {
        let list = r#"{"routes":[
            {"routeKey":"NCC_173_combined","mapId":"NCC","lineNumber":"173","subRoutes":["NCC_173_a","NCC_173_b"]},
            {"routeKey":"NCC_173-1_combined","mapId":"NCC","lineNumber":"173-1","subRoutes":["NCC_173-1_a"]},
            {"routeKey":"X_173","mapId":"X","lineNumber":" 173 "},
            {"routeKey":"Y_173_combined","mapId":"Y","lineNumber":"173","subRoutes":["../evil","AM_Munsan Stn."]}
        ]}"#;
        assert_eq!(super::line_routes(list, "173"), ["NCC_173_a", "NCC_173_b", "X_173", "AM_Munsan Stn."]);
        assert!(super::line_routes(list, "17").is_empty());
        assert!(super::line_routes("not json", "173").is_empty());
        assert_eq!(super::url_part("AM_Munsan Stn."), "AM_Munsan%20Stn.");
        assert_eq!(super::url_part("NCC_173-2_a"), "NCC_173-2_a");
    }

    fn gz_pack(files: &[(&str, &str)]) -> (Vec<u8>, String) {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let mut sorted: Vec<_> = files.to_vec();
        sorted.sort();
        let mut h = Sha256::new();
        for (k, v) in &sorted {
            h.update(format!("{k}\0{v}\0").as_bytes());
        }
        let hash: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let m: serde_json::Map<String, serde_json::Value> = files.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
        let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(serde_json::json!({ "hash": hash, "files": m }).to_string().as_bytes()).unwrap();
        (z.finish().unwrap(), hash)
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("komsi-obis-pack-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn pack_paths_stay_in_routes_and_maps() {
        assert!(super::pack_path_ok("routes/NCC_173_a.json"));
        assert!(super::pack_path_ok("maps/Anrak&Munsan/map.json"));
        assert!(super::pack_path_ok("maps/ARMR(0.3.1)/map.json"));
        for bad in ["routes", "routes/", "data/x.json", "/routes/x.json", "routes/../x.json", "routes/x.txt", "routes\\x.json", "maps/C:/x.json", "maps//map.json"] {
            assert!(!super::pack_path_ok(bad), "{bad}");
        }
    }

    #[test]
    fn a_pack_is_taken_only_whole_and_as_hashed() {
        let files = [("routes/A_1_a.json", "{\"mapId\":\"A\"}"), ("maps/A/map.json", "{\"stations\":{\"X\":\"엑스\"}}")];
        let (body, hash) = gz_pack(&files);
        let got = super::parse_pack(&body, &hash).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got["maps/A/map.json"], files[1].1);
        assert!(super::parse_pack(&body, &hash.to_uppercase()).is_ok());
        assert!(super::parse_pack(&body, "00").unwrap_err().contains("hash"));
        assert!(super::parse_pack(b"not gzip", &hash).is_err());
        let (body, hash) = gz_pack(&[("maps/A/map.json", "{}")]);
        assert!(super::parse_pack(&body, &hash).unwrap_err().contains("no routes"));
        let (body, hash) = gz_pack(&[("routes/A.json", "{}"), ("routes/../../evil.json", "{}")]);
        assert!(super::parse_pack(&body, &hash).unwrap_err().contains("not allowed"));
    }

    #[test]
    fn a_new_pack_replaces_the_old_and_comes_before_the_client() {
        let root = temp("install");
        let pack = root.join("obis_pack");
        let one = [("routes/A_1_a.json", r#"{"routeKey":"A_1_a","mapId":"A","fileName":"1 A.ttp","lineNumber":"1","stations":[{"id":5}]}"#), ("routes/Gone.json", "{}"), ("maps/A/map.json", "{}")];
        let (body, hash) = gz_pack(&one);
        super::install_pack(&pack, &super::parse_pack(&body, &hash).unwrap(), &hash).unwrap();
        assert_eq!(super::pack_hash(&pack).as_deref(), Some(hash.as_str()));
        let two = [("routes/A_1_a.json", r#"{"routeKey":"A_1_a","mapId":"A","fileName":"1 A.ttp","lineNumber":"1","stations":[{"id":7}]}"#), ("maps/B/map.json", "{}")];
        let (body, hash2) = gz_pack(&two);
        super::install_pack(&pack, &super::parse_pack(&body, &hash2).unwrap(), &hash2).unwrap();
        assert_eq!(super::pack_hash(&pack).as_deref(), Some(hash2.as_str()));
        assert!(!pack.join("routes/Gone.json").exists() && !pack.join("maps/A").exists(), "the old pack's files go");
        assert!(!root.join("obis_pack.stage").exists() && !root.join("obis_pack.old").exists());
        // the client still has the older copy of the route and one of its own
        let client = root.join("client");
        std::fs::create_dir_all(client.join("data/routes")).unwrap();
        std::fs::create_dir_all(client.join("maps/C")).unwrap();
        std::fs::write(client.join("data/routes/A_1_a.json"), one[0].1).unwrap();
        std::fs::write(client.join("data/routes/C_2_a.json"), r#"{"routeKey":"C_2_a","mapId":"C","fileName":"2 A.ttp","lineNumber":"2","stations":[]}"#).unwrap();
        let idx = crate::api::json::encode(&super::build_route_index(&client, Some(&pack)));
        assert!(idx.contains(r#""A_1_a":{"file":"A_1_a.json","line":"1","map":"A","stops":[7]}"#), "{idx}");
        assert!(idx.contains("C_2_a") && idx.contains(r#""default_map":"B""#), "{idx}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The real OBIS: `OMSI_OBIS_TOKEN` the OBIS client's token;
    /// `cargo test -p omsi-plugin --lib fetches_obis_route_pack -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_obis_route_pack() {
        let token = std::env::var("OMSI_OBIS_TOKEN").unwrap();
        let dir = temp("real").join("obis_pack");
        assert!(super::sync_pack_from(super::PACK_URL, &token, &dir).unwrap(), "fetched the first time");
        assert!(std::fs::read_dir(dir.join("routes")).unwrap().count() > 100);
        assert!(dir.join("maps/NCC/map.json").is_file());
        assert!(!super::sync_pack_from(super::PACK_URL, &token, &dir).unwrap(), "the same pack is not fetched again");
        assert!(super::sync_pack_from(super::PACK_URL, "not-a-token", &temp("bad").join("p")).unwrap_err().contains("token"));
    }

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

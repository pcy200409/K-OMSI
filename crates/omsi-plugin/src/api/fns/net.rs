//! `omsi.ws`: a read-only WebSocket client for the public live feeds a plugin may listen to
//! (today OBIS, `wss://obis.kr/...`: the buses of the other players). It never sends
//! anything, only the hosts of [`ALLOWED_HOSTS`] are reachable and a plugin holds one
//! connection. The connection lives on its own thread, so the game never waits for the
//! network; a plugin drains the messages that came in with `omsi.ws.poll()`.

use super::{def, p};
use crate::api::fns::core::ok_or_reason;
use crate::api::{ApiError, ApiFn, Value};
use std::collections::VecDeque;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
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

fn connect(url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let cfg = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
    // (no redirects: the host must stay the checked one)
    let (mut ws, _) = tungstenite::client::connect_with_config(url, Some(cfg), 0).map_err(|e| e.to_string())?;
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(Duration::from_millis(200))),
        MaybeTlsStream::Rustls(s) => s.get_mut().set_read_timeout(Some(Duration::from_millis(200))),
        _ => Ok(()),
    }
    .map_err(|e| e.to_string())?;
    Ok(ws)
}

fn sleep_unless_stopped(s: &Shared, d: Duration) {
    let end = Instant::now() + d;
    while !s.stop.load(Ordering::Relaxed) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run(s: Arc<Shared>, url: String) {
    while !s.stop.load(Ordering::Relaxed) {
        s.state.store(CONNECTING, Ordering::Relaxed);
        match connect(&url) {
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
        sleep_unless_stopped(&s, RETRY);
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

//! What two games of different LAN protocols are told (K-OMSI): in Korean, which side is the
//! older one and what to do about it. An older game shows the host's refusal as it comes,
//! untranslated, so the words are Korean already.

/// The words for a host speaking LAN protocol `host` and a player's game speaking `mine`
/// (0: an older one, its number not known). `host_version` is the host's K-OMSI version, when
/// the host itself is speaking.
pub fn version_mismatch_text(host: u32, mine: u32, host_version: Option<&str>) -> String {
    let host_side = match host_version.filter(|v| !v.trim().is_empty()) {
        Some(v) => format!("호스트는 K-OMSI {}(LAN 프로토콜 {host})", v.trim()),
        None => format!("호스트는 LAN 프로토콜 {host}"),
    };
    let mine_side = if mine == 0 { "내 게임은 더 오래된 버전".to_string() } else { format!("내 게임은 LAN 프로토콜 {mine}") };
    if mine == 0 || mine < host {
        format!("{host_side}, {mine_side}이라 접속할 수 없습니다. 런처에서 K-OMSI를 최신 버전으로 업데이트한 뒤 다시 접속하세요.")
    } else {
        format!("{host_side}, {mine_side}이라 접속할 수 없습니다. 호스트의 게임이 더 오래된 버전이니, 호스트에게 K-OMSI를 최신 버전으로 업데이트해 달라고 하세요.")
    }
}

/// This game's version as the release build was given it (none in a local build).
pub fn this_version() -> Option<&'static str> {
    option_env!("OPENOMSI_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version_mismatch_text;

    #[test]
    fn the_older_side_is_told_to_update() {
        let old_player = version_mismatch_text(15, 7, Some("0.4.4"));
        assert!(old_player.contains("K-OMSI 0.4.4(LAN 프로토콜 15)") && old_player.contains("LAN 프로토콜 7") && old_player.contains("런처에서"), "{old_player}");
        let old_host = version_mismatch_text(7, 15, None);
        assert!(old_host.starts_with("호스트는 LAN 프로토콜 7") && old_host.contains("호스트에게"), "{old_host}");
        let unknown = version_mismatch_text(15, 0, Some(" "));
        assert!(unknown.starts_with("호스트는 LAN 프로토콜 15,") && unknown.contains("더 오래된 버전이라") && unknown.contains("런처에서"), "{unknown}");
        // (a refusal travels as at most 300 characters)
        assert!(old_player.chars().count() <= 300);
    }

    /// A game of protocol 7 (K-OMSI 0.2) knocking at the door hears in Korean that it is the
    /// older one, as it shows the host's words untranslated.
    #[test]
    fn an_older_game_at_the_door_is_told_in_korean_to_update() {
        use crate::{LanSession, Pose, WorldInfo};
        use std::time::Duration;
        let mut host = LanSession::host(28210, "host", WorldInfo::default(), true).unwrap();
        let port = host.local_addr().unwrap().port();
        let s = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut buf = [0u8; 1500];
        let mut got = None;
        for _ in 0..20 {
            let _ = s.send_to(b"HELLO|7|-|old||m|1989-05-30|32400|||", ("127.0.0.1", port));
            host.tick(0.02, &Pose::default());
            if let Ok((n, _)) = s.recv_from(&mut buf) {
                got = Some(String::from_utf8_lossy(&buf[..n]).to_string());
                break;
            }
        }
        let got = got.expect("an answer");
        assert!(got.starts_with("REJECT|") && got.contains("LAN 프로토콜 7") && got.contains("런처에서"), "{got}");
    }
}

//! Station links rebuilt over the roads the object editor changed (R): every link of the
//! map's `TTData\StnLinks.cfg` that runs over an edited tile (or over a lane that is no
//! more) is routed anew over the lane network - from its first lane to its last, the
//! lanes its stops stand on, so its own numbers about the stops stay right - and the file
//! is written as a copy into the content folder (read in place of the map's own from the
//! next time the map is loaded). Links whose stops' tiles are not loaded are left as they
//! are (the network has their lanes only while loaded).

use crate::App;
use omsi_sim::traffic::{LaneKey, Network};
use std::collections::{HashMap, HashSet};

/// The lines of the `[StnLink]` records: from each keyword line to one past its last
/// entry's last value.
pub(crate) fn link_blocks(lines: &[&str]) -> Vec<(usize, usize)> {
    let t = |l: &str| l.trim_end_matches(['\r', '\n']).trim().to_string();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !t(lines[i]).eq_ignore_ascii_case("[stnlink]") {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = (i + 10).min(lines.len());
        let mut k = end;
        loop {
            while k < lines.len() && (t(lines[k]).is_empty() || is_count(&t(lines[k]))) {
                k += 1;
            }
            if k < lines.len() && t(lines[k]).eq_ignore_ascii_case("[stnlink_entry]") {
                k = (k + 8).min(lines.len());
                end = k;
            } else {
                break;
            }
        }
        out.push((start, end));
        i = end;
    }
    out
}

/// `   3:`, the count the editor writes before each entry.
fn is_count(l: &str) -> bool {
    l.strip_suffix(':').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// A link's record written anew: its header lines (`[StnLink]`, the length, the stops and
/// their six numbers) with the new length, then its entries (id, path, tile index, length).
pub(crate) fn link_text(header: &[&str], length: f64, entries: &[(i64, u16, usize, f64)], eol: &str) -> String {
    let mut s = String::new();
    let line = |s: &mut String, v: &str| {
        s.push_str(v);
        s.push_str(eol);
    };
    line(&mut s, "[StnLink]");
    line(&mut s, &format!("{length:.10}"));
    for h in header.iter().skip(2).take(8) {
        line(&mut s, h.trim_end_matches(['\r', '\n']));
    }
    for (k, (id, path, tile, len)) in entries.iter().enumerate() {
        line(&mut s, "");
        line(&mut s, &format!("{k:>4}:"));
        line(&mut s, "[StnLink_entry]");
        for v in [id.to_string(), path.to_string(), tile.to_string(), format!("{len:.10}"), "-1".into(), "0".into(), "0".into()] {
            line(&mut s, &v);
        }
    }
    s
}

/// The shortest way over the network from a lane of `first` to a lane of `last` (each
/// direction of a two-way path tried): its lanes.
pub(crate) fn route(net: &Network, first: &LaneKey, last: &LaneKey) -> Option<Vec<usize>> {
    let (a, b) = (net.by_key.get(first)?, net.by_key.get(last)?);
    let mut best: Option<(f64, Vec<usize>)> = None;
    for &x in a {
        for &y in b {
            let Some(p) = net.shortest_path(x, y) else { continue };
            let len: f64 = p.iter().map(|&l| net.lanes[l].length() as f64).sum();
            if best.as_ref().is_none_or(|b| len < b.0) {
                best = Some((len, p));
            }
        }
    }
    best.map(|b| b.1)
}

impl App {
    /// Rebuild the station links over the edited tiles (R).
    pub(crate) fn rebuild_station_links(&mut self) {
        let msg = match self.rebuild_links_now() {
            Ok((done, kept, file)) if done == 0 => omsi_ui::tr("No station link to change ({kept} over edited roads not loaded or not routed)").replace("{kept}", &kept.to_string()) + &file.map(|_| String::new()).unwrap_or_default(),
            Ok((done, kept, file)) => omsi_ui::tr("{n} station links routed anew ({kept} left), written to {file} - read when the map is loaded again")
                .replace("{n}", &done.to_string())
                .replace("{kept}", &kept.to_string())
                .replace("{file}", &file.map(|f| f.display().to_string()).unwrap_or_default()),
            Err(e) => omsi_ui::tr("Station links not rebuilt ({e})").replace("{e}", &e),
        };
        log::info!("object editor: {msg}");
        self.service_msg = Some((msg, 8.0));
    }

    /// The links rebuilt and written: how many changed, how many over edited tiles could
    /// not be, and the file written.
    fn rebuild_links_now(&mut self) -> Result<(usize, usize, Option<std::path::PathBuf>), String> {
        let world = self.world.clone().ok_or("no map")?;
        let edited: HashSet<(i32, i32)> = world.tile_texts.lock().keys().copied().collect();
        if edited.is_empty() {
            return Err(omsi_ui::tr("no road edited yet").into_owned());
        }
        let net = &self.session.traffic.as_ref().ok_or("no traffic")?.net;
        let loaded: HashSet<(i32, i32)> = world.loaded_tiles().into_iter().collect();
        let raw = &world.global.raw_tiles;
        let mut index_of: HashMap<(i32, i32), usize> = HashMap::new();
        for (i, t) in raw.iter().enumerate() {
            index_of.entry(*t).or_insert(i);
        }
        // the file: the content folder's copy (an earlier rebuild), else the map's
        let content = crate::startup::content_dir().ok_or("no content folder")?;
        let map_rel = std::path::Path::new(&self.args.map).parent().unwrap_or(std::path::Path::new("")).to_path_buf();
        let out = content.join(&map_rel).join("TTData").join("StnLinks.cfg");
        let src = if out.is_file() { out.clone() } else { world.map_dir.join("TTData").join("StnLinks.cfg") };
        let bytes = omsi_cfg::vfs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
        let text = omsi_cfg::decode_text(&bytes);
        let links = omsi_timetable::parse_stnlinks(&omsi_cfg::CfgFile::from_str("StnLinks.cfg", &text));
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let blocks = link_blocks(&lines);
        if blocks.len() != links.len() {
            return Err(format!("{}: {} links read, {} records found", src.display(), links.len(), blocks.len()));
        }
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let key_of = |e: &omsi_timetable::StnLinkEntry| -> Option<LaneKey> {
            Some(LaneKey { tile: *raw.get(e.values[2] as usize)?, id: e.values[0] as i64, path: e.values[1] as u16 })
        };
        let mut replaced: HashMap<usize, String> = HashMap::new();
        let mut kept = 0;
        for (n, l) in links.iter().enumerate() {
            let keys: Vec<Option<LaneKey>> = l.entries.iter().map(key_of).collect();
            let touches = keys.iter().flatten().any(|k| edited.contains(&k.tile));
            if !touches {
                continue;
            }
            let (Some(Some(first)), Some(Some(last))) = (keys.first(), keys.last()) else {
                kept += 1;
                continue;
            };
            if !loaded.contains(&first.tile) || !loaded.contains(&last.tile) {
                kept += 1;
                continue;
            }
            let Some(lanes) = route(net, first, last) else {
                kept += 1;
                continue;
            };
            let mut entries: Vec<(i64, u16, usize, f64)> = Vec::new();
            let mut new_keys: Vec<LaneKey> = Vec::new();
            for &ln in &lanes {
                let Some(k) = net.lanes[ln].key else { continue };
                if new_keys.last() == Some(&k) {
                    continue;
                }
                let Some(&ti) = index_of.get(&k.tile) else { continue };
                let len = if entries.is_empty() { 0.0 } else { net.lanes[ln].length() as f64 };
                entries.push((k.id, k.path, ti, len));
                new_keys.push(k);
            }
            if keys.iter().flatten().copied().collect::<Vec<_>>() == new_keys || entries.is_empty() {
                continue;
            }
            let total: f64 = entries.iter().map(|e| e.3).sum();
            let (s, e) = blocks[n];
            replaced.insert(n, link_text(&lines[s..e], total, &entries, eol));
        }
        if replaced.is_empty() {
            return Ok((0, kept, None));
        }
        let mut new = String::with_capacity(text.len());
        let mut i = 0;
        for (n, (s, e)) in blocks.iter().enumerate() {
            for l in &lines[i..*s] {
                new.push_str(l);
            }
            match replaced.get(&n) {
                Some(t) => new.push_str(t),
                None => {
                    for l in &lines[*s..*e] {
                        new.push_str(l);
                    }
                }
            }
            i = *e;
        }
        for l in &lines[i..] {
            new.push_str(l);
        }
        std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
        let data: Vec<u8> = [0xFF, 0xFE].into_iter().chain(new.encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        std::fs::write(&out, data).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok((replaced.len(), kept, Some(out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINKS: &str = "---\r\nTime Table StnLinkList File\r\n\r\nA ==> B\r\n\r\n[StnLink]\r\n288.0000000000\r\n1\r\n2\r\n2.13\r\n0.445\r\n13.3\r\n56.1\r\n1\r\n10\r\n\r\n   0:\r\n[StnLink_entry]\r\n500\r\n0\r\n4\r\n0.0000000000\r\n-1\r\n0\r\n0\r\n\r\n   1:\r\n[StnLink_entry]\r\n501\r\n1\r\n4\r\n15.0000000000\r\n-1\r\n0\r\n0\r\n\r\nB ==> C\r\n\r\n[StnLink]\r\n50.0000000000\r\n2\r\n3\r\n0\r\n0\r\n0\r\n0\r\n0\r\n0\r\n\r\n   0:\r\n[StnLink_entry]\r\n600\r\n0\r\n5\r\n0.0000000000\r\n-1\r\n0\r\n0\r\n";

    #[test]
    fn records_are_found_with_their_entries_and_written_back() {
        let lines: Vec<&str> = LINKS.split_inclusive('\n').collect();
        let blocks = link_blocks(&lines);
        assert_eq!(blocks.len(), 2);
        // the next link's comment is not the first's
        assert!(!lines[blocks[0].0..blocks[0].1].concat().contains("B ==> C"));
        let t = link_text(&lines[blocks[0].0..blocks[0].1], 40.0, &[(500, 0, 4, 0.0), (700, 2, 9, 25.0), (501, 1, 4, 15.0)], "\r\n");
        let mut whole = lines[..blocks[0].0].concat();
        whole.push_str(&t);
        whole.push_str(&lines[blocks[0].1..].concat());
        let links = omsi_timetable::parse_stnlinks(&omsi_cfg::CfgFile::from_str("StnLinks.cfg", &whole));
        assert_eq!(links.len(), 2);
        let l = &links[0];
        assert_eq!((l.length, l.from_id, l.to_id), (40.0, 1, 2));
        assert_eq!(l.params, [2.13, 0.445, 13.3, 56.1, 1.0, 10.0]);
        assert_eq!(l.entries.iter().map(|e| (e.values[0], e.values[1], e.values[2], e.values[3])).collect::<Vec<_>>(), vec![(500.0, 0.0, 4.0, 0.0), (700.0, 2.0, 9.0, 25.0), (501.0, 1.0, 4.0, 15.0)]);
        assert_eq!(links[1].entries.len(), 1);
        assert!(whole.contains("B ==> C"));
        // the same records as read
        assert_eq!(link_blocks(&whole.split_inclusive('\n').collect::<Vec<_>>()).len(), 2);
    }
}

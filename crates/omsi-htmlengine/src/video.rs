//! Moving pictures for a page: `<video src="clip.mp4" autoplay loop muted poster="still.png">`.
//!
//! * The file is looked up like a picture (`images`): relative to the page's folder first.
//! * Every `<video>` that is drawn has a [`Player`]. Its decoder runs on a thread of its own
//!   (`omsi_media`) and hands over finished RGBA frames already at the size of the element's
//!   box, a couple of them ahead of the clock; when nobody takes them the thread waits, so a
//!   paused or hidden video costs no CPU. Sound is never decoded: a page's video is silent.
//! * The clock is the page's own (`js.now`, real time). A frame becomes visible once its time
//!   has come; a player that fell far behind (the page was not drawn for a while) seeks
//!   instead of decoding its way through everything it missed.
//! * When only video frames changed, the renderer does not lay out and paint the page again:
//!   it puts back the pixels that were under each video in the last full frame and draws the
//!   new frame over them ([`ImageStore::repaint_videos`]).
//! * Decoding uses Windows Media Foundation (see `omsi_media`); elsewhere a video shows its
//!   `poster`.
//!
//! From scripts: `play()`, `pause()`, `load()` (back to the start), `paused`, `ended`,
//! `currentTime` (read and set, setting seeks), `duration` (`NaN` until known), `loop`,
//! `autoplay`. See `Interp::media`.

use super::*;
use omsi_media::{Cmd, Msg};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// Largest box a video is drawn into (pixels).
const MAX_FRAME_PX: u64 = omsi_media::MAX_FRAME_PX;
/// The clock this far (s) past the newest frame seeks instead of decoding up to it.
const CATCH_UP: f64 = 1.0;
/// Time (s) given to a decoder after it started or seeked before it counts as behind.
const SEEK_GRACE: f64 = 2.0;

/// The decoder of one `<video>` and its clock.
pub(crate) struct Player {
    pub(crate) src: String,
    link: Option<(Sender<Cmd>, Receiver<Msg>)>,
    epoch: u32,
    pub(crate) size: (u32, u32),
    looping: bool,
    pub(crate) playing: bool,
    /// Timeline position at `anchor` (page time).
    base: f64,
    anchor: f64,
    pub(crate) duration: f64,
    end_seen: bool,
    pub(crate) ended: bool,
    pending: Option<(Arc<Img>, f64)>,
    pub(crate) frame: Option<Arc<Img>>,
    shown_t: f64,
    last_seek: f64,
    /// Made by the last paint and not given its state by the page yet.
    pub(crate) new: bool,
}

impl Player {
    /// A paused player for `src` (a file found with `dirs`); one that cannot play is kept
    /// too, so the file is not looked for in every frame.
    pub(crate) fn open(dirs: &[PathBuf], src: &str, size: (u32, u32), looping: bool, now: f64) -> Player {
        let link = find(dirs, src).and_then(|p| omsi_media::spawn(p, size, looping));
        Player {
            src: src.to_string(),
            link,
            epoch: 0,
            size,
            looping,
            playing: false,
            base: 0.0,
            anchor: now,
            duration: 0.0,
            end_seen: false,
            ended: false,
            pending: None,
            frame: None,
            shown_t: 0.0,
            last_seek: now,
            new: true,
        }
    }

    fn send(&mut self, c: Cmd) {
        if let Some((tx, _)) = &self.link {
            if tx.send(c).is_err() {
                self.link = None;
            }
        }
    }

    fn position(&self, now: f64) -> f64 {
        if self.playing {
            self.base + (now - self.anchor).max(0.0)
        } else {
            self.base
        }
    }

    /// `currentTime`: seconds into the file.
    pub(crate) fn current_time(&self, now: f64) -> f64 {
        let p = self.position(now);
        match self.duration {
            d if d > 0.0 && self.looping => p.rem_euclid(d),
            d if d > 0.0 => p.min(d),
            _ => p,
        }
    }

    pub(crate) fn set_playing(&mut self, on: bool, now: f64) {
        if on == self.playing {
            return;
        }
        if on && self.ended {
            self.playing = true;
            self.seek(0.0, now);
            return;
        }
        self.base = self.position(now);
        self.anchor = now;
        self.playing = on;
    }

    pub(crate) fn set_loop(&mut self, on: bool, now: f64) {
        if on == self.looping {
            return;
        }
        // the timeline of a loop runs on past the length: start a fresh one where we are
        let at = self.current_time(now);
        self.looping = on;
        self.send(Cmd::Loop(on));
        if !self.ended {
            self.seek(at, now);
        }
    }

    pub(crate) fn resize(&mut self, w: u32, h: u32) {
        if self.size != (w, h) {
            self.size = (w, h);
            self.send(Cmd::Size(w, h));
        }
    }

    pub(crate) fn seek(&mut self, to: f64, now: f64) {
        let to = if self.duration > 0.0 { to.clamp(0.0, self.duration) } else { to.max(0.0) };
        self.epoch = self.epoch.wrapping_add(1);
        self.base = to;
        self.anchor = now;
        self.ended = false;
        self.end_seen = false;
        self.pending = None;
        self.last_seek = now;
        // frames of the old position are of no use (a length may be among them)
        if let Some((_, rx)) = &self.link {
            while let Ok(m) = rx.try_recv() {
                if let Msg::Info { duration } = m {
                    self.duration = duration;
                }
            }
        }
        let epoch = self.epoch;
        self.send(Cmd::Seek { to, epoch });
    }

    /// Take the frames whose time has come; true when the visible frame changed.
    pub(crate) fn tick(&mut self, now: f64) -> bool {
        let pos = self.position(now);
        let mut changed = false;
        loop {
            if self.pending.is_none() {
                let Some((_, rx)) = &self.link else { break };
                match rx.try_recv() {
                    Ok(Msg::Info { duration }) => self.duration = duration,
                    Ok(Msg::Frame { frame, t, epoch }) if epoch == self.epoch => {
                        let img = Img { w: frame.w, h: frame.h, rgba: frame.rgba, alpha: false };
                        self.pending = Some((Arc::new(img), t));
                    }
                    Ok(Msg::End { epoch }) if epoch == self.epoch => self.end_seen = true,
                    Ok(Msg::Frame { .. } | Msg::End { .. }) => {}
                    Ok(Msg::Failed) | Err(TryRecvError::Disconnected) => {
                        self.link = None;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
                continue;
            }
            let t = self.pending.as_ref().map_or(0.0, |p| p.1);
            // the first frame is shown at once, so a paused video is not empty; at the end of
            // a video that does not loop, what is left is shown (its times may run a little
            // past the length)
            let at_end = !self.looping && self.duration > 0.0 && pos >= self.duration;
            if self.frame.is_some() && t > pos && !at_end {
                break;
            }
            let (img, t) = self.pending.take().unwrap();
            self.frame = Some(img);
            self.shown_t = t;
            changed = true;
        }
        if self.end_seen && self.pending.is_none() {
            let end = if self.duration > 0.0 { self.duration } else { self.shown_t };
            if pos >= end {
                self.ended = true;
                self.playing = false;
                self.base = end;
            }
        } else if self.playing && self.link.is_some() && now - self.last_seek > SEEK_GRACE {
            let newest = self.pending.as_ref().map_or(self.shown_t, |p| p.1);
            if pos - newest > CATCH_UP {
                log::debug!("htmltexture: video {} is {:.1}s behind, seeking", self.src, pos - newest);
                let at = self.current_time(now);
                self.seek(at, now);
            }
        }
        changed
    }
}

/// Where a video once stood in the last full frame, and the pixels under it.
pub(crate) struct Placed {
    node: usize,
    clip: [f32; 4],
    radius: f32,
    /// Integer rectangle of `under` on the canvas: x0, y0, x1, y1.
    rect: [i32; 4],
    under: Vec<u8>,
    poster: String,
}

/// The players of a page and where they were drawn.
#[derive(Default)]
pub(crate) struct VideoStore {
    pub(crate) players: Mutex<HashMap<usize, Player>>,
    placed: Mutex<Vec<Placed>>,
    /// Page time of the frame being painted.
    pub(crate) now: Mutex<f64>,
}

impl VideoStore {
    /// A full frame is about to be painted.
    pub(crate) fn begin(&self, now: f64) {
        self.placed.lock().unwrap().clear();
        *self.now.lock().unwrap() = now;
    }

    /// A full frame was painted: the players of videos that were not drawn are dropped
    /// (which ends their threads).
    pub(crate) fn end(&self) {
        let placed = self.placed.lock().unwrap();
        self.players.lock().unwrap().retain(|n, _| placed.iter().any(|p| p.node == *n));
    }

    pub(crate) fn any_placed(&self) -> bool {
        !self.placed.lock().unwrap().is_empty()
    }
}

/// The source of a `<video>`: its `src`, else the `src` of its first `<source>`.
pub(crate) fn video_src(dom: &Dom, node: usize) -> String {
    let n = &dom.nodes[node];
    if !n.src.is_empty() {
        return n.src.clone();
    }
    n.kids.iter().map(|&k| &dom.nodes[k]).find(|k| k.tag == "source" && !k.src.is_empty()).map(|k| k.src.clone()).unwrap_or_default()
}

/// A `<video>`: its current frame (or its poster) stretched over the content box.
pub(crate) fn paint_video(cv: &mut Canvas, lay: &Layouter, b: &LBox) {
    let n = &lay.dom.nodes[b.node];
    if n.tag != "video" {
        return;
    }
    let [pt, pr, pb, pl] = b.st.padding;
    let area = [b.rect[0] + pl, b.rect[1] + pt, b.rect[2] - pl - pr, b.rect[3] - pt - pb];
    if !area.iter().all(|v| v.is_finite()) || area[2] < 0.5 || area[3] < 0.5 {
        return;
    }
    let w = (area[2].round() as i64).clamp(1, 8192) as u32;
    let h = (area[3].round() as i64).clamp(1, 8192) as u32;
    let src = video_src(lay.dom, b.node);
    let store = &lay.imgs.videos;
    let now = *store.now.lock().unwrap();
    let frame = if src.is_empty() || w as u64 * h as u64 > MAX_FRAME_PX {
        None
    } else {
        let mut players = store.players.lock().unwrap();
        let p = players.entry(b.node).or_insert_with(|| Player::open(&lay.imgs.dirs, &src, (w, h), n.attr("loop").is_some(), now));
        if p.src != src {
            *p = Player::open(&lay.imgs.dirs, &src, (w, h), n.attr("loop").is_some(), now);
        }
        p.resize(w, h);
        p.frame.clone()
    };
    let x0 = (area[0].round() as i32).clamp(0, cv.w as i32);
    let y0 = (area[1].round() as i32).clamp(0, cv.h as i32);
    let x1 = ((area[0] + area[2]).round() as i32).clamp(0, cv.w as i32);
    let y1 = ((area[1] + area[3]).round() as i32).clamp(0, cv.h as i32);
    let mut under = Vec::with_capacity(((x1 - x0).max(0) * (y1 - y0).max(0) * 4) as usize);
    for y in y0..y1 {
        let i = ((y as u32 * cv.w + x0 as u32) * 4) as usize;
        under.extend_from_slice(&cv.px[i..i + ((x1 - x0) * 4) as usize]);
    }
    let placed = Placed {
        node: b.node,
        clip: area,
        radius: b.st.radius,
        rect: [x0, y0, x1, y1],
        under,
        poster: n.attr("poster").unwrap_or_default().to_string(),
    };
    draw_placed(cv, lay.imgs, &placed, frame.as_deref());
    store.placed.lock().unwrap().push(placed);
}

/// The frame (or else the poster) of a placed video, over whatever is on the canvas.
fn draw_placed(cv: &mut Canvas, imgs: &ImageStore, p: &Placed, frame: Option<&Img>) {
    let (w, h) = ((p.clip[2].round() as u32).max(1), (p.clip[3].round() as u32).max(1));
    let (ox, oy) = (p.clip[0].round() as i32, p.clip[1].round() as i32);
    match frame {
        // the usual case, an opaque frame in a square box: rows are copied
        Some(f) if f.w == w && f.h == h && !f.alpha && p.radius <= 0.5 => {
            let [x0, y0, x1, y1] = p.rect;
            let (x0, y0) = (x0.max(ox), y0.max(oy));
            let (x1, y1) = (x1.min(ox + w as i32), y1.min(oy + h as i32));
            if x0 < x1 && y0 < y1 {
                let n = ((x1 - x0) * 4) as usize;
                for y in y0..y1 {
                    let d = ((y as u32 * cv.w + x0 as u32) * 4) as usize;
                    let s = ((((y - oy) as u32) * f.w + (x0 - ox) as u32) * 4) as usize;
                    cv.px[d..d + n].copy_from_slice(&f.rgba[s..s + n]);
                }
            }
        }
        Some(f) if f.w == w && f.h == h => draw_tiles(cv, f, p.clip, p.radius, ox, oy, (false, false)),
        // (for a moment after the box changed size)
        Some(f) => draw_tiles(cv, &resize(f, w, h), p.clip, p.radius, ox, oy, (false, false)),
        None if !p.poster.is_empty() => {
            if let Some(pic) = imgs.scaled(&p.poster, w, h) {
                draw_tiles(cv, &pic, p.clip, p.radius, ox, oy, (false, false));
            }
        }
        None => {}
    }
}

impl ImageStore {
    /// Draw the current frame of every video of the last full frame onto `cv` (that frame).
    pub(crate) fn repaint_videos(&self, cv: &mut Canvas) {
        let placed = self.videos.placed.lock().unwrap();
        let players = self.videos.players.lock().unwrap();
        for p in placed.iter() {
            let [x0, y0, x1, y1] = p.rect;
            let row = ((x1 - x0) * 4) as usize;
            for (k, y) in (y0..y1).enumerate() {
                let i = ((y as u32 * cv.w + x0 as u32) * 4) as usize;
                cv.px[i..i + row].copy_from_slice(&p.under[k * row..(k + 1) * row]);
            }
            let frame = players.get(&p.node).and_then(|pl| pl.frame.clone());
            draw_placed(cv, self, p, frame.as_deref());
        }
    }
}

fn find(dirs: &[PathBuf], src: &str) -> Option<PathBuf> {
    let rel = src.split(['?', '#']).next().unwrap_or(src).trim();
    if rel.is_empty() || rel.contains("://") || rel.starts_with("data:") {
        return None;
    }
    let path = dirs.iter().map(|d| omsi_cfg::resolve_path(d, rel)).find(|p| omsi_cfg::vfs::is_file(p));
    if path.is_none() {
        log::warn!("htmltexture: video {rel} not found");
    }
    path
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;

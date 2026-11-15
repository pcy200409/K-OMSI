//! Video frames for the game's pages (`<video>` of an `[htmltexture]`): a file is decoded on
//! a thread of its own into RGBA frames of a chosen size, without sound.
//!
//! * [`spawn`] starts the thread. It hands over [`Msg::Frame`]s through a channel that holds
//!   [`AHEAD`] of them; when nobody takes them the thread waits, so a paused or hidden video
//!   costs no CPU. [`Cmd`]s seek, change the frame size and switch looping.
//! * Frame times count from the first frame (H.264 files often start at the encoder's delay)
//!   and run on past the length when the video loops (every round adds the length).
//! * Decoding uses Windows Media Foundation's source reader (H.264 in .mp4 everywhere, other
//!   codecs where Windows has them); its video processor converts and scales. Its DLLs are
//!   loaded at run time, so the game still starts where they are missing (Windows "N"
//!   editions without the Media Feature Pack). Other systems get `None` from [`spawn`].

use std::path::PathBuf;
#[cfg(windows)]
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::{Receiver, Sender};

/// Frames decoded ahead (each one is width x height x 4 bytes).
pub const AHEAD: usize = 2;
/// Largest frame that is handed over (pixels).
pub const MAX_FRAME_PX: u64 = 4 * 1024 * 1024;

/// A decoded picture: RGBA8, opaque, top-left origin.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// What the page asks of a decoder thread.
pub enum Cmd {
    /// Go to `to` (s into the file); frames from then on carry `epoch`.
    Seek { to: f64, epoch: u32 },
    /// Hand over frames of this size from now on.
    Size(u32, u32),
    /// Start again at the end (true) or stop there.
    Loop(bool),
}

/// What a decoder thread hands back.
pub enum Msg {
    /// Length of the file (s); 0 when the file does not say.
    Info { duration: f64 },
    /// A frame and its time on the player's timeline (s, every loop adds the length).
    Frame { frame: Frame, t: f64, epoch: u32 },
    /// The last frame was handed over and the video does not loop.
    End { epoch: u32 },
    /// The file cannot be played.
    Failed,
}

/// Start decoding `path` (a file of the content folders, also one inside an archive) on a
/// thread of its own, frames at `size`. `None` where videos cannot be played; the thread ends
/// when the receiver is dropped.
#[cfg(windows)]
pub fn spawn(path: PathBuf, size: (u32, u32), looping: bool) -> Option<(Sender<Cmd>, Receiver<Msg>)> {
    mf::spawn(path, size, looping)
}

#[cfg(not(windows))]
pub fn spawn(path: PathBuf, _size: (u32, u32), _looping: bool) -> Option<(Sender<Cmd>, Receiver<Msg>)> {
    log::warn!("htmltexture: video {} cannot be played on this system (only on Windows); the poster is shown", path.display());
    None
}

/// BGRX rows (`pitch` bytes apart, negative for a bottom-up picture) to straight RGBA.
#[cfg_attr(not(windows), allow(dead_code))]
fn bgrx_to_rgba(scan0: *const u8, pitch: isize, w: u32, h: u32) -> Frame {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        // SAFETY: the caller guarantees `h` rows of `w` pixels starting at `scan0` + y * pitch
        let row = unsafe { std::slice::from_raw_parts(scan0.offset(y as isize * pitch), w as usize * 4) };
        let out = &mut rgba[y * w as usize * 4..(y + 1) * w as usize * 4];
        for (o, i) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
            o[0] = i[2];
            o[1] = i[1];
            o[2] = i[0];
            o[3] = 255;
        }
    }
    Frame { w, h, rgba }
}

/// The decoder on Windows: Media Foundation's source reader, asked for RGB32 frames at the
/// size of the box (its video processor converts and scales).
#[cfg(windows)]
mod mf {
    use super::*;
    use std::ffi::c_void;
    use std::path::Path;
    use std::sync::mpsc::{self, SyncSender};
    use std::sync::OnceLock;
    use windows::core::{Interface, GUID, HRESULT, HSTRING, PCWSTR};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::System::Variant::VT_I8;
    use windows::Win32::UI::Shell::SHCreateMemStream;

    const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    type Pp = *mut *mut c_void;

    /// The few plain functions of Media Foundation, found at run time.
    struct Api {
        startup: unsafe extern "system" fn(u32, u32) -> HRESULT,
        shutdown: unsafe extern "system" fn() -> HRESULT,
        create_attributes: unsafe extern "system" fn(Pp, u32) -> HRESULT,
        create_media_type: unsafe extern "system" fn(Pp) -> HRESULT,
        stream_on_istream: unsafe extern "system" fn(*mut c_void, Pp) -> HRESULT,
        reader_from_url: unsafe extern "system" fn(PCWSTR, *mut c_void, Pp) -> HRESULT,
        reader_from_stream: unsafe extern "system" fn(*mut c_void, *mut c_void, Pp) -> HRESULT,
    }

    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| {
            // SAFETY: system libraries, kept loaded for the life of the process; every symbol
            // is cast to its documented signature
            unsafe {
                let plat = LoadLibraryW(windows::core::w!("mfplat.dll")).ok()?;
                let rw = LoadLibraryW(windows::core::w!("mfreadwrite.dll")).ok()?;
                macro_rules! sym {
                    ($lib:expr, $name:literal) => {
                        std::mem::transmute(GetProcAddress($lib, windows::core::s!($name))?)
                    };
                }
                Some(Api {
                    startup: sym!(plat, "MFStartup"),
                    shutdown: sym!(plat, "MFShutdown"),
                    create_attributes: sym!(plat, "MFCreateAttributes"),
                    create_media_type: sym!(plat, "MFCreateMediaType"),
                    stream_on_istream: sym!(plat, "MFCreateMFByteStreamOnStream"),
                    reader_from_url: sym!(rw, "MFCreateSourceReaderFromURL"),
                    reader_from_stream: sym!(rw, "MFCreateSourceReaderFromByteStream"),
                })
            }
        })
        .as_ref()
    }

    /// The interface a creating function wrote out.
    unsafe fn made<T: Interface>(hr: HRESULT, p: *mut c_void) -> windows::core::Result<T> {
        hr.ok()?;
        if p.is_null() {
            return Err(windows::core::Error::from(windows::Win32::Foundation::E_POINTER));
        }
        Ok(unsafe { T::from_raw(p) })
    }

    pub(super) fn spawn(path: PathBuf, size: (u32, u32), looping: bool) -> Option<(Sender<Cmd>, Receiver<Msg>)> {
        let Some(api) = api() else {
            log::warn!("htmltexture: video {} cannot be played: Media Foundation is not installed (Windows N: Media Feature Pack)", path.display());
            return None;
        };
        let (ctx, crx) = mpsc::channel();
        let (mtx, mrx) = mpsc::sync_channel(AHEAD);
        let started = std::thread::Builder::new().name("htmltexture video".into()).spawn(move || {
            // SAFETY: COM and Media Foundation are set up and torn down on this thread only
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            if unsafe { (api.startup)(MF_VERSION, MFSTARTUP_NOSOCKET) }.is_ok() {
                match Decoder::open(api, &path, size) {
                    Ok(d) => d.run(looping, &crx, &mtx),
                    Err(e) => {
                        log::warn!("htmltexture: video {} cannot be played: {e}", path.display());
                        let _ = mtx.send(Msg::Failed);
                    }
                }
                unsafe {
                    let _ = (api.shutdown)();
                }
            } else {
                let _ = mtx.send(Msg::Failed);
            }
            if com {
                unsafe { CoUninitialize() };
            }
        });
        started.ok()?;
        Some((ctx, mrx))
    }

    struct Decoder {
        api: &'static Api,
        reader: IMFSourceReader,
        path: PathBuf,
        w: u32,
        h: u32,
        stride: i32,
        duration: f64,
    }

    impl Decoder {
        fn open(api: &'static Api, path: &Path, size: (u32, u32)) -> windows::core::Result<Decoder> {
            unsafe {
                let mut p = std::ptr::null_mut();
                let attrs: IMFAttributes = made((api.create_attributes)(&mut p, 2), p)?;
                attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
                let mut p = std::ptr::null_mut();
                let reader: IMFSourceReader = if omsi_cfg::vfs::is_virtual(path) {
                    // inside an archive: from memory
                    let bytes = omsi_cfg::vfs::read(path).map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;
                    let stream = SHCreateMemStream(Some(&bytes)).ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_OUTOFMEMORY))?;
                    let mut q = std::ptr::null_mut();
                    let bs: IMFByteStream = made((api.stream_on_istream)(stream.as_raw(), &mut q), q)?;
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    let mime = match ext.as_str() {
                        "mp4" | "m4v" | "mov" => Some(windows::core::w!("video/mp4")),
                        "wmv" => Some(windows::core::w!("video/x-ms-wmv")),
                        _ => None,
                    };
                    if let (Some(m), Ok(a)) = (mime, bs.cast::<IMFAttributes>()) {
                        let _ = a.SetString(&MF_BYTESTREAM_CONTENT_TYPE, m);
                    }
                    made((api.reader_from_stream)(bs.as_raw(), attrs.as_raw(), &mut p), p)?
                } else {
                    let url = HSTRING::from(path.as_os_str());
                    made((api.reader_from_url)(PCWSTR(url.as_ptr()), attrs.as_raw(), &mut p), p)?
                };
                reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
                reader.SetStreamSelection(STREAM, true)?;
                let duration = reader
                    .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
                    .map(|v| v.Anonymous.Anonymous.Anonymous.uhVal as f64 / 1e7)
                    .unwrap_or(0.0);
                let mut d = Decoder { api, reader, path: path.to_path_buf(), w: 0, h: 0, stride: 0, duration };
                d.set_size(size.0, size.1)?;
                log::info!("htmltexture: video {} plays at {}x{} ({:.1}s)", path.display(), d.w, d.h, duration);
                Ok(d)
            }
        }

        /// Ask for RGB32 frames of `w` x `h`; the file's own size when scaling is refused.
        fn set_size(&mut self, w: u32, h: u32) -> windows::core::Result<()> {
            unsafe {
                let ty = |sized: bool| -> windows::core::Result<IMFMediaType> {
                    let mut p = std::ptr::null_mut();
                    let t: IMFMediaType = made((self.api.create_media_type)(&mut p), p)?;
                    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                    if sized {
                        t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
                        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
                        t.SetUINT32(&MF_MT_DEFAULT_STRIDE, w * 4)?;
                    }
                    Ok(t)
                };
                if let Err(e) = self.reader.SetCurrentMediaType(STREAM, None, &ty(true)?) {
                    log::debug!("htmltexture: video {}: no scaled frames ({e}), the page scales them", self.path.display());
                    self.reader.SetCurrentMediaType(STREAM, None, &ty(false)?)?;
                }
            }
            self.read_type()
        }

        fn read_type(&mut self) -> windows::core::Result<()> {
            unsafe {
                let cur = self.reader.GetCurrentMediaType(STREAM)?;
                let fs = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
                self.w = (fs >> 32) as u32;
                self.h = fs as u32;
                self.stride = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(self.w as i32 * 4);
            }
            Ok(())
        }

        fn seek(&self, to: f64) -> windows::core::Result<()> {
            let mut v = PROPVARIANT::default();
            unsafe {
                (*v.Anonymous.Anonymous).vt = VT_I8;
                (*v.Anonymous.Anonymous).Anonymous.hVal = (to * 1e7) as i64;
                self.reader.SetCurrentPosition(&GUID::zeroed(), &v)
            }
        }

        /// The picture of a sample as RGBA.
        fn picture(&self, s: &IMFSample) -> windows::core::Result<Option<Frame>> {
            let (w, h) = (self.w, self.h);
            if w == 0 || h == 0 || w as u64 * h as u64 > MAX_FRAME_PX {
                return Ok(None);
            }
            unsafe {
                let buf = s.ConvertToContiguousBuffer()?;
                if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                    let mut scan0 = std::ptr::null_mut();
                    let mut pitch = 0i32;
                    b2.Lock2D(&mut scan0, &mut pitch)?;
                    let ok = !scan0.is_null() && pitch.unsigned_abs() >= w * 4;
                    let img = ok.then(|| bgrx_to_rgba(scan0, pitch as isize, w, h));
                    let _ = b2.Unlock2D();
                    return Ok(img);
                }
                let mut p = std::ptr::null_mut();
                let mut len = 0u32;
                buf.Lock(&mut p, None, Some(&mut len))?;
                let row = self.stride.unsigned_abs().max(w * 4);
                let img = if p.is_null() || (len as u64) < row as u64 * h as u64 {
                    None
                } else if self.stride < 0 {
                    Some(bgrx_to_rgba(p.add((row * (h - 1)) as usize), -(row as isize), w, h))
                } else {
                    Some(bgrx_to_rgba(p, row as isize, w, h))
                };
                let _ = buf.Unlock();
                Ok(img)
            }
        }

        fn run(mut self, mut looping: bool, cmds: &Receiver<Cmd>, out: &SyncSender<Msg>) {
            if out.send(Msg::Info { duration: self.duration }).is_err() {
                return;
            }
            let (mut epoch, mut offset, mut ended, mut last_t) = (0u32, 0.0f64, false, 0.0f64);
            // time of the first frame: H.264 files often start at the encoder's delay
            // (0.1-0.2 s), which would put every frame that late
            let mut t0: Option<f64> = None;
            loop {
                let cmd = if ended {
                    match cmds.recv() {
                        Ok(c) => Some(c),
                        Err(_) => return,
                    }
                } else {
                    match cmds.try_recv() {
                        Ok(c) => Some(c),
                        Err(TryRecvError::Empty) => None,
                        Err(TryRecvError::Disconnected) => return,
                    }
                };
                if let Some(c) = cmd {
                    match c {
                        Cmd::Seek { to, epoch: g } => {
                            epoch = g;
                            offset = 0.0;
                            ended = false;
                            // (the length counts from the first frame, a position past it is refused)
                            let raw = to + t0.unwrap_or(0.0);
                            let raw = if self.duration > 0.0 { raw.min(self.duration) } else { raw };
                            if let Err(e) = self.seek(raw) {
                                log::warn!("htmltexture: video {}: seek failed: {e}", self.path.display());
                            }
                        }
                        Cmd::Size(w, h) => {
                            if let Err(e) = self.set_size(w, h) {
                                log::warn!("htmltexture: video {}: new size refused: {e}", self.path.display());
                            }
                        }
                        Cmd::Loop(l) => looping = l,
                    }
                    continue;
                }
                let (mut flags, mut ts, mut sample) = (0u32, 0i64, None::<IMFSample>);
                // SAFETY: plain out-parameters
                let read = unsafe { self.reader.ReadSample(STREAM, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample)) };
                if let Err(e) = read.and_then(|_| if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 { Err(windows::Win32::Foundation::E_FAIL.into()) } else { Ok(()) }) {
                    log::warn!("htmltexture: video {} cannot be decoded: {e}", self.path.display());
                    let _ = out.send(Msg::Failed);
                    return;
                }
                if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                    let _ = self.read_type();
                }
                if let Some(s) = sample {
                    let raw = ts as f64 / 1e7;
                    let t = (raw - *t0.get_or_insert(raw)).max(0.0);
                    let dur = unsafe { s.GetSampleDuration() }.unwrap_or(0) as f64 / 1e7;
                    last_t = last_t.max(t + dur);
                    match self.picture(&s) {
                        Ok(Some(frame)) => {
                            if out.send(Msg::Frame { frame, t: offset + t, epoch }).is_err() {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => log::debug!("htmltexture: video {}: frame skipped: {e}", self.path.display()),
                    }
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    if self.duration <= 0.0 && last_t > 0.0 {
                        self.duration = last_t;
                        if out.send(Msg::Info { duration: last_t }).is_err() {
                            return;
                        }
                    }
                    if looping && self.duration > 0.0 {
                        offset += self.duration;
                        if let Err(e) = self.seek(0.0) {
                            log::warn!("htmltexture: video {}: cannot start again: {e}", self.path.display());
                            ended = true;
                        }
                    } else {
                        ended = true;
                        if out.send(Msg::End { epoch }).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}


//! Plays animated GIF, WebP and APNG pictures a frame at a time.
//!
//! Rune's image cache decodes every frame of an animation up front into RGBA plus mips, so a 60 KB
//! file can take 50 MB of RAM and a long GIF hundreds. Here only the encoded bytes are kept (tens of
//! kilobytes to a couple of megabytes) and one thread decodes the frame that is due, publishing it
//! to a `Pixels` the view draws, which wakes only the windows that show it. A loop costs a little
//! CPU (about 0.2 to 2 ms a frame) instead of memory, and nothing is decoded while the animation is
//! paused, which is whenever it is off screen, in an unfocused window, or no longer wanted.

use crate::net::Done;
use image::AnimationDecoder;
use rune::{Pixels, Waker};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

/// Largest canvas, in pixels, that is played (about 48 MB of working buffers); a bigger one stays a still.
pub const MAX_CANVAS_PIXELS: usize = 2048 * 2048;

type Frames = Box<dyn Iterator<Item = image::ImageResult<image::Frame>>>;

enum Cmd {
    Open { id: u64, url: String, bytes: Arc<[u8]>, pixels: Pixels },
    Play(u64, bool),
    Close(u64),
}

/// The handle the app keeps: commands to the decoding thread.
pub struct Anims {
    tx: Sender<Cmd>,
    next: u64,
}

impl Anims {
    /// `done` gets a `Done::Animated` when an animation shows its first frame, so the view can switch
    /// from the still picture to the moving one; `wake` runs the app's poll.
    pub fn start(done: Sender<Done>, wake: Waker) -> Anims {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("fluxer-anim".into()).spawn(move || run(rx, done, wake)).ok();
        Anims { tx, next: 1 }
    }

    /// Registers an animation, paused. Returns its id and the pixels its frames land in.
    pub fn open(&mut self, url: &str, bytes: Arc<[u8]>) -> (u64, Pixels) {
        let (id, pixels) = (self.next, Pixels::new());
        self.next += 1;
        let _ = self.tx.send(Cmd::Open { id, url: url.to_string(), bytes, pixels: pixels.clone() });
        (id, pixels)
    }

    pub fn play(&self, id: u64, on: bool) { let _ = self.tx.send(Cmd::Play(id, on)); }

    pub fn close(&self, id: u64) { let _ = self.tx.send(Cmd::Close(id)); }
}

struct Slot {
    url: String,
    bytes: Arc<[u8]>,
    pixels: Pixels,
    /// The decoder, alive only while playing: pausing frees its canvas.
    frames: Option<Frames>,
    due: Instant,
    playing: bool,
    shown: bool,
}

fn open_frames(bytes: &Arc<[u8]>) -> Option<Frames> {
    let cursor = Cursor::new(bytes.clone());
    if bytes.starts_with(b"GIF8") {
        Some(Box::new(image::codecs::gif::GifDecoder::new(cursor).ok()?.into_frames()))
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let mut d = image::codecs::webp::WebPDecoder::new(cursor).ok()?;
        // Browsers composite on transparency, whatever background the file names.
        d.set_background_color(image::Rgba([0, 0, 0, 0])).ok()?;
        Some(Box::new(d.into_frames()))
    } else {
        let d = image::codecs::png::PngDecoder::new(cursor).ok()?;
        if !d.is_apng().ok()? { return None }
        Some(Box::new(d.apng().ok()?.into_frames()))
    }
}

/// How long a frame shows: browsers show one of 10 ms or less for 100 ms, since such files mean
/// "as fast as you can", which would otherwise spin.
fn delay_of(frame: &image::Frame) -> Duration {
    let (n, d) = frame.delay().numer_denom_ms();
    let ms = f64::from(n) / f64::from(d.max(1));
    Duration::from_secs_f64(if ms <= 10.0 { 0.1 } else { ms / 1000.0 })
}

/// Decodes the next frame (the first again once the last has shown) and publishes it.
/// False when the picture cannot be decoded at all.
fn step(s: &mut Slot, now: Instant, done: &Sender<Done>, wake: &Waker) -> bool {
    // The second attempt is the loop starting over; a file that fails twice has nothing to show.
    for _ in 0..2 {
        if s.frames.is_none() { s.frames = open_frames(&s.bytes) }
        let Some(frames) = s.frames.as_mut() else { return false };
        match frames.next() {
            Some(Ok(frame)) => {
                let delay = delay_of(&frame);
                let buf = frame.into_buffer();
                s.pixels.set(buf.width(), buf.height(), buf.into_raw());
                // Frames are timed from the schedule, not from when they happened to be decoded, unless the thread fell behind.
                s.due = if now.saturating_duration_since(s.due) > delay { now + delay } else { s.due + delay };
                if !s.shown {
                    s.shown = true;
                    let _ = done.send(Done::Animated { url: s.url.clone() });
                    wake();
                }
                return true;
            }
            _ => s.frames = None,
        }
    }
    false
}

fn run(rx: Receiver<Cmd>, done: Sender<Done>, wake: Waker) {
    let mut slots: HashMap<u64, Slot> = HashMap::new();
    loop {
        let next_due = slots.values().filter(|s| s.playing).map(|s| s.due).min();
        let cmd = match next_due {
            Some(t) => match rx.recv_timeout(t.saturating_duration_since(Instant::now())) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() { Ok(c) => Some(c), Err(_) => return },
        };
        match cmd {
            Some(Cmd::Open { id, url, bytes, pixels }) => {
                slots.insert(id, Slot { url, bytes, pixels, frames: None, due: Instant::now(), playing: false, shown: false });
            }
            Some(Cmd::Play(id, on)) => if let Some(s) = slots.get_mut(&id) && s.playing != on {
                s.playing = on;
                s.due = Instant::now();
                if !on { s.frames = None }
            },
            Some(Cmd::Close(id)) => { slots.remove(&id); }
            None => {}
        }
        let now = Instant::now();
        for s in slots.values_mut().filter(|s| s.playing && s.due <= now) {
            if !step(s, now, &done, &wake) { s.playing = false }
        }
    }
}

/// A GIF of `n` frames on a `w` x `h` canvas, 20 ms each, frame `i` painting colour `i` in full.
#[cfg(test)]
pub fn test_gif(n: u8, w: u32, h: u32) -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for i in 0..n {
            let img = image::RgbaImage::from_pixel(w, h, image::Rgba([i.wrapping_mul(40), 0, 0, 255]));
            enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(20, 1))).unwrap();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gif(n: u8) -> Arc<[u8]> { test_gif(n, 4, 2).into() }

    #[test]
    fn frames_are_decoded_one_at_a_time_loop_and_report_the_first() {
        let (done, got) = channel();
        let mut anims = Anims::start(done, Arc::new(|| {}));
        let (id, pixels) = anims.open("u", gif(3));
        assert_eq!(pixels.size(), (0, 0), "nothing is decoded until it plays");
        anims.play(id, true);
        assert!(matches!(got.recv_timeout(Duration::from_secs(2)), Ok(Done::Animated { url }) if url == "u"));
        assert_eq!(pixels.size(), (4, 2), "frames are the full canvas");
        // Three frames of 20 ms loop: after a good while it is still going, with no further first-frame reports.
        std::thread::sleep(Duration::from_millis(300));
        assert!(got.try_recv().is_err());
        anims.play(id, false);
        let after_pause = pixels.size();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(after_pause, pixels.size());
    }

    #[test]
    fn an_undecodable_file_stops_instead_of_spinning() {
        let (done, got) = channel();
        let mut anims = Anims::start(done, Arc::new(|| {}));
        let (id, pixels) = anims.open("bad", Arc::from(&b"GIF89a not really a gif"[..]));
        anims.play(id, true);
        std::thread::sleep(Duration::from_millis(200));
        assert!(got.try_recv().is_err());
        assert_eq!(pixels.size(), (0, 0));
    }
}

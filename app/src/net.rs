//! Background network work for the UI: history pages, sends, endpoint refresh and media. Every job
//! runs on a short-lived thread, posts a [`Done`] and wakes Rune; `poll` drains them. Images are
//! decoded by Rune's own `ImageCache` workers.

use fluxer_client::api::{self, Api, Message};
use fluxer_client::rest::{self, Endpoints};
use rune::{Image, ImageCache, Pixels, Waker};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Largest media download we accept.
const MAX_MEDIA_BYTES: u64 = 12 * 1024 * 1024;
/// Largest file we upload (the server may allow less).
const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_IN_FLIGHT_IMAGES: usize = 24;
/// Decoded pictures nothing asks for any more are kept (so going back is instant) up to this much.
const COLD_BUDGET: usize = 64 << 20;
/// An animation nothing asks for any more (scrolled away, window unfocused) is let go after this.
const ANIMATION_GRACE: Duration = Duration::from_secs(15);
/// The media cache on disk is cut down to this at launch, oldest files first.
const DISK_CACHE_BYTES: u64 = 256 << 20;

/// A media type from the file extension; anything unknown is sent as plain bytes.
pub fn content_type_of(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png", Some("jpg" | "jpeg") => "image/jpeg", Some("gif") => "image/gif", Some("webp") => "image/webp",
        Some("mp4") => "video/mp4", Some("webm") => "video/webm", Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg", Some("ogg" | "opus") => "audio/ogg", Some("wav") => "audio/wav", Some("flac") => "audio/flac",
        Some("pdf") => "application/pdf", Some("zip") => "application/zip", Some("json") => "application/json", Some("txt" | "md") => "text/plain",
        _ => "application/octet-stream",
    }
}

fn cache_dir() -> Option<std::path::PathBuf> {
    let configured = std::env::var_os("FLUXER_RUX_CACHE_DIR").map(std::path::PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = configured.or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library").join("Caches")))?;
    #[cfg(not(target_os = "macos"))]
    let base = configured.or_else(|| std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    Some(base.join("fluxer-rune").join("media"))
}

/// Downloaded pictures are kept on disk so the next launch shows them at once.
fn cache_path(url: &str) -> Option<std::path::PathBuf> {
    // FNV-1a: stable across runs, unlike the std hasher's guarantees.
    let h = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    Some(cache_dir()?.join(format!("{h:016x}")))
}

/// Deletes the oldest cached pictures until the folder fits in `max` bytes (and any half-written
/// leftovers). Runs on its own thread at launch; nothing here touches the UI thread.
fn prune_cache(max: u64) {
    let Some(dir) = cache_dir() else { return };
    let Ok(read) = std::fs::read_dir(&dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, std::path::PathBuf)> = read.flatten().filter_map(|e| {
        let m = e.metadata().ok()?;
        m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
    }).collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        let partial = path.extension().is_some_and(|e| e == "part");
        if total <= max && !partial { continue }
        if std::fs::remove_file(&path).is_ok() { total = total.saturating_sub(len) }
    }
}

fn fetch_cached(url: &str) -> Result<Vec<u8>, String> {
    let path = cache_path(url);
    if let Some(bytes) = path.as_ref().and_then(|p| std::fs::read(p).ok()).filter(|b| !b.is_empty()) { return Ok(bytes) }
    let bytes = api::fetch_bytes(url, MAX_MEDIA_BYTES).map_err(|e| e.message)?;
    if let Some(p) = path {
        // Written beside, then renamed, so a half-written file is never read back.
        let tmp = p.with_extension("part");
        if std::fs::create_dir_all(p.parent().unwrap_or(&p)).is_ok() && std::fs::write(&tmp, &bytes).is_ok() { let _ = std::fs::rename(&tmp, &p); }
    }
    Ok(bytes)
}

pub fn profile_key(user: &str, guild: Option<&str>) -> String { format!("{user}@{}", guild.unwrap_or("")) }

pub enum Done {
    History { channel: String, res: Result<Vec<Message>, String> },
    Sent { channel: String, local: String, res: Result<Message, String> },
    /// A downloaded picture, and its canvas and frame count when it is an animation.
    Image { url: String, res: Result<(Vec<u8>, Option<(usize, usize, usize)>), String> },
    /// An animation showed its first frame: the picture can switch from its still to the moving one.
    Animated { url: String },
    Endpoints(Result<Endpoints, String>),
    Reacted(Result<(), String>),
    ProfileSaved { guild: Option<String>, res: Result<serde_json::Value, String> },
    /// A message was edited or deleted; the gateway echo will confirm it too.
    Edited(Result<Message, String>),
    Deleted { channel: String, id: String, res: Result<(), String> },
    /// A read-state acknowledgement (failures are quiet: it is retried on the next message).
    Acked(Result<(), String>),
    /// The account's avatar (None when it was removed).
    AvatarSaved(Result<serde_json::Value, String>),
    /// The account's profile time zone.
    TimezoneSaved(Result<serde_json::Value, String>),
    /// Messages matching a channel search; the query is echoed so a late answer for an older
    /// query does not replace the newer results.
    Search { query: String, res: Result<api::SearchOutcome, String> },
    /// A page centered on a message (a search hit outside the loaded history).
    HistoryAround { channel: String, target: String, res: Result<Vec<Message>, String> },
    Profile { key: String, res: Result<api::Profile, String> },
    /// A message sent without a local copy (a sticker): the gateway brings it back; only failures matter.
    Posted(Result<(), String>),
}

/// A picture the UI can draw: its pixels, and its size once known.
#[derive(Clone)]
pub struct Shown {
    pub pixels: Pixels,
    pub size: Option<[u32; 2]>,
}

enum Kind {
    Still(Image),
    /// Played by `anim::Anims`: only the encoded bytes and the current frame are held.
    Animation { id: u64, pixels: Pixels, encoded: usize, canvas: (usize, usize), shown: bool, playing: bool },
}

/// A picture the app holds, and what it needs to decide when to let it go.
struct Entry {
    kind: Kind,
    /// Since when nothing on screen or about to be asks for it.
    cold_since: Option<Instant>,
}

impl Entry {
    /// Memory it holds: the mips of a still, or an animation's encoded bytes and working frames.
    fn bytes(&self) -> usize {
        match &self.kind {
            Kind::Still(img) => img.size().map_or(0, |[w, h]| w as usize * h as usize * 16 / 3),
            Kind::Animation { encoded, canvas, .. } => encoded + canvas.0 * canvas.1 * 4 * 3,
        }
    }
}

pub struct Net {
    api: Option<Arc<Api>>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    wake: Option<Waker>,
    cache: Option<ImageCache>,
    logo: Option<Image>,
    ready: HashMap<String, Entry>,
    requested: HashSet<String>,
    anims: Option<crate::anim::Anims>,
    /// Animated pictures whose element is on screen (reported by the view), and whether the window
    /// has the focus: an animation plays only when both hold and it is still wanted.
    on_screen: HashSet<String>,
    focused: bool,
    queue: VecDeque<String>,
    in_flight: usize,
}

impl Default for Net {
    fn default() -> Self {
        let (tx, rx) = channel();
        Net { api: None, tx, rx, wake: None, cache: None, logo: None, ready: HashMap::new(), requested: HashSet::new(), anims: None, on_screen: HashSet::new(), focused: true, queue: VecDeque::new(), in_flight: 0 }
    }
}

impl Net {
    pub fn set_waker(&mut self, wake: Waker) {
        let cache = ImageCache::new(wake.clone());
        self.logo = Some(cache.svg(&include_bytes!("../assets/logo.svg")[..], 128.0, 2.0));
        self.cache = Some(cache);
        self.anims = Some(crate::anim::Anims::start(self.tx.clone(), wake.clone()));
        self.wake = Some(wake);
        std::thread::Builder::new().name("fluxer-prune".into()).spawn(|| prune_cache(DISK_CACHE_BYTES)).ok();
    }

    pub fn set_api(&mut self, api: Option<Arc<Api>>) { self.api = api }
    pub fn api(&self) -> Option<&Arc<Api>> { self.api.as_ref() }
    pub fn has_api(&self) -> bool { self.api.is_some() }

    fn job<T: Send + 'static>(&self, work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) -> Done + Send + 'static) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        std::thread::spawn(move || {
            let _ = tx.send(done(work()));
            if let Some(w) = wake { w() }
        });
    }

    /// The newest page of a channel, or the page before message `before`.
    pub fn history(&self, channel: &str, before: Option<&str>) {
        let Some(api) = self.api.clone() else { return };
        let (channel, before) = (channel.to_string(), before.map(String::from));
        let c = channel.clone();
        self.job(move || api.messages(&c, crate::model::PAGE as u32, before.as_deref()).map_err(|e| e.message),
            move |res| Done::History { channel, res });
    }

    pub fn send(&self, channel: &str, local: &str, content: &str) {
        let Some(api) = self.api.clone() else { return };
        let (channel, local, content) = (channel.to_string(), local.to_string(), content.to_string());
        let (c, n, body) = (channel.clone(), local.clone(), content);
        // The nonce lets the server tell a retry from a new message.
        self.job(move || api.send(&c, &body, &n).map_err(|e| e.message), move |res| Done::Sent { channel, local, res });
    }

    pub fn send_sticker(&self, channel: &str, sticker: &str) {
        let Some(api) = self.api.clone() else { return };
        let (c, st) = (channel.to_string(), sticker.to_string());
        let nonce = crate::clock::now_ms().to_string();
        self.job(move || api.send_sticker(&c, &st, &nonce).map(|_| ()).map_err(|e| e.message), Done::Posted);
    }

    /// Sends `content` as a reply to `reply_to`; `local` is the optimistic message id.
    pub fn send_reply(&self, channel: &str, local: &str, content: &str, reply_to: &str) {
        let Some(api) = self.api.clone() else { return };
        let (channel, local, content, reply_to) = (channel.to_string(), local.to_string(), content.to_string(), reply_to.to_string());
        let (c, n, body, r) = (channel.clone(), local.clone(), content, reply_to);
        self.job(move || api.send_reply(&c, &body, &n, &r).map_err(|e| e.message),
            move |res| Done::Sent { channel, local, res });
    }

    /// Rewrites one of this account's messages.
    pub fn edit_message(&self, channel: &str, id: &str, content: &str) {
        let Some(api) = self.api.clone() else { return };
        let (c, m, body) = (channel.to_string(), id.to_string(), content.to_string());
        self.job(move || api.edit_message(&c, &m, &body).map_err(|e| e.message), Done::Edited);
    }

    /// Deletes one of this account's messages.
    pub fn delete_message(&self, channel: &str, id: &str) {
        let Some(api) = self.api.clone() else { return };
        let (c, m) = (channel.to_string(), id.to_string());
        let (channel, mid) = (c.clone(), m.clone());
        self.job(move || api.delete_message(&c, &m).map_err(|e| e.message), move |res| Done::Deleted { channel, id: mid, res });
    }

    /// Marks a channel read up to `message_id`.
    pub fn ack(&self, channel: &str, message_id: &str) {
        let Some(api) = self.api.clone() else { return };
        let (c, m) = (channel.to_string(), message_id.to_string());
        self.job(move || api.ack(&c, &m).map_err(|e| e.message), Done::Acked);
    }

    /// Searches the server (`guild`) or a single channel.
    pub fn search(&self, guild: Option<&str>, channel: Option<&str>, query: &str) {
        let Some(api) = self.api.clone() else { return };
        let (g, c, q) = (guild.map(String::from), channel.map(String::from), query.to_string());
        let echo = q.clone();
        self.job(move || api.search(g.as_deref(), c.as_deref(), &q).map_err(|e| e.message),
            move |res| Done::Search { query: echo, res });
    }

    /// Loads the page around `target` when a search hit is not in the loaded history.
    pub fn history_around(&self, channel: &str, target: &str) {
        let Some(api) = self.api.clone() else { return };
        let (c, t) = (channel.to_string(), target.to_string());
        let (cc, tt) = (c.clone(), t.clone());
        self.job(move || api.messages_around(&c, &t, crate::model::PAGE as u32).map_err(|e| e.message),
            move |res| Done::HistoryAround { channel: cc, target: tt, res });
    }

    /// Sets (a base64 PNG) or removes the account's avatar.
    pub fn set_avatar(&self, avatar: Option<String>) {
        let Some(api) = self.api.clone() else { return };
        self.job(move || api.patch_profile(None, &serde_json::json!({ "avatar": avatar })).map_err(|e| e.message), Done::AvatarSaved);
    }

    /// Saves the account's IANA time zone for the profile's local time.
    pub fn save_timezone(&self, timezone: String) {
        let Some(api) = self.api.clone() else { return };
        self.job(move || api.patch_profile(None, &serde_json::json!({ "timezone": timezone })).map_err(|e| e.message), Done::TimezoneSaved);
    }

    /// Reads, squares and encodes a picture for the avatar, then saves it (blocking, off-thread).
    pub fn upload_avatar(&self, path: std::path::PathBuf) {
        let Some(api) = self.api.clone() else { return };
        self.job(move || {
            let data = std::fs::read(&path).map_err(|e| format!("não foi possível ler o arquivo: {e}"))?;
            let image = image::load_from_memory(&data).map_err(|e| format!("imagem inválida: {e}"))?;
            let image = image.resize_to_fill(512, 512, image::imageops::FilterType::Lanczos3);
            let mut png = Vec::new();
            image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| format!("não foi possível converter a imagem: {e}"))?;
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
            api.patch_profile(None, &serde_json::json!({ "avatar": b64 })).map_err(|e| e.message)
        }, Done::AvatarSaved);
    }

    /// Saves the profile form: the account's own (`guild` None) or the one in a server.
    pub fn save_profile(&self, guild: Option<&str>, body: serde_json::Value) {
        let Some(api) = self.api.clone() else { return };
        let guild = guild.map(String::from);
        let g = guild.clone();
        self.job(move || api.patch_profile(g.as_deref(), &body).map_err(|e| e.message), move |res| Done::ProfileSaved { guild, res });
    }

    pub fn react(&self, channel: &str, message: &str, emoji: &str, add: bool) {
        let Some(api) = self.api.clone() else { return };
        let (c, m, e) = (channel.to_string(), message.to_string(), emoji.to_string());
        self.job(move || api.react(&c, &m, &e, add).map_err(|e| e.message), Done::Reacted);
    }

    /// A person's profile, kept under `key` (user id, and server id when there is one).
    pub fn profile(&self, user: &str, guild: Option<&str>) {
        let Some(api) = self.api.clone() else { return };
        let (user, guild) = (user.to_string(), guild.map(String::from));
        let key = profile_key(&user, guild.as_deref());
        self.job(move || api.profile(&user, guild.as_deref()).map_err(|e| e.message), move |res| Done::Profile { key, res });
    }

    /// Uploads a file from disk as a message with `content` (blocking read and upload, off-thread).
    pub fn send_file(&self, channel: &str, content: &str, path: std::path::PathBuf) {
        let Some(api) = self.api.clone() else { return };
        let (c, text) = (channel.to_string(), content.to_string());
        let nonce = crate::clock::now_ms().to_string();
        self.job(move || {
            let meta = std::fs::metadata(&path).map_err(|e| format!("não foi possível ler o arquivo: {e}"))?;
            if meta.len() > MAX_UPLOAD_BYTES { return Err(format!("o arquivo passa de {} MB", MAX_UPLOAD_BYTES / 1_048_576)) }
            let data = std::fs::read(&path).map_err(|e| format!("não foi possível ler o arquivo: {e}"))?;
            let name = path.file_name().map_or("file".to_string(), |n| n.to_string_lossy().to_string());
            api.send_file(&c, &text, &name, content_type_of(&name), &data, &nonce).map(|_| ()).map_err(|e| e.message)
        }, Done::Posted);
    }

    /// Looks the instance up again: gateway URLs and the media host may have moved since the
    /// session was saved, and old sessions did not store the media host at all.
    pub fn refresh_endpoints(&self, instance: &str) {
        let instance = instance.to_string();
        self.job(move || rest::discover(&instance).map_err(|e| e.message), Done::Endpoints);
    }

    /// Asks for an image once; it arrives through `drain` and then `image`.
    pub fn want_image(&mut self, url: &str) {
        if self.requested.contains(url) { return }
        self.requested.insert(url.to_string());
        // Animated pictures are the slow ones and what the open channel is waiting on: they go first.
        if url.contains("animated=true") { self.queue.push_front(url.to_string()) } else { self.queue.push_back(url.to_string()) }
        self.pump_images();
    }

    /// Moves a queued picture to the front (what the screen shows now goes before what was only
    /// fetched ahead of time); asks for it when it was never asked for.
    pub fn want_image_first(&mut self, url: &str) {
        if let Some(i) = self.queue.iter().position(|u| u == url) { self.queue.remove(i); self.queue.push_front(url.to_string()); self.pump_images(); return }
        if self.requested.contains(url) { return }
        self.requested.insert(url.to_string());
        self.queue.push_front(url.to_string());
        self.pump_images();
    }

    fn pump_images(&mut self) {
        while self.in_flight < MAX_IN_FLIGHT_IMAGES {
            let Some(url) = self.queue.pop_front() else { return };
            self.in_flight += 1;
            let u = url.clone();
            self.job(move || fetch_cached(&u).and_then(|bytes| match crate::animsize::canvas(&bytes) {
                Some((w, h, _)) if w * h > crate::anim::MAX_CANVAS_PIXELS => Err(format!("animation canvas too large ({w}x{h})")),
                c => Ok((bytes, c)),
            }), move |res| Done::Image { url, res });
        }
    }

    pub fn logo(&self) -> Option<&Image> { self.logo.as_ref() }

    /// A still picture (not an animation: see `shown`).
    pub fn image(&self, url: &str) -> Option<&Image> {
        match &self.ready.get(url)?.kind { Kind::Still(img) => Some(img), _ => None }
    }

    /// What to draw for `url`: a still, or an animation once its first frame is decoded.
    pub fn shown(&self, url: &str) -> Option<Shown> {
        match &self.ready.get(url)?.kind {
            Kind::Still(img) => Some(Shown { pixels: img.pixels().clone(), size: img.size() }),
            Kind::Animation { pixels, shown: true, .. } => Some(Shown { pixels: pixels.clone(), size: Some({ let (w, h) = pixels.size(); [w, h] }) }),
            Kind::Animation { .. } => None,
        }
    }

    /// How many animations are playing now.
    #[cfg(test)]
    pub fn playing_count(&self) -> usize { self.ready.values().filter(|e| matches!(e.kind, Kind::Animation { playing: true, .. })).count() }

    /// Memory held by loaded pictures (an animation: its encoded bytes and working frames).
    pub fn held_bytes(&self) -> usize { self.ready.values().map(Entry::bytes).sum() }

    /// The view says the element showing animated picture `url` came on screen or left it.
    pub fn set_on_screen(&mut self, url: &str, on: bool) {
        if on { self.on_screen.insert(url.to_string()); } else { self.on_screen.remove(url); }
        self.apply_playing(url);
    }

    /// The window gained or lost the focus: animations stop in the background and resume after.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        let urls: Vec<String> = self.ready.iter().filter(|(_, e)| matches!(e.kind, Kind::Animation { .. })).map(|(u, _)| u.clone()).collect();
        for u in urls { self.apply_playing(&u) }
    }

    /// Plays or pauses the animation at `url` to match what it should be doing now.
    fn apply_playing(&mut self, url: &str) {
        let want = self.focused && self.on_screen.contains(url);
        let Some(e) = self.ready.get_mut(url) else { return };
        let Kind::Animation { id, playing, .. } = &mut e.kind else { return };
        // A picture nothing asks for any more is paused whatever the view last said: its element may
        // be gone without a last report.
        let want = want && e.cold_since.is_none();
        if *playing != want {
            *playing = want;
            if let Some(a) = &self.anims { a.play(*id, want) }
        }
    }

    /// Lets go of pictures the screen no longer needs. `keep` is everything wanted now. Pictures
    /// nothing wants are paused at once; animations go after a few seconds, stills stay while the
    /// unwanted ones fit in a budget, oldest first. What is let go is asked for again, from the disk
    /// cache, if it is wanted later; what is still drawn stays alive in the tree that draws it.
    pub fn trim(&mut self, keep: &HashSet<String>) {
        let now = Instant::now();
        for (url, e) in self.ready.iter_mut() {
            if keep.contains(url) { e.cold_since = None } else if e.cold_since.is_none() { e.cold_since = Some(now) }
        }
        let urls: Vec<String> = self.ready.iter().filter(|(_, e)| matches!(e.kind, Kind::Animation { .. })).map(|(u, _)| u.clone()).collect();
        for u in urls { self.apply_playing(&u) }
        let mut gone: Vec<String> = self.ready.iter()
            .filter(|(_, e)| matches!(e.kind, Kind::Animation { .. }) && e.cold_since.is_some_and(|t| now.duration_since(t) >= ANIMATION_GRACE))
            .map(|(u, _)| u.clone()).collect();
        let mut cold: Vec<(Instant, &String, usize)> = self.ready.iter()
            .filter(|(u, _)| !gone.contains(u))
            .filter_map(|(u, e)| Some((e.cold_since?, u, e.bytes()))).collect();
        let mut total: usize = cold.iter().map(|c| c.2).sum();
        cold.sort_by_key(|c| c.0);
        for (_, url, bytes) in cold {
            if total <= COLD_BUDGET { break }
            total -= bytes;
            gone.push(url.clone());
        }
        for url in gone {
            if let Some(Entry { kind: Kind::Animation { id, .. }, .. }) = self.ready.remove(&url) && let Some(a) = &self.anims { a.close(id) }
            self.requested.remove(&url);
        }
    }

    /// Drains finished jobs. Images are handled here; the rest is returned for the app to apply.
    pub fn drain(&mut self) -> Vec<Done> {
        let mut out = Vec::new();
        while let Ok(d) = self.rx.try_recv() {
            match d {
                Done::Image { url, res } => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    if let (Ok((bytes, canvas)), Some(cache)) = (res, &self.cache) {
                        let kind = match (canvas, &mut self.anims) {
                            (Some((w, h, _)), Some(anims)) => {
                                let encoded = bytes.len();
                                let (id, pixels) = anims.open(&url, bytes.into());
                                Kind::Animation { id, pixels, encoded, canvas: (w, h), shown: false, playing: false }
                            }
                            _ => Kind::Still(cache.load(bytes)),
                        };
                        self.ready.insert(url.clone(), Entry { kind, cold_since: None });
                        self.apply_playing(&url);
                    }
                    self.pump_images();
                }
                Done::Animated { url } => {
                    if let Some(Entry { kind: Kind::Animation { shown, .. }, .. }) = self.ready.get_mut(&url) { *shown = true }
                    out.push(Done::Animated { url });
                }
                other => out.push(other),
            }
        }
        out
    }

    /// True while some image is still downloading or queued.
    pub fn busy_with_images(&self) -> bool { self.in_flight > 0 || !self.queue.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn images_are_fetched_once_decoded_and_served_from_the_cache() {
        // A 1x1 PNG.
        const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
            0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0, 5, 0xFE, 2, 0xFE, 0xDC, 0xCC, 0x59, 0xE7,
            0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];
        // SAFETY-free but process-wide: keeps the test's download out of the real cache.
        unsafe { std::env::set_var("FLUXER_RUX_CACHE_DIR", std::env::temp_dir().join(format!("fluxer-cache-{}", std::process::id()))) };
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/a.png", l.local_addr().unwrap());
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || while let Ok((mut c, _)) = l.accept() {
            h.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut buf = [0u8; 2048];
            let _ = c.read(&mut buf);
            let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", PNG.len());
            let _ = c.write_all(PNG);
        });
        let mut net = Net::default();
        net.set_waker(Arc::new(|| {}));
        net.want_image(&url);
        net.want_image(&url);
        let start = std::time::Instant::now();
        while net.image(&url).is_none() {
            net.drain();
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "image never arrived");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(hits.load(std::sync::atomic::Ordering::Relaxed), 1, "asked twice, downloaded once");
        assert!(!net.busy_with_images());
    }
}

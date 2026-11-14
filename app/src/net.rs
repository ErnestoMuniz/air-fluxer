//! Background network work for the UI: history pages, sends, endpoint refresh and media. Every job
//! runs on a short-lived thread, posts a [`Done`] and wakes Rune; `poll` drains them. Images are
//! decoded by Rune's own `ImageCache` workers.

use fluxer_client::api::{self, Api, Message};
use fluxer_client::rest::{self, Endpoints};
use rune::{Image, ImageCache, Waker};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;

/// Largest media download we accept.
const MAX_MEDIA_BYTES: u64 = 12 * 1024 * 1024;
/// Largest file we upload (the server may allow less).
const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_IN_FLIGHT_IMAGES: usize = 24;

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

/// Downloaded pictures are kept on disk so the next launch shows them at once.
fn cache_path(url: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var_os("FLUXER_RUX_CACHE_DIR").map(std::path::PathBuf::from).or_else(|| std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    // FNV-1a: stable across runs, unlike the std hasher's guarantees.
    let h = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    Some(base.join("fluxer-rune").join("media").join(format!("{h:016x}")))
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
    Image { url: String, res: Result<Vec<u8>, String> },
    Endpoints(Result<Endpoints, String>),
    Reacted(Result<(), String>),
    ProfileSaved { guild: Option<String>, res: Result<serde_json::Value, String> },
    Profile { key: String, res: Result<api::Profile, String> },
    /// A message sent without a local copy (a sticker): the gateway brings it back; only failures matter.
    Posted(Result<(), String>),
}

pub struct Net {
    api: Option<Arc<Api>>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    wake: Option<Waker>,
    cache: Option<ImageCache>,
    logo: Option<Image>,
    ready: HashMap<String, Image>,
    requested: HashSet<String>,
    queue: VecDeque<String>,
    in_flight: usize,
}

impl Default for Net {
    fn default() -> Self {
        let (tx, rx) = channel();
        Net { api: None, tx, rx, wake: None, cache: None, logo: None, ready: HashMap::new(), requested: HashSet::new(), queue: VecDeque::new(), in_flight: 0 }
    }
}

impl Net {
    pub fn set_waker(&mut self, wake: Waker) {
        let cache = ImageCache::new(wake.clone());
        self.logo = Some(cache.svg(&include_bytes!("../assets/logo.svg")[..], 128.0, 2.0));
        self.cache = Some(cache);
        self.wake = Some(wake);
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
            self.job(move || fetch_cached(&u), move |res| Done::Image { url, res });
        }
    }

    pub fn logo(&self) -> Option<&Image> { self.logo.as_ref() }

    pub fn image(&self, url: &str) -> Option<&Image> { self.ready.get(url) }

    /// Drains finished jobs. Images are handled here; the rest is returned for the app to apply.
    pub fn drain(&mut self) -> Vec<Done> {
        let mut out = Vec::new();
        while let Ok(d) = self.rx.try_recv() {
            match d {
                Done::Image { url, res } => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    if let (Ok(bytes), Some(cache)) = (res, &self.cache) {
                        self.ready.insert(url, cache.load(bytes));
                    }
                    self.pump_images();
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

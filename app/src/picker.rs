//! The GIF / media / sticker / emoji picker next to the message box: what it shows and the
//! requests behind it. Network calls run on short threads and report through a channel that
//! `poll` drains, like the rest of the app.

use fluxer_client::api::{Api, attachment_image_url};
use rune::{TextEdit, Waker};
use serde_json::Value;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab { Gifs, Media, Stickers, Emojis }

#[derive(Clone, Debug, PartialEq)]
pub struct Gif {
    pub id: String, pub url: String, pub src: String, pub width: u32, pub height: u32,
    /// A small animated GIF of it (the provider's "tinygif"), which the grid plays.
    pub anim: Option<String>,
}

impl Gif {
    /// A still, small version for the grid.
    pub fn preview_url(&self) -> String { attachment_image_url(&self.src, 360) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Category { pub name: String, pub src: String, pub anim: Option<String> }

impl Category {
    pub fn preview_url(&self) -> String { attachment_image_url(&self.src, 400) }
}

/// A picture the user saved ("favorite meme").
#[derive(Clone, Debug, PartialEq)]
pub struct Meme { pub id: String, pub name: String, pub url: String, pub width: u32, pub height: u32 }

impl Meme {
    pub fn preview_url(&self) -> String { attachment_image_url(&self.url, 360) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GifView { Home, Trending, Search(String) }

enum Msg {
    Featured(Result<(Vec<Category>, Vec<Gif>), String>),
    Gifs(GifView, Result<Vec<Gif>, String>),
    Memes(Result<Vec<Meme>, String>),
}

pub struct Picker {
    pub open: Option<Tab>,
    pub query: TextEdit,
    pub view: GifView,
    pub categories: Vec<Category>,
    pub trending: Vec<Gif>,
    pub results: Vec<Gif>,
    pub loading: bool,
    pub memes: Option<Vec<Meme>>,
    pub emoji_group: usize,
    pub error: Option<String>,
    /// The animated version of the tile the pointer is over: the only one that is loaded and plays.
    pub hover: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    wake: Option<Waker>,
}

impl Default for Picker {
    fn default() -> Self {
        let (tx, rx) = channel();
        Picker { open: None, query: TextEdit::new(""), view: GifView::Home, categories: vec![], trending: vec![], results: vec![], loading: false,
            memes: None, emoji_group: 0, error: None, hover: None, tx, rx, wake: None }
    }
}

fn encode(q: &str) -> String {
    q.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn s(v: &Value, k: &str) -> Option<String> { v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from) }

fn gif_of(v: &Value) -> Option<Gif> {
    let src = s(v, "proxy_src").or_else(|| s(v, "src"))?;
    // Small formats first: they download and decode quickly and are all a grid tile needs.
    let anim = ["tinygif", "nanogif", "mediumgif", "gif"].iter().find_map(|k| v.pointer(&format!("/media/{k}")).and_then(|m| s(m, "proxy_src").or_else(|| s(m, "src"))));
    Some(Gif { id: s(v, "id")?, url: s(v, "url").unwrap_or_else(|| src.clone()), src, anim,
        width: v.get("width").and_then(Value::as_u64).unwrap_or(0) as u32, height: v.get("height").and_then(Value::as_u64).unwrap_or(0) as u32 })
}

fn gifs_of(v: &Value) -> Vec<Gif> { v.as_array().into_iter().flatten().filter_map(gif_of).collect() }

impl Picker {
    pub fn set_waker(&mut self, w: Waker) { self.wake = Some(w) }

    fn job(&self, api: &Option<Arc<Api>>, f: impl FnOnce(&Api) -> Msg + Send + 'static) {
        let Some(api) = api.clone() else { return };
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        std::thread::spawn(move || { let _ = tx.send(f(&api)); if let Some(w) = wake { w() } });
    }

    /// Opens the picker on `tab` (closing it when it is already there) and starts what that tab needs.
    pub fn toggle(&mut self, tab: Tab, api: &Option<Arc<Api>>) {
        if self.open == Some(tab) { self.open = None; return }
        self.open = Some(tab);
        self.query = TextEdit::new("");
        self.error = None;
        match tab {
            Tab::Gifs => {
                self.view = GifView::Home;
                if self.categories.is_empty() { self.request_featured(api) }
            }
            Tab::Media => if self.memes.is_none() {
                self.loading = true;
                self.job(api, |a| Msg::Memes(a.get("/users/@me/memes").map_err(|e| e.message).map(|v| v.as_array().into_iter().flatten().filter_map(|m| Some(Meme {
                    id: s(m, "id")?, name: s(m, "name").unwrap_or_default(), url: s(m, "url")?,
                    width: m.get("width").and_then(Value::as_u64).unwrap_or(0) as u32, height: m.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
                })).collect())));
            },
            _ => {}
        }
    }

    pub fn close(&mut self) { self.open = None; self.hover = None }

    /// Loads the GIF home page ahead of time, so the picker opens with its pictures already there.
    pub fn prefetch(&mut self, api: &Option<Arc<Api>>) {
        if !self.categories.is_empty() || self.loading { return }
        self.request_featured(api);
    }

    fn request_featured(&mut self, api: &Option<Arc<Api>>) {
        self.loading = true;
        self.job(api, |a| Msg::Featured(a.get("/gifs/featured?locale=en-US").map_err(|e| e.message).map(|v| (
            v.get("categories").and_then(Value::as_array).into_iter().flatten()
                .filter_map(|c| Some(Category { name: s(c, "name")?, src: s(c, "proxy_src").or_else(|| s(c, "src"))?, anim: c.get("gif").and_then(gif_of).and_then(|g| g.anim) })).collect(),
            gifs_of(v.get("gifs").unwrap_or(&Value::Null))))));
    }

    /// The home page's pictures, wanted even while the picker is closed.
    pub fn home_images(&self) -> Vec<String> {
        self.categories.iter().map(Category::preview_url).chain(self.trending.first().map(Gif::preview_url)).collect()
    }

    pub fn show_home(&mut self) { self.view = GifView::Home; self.query = TextEdit::new("") }

    pub fn show_trending(&mut self, api: &Option<Arc<Api>>) {
        self.view = GifView::Trending;
        if !self.trending.is_empty() { return }
        self.loading = true;
        self.job(api, |a| Msg::Gifs(GifView::Trending, a.get("/gifs/trending?locale=en-US").map_err(|e| e.message).map(|v| gifs_of(&v))));
    }

    pub fn search(&mut self, q: &str, api: &Option<Arc<Api>>) {
        let q = q.trim().to_string();
        if q.is_empty() { self.show_home(); return }
        self.view = GifView::Search(q.clone());
        self.results.clear();
        self.loading = true;
        let path = format!("/gifs/search?q={}&locale=en-US", encode(&q));
        self.job(api, move |a| Msg::Gifs(GifView::Search(q), a.get(&path).map_err(|e| e.message).map(|v| gifs_of(&v))));
    }

    /// Applies finished requests; true when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(m) = self.rx.try_recv() {
            changed = true;
            self.loading = false;
            match m {
                Msg::Featured(Ok((c, g))) => { self.categories = c; self.trending = g }
                Msg::Gifs(GifView::Trending, Ok(g)) => self.trending = g,
                Msg::Gifs(v, Ok(g)) => if self.view == v { self.results = g },
                Msg::Memes(Ok(m)) => self.memes = Some(m),
                Msg::Featured(Err(e)) | Msg::Gifs(_, Err(e)) | Msg::Memes(Err(e)) => self.error = Some(e),
            }
        }
        changed
    }

    /// The playing versions of what the open tab shows, loaded after the still ones.
    /// The moving pictures to load: only the one under the pointer (every frame of an animation is
    /// decoded to memory, so a page of them would take hundreds of megabytes). The rest show stills.
    pub fn animated_images(&self) -> Vec<String> {
        if self.open != Some(Tab::Gifs) { return vec![] }
        self.hover.iter().cloned().collect()
    }

    pub fn wanted_images(&self) -> Vec<String> {
        match self.open {
            Some(Tab::Gifs) => match &self.view {
                GifView::Home => self.categories.iter().map(Category::preview_url).chain(self.trending.first().map(Gif::preview_url)).collect(),
                GifView::Trending => self.trending.iter().map(Gif::preview_url).collect(),
                GifView::Search(_) => self.results.iter().map(Gif::preview_url).collect(),
            },
            Some(Tab::Media) => self.memes.iter().flatten().map(Meme::preview_url).collect(),
            _ => vec![],
        }
    }
}

/// Emoji groups in the order of the picker's sidebar, each with the emoji that stands for it.
pub fn emoji_groups() -> [(&'static str, &'static str, Vec<emojis::Group>); 8] {
    use emojis::Group::*;
    [
        ("People", "😀", vec![SmileysAndEmotion, PeopleAndBody]),
        ("Nature", "🌿", vec![AnimalsAndNature]),
        ("Food", "🍔", vec![FoodAndDrink]),
        ("Activities", "🎮", vec![Activities]),
        ("Travel", "🚲", vec![TravelAndPlaces]),
        ("Objects", "🧲", vec![Objects]),
        ("Symbols", "❤", vec![Symbols]),
        ("Flags", "🏁", vec![Flags]),
    ]
}

pub fn emojis_in(groups: &[emojis::Group]) -> Vec<&'static emojis::Emoji> {
    emojis::iter().filter(|e| groups.contains(&e.group()) && e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default)).collect()
}

/// Emoji whose name or shortcode contains `q`.
pub fn emoji_search(q: &str) -> Vec<&'static emojis::Emoji> {
    let q = q.trim().to_lowercase();
    if q.is_empty() { return vec![] }
    emojis::iter().filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default)
        && (e.name().to_lowercase().contains(&q) || e.shortcodes().any(|c| c.contains(&q)))).take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_percent_encoded() {
        assert_eq!(encode("good cat/ç"), "good%20cat%2F%C3%A7");
    }

    #[test]
    fn gifs_parse_from_the_provider_shape_and_skip_broken_ones() {
        let v: Value = serde_json::json!([{"id": "1", "url": "https://k/1", "src": "https://s/1.gif", "proxy_src": "https://p/1.gif", "width": 200, "height": 100}, {"nothing": true}]);
        let g = gifs_of(&v);
        assert_eq!(g.len(), 1);
        assert_eq!((g[0].src.as_str(), g[0].width), ("https://p/1.gif", 200), "the proxied copy is preferred");
        assert!(g[0].preview_url().contains("format=png"));
        let with_media: Value = serde_json::json!({"id": "2", "src": "s", "proxy_src": "p", "media": {"gif": {"proxy_src": "big.gif"}, "tinygif": {"proxy_src": "tiny.gif"}}});
        assert_eq!(gif_of(&with_media).unwrap().anim.as_deref(), Some("tiny.gif"), "the smallest animated format is used");
    }

    #[test]
    fn emoji_groups_and_search_find_things() {
        let groups = emoji_groups();
        assert!(emojis_in(&groups[0].2).len() > 100);
        assert!(emoji_search("heart").iter().any(|e| e.as_str() == "❤️"));
        assert!(emoji_search("").is_empty());
    }
}

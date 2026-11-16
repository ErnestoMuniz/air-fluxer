//! Interface preferences kept between launches: which buttons the message box shows.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub gifs_button: bool,
    pub media_button: bool,
    pub stickers_button: bool,
    pub emoji_button: bool,
    pub send_button: bool,
    /// Widths (points) of the channel list and of the member list, set by dragging their edges.
    pub list_w: u32,
    pub members_w: u32,
    /// The status this account shows at launch (online/idle/dnd/invisible).
    pub status: String,
    /// 0 dark, 1 light, 2 Midnight (see `appearance`).
    pub appearance: u8,
}

impl Default for Prefs {
    fn default() -> Self { Prefs { gifs_button: true, media_button: true, stickers_button: true, emoji_button: true, send_button: false, list_w: 248, members_w: 240, status: "online".into(), appearance: 0 } }
}

fn file() -> Option<std::path::PathBuf> { Some(crate::store::dir()?.join("prefs.json")) }

pub fn load() -> Prefs {
    file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(p: &Prefs) {
    let Some(f) = file() else { return };
    let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
    let _ = std::fs::write(f, serde_json::to_vec_pretty(p).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_web_app_and_partial_files_fill_in() {
        let d = Prefs::default();
        assert!(d.gifs_button && d.media_button && d.stickers_button && d.emoji_button && !d.send_button);
        let p: Prefs = serde_json::from_str(r#"{"send_button": true}"#).unwrap();
        assert!(p.send_button && p.gifs_button, "missing keys keep their defaults");
    }
}

//! Files waiting in the message box: pasted pictures (and files copied in a file manager) that go
//! out with the next message, as its attachments.

use crate::State;
use std::sync::Arc;

/// Most files one message carries.
pub const MAX_FILES: usize = 10;
/// The thumbnail's longest side, in pixels (drawn at 48 pt, so 2x).
const THUMB_PX: u32 = 96;

pub struct Pending {
    pub name: String,
    pub content_type: String,
    pub data: Arc<[u8]>,
    pub thumb: Option<rune::Pixels>,
}

/// A file ready to upload.
pub struct Outgoing { pub name: String, pub content_type: String, pub data: Arc<[u8]> }

impl Pending {
    pub fn new(name: String, content_type: String, data: Arc<[u8]>) -> Self {
        let thumb = if content_type.starts_with("image/") { thumbnail(&data) } else { None };
        Pending { name, content_type, data, thumb }
    }
    pub fn outgoing(&self) -> Outgoing { Outgoing { name: self.name.clone(), content_type: self.content_type.clone(), data: self.data.clone() } }
}

fn thumbnail(data: &[u8]) -> Option<rune::Pixels> {
    let img = image::load_from_memory(data).ok()?;
    let img = if img.width().max(img.height()) > THUMB_PX { img.thumbnail(THUMB_PX, THUMB_PX) } else { img };
    let rgba = img.into_rgba8();
    let pixels = rune::Pixels::new();
    pixels.set(rgba.width(), rgba.height(), rgba.into_raw());
    Some(pixels)
}

/// `image.png`, or `image-2.png` when that name is taken: a pasted picture has no name of its own.
fn pasted_name(taken: &[Pending]) -> String {
    let n = taken.iter().filter(|p| p.name.starts_with("image")).count();
    if n == 0 { "image.png".into() } else { format!("image-{}.png", n + 1) }
}

/// Takes what was pasted into the message box. True when it was files or a picture (the text
/// field then pastes nothing); false leaves a plain-text paste to the field.
pub fn paste(s: &mut State, content: &rune::ClipboardContent) -> bool {
    if !content.files().is_empty() {
        let mut took = false;
        for path in content.files() {
            if let Err(e) = add_path(s, path) { s.notice = Some(e) } else { took = true }
        }
        return took || content.text().is_empty();
    }
    if let Some(img) = content.image() {
        let name = pasted_name(&s.attachments);
        return add(s, Pending::new(name, "image/png".into(), Arc::from(img.png())));
    }
    false
}

fn add(s: &mut State, p: Pending) -> bool {
    if s.attachments.len() >= MAX_FILES { s.notice = Some(format!("No máximo {MAX_FILES} arquivos por mensagem")); return true }
    s.notice = None;
    s.attachments.push(p);
    true
}

/// Reads `path` from disk into the pending list.
pub fn add_path(s: &mut State, path: &std::path::Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("não foi possível ler o arquivo: {e}"))?;
    if !meta.is_file() { return Err("só arquivos podem ser anexados".into()) }
    if meta.len() > crate::net::MAX_UPLOAD_BYTES { return Err(format!("o arquivo passa de {} MB", crate::net::MAX_UPLOAD_BYTES / 1_048_576)) }
    let data = std::fs::read(path).map_err(|e| format!("não foi possível ler o arquivo: {e}"))?;
    let name = path.file_name().map_or("file".to_string(), |n| n.to_string_lossy().to_string());
    let ct = crate::net::content_type_of(&name).to_string();
    add(s, Pending::new(name, ct, Arc::from(data)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_rgba8(200, 100).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn a_picture_gets_a_small_thumbnail() {
        let p = Pending::new("a.png".into(), "image/png".into(), Arc::from(png()));
        let (w, h) = p.thumb.expect("decoded").size();
        assert_eq!((w, h), (96, 48));
    }

    #[test]
    fn other_files_have_no_thumbnail() {
        assert!(Pending::new("a.zip".into(), "application/zip".into(), Arc::from(vec![1u8, 2])).thumb.is_none());
    }

    #[test]
    fn pasted_pictures_get_distinct_names() {
        let mut v = vec![];
        assert_eq!(pasted_name(&v), "image.png");
        v.push(Pending::new("image.png".into(), "image/png".into(), Arc::from(png())));
        assert_eq!(pasted_name(&v), "image-2.png");
    }
}

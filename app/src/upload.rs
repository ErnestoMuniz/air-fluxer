//! "Upload file": the system's file chooser, then the file goes to the open channel as a message
//! (with whatever is typed in the message box as its text).

use crate::State;
use rune::{FileDialogOptions, Tx};

pub fn choose_file(_s: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions { title: "Upload file".into(), ..Default::default() };
    tx.open_file(options, |s: &mut State, result, _| match result {
        Ok(Some(path)) => upload(s, path),
        Ok(None) => {}
        Err(e) => s.notice = Some(format!("Não foi possível abrir o seletor de arquivos: {e}")),
    });
}

/// Sends `path` to the open channel with the draft as its text.
pub fn upload(s: &mut State, path: std::path::PathBuf) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    let text = s.draft.text.trim().to_string();
    s.notice = None;
    s.net.send_file(&cid, &text, path);
    s.draft = rune::TextEdit::new("");
}

#[cfg(test)]
mod tests {
    use crate::net::content_type_of;

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(content_type_of("a.PNG"), "image/png");
        assert_eq!(content_type_of("clip.mp4"), "video/mp4");
        assert_eq!(content_type_of("noext"), "application/octet-stream");
        assert_eq!(content_type_of("x.unknownthing"), "application/octet-stream");
    }
}

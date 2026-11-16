//! Desktop notifications for mentions and direct messages while the window is in the background.
//! The web app leaves this to the browser; a native window has to ask the system itself.

use crate::State;
use fluxer_client::api::Message;

/// A message arrived: notify when it is a DM or mentions this account, and the window is not focused.
pub fn incoming(s: &State, m: &Message) {
    let Some(me) = s.chat.me.as_ref() else { return };
    if m.author.id == me.id { return }
    let dm = s.chat.dms.iter().any(|d| d.id == m.channel_id);
    let mention = m.mentions.iter().any(|u| u.id == me.id) || m.content.contains("@everyone");
    if !dm && !mention { return }
    show("Fluxer", &format!("{}: {}", m.author.display(), snippet(&m.content)));
}

fn snippet(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() { return "sent a message".into() }
    if t.chars().count() > 140 { format!("{}…", t.chars().take(140).collect::<String>()) } else { t.to_string() }
}

#[cfg(target_os = "macos")]
fn show(title: &str, body: &str) {
    // osascript quotes AppleScript strings; escaping the two characters that end them is enough.
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"{}\"", quote(body), quote(title));
    let _ = std::process::Command::new("osascript").arg("-e").arg(script).spawn();
}

#[cfg(not(target_os = "macos"))]
fn show(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send").arg(title).arg(body).spawn();
}

#[cfg(test)]
mod tests {
    use super::snippet;

    #[test]
    fn snippets_are_trimmed_and_shortened() {
        assert_eq!(snippet("  oi  "), "oi");
        assert_eq!(snippet(""), "sent a message");
        let long = "a".repeat(200);
        assert_eq!(snippet(&long).chars().count(), 141);
    }
}

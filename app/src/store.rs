//! The saved session: one JSON file in the user's config dir, readable by the owner only.
//! (A system keyring is the next step; until then this is the same exposure as a browser profile.)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub instance: String,
    pub gateway: String,
    pub api_client: String,
    /// Media proxy host (avatars, icons, attachments); old sessions lack it until the next lookup.
    #[serde(default)]
    pub media: Option<String>,
    /// Web origin to send with API writes (see `fluxer_client::rest::Endpoints::origin`).
    #[serde(default)]
    pub origin: Option<String>,
    pub token: String,
    pub user_id: String,
    pub username: Option<String>,
}

pub fn dir() -> Option<PathBuf> {
    // Tests and portable setups point this elsewhere so they never touch a real session.
    if let Some(d) = std::env::var_os("FLUXER_RUX_CONFIG_DIR") { return Some(PathBuf::from(d)) }
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support"))?;
    #[cfg(not(target_os = "macos"))]
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("fluxer-rune"))
}

fn file() -> Option<PathBuf> { Some(dir()?.join("session.json")) }

/// macOS keeps the token in the login keychain; the JSON file keeps the rest. The `security`
/// command is the system's own tool, so no extra framework is linked.
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "fluxer-rune";

#[cfg(target_os = "macos")]
fn keychain_set(user: &str, token: &str) -> bool {
    std::process::Command::new("/usr/bin/security")
        .args(["add-generic-password", "-U", "-s", KEYCHAIN_SERVICE, "-a", user, "-w", token])
        .status().map(|s| s.success()).unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn keychain_get(user: &str) -> Option<String> {
    let out = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", user, "-w"])
        .output().ok()?;
    if !out.status.success() { return None }
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

pub fn load() -> Option<Saved> {
    let mut saved = load_from(&file()?)?;
    #[cfg(target_os = "macos")]
    {
        if saved.token.is_empty() {
            saved.token = keychain_get(&saved.user_id)?;
        } else if keychain_set(&saved.user_id, &saved.token)
            && keychain_get(&saved.user_id).as_deref() == Some(saved.token.as_str())
        {
            // Migrate the plaintext token out of the file only after reading it back.
            let mut clean = saved.clone();
            clean.token.clear();
            if let Some(f) = file() { let _ = save_to(&f, &clean); }
        }
    }
    if saved.token.is_empty() { return None }
    Some(saved)
}

pub fn save(s: &Saved) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let path = file().ok_or(std::io::ErrorKind::NotFound)?;
        if keychain_set(&s.user_id, &s.token)
            && keychain_get(&s.user_id).as_deref() == Some(s.token.as_str())
        {
            let mut clean = s.clone();
            clean.token.clear();
            return save_to(&path, &clean);
        }
    }
    save_to(&file().ok_or(std::io::ErrorKind::NotFound)?, s)
}

/// Where the user left off: (guild id or None for DMs, channel id).
pub fn load_nav() -> Option<(Option<String>, String)> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(nav_file()?).ok()?).ok()?;
    Some((v.get("guild").and_then(|g| g.as_str()).map(String::from), v.get("channel")?.as_str()?.to_string()))
}

pub fn save_nav(pos: &(Option<String>, String)) {
    let Some(f) = nav_file() else { return };
    let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
    let _ = std::fs::write(f, serde_json::json!({ "guild": pos.0, "channel": pos.1 }).to_string());
}

fn nav_file() -> Option<PathBuf> { Some(dir()?.join("nav.json")) }

pub fn clear() {
    if let Some(f) = nav_file() { let _ = std::fs::remove_file(f); } if let Some(f) = file() { let _ = std::fs::remove_file(f); } }

pub fn load_from(path: &std::path::Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn save_to(path: &std::path::Path, s: &Saved) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(path.parent().ok_or(std::io::ErrorKind::InvalidInput)?)?;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(&serde_json::to_vec_pretty(s).expect("session serializes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn session_roundtrips_and_is_private() {
        let dir = std::env::temp_dir().join(format!("fluxer-store-{}", std::process::id()));
        let path = dir.join("session.json");
        let s = Saved { instance: "https://i".into(), gateway: "wss://g".into(), api_client: "https://a".into(), media: None, origin: None,
            token: "secret".into(), user_id: "1".into(), username: Some("e".into()) };
        save_to(&path, &s).unwrap();
        assert_eq!(load_from(&path), Some(s));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

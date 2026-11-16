//! Debug probe: connects with the saved session and prints what the gateway does (never the token).
//! `cargo run -p fluxer-client --example probe -- [zstd]`
use fluxer_client::connection::{Config, Event, run};
use fluxer_client::session::Session;
use fluxer_client::Compression;
use serde_json::{Value, json};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let dir = std::env::var("FLUXER_RUX_CONFIG_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        #[cfg(target_os = "macos")]
        { home.join("Library/Application Support/fluxer-rune") }
        #[cfg(not(target_os = "macos"))]
        { home.join(".config/fluxer-rune") }
    });
    let saved: Value = serde_json::from_slice(&std::fs::read(dir.join("session.json")).expect("no saved session")).unwrap();
    let gateway = saved["gateway"].as_str().unwrap().to_string();
    println!("gateway url: {gateway}");
    let compression = if std::env::args().any(|a| a == "zstd") { Compression::ZstdStream } else { Compression::None };
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let (o, out) = tokio::sync::mpsc::channel(4);
    let cfg = Config { gateway, api_version: 1, compression };
    let props = json!({"os": std::env::consts::OS, "browser": "fluxer-rune", "device": "fluxer-rune"});
    let saved3 = saved.clone();
    tokio::spawn(run(cfg, Session::new(saved["token"].as_str().unwrap(), props), tx, out));
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(15));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            Some(e) = rx.recv() => match e {
                Event::Payload(p) => {
                    println!("payload op={:?} t={:?}", p.op, p.t);
                    if p.t.as_deref() == Some("READY") { if let Some(d) = &p.d { shape(d); history(&saved3, d);
                        // Read-only: subscribe to the member list of the first guild's first text channel.
                        let g = &d["guilds"][0]; let gid = g["id"].as_str().unwrap().to_string();
                        let cid = g["channels"].as_array().unwrap().iter().find(|c| c["type"] == 0).and_then(|c| c["id"].as_str()).unwrap().to_string();
                        let _ = o.send(fluxer_client::Payload { op: fluxer_client::Opcode::LAZY_REQUEST, s: None, t: None,
                            d: Some(json!({"subscriptions": { gid: {"active": true, "sync": true, "typing": true, "member_list_channels": { cid: [[0, 99]] }}}})) }).await;
                    } }
                    if p.t.as_deref().is_some_and(|t| t != "READY") {
                        if let Some(d) = &p.d {
                            println!("  dispatch keys: {}", keys(d));
                            if p.t.as_deref() == Some("GUILD_MEMBER_LIST_UPDATE") {
                                println!("  member_count {} online {} groups {} ops {}", d["member_count"], d["online_count"], d["groups"], d["ops"].as_array().map_or(0, Vec::len));
                                let items = d["ops"][0]["items"].as_array().cloned().unwrap_or_default();
                                println!("  op0 {} range {} items {}", d["ops"][0]["op"], d["ops"][0]["range"], items.len());
                                for it in items.iter().take(4) { println!("  item keys {} | member keys {} | user keys {} | presence {}", keys(it), keys(&it["member"]), keys(&it["member"]["user"]), it["member"]["presence"]["status"]) }
                            }
                        }
                    }
                }
                other => println!("{other:?}"),
            },
            _ = &mut deadline => { println!("done"); return }
        }
    }
}

fn keys(v: &Value) -> String { v.as_object().map_or("-".into(), |o| o.keys().cloned().collect::<Vec<_>>().join(",")) }

/// Prints the structure of READY (key names and counts only, no values).
fn shape(d: &Value) {
    println!("READY keys: {}", keys(d));
    println!("user keys: {}", keys(&d["user"]));
    let guilds = d["guilds"].as_array().map_or(0, Vec::len);
    println!("guilds: {guilds}");
    if let Some(g) = d["guilds"].get(0) {
        println!("guild keys: {}", keys(g));
        println!("guild.properties keys: {}", keys(&g["properties"]));
        println!("channels: {}", g["channels"].as_array().map_or(0, Vec::len));
        if let Some(c) = g["channels"].get(0) { println!("channel keys: {}", keys(c)); }
        for g in d["guilds"].as_array().into_iter().flatten() {
            let ch = g["channels"].as_array().cloned().unwrap_or_default();
            let types: std::collections::BTreeMap<i64, usize> = ch.iter().fold(Default::default(), |mut m, c| { *m.entry(c["type"].as_i64().unwrap_or(-1)).or_default() += 1; m });
            let all: std::collections::BTreeSet<String> = ch.iter().flat_map(|c| c.as_object().into_iter().flat_map(|o| o.keys().cloned())).collect();
            println!("G channel types {types:?}; parent_id set on {}; keys {all:?}", ch.iter().filter(|c| !c["parent_id"].is_null()).count());
            println!("G members {} (online_count {}), presences {}, roles {}", g["members"].as_array().map_or(0, Vec::len), g["online_count"], g["presences"].as_array().map_or(0, Vec::len), g["roles"].as_array().map_or(0, Vec::len));
            println!("G member keys: {} | user keys: {}", keys(&g["members"][0]), keys(&g["members"][0]["user"]));
            println!("G presence keys: {} | statuses: {:?}", keys(&g["presences"][0]), g["presences"].as_array().into_iter().flatten().filter_map(|p| p["status"].as_str()).collect::<std::collections::BTreeSet<_>>());
            println!("G role keys: {} | color sample {}", keys(&g["roles"][0]), g["roles"][0]["color"]);
        }
        println!("top presences {} keys {}", d["presences"].as_array().map_or(0, Vec::len), keys(&d["presences"][0]));
        println!("read_states {} keys {}", d["read_states"].as_array().map_or(0, Vec::len), keys(&d["read_states"][0]));
        println!("user_guild_settings keys {}", keys(&d["user_guild_settings"][0]));
    }
    let dms = d["private_channels"].as_array().map_or(0, Vec::len);
    println!("private_channels: {dms}");
    if let Some(c) = d["private_channels"].get(0) {
        println!("dm keys: {}", keys(c));
        println!("recipient kind: {}", match c["recipients"].get(0) { Some(Value::String(_)) => "id string", Some(Value::Object(_)) => "object", _ => "none" });
    }
    println!("users: {}", d["users"].as_array().map_or(0, Vec::len));
}

/// Read-only check of REST: fetches the newest page of the first text channel (counts only).
fn history(saved: &Value, d: &Value) {
    use fluxer_client::{api::Api, rest::Endpoints};
    let ep = Endpoints {
        api_client: saved["api_client"].as_str().unwrap().into(), gateway: saved["gateway"].as_str().unwrap().into(),
        media: saved["media"].as_str().map(String::from), origin: saved["origin"].as_str().map(String::from),
    };
    let api = Api::new(ep, saved["token"].as_str().unwrap());
    let cid = d["guilds"].as_array().into_iter().flatten().flat_map(|g| g["channels"].as_array().into_iter().flatten())
        .find(|c| c["type"] == 0).and_then(|c| c["id"].as_str().map(String::from));
    let Some(cid) = cid else { println!("no text channel"); return };
    match api.messages(&cid, 50, None) {
        Ok(m) => println!("history ok: {} messages, with attachments: {}, avatars: {}", m.len(),
            m.iter().filter(|x| !x.attachments.is_empty()).count(), m.iter().filter(|x| x.author.avatar.is_some()).count()),
        Err(e) if false => { let _ = e; }
        Err(e) => println!("history FAILED: status={:?} code={:?} msg={}", e.status, e.code, e.message),
    }
    // Shape of the raw messages: which optional features the real data uses (names and counts only).
    if let Ok(v) = api.get(&format!("/channels/{cid}/messages?limit=50")) {
        let arr = v.as_array().cloned().unwrap_or_default();
        let mut all = std::collections::BTreeSet::new();
        for m in &arr { if let Some(o) = m.as_object() { all.extend(o.keys().cloned()) } }
        let parsed: Vec<_> = arr.iter().filter_map(fluxer_client::api::Message::from_json).collect();
        println!("parsed {} of {}: timestamps {}, replies {}, mentions {}, embeds {} (with image {}), kinds {:?}", parsed.len(), arr.len(),
            parsed.iter().filter(|m| m.timestamp.is_some()).count(), parsed.iter().filter(|m| m.reply.is_some()).count(),
            parsed.iter().filter(|m| !m.mentions.is_empty()).count(), parsed.iter().map(|m| m.embeds.len()).sum::<usize>(),
            parsed.iter().flat_map(|m| &m.embeds).filter(|e| e.image.is_some()).count(),
            parsed.iter().map(|m| m.kind).collect::<std::collections::BTreeSet<_>>());
        println!("message keys: {}", all.into_iter().collect::<Vec<_>>().join(","));
        let n = |k: &str| arr.iter().filter(|m| m.get(k).is_some_and(|x| !x.is_null() && x.as_array().is_none_or(|a| !a.is_empty()))).count();
        for k in ["embeds", "reactions", "message_reference", "referenced_message", "mentions", "edited_timestamp", "stickers", "pinned"] {
            println!("  {k}: {} of {}", n(k), arr.len());
        }
        println!("  timestamp sample: {}", arr.first().map_or("-", |m| m["timestamp"].as_str().unwrap_or("-")));
        println!("  types: {:?}", arr.iter().filter_map(|m| m["type"].as_i64()).collect::<std::collections::BTreeSet<_>>());
        println!("  embed keys: {}", arr.iter().find_map(|m| m["embeds"].get(0)).map_or("-".into(), keys));
    }
    // Media: rediscover the instance, then download one avatar (status and size only).
    let Ok(found) = fluxer_client::rest::discover(saved["instance"].as_str().unwrap()) else { println!("discover failed"); return };
    println!("media host published: {}", found.media.is_some());
    if let (Some(media), Ok(msgs)) = (&found.media, api.messages(&cid, 20, None)) {
        if let Some(u) = msgs.iter().find(|m| m.author.avatar.is_some()).map(|m| &m.author) {
            let url = fluxer_client::api::avatar_url(media, u, 128).unwrap();
            match fluxer_client::api::fetch_bytes(&url, 5_000_000) {
                Ok(b) => println!("avatar ok: {} bytes, png magic: {}", b.len(), b.starts_with(&[0x89, b'P', b'N', b'G'])),
                Err(e) => println!("avatar FAILED: {}", e.message),
            }
        }
    }
}

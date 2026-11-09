//! Authenticated REST: message history, sending, and media downloads. Blocking; call it from a
//! worker thread. The token goes in `Authorization` as is (`RestTransport.ts`), and writes carry
//! the web origin like in [`crate::rest`].

use crate::rest::{Endpoints, Error, agent, failure};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
}

impl User {
    pub fn from_json(v: &Value) -> Option<User> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        Some(User { id: text("id")?, username: text("username").unwrap_or_else(|| "?".into()), global_name: text("global_name"), avatar: text("avatar") })
    }
    pub fn display(&self) -> &str { self.global_name.as_deref().unwrap_or(&self.username) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: u64,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.content_type.as_deref().is_some_and(|t| t.starts_with("image/") && !t.contains("svg"))
            && self.url.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Embed {
    pub url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub provider: Option<String>,
    pub author: Option<String>,
    /// Picture to show: the large image, else the thumbnail.
    pub image: Option<String>,
    pub video: bool,
    pub color: Option<u32>,
}

impl Embed {
    fn from_json(v: &Value) -> Embed {
        let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let pic = |k: &str| v.get(k).filter(|x| x.is_object()).and_then(|x| s(x, "proxy_url").or_else(|| s(x, "url")));
        Embed {
            url: s(v, "url"), title: s(v, "title"), description: s(v, "description"),
            provider: v.get("provider").and_then(|p| s(p, "name")), author: v.get("author").and_then(|p| s(p, "name")),
            image: pic("image").or_else(|| pic("thumbnail")),
            video: v.get("video").is_some_and(Value::is_object),
            color: v.get("color").and_then(Value::as_u64).map(|c| c as u32),
        }
    }
    /// Nothing worth a card (no text and no picture).
    pub fn is_empty(&self) -> bool { self.title.is_none() && self.description.is_none() && self.image.is_none() }
}

/// The message a reply points at, as the server embeds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quoted {
    pub author: User,
    pub content: String,
    pub has_attachments: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub content: String,
    pub author: User,
    pub attachments: Vec<Attachment>,
    /// 0 default, 19 reply, 7 member joined, and so on.
    pub kind: u32,
    /// Milliseconds since the Unix epoch.
    pub timestamp: Option<i64>,
    pub edited: bool,
    pub reply: Option<Quoted>,
    pub mentions: Vec<User>,
    pub embeds: Vec<Embed>,
}

/// Parses `2026-11-06T15:37:45.564Z` (UTC, optional fraction, or a `+hh:mm` offset) to Unix milliseconds.
pub fn parse_timestamp(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time_end = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(time_end);
    let mut t = clock.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let sec_s = t.next().unwrap_or("0");
    let (sec_i, frac) = sec_s.split_once('.').unwrap_or((sec_s, "0"));
    let sec: i64 = sec_i.parse().ok()?;
    let ms: i64 = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let (oh, om) = zone[1..].split_once(':').unwrap_or((&zone[1..], "0"));
            let o = oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?;
            if sign == '+' { o } else { -o }
        }
        _ => 0,
    };
    // Days from civil (Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((((days * 24 + h) * 60 + min - offset) * 60 + sec) * 1000 + ms)
}

impl Message {
    /// A plain text message (no reply, mentions or embeds), for local echoes and tests.
    pub fn plain(id: impl Into<String>, channel_id: impl Into<String>, content: impl Into<String>, author: User) -> Message {
        Message { id: id.into(), channel_id: channel_id.into(), content: content.into(), author, attachments: vec![], kind: 0,
            timestamp: None, edited: false, reply: None, mentions: vec![], embeds: vec![] }
    }

    pub fn from_json(v: &Value) -> Option<Message> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        let attachments = v.get("attachments").and_then(Value::as_array).into_iter().flatten().filter_map(|a| {
            let t = |k: &str| a.get(k).and_then(Value::as_str).map(String::from);
            let n = |k: &str| a.get(k).and_then(Value::as_u64);
            Some(Attachment {
                id: t("id")?, filename: t("filename").unwrap_or_default(), content_type: t("content_type"),
                url: t("url").or_else(|| t("proxy_url")), width: n("width").map(|w| w as u32), height: n("height").map(|h| h as u32),
                size: n("size").unwrap_or(0),
            })
        }).collect();
        let reply = v.get("referenced_message").filter(|r| r.is_object()).and_then(|r| {
            Some(Quoted {
                author: User::from_json(r.get("author")?)?,
                content: r.get("content").and_then(Value::as_str).unwrap_or_default().to_string(),
                has_attachments: r.get("attachments").and_then(Value::as_array).is_some_and(|a| !a.is_empty()),
            })
        });
        Some(Message {
            id: text("id")?,
            channel_id: text("channel_id")?,
            content: text("content").unwrap_or_default(),
            author: User::from_json(v.get("author")?)?,
            attachments,
            kind: v.get("type").and_then(Value::as_u64).unwrap_or(0) as u32,
            timestamp: text("timestamp").as_deref().and_then(parse_timestamp),
            edited: v.get("edited_timestamp").is_some_and(|e| !e.is_null()),
            reply,
            mentions: v.get("mentions").and_then(Value::as_array).into_iter().flatten().filter_map(User::from_json).collect(),
            embeds: v.get("embeds").and_then(Value::as_array).into_iter().flatten().map(Embed::from_json).filter(|e| !e.is_empty()).collect(),
        })
    }
}

/// Snowflakes are decimal strings; compare them as numbers.
pub fn snowflake_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    (a.len(), a).cmp(&(b.len(), b))
}

/// `a_` marks an animated avatar; the still frame uses the hash without it.
fn still_hash(hash: &str) -> &str { hash.strip_prefix("a_").unwrap_or(hash) }

pub fn avatar_url(media: &str, user: &User, size: u32) -> Option<String> {
    Some(format!("{media}/avatars/{}/{}.png?size={size}", user.id, still_hash(user.avatar.as_deref()?)))
}

pub fn guild_icon_url(media: &str, guild_id: &str, hash: &str, size: u32) -> String {
    format!("{media}/icons/{guild_id}/{}.png?size={size}", still_hash(hash))
}

/// Attachment URL asking the media proxy for a PNG no wider than `width` (Rune decodes PNG/JPEG).
pub fn attachment_image_url(url: &str, width: u32) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}format=png&width={width}")
}

pub struct Api {
    ep: Endpoints,
    token: String,
}

impl Api {
    pub fn new(ep: Endpoints, token: impl Into<String>) -> Api { Api { ep, token: token.into() } }
    pub fn endpoints(&self) -> &Endpoints { &self.ep }

    pub fn get(&self, path: &str) -> Result<Value, Error> {
        let url = format!("{}{path}", self.ep.api_client);
        let mut res = agent().get(&url).header("Accept", "application/json").header("Authorization", &self.token)
            .call().map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if (200..300).contains(&status) { Ok(body) } else { Err(failure(&crate::rest::Reply { status, body })) }
    }

    /// Up to `limit` messages of a channel, oldest first. `before` pages backwards.
    pub fn messages(&self, channel_id: &str, limit: u32, before: Option<&str>) -> Result<Vec<Message>, Error> {
        let mut path = format!("/channels/{channel_id}/messages?limit={limit}");
        if let Some(b) = before { path.push_str(&format!("&before={b}")) }
        let body = self.get(&path)?;
        let mut msgs: Vec<Message> = body.as_array().into_iter().flatten().filter_map(Message::from_json).collect();
        msgs.sort_by(|a, b| snowflake_cmp(&a.id, &b.id));
        Ok(msgs)
    }

    pub fn send(&self, channel_id: &str, content: &str, nonce: &str) -> Result<Message, Error> {
        let url = format!("{}/channels/{channel_id}/messages", self.ep.api_client);
        let body = json!({ "content": content, "nonce": nonce });
        let mut req = agent().post(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { req = req.header("Origin", o.as_str()) }
        let mut res = req.send_json(&body).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if !(200..300).contains(&status) { return Err(failure(&crate::rest::Reply { status, body })) }
        Message::from_json(&body).ok_or_else(|| Error::local("resposta de envio inesperada"))
    }
}

/// Downloads public media (no token is sent), at most `max` bytes.
pub fn fetch_bytes(url: &str, max: u64) -> Result<Vec<u8>, Error> {
    let mut res = agent().get(url).call().map_err(|e| Error::local(format!("falha de rede: {e}")))?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) { return Err(Error { status: Some(status), code: None, message: format!("mídia respondeu {status}") }) }
    res.body_mut().with_config().limit(max).read_to_vec().map_err(|e| Error::local(format!("mídia inválida: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn mock(replies: Vec<(u16, Value)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || for (status, body) in replies {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 16384];
            let mut n = 0;
            loop {
                n += s.read(&mut buf[n..]).unwrap();
                let t = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(i) = t.find("\r\n\r\n") {
                    let len = t.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                    if n >= i + 4 + len { log.lock().unwrap().push(t); break }
                }
            }
            let b = body.to_string();
            write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).unwrap();
        });
        (url, seen)
    }

    fn api(url: &str) -> Api {
        Api::new(Endpoints { api_client: url.into(), gateway: "wss://g".into(), media: None, origin: Some("https://web.example".into()) }, "tok")
    }

    fn wire(id: &str, content: &str) -> Value {
        json!({"id": id, "channel_id": "c1", "content": content, "author": {"id": "u1", "username": "ana", "global_name": "Ana", "avatar": "a_hash"},
               "attachments": [{"id": "a1", "filename": "f.png", "content_type": "image/png", "url": "https://m/x", "width": 800, "height": 600, "size": 10}]})
    }

    #[test]
    fn history_is_sent_with_the_token_and_comes_back_oldest_first() {
        let (url, seen) = mock(vec![(200, json!([wire("30", "c"), wire("20", "b"), wire("100", "d")]))]);
        let msgs = api(&url).messages("c1", 50, Some("999")).unwrap();
        assert_eq!(msgs.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["20", "30", "100"], "numeric order, not text order");
        let req = seen.lock().unwrap()[0].clone();
        assert!(req.starts_with("GET /channels/c1/messages?limit=50&before=999 "));
        assert!(req.to_lowercase().contains("\r\nauthorization: tok\r\n"));
        assert_eq!((msgs[0].author.display(), msgs[0].attachments[0].is_image()), ("Ana", true));
    }

    #[test]
    fn sending_posts_content_and_nonce_with_the_web_origin() {
        let (url, seen) = mock(vec![(200, wire("55", "olá")), (403, json!({"code": "MISSING_PERMISSIONS", "message": "Sem permissão"}))]);
        let a = api(&url);
        assert_eq!(a.send("c1", "olá", "n1").unwrap().id, "55");
        let req = seen.lock().unwrap()[0].clone();
        assert!(req.starts_with("POST /channels/c1/messages "));
        assert!(req.to_lowercase().contains("\r\norigin: https://web.example\r\n") && req.to_lowercase().contains("\r\nauthorization: tok\r\n"));
        let body: Value = serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!((body["content"].as_str(), body["nonce"].as_str()), (Some("olá"), Some("n1")));
        assert_eq!(a.send("c1", "x", "n2").unwrap_err().message, "Sem permissão");
    }

    #[test]
    fn media_urls_follow_the_web_client() {
        let u = User { id: "7".into(), username: "x".into(), global_name: None, avatar: Some("a_abc".into()) };
        assert_eq!(avatar_url("https://media", &u, 128).as_deref(), Some("https://media/avatars/7/abc.png?size=128"));
        assert_eq!(avatar_url("https://media", &User { avatar: None, ..u }, 128), None);
        assert_eq!(guild_icon_url("https://media", "g", "h", 128), "https://media/icons/g/h.png?size=128");
        assert_eq!(attachment_image_url("https://m/x.webp", 400), "https://m/x.webp?format=png&width=400");
        assert_eq!(attachment_image_url("https://m/x?a=1", 400), "https://m/x?a=1&format=png&width=400");
    }
}

#[cfg(test)]
mod time_tests {
    use super::*;
    #[test]
    fn timestamps_parse_to_unix_milliseconds() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("2026-11-06T15:37:45.564Z"), Some(1_793_979_465_564));
        assert_eq!(parse_timestamp("2026-11-06T12:37:45.5-03:00"), Some(1_793_979_465_500));
        assert_eq!(parse_timestamp("lixo"), None);
    }
}

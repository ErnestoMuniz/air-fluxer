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
    /// The four digits after the name (`name#1234`), when the server sends one.
    pub discriminator: Option<String>,
}

impl User {
    pub fn from_json(v: &Value) -> Option<User> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        Some(User { id: text("id")?, username: text("username").unwrap_or_else(|| "?".into()), global_name: text("global_name"), avatar: text("avatar"), discriminator: text("discriminator") })
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
    /// A GIF: shown animated, with a "GIF" badge.
    pub fn is_gif(&self) -> bool {
        self.url.is_some() && (self.content_type.as_deref() == Some("image/gif") || self.filename.to_ascii_lowercase().ends_with(".gif"))
    }
    /// A video clip: the app plays it in place instead of downloading.
    pub fn is_video(&self) -> bool {
        if self.url.is_none() { return false }
        if self.content_type.as_deref().is_some_and(|t| t.starts_with("video/")) { return true }
        let name = self.filename.to_ascii_lowercase();
        ["mp4", "webm", "mov", "mkv", "avi", "m3u8", "ts"].iter().any(|ext| name.ends_with(&format!(".{ext}")))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Embed {
    pub url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub provider: Option<String>,
    pub author: Option<String>,
    /// The large picture, drawn under the text.
    pub image: Option<String>,
    /// The small square picture drawn to the right of the text (unless it was promoted to `image`).
    pub thumbnail: Option<String>,
    pub video: bool,
    /// A GIF (or a GIF-as-video from a provider such as Tenor): shown animated, without the card.
    pub gif: bool,
    /// Pixel size of the picture, when the server says: lets it get its final shape before it loads.
    pub image_size: Option<(u32, u32)>,
    pub color: Option<u32>,
}

impl Embed {
    fn from_json(v: &Value) -> Embed {
        let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let pic = |k: &str| v.get(k).filter(|x| x.is_object()).and_then(|x| s(x, "proxy_url").or_else(|| s(x, "url")));
        let kind = s(v, "type");
        let video = v.get("video").is_some_and(Value::is_object);
        // An article or image embed (and any video, such as YouTube) shows its thumbnail big,
        // under the text; every other link keeps it as the small square on the right.
        let promoted = video || matches!(kind.as_deref(), Some("article") | Some("image"));
        let thumbnail = pic("thumbnail");
        Embed {
            url: s(v, "url"), title: s(v, "title"), description: s(v, "description"),
            provider: v.get("provider").and_then(|p| s(p, "name")), author: v.get("author").and_then(|p| s(p, "name")),
            image: pic("image").or_else(|| if promoted { thumbnail.clone() } else { None }),
            thumbnail: if promoted { None } else { thumbnail },
            video,
            image_size: ["image", "thumbnail", "video"].iter().filter_map(|k| v.get(*k)).find_map(|m|
                Some((m.get("width")?.as_u64()? as u32, m.get("height")?.as_u64()? as u32)).filter(|(w, h)| *w > 0 && *h > 0)),
            gif: s(v, "type").as_deref() == Some("gifv")
                || ["image", "thumbnail"].iter().filter_map(|k| v.get(*k)).any(|m| s(m, "content_type").as_deref() == Some("image/gif")
                    || s(m, "url").is_some_and(|u| u.split('?').next().unwrap_or("").to_ascii_lowercase().ends_with(".gif"))),
            color: v.get("color").and_then(Value::as_u64).map(|c| c as u32),
        }
    }
    /// Nothing worth a card (no text and no picture).
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none() && self.image.is_none() && self.thumbnail.is_none()
    }
}

/// The message a reply points at, as the server embeds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quoted {
    /// The original message's id, so a quote click can jump to it (absent in older payloads).
    pub id: Option<String>,
    pub author: User,
    pub content: String,
    pub has_attachments: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Reactions on it, with how many people used each and whether this account did.
    pub reactions: Vec<Reaction>,
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
    /// Role ids of the author in the server, when the message carries its `member`.
    pub member_roles: Option<Vec<String>>,
    /// The author's nickname in the server, when the message carries its `member`.
    pub member_nick: Option<String>,
}

/// Parses `2026-11-06T15:37:45.564Z` (UTC, optional fraction, or a `+hh:mm` offset) to Unix milliseconds.
/// One emoji reacted to a message with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    /// What the API calls the emoji: the character itself, or `name:id` for a custom one.
    pub key: String,
    pub name: String,
    pub id: Option<String>,
    pub count: u32,
    pub me: bool,
}

/// The emoji of a reaction event or object: `{ "id": ?, "name": ? }`.
pub fn emoji_key(v: &Value) -> Option<(String, String, Option<String>)> {
    let name = v.get("name").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_string();
    let id = v.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    let key = match &id { Some(i) => format!("{name}:{i}"), None => name.clone() };
    Some((key, name, id))
}

impl Reaction {
    pub fn from_json(v: &Value) -> Option<Reaction> {
        let (key, name, id) = emoji_key(v.get("emoji")?)?;
        Some(Reaction { key, name, id, count: v.get("count").and_then(Value::as_u64).unwrap_or(1) as u32, me: v.get("me").and_then(Value::as_bool).unwrap_or(false) })
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

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
        Message { reactions: vec![], id: id.into(), channel_id: channel_id.into(), content: content.into(), author, attachments: vec![], kind: 0,
            timestamp: None, edited: false, reply: None, mentions: vec![], embeds: vec![], member_roles: None, member_nick: None }
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
                id: r.get("id").and_then(Value::as_str).map(String::from),
                author: User::from_json(r.get("author")?)?,
                content: r.get("content").and_then(Value::as_str).unwrap_or_default().to_string(),
                has_attachments: r.get("attachments").and_then(Value::as_array).is_some_and(|a| !a.is_empty()),
            })
        });
        Some(Message {
            reactions: v.get("reactions").and_then(Value::as_array).into_iter().flatten().filter_map(Reaction::from_json).collect(),
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
            member_nick: v.pointer("/member/nick").and_then(Value::as_str).filter(|n| !n.is_empty()).map(String::from),
            member_roles: v.pointer("/member/roles").and_then(Value::as_array).map(|r| r.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
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

/// A role's custom icon (the web client's `role-icons` webp bucket).
pub fn role_icon_url(media: &str, role_id: &str, hash: &str, size: u32) -> String {
    format!("{media}/role-icons/{role_id}/{}.webp?size={size}", still_hash(hash))
}

/// Attachment URL asking the media proxy for a PNG no wider than `width` (Rune decodes PNG/JPEG).
pub fn attachment_image_url(url: &str, width: u32) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}format=png&width={width}")
}

/// The same picture, animated: the media proxy keeps every frame when asked with `animated=true`.
pub fn animated_image_url(url: &str, width: u32) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}width={width}&animated=true")
}

/// A server's banner, moving when its hash starts with `a_`. The proxy only animates a request that
/// names the whole hash, `a_` included (the still frame is asked for without it), as the web client does.
pub fn banner_url(media: &str, guild_id: &str, hash: &str, size: u32) -> String {
    if hash.starts_with("a_") { format!("{media}/banners/{guild_id}/{hash}.webp?size={size}&animated=true") }
    else { format!("{media}/banners/{guild_id}/{hash}.webp?size={size}") }
}

/// The still first frame of a server banner.
pub fn banner_still_url(media: &str, guild_id: &str, hash: &str, size: u32) -> String {
    format!("{media}/banners/{guild_id}/{}.png?size={size}", still_hash(hash))
}

/// What the profile endpoint says about someone, the server's own profile laid over the account's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    /// 24-bit RGB the person picked (the server's own pick wins), else the colour taken from the banner.
    pub color: Option<u32>,
    pub banner: Option<String>,
    /// When they joined the open server (Unix ms), if they are in it.
    pub joined_ms: Option<i64>,
    /// The account-wide and the in-server profile apart, as the settings page edits them.
    pub user_part: ProfilePart,
    pub guild_part: ProfilePart,
    /// Their nickname in the server.
    pub nick: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfilePart { pub bio: Option<String>, pub pronouns: Option<String>, pub accent: Option<u32> }

impl ProfilePart {
    fn from_json(o: &Value) -> ProfilePart {
        ProfilePart {
            bio: o.get("bio").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
            pronouns: o.get("pronouns").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
            accent: o.get("accent_color").and_then(Value::as_u64).map(|n| n as u32),
        }
    }
}

impl Profile {
    pub fn from_json(v: &Value) -> Profile {
        let text = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let num = |o: &Value, k: &str| o.get(k).and_then(Value::as_u64).map(|n| n as u32);
        let (up, gp) = (v.get("user_profile").unwrap_or(&Value::Null), v.get("guild_member_profile").unwrap_or(&Value::Null));
        Profile {
            bio: text(gp, "bio").or_else(|| text(up, "bio")),
            pronouns: text(gp, "pronouns").or_else(|| text(up, "pronouns")),
            color: num(gp, "accent_color").or_else(|| num(up, "accent_color")).or_else(|| num(up, "banner_color")),
            banner: text(up, "banner"),
            user_part: ProfilePart::from_json(up),
            guild_part: ProfilePart::from_json(gp),
            nick: v.pointer("/guild_member/nick").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
            joined_ms: v.get("guild_member").and_then(|m| m.get("joined_at")).and_then(Value::as_str).and_then(parse_timestamp),
        }
    }
}

pub struct Api {
    ep: Endpoints,
    token: String,
}

/// What a message search returned.
#[derive(Clone, Debug)]
pub enum SearchOutcome {
    /// The server is still building its index for this channel.
    Indexing,
    Hits(Vec<Message>),
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

    /// Adds (or takes back) this account's reaction `emoji` (an `emoji_key`) on a message.
    pub fn react(&self, channel_id: &str, message_id: &str, emoji: &str, add: bool) -> Result<(), Error> {
        let url = format!("{}/channels/{channel_id}/messages/{message_id}/reactions/{}/@me", self.ep.api_client, percent_encode(emoji));
        let ag = agent();
        let origin = self.ep.origin.clone();
        let mut res = if add {
            let mut r = ag.put(&url).header("Accept", "application/json").header("Authorization", &self.token);
            if let Some(o) = &origin { r = r.header("Origin", o.as_str()) }
            r.send_empty()
        } else {
            let mut r = ag.delete(&url).header("Accept", "application/json").header("Authorization", &self.token);
            if let Some(o) = &origin { r = r.header("Origin", o.as_str()) }
            r.call()
        }.map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if (200..300).contains(&status) { Ok(()) } else { Err(failure(&crate::rest::Reply { status, body })) }
    }

    /// Changes this account (`guild` None) or its membership in `guild`; returns the updated object.
    pub fn patch_profile(&self, guild: Option<&str>, body: &Value) -> Result<Value, Error> {
        let path = match guild { Some(g) => format!("/guilds/{g}/members/@me"), None => "/users/@me".to_string() };
        let url = format!("{}{path}", self.ep.api_client);
        let mut r = agent().patch(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { r = r.header("Origin", o.as_str()) }
        let mut res = r.send_json(body).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if (200..300).contains(&status) { Ok(body) } else { Err(failure(&crate::rest::Reply { status, body })) }
    }

    /// The profile of `user_id`, with their membership in `guild_id` when given.
    pub fn profile(&self, user_id: &str, guild_id: Option<&str>) -> Result<Profile, Error> {
        let mut path = format!("/users/{user_id}/profile");
        if let Some(g) = guild_id { path.push_str(&format!("?guild_id={g}")) }
        Ok(Profile::from_json(&self.get(&path)?))
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
        self.post_message(channel_id, json!({ "content": content, "nonce": nonce }))
    }

    /// A message made only of stickers.
    pub fn send_sticker(&self, channel_id: &str, sticker_id: &str, nonce: &str) -> Result<Message, Error> {
        self.post_message(channel_id, json!({ "sticker_ids": [sticker_id], "nonce": nonce }))
    }

    /// A message that answers `reply_to` (the composer's reply bar).
    pub fn send_reply(&self, channel_id: &str, content: &str, nonce: &str, reply_to: &str) -> Result<Message, Error> {
        self.post_message(channel_id, json!({
            "content": content, "nonce": nonce,
            "message_reference": { "message_id": reply_to, "channel_id": channel_id, "type": 0 },
            "allowed_mentions": { "replied_user": true },
        }))
    }

    /// Rewrites one of this account's messages.
    pub fn edit_message(&self, channel_id: &str, message_id: &str, content: &str) -> Result<Message, Error> {
        let url = format!("{}/channels/{channel_id}/messages/{message_id}", self.ep.api_client);
        let mut r = agent().patch(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { r = r.header("Origin", o.as_str()) }
        let mut res = r.send_json(json!({ "content": content })).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if !(200..300).contains(&status) { return Err(failure(&crate::rest::Reply { status, body })) }
        Message::from_json(&body).ok_or_else(|| Error::local("resposta de edição inesperada"))
    }

    /// Deletes one of this account's messages.
    pub fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<(), Error> {
        let url = format!("{}/channels/{channel_id}/messages/{message_id}", self.ep.api_client);
        let mut r = agent().delete(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { r = r.header("Origin", o.as_str()) }
        let mut res = r.call().map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if (200..300).contains(&status) { Ok(()) } else { Err(failure(&crate::rest::Reply { status, body })) }
    }

    /// Marks a channel read up to `message_id`.
    pub fn ack(&self, channel_id: &str, message_id: &str) -> Result<(), Error> {
        let url = format!("{}/read-states/ack", self.ep.api_client);
        let mut r = agent().post(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { r = r.header("Origin", o.as_str()) }
        let body = json!({ "read_states": [{ "channel_id": channel_id, "message_id": message_id }] });
        let mut res = r.send_json(&body).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if (200..300).contains(&status) { Ok(()) } else { Err(failure(&crate::rest::Reply { status, body })) }
    }

    /// Messages matching `query`: the whole server when `guild` is given, else the `channel`.
    pub fn search(&self, guild: Option<&str>, channel: Option<&str>, query: &str) -> Result<SearchOutcome, Error> {
        let url = format!("{}/search/messages", self.ep.api_client);
        let mut r = agent().post(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { r = r.header("Origin", o.as_str()) }
        let mut body = json!({ "content": query, "hits_per_page": 25, "page": 1 });
        if let Some(g) = guild { body["context_guild_id"] = g.into() }
        if let Some(c) = channel { body["channel_id"] = json!([c]); body["context_channel_id"] = c.into() }
        let mut res = r.send_json(&body).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if !(200..300).contains(&status) { return Err(failure(&crate::rest::Reply { status, body })) }
        if body.get("indexing").and_then(Value::as_bool) == Some(true) { return Ok(SearchOutcome::Indexing) }
        let msgs = body.get("messages").and_then(Value::as_array).into_iter().flatten()
            .filter_map(Message::from_json).collect();
        Ok(SearchOutcome::Hits(msgs))
    }

    /// Starts ringing the people in a DM call.
    pub fn ring_call(&self, channel_id: &str, recipients: &[String]) -> Result<(), Error> {
        self.call_action(channel_id, "ring", json!({ "recipients": recipients }))
    }

    /// Stops ringing (all, or just `recipients`).
    pub fn stop_ringing(&self, channel_id: &str, recipients: Option<&[String]>) -> Result<(), Error> {
        let body = match recipients { Some(r) => json!({ "recipients": r }), None => Value::Object(Default::default()) };
        self.call_action(channel_id, "stop-ringing", body)
    }

    fn call_action(&self, channel_id: &str, action: &str, body: Value) -> Result<(), Error> {
        let url = format!("{}/channels/{channel_id}/call/{action}", self.ep.api_client);
        let mut r = agent().post(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { r = r.header("Origin", o.as_str()) }
        let mut res = r.send_json(&body).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if (200..300).contains(&status) { Ok(()) } else { Err(failure(&crate::rest::Reply { status, body })) }
    }

    /// A page of messages centered on `message_id` (jumping to a search hit).
    pub fn messages_around(&self, channel_id: &str, message_id: &str, limit: u32) -> Result<Vec<Message>, Error> {
        let body = self.get(&format!("/channels/{channel_id}/messages?around={message_id}&limit={limit}"))?;
        let mut msgs: Vec<Message> = body.as_array().into_iter().flatten().filter_map(Message::from_json).collect();
        msgs.sort_by(|a, b| snowflake_cmp(&a.id, &b.id));
        Ok(msgs)
    }

    /// A message with one file attached: `multipart/form-data` with the message as `payload_json`
    /// and the file as `files[0]`, as the web client does when it does not upload ahead of time.
    pub fn send_file(&self, channel_id: &str, content: &str, filename: &str, content_type: &str, data: &[u8], nonce: &str) -> Result<Message, Error> {
        let (body, boundary) = multipart_body(&json!({ "content": content, "nonce": nonce, "attachments": [{ "id": 0, "filename": filename }] }), filename, content_type, data);
        let url = format!("{}/channels/{channel_id}/messages", self.ep.api_client);
        let mut req = agent().post(&url).header("Accept", "application/json").header("Authorization", &self.token)
            .header("Content-Type", format!("multipart/form-data; boundary={boundary}"));
        if let Some(o) = &self.ep.origin { req = req.header("Origin", o.as_str()) }
        let mut res = req.send(&body[..]).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if !(200..300).contains(&status) { return Err(failure(&crate::rest::Reply { status, body })) }
        Message::from_json(&body).ok_or_else(|| Error::local("resposta de envio inesperada"))
    }

    fn post_message(&self, channel_id: &str, body: Value) -> Result<Message, Error> {
        let url = format!("{}/channels/{channel_id}/messages", self.ep.api_client);
        let mut req = agent().post(&url).header("Accept", "application/json").header("Authorization", &self.token);
        if let Some(o) = &self.ep.origin { req = req.header("Origin", o.as_str()) }
        let mut res = req.send_json(&body).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_json().unwrap_or(Value::Null);
        if !(200..300).contains(&status) { return Err(failure(&crate::rest::Reply { status, body })) }
        Message::from_json(&body).ok_or_else(|| Error::local("resposta de envio inesperada"))
    }
}

/// The body of a one-file message upload and its boundary.
fn multipart_body(payload: &Value, filename: &str, content_type: &str, data: &[u8]) -> (Vec<u8>, String) {
    let boundary = format!("----FluxerRux{:x}", data.len() ^ filename.len().rotate_left(7) ^ 0x5bd1e995);
    let esc = |n: &str| n.replace('\\', "_").replace('"', "_").replace(['\r', '\n'], "_");
    let mut out = Vec::with_capacity(data.len() + 512);
    out.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{payload}\r\n").as_bytes());
    out.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n", esc(filename)).as_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (out, boundary)
}

/// Downloads public media (no token is sent), at most `max` bytes.
pub fn fetch_bytes(url: &str, max: u64) -> Result<Vec<u8>, Error> {
    let mut res = agent().get(url).call().map_err(|e| Error::local(format!("falha de rede: {e}")))?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) { return Err(Error { status: Some(status), code: None, message: format!("mídia respondeu {status}") }) }
    res.body_mut().with_config().limit(max).read_to_vec().map_err(|e| Error::local(format!("mídia inválida: {e}")))
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    #[test]
    fn a_file_message_is_multipart_with_payload_json_and_files_0() {
        let (body, boundary) = multipart_body(&json!({"content": "hi", "attachments": [{"id": 0, "filename": "a\"b.png"}]}), "a\"b.png", "image/png", &[1, 2, 3]);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with(&format!("--{boundary}\r\n")) && text.ends_with(&format!("--{boundary}--\r\n")));
        assert!(text.contains("name=\"payload_json\""));
        assert!(text.contains("name=\"files[0]\"; filename=\"a_b.png\""), "a quote in a name cannot break the header");
        assert!(body.windows(5).any(|w| w == [b'\r', b'\n', 1, 2, 3]), "the file bytes go in as they are");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_animated_banner_is_asked_for_with_its_whole_hash_and_a_still_one_is_not() {
        assert_eq!(super::banner_url("https://m", "g", "a_abc", 480), "https://m/banners/g/a_abc.webp?size=480&animated=true");
        assert_eq!(super::banner_url("https://m", "g", "abc", 480), "https://m/banners/g/abc.webp?size=480");
        assert_eq!(super::banner_still_url("https://m", "g", "a_abc", 640), "https://m/banners/g/abc.png?size=640", "the still frame goes without the a_");
    }

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
        let u = User { id: "7".into(), username: "x".into(), global_name: None, avatar: Some("a_abc".into()), discriminator: None };
        assert_eq!(avatar_url("https://media", &u, 128).as_deref(), Some("https://media/avatars/7/abc.png?size=128"));
        assert_eq!(avatar_url("https://media", &User { avatar: None, ..u }, 128), None);
        assert_eq!(guild_icon_url("https://media", "g", "h", 128), "https://media/icons/g/h.png?size=128");
        assert_eq!(attachment_image_url("https://m/x.webp", 400), "https://m/x.webp?format=png&width=400");
        assert_eq!(attachment_image_url("https://m/x?a=1", 400), "https://m/x?a=1&format=png&width=400");
    }

    #[test]
    fn a_link_thumbnail_stays_small_and_an_article_or_video_one_is_the_picture() {
        let link = Embed::from_json(&json!({"type": "link", "title": "t", "thumbnail": {"url": "https://m/t.png"}}));
        assert_eq!((link.image.as_deref(), link.thumbnail.as_deref()), (None, Some("https://m/t.png")));
        let article = Embed::from_json(&json!({"type": "article", "title": "t", "thumbnail": {"url": "https://m/t.png"}}));
        assert_eq!((article.image.as_deref(), article.thumbnail.as_deref()), (Some("https://m/t.png"), None));
        let video = Embed::from_json(&json!({"type": "video", "video": {"url": "https://m/v"}, "thumbnail": {"url": "https://m/t.png"}}));
        assert_eq!((video.image.as_deref(), video.thumbnail.as_deref(), video.video), (Some("https://m/t.png"), None, true));
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

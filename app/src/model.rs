//! Chat data: guilds, DMs, channels and per-channel message logs, filled from the gateway's READY
//! and dispatches and from REST history. No I/O here, so all of it is unit-testable.

use fluxer_client::api::{Attachment, Embed, Message, Quoted, User, snowflake_cmp};
use crate::roster::Roster;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery { Sent, Pending, Failed(String) }

#[derive(Clone, Debug)]
pub struct Msg {
    pub id: String,
    pub author: User,
    pub body: String,
    pub attachments: Vec<Attachment>,
    pub delivery: Delivery,
    /// Message type (7 = member joined, 19 = reply, ...).
    pub kind: u32,
    /// Milliseconds since the Unix epoch.
    pub ts: Option<i64>,
    pub edited: bool,
    pub reply: Option<Quoted>,
    pub mentions: Vec<User>,
    pub embeds: Vec<Embed>,
}

impl Msg {
    pub fn local(id: String, author: User, body: String, delivery: Delivery) -> Msg {
        Msg { id, author, body, attachments: vec![], delivery, kind: 0, ts: None, edited: false, reply: None, mentions: vec![], embeds: vec![] }
    }
}

impl From<Message> for Msg {
    fn from(m: Message) -> Msg {
        Msg { id: m.id, author: m.author, body: m.content, attachments: m.attachments, delivery: Delivery::Sent,
            kind: m.kind, ts: m.timestamp, edited: m.edited, reply: m.reply, mentions: m.mentions, embeds: m.embeds }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// Mentions not yet seen (the red badge).
    pub unread: u32,
    /// For a 1:1 DM, the other person (their avatar stands for the channel).
    pub peer: Option<User>,
    /// Name of the category the channel sits in (servers only).
    pub category: Option<String>,
    /// Something new arrived since the last read; the name shows in bold until the channel is opened.
    pub new: bool,
    /// Newest message the account has read, from the server's read state. We never acknowledge, so
    /// the "new messages" line stays where the web app left it.
    pub last_read: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Guild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub channels: Vec<Channel>,
}

#[derive(Default)]
pub struct Log {
    pub msgs: Vec<Msg>,
    pub loaded: bool,
    pub loading: bool,
    /// History may have older pages (the last page came back full).
    pub has_more: bool,
}

pub const PAGE: usize = 50;
/// Width asked from the media proxy for attachment images (drawn at most 360 pt wide, so 2x).
pub const EMBED_FETCH_WIDTH: u32 = 480;
pub const ATTACHMENT_FETCH_WIDTH: u32 = 720;

/// What is open: a guild channel, or a DM when `guild` is None.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Nav { pub guild: Option<usize>, pub channel: usize }

#[derive(Default)]
pub struct Chat {
    pub me: Option<User>,
    pub guilds: Vec<Guild>,
    pub dms: Vec<Channel>,
    pub nav: Nav,
    pub logs: HashMap<String, Log>,
    pub media: Option<String>,
    pub next_local: u64,
    pub roster: Roster,
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> { v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()) }

impl Chat {
    pub fn channels(&self) -> &[Channel] {
        match self.nav.guild {
            Some(g) => self.guilds.get(g).map_or(&[], |g| &g.channels),
            None => &self.dms,
        }
    }
    pub fn channel(&self) -> Option<&Channel> { self.channels().get(self.nav.channel) }
    pub fn channel_id(&self) -> Option<&str> { self.channel().map(|c| c.id.as_str()) }
    pub fn log(&self) -> Option<&Log> { self.logs.get(self.channel_id()?) }

    pub fn open_guild(&mut self, guild: Option<usize>) {
        self.nav = Nav { guild, channel: 0 };
        self.mark_read();
    }

    pub fn open_channel(&mut self, i: usize) {
        self.nav.channel = i;
        self.mark_read();
    }

    fn mark_read(&mut self) {
        let (g, c) = (self.nav.guild, self.nav.channel);
        let list = match g { Some(g) => self.guilds.get_mut(g).map(|g| &mut g.channels), None => Some(&mut self.dms) };
        if let Some(ch) = list.and_then(|l| l.get_mut(c)) { ch.unread = 0; ch.new = false }
    }

    /// Unread messages across a guild's channels (None → DMs).
    pub fn unread(&self, guild: Option<usize>) -> u32 {
        match guild {
            Some(g) => self.guilds.get(g).map_or(0, |g| g.channels.iter().map(|c| c.unread).sum()),
            None => self.dms.iter().map(|c| c.unread).sum(),
        }
    }

    /// True when some channel of the guild (None → DMs) has unseen messages.
    pub fn has_new(&self, guild: Option<usize>) -> bool {
        match guild {
            Some(g) => self.guilds.get(g).is_some_and(|g| g.channels.iter().any(|c| c.new)),
            None => self.dms.iter().any(|c| c.new),
        }
    }

    /// Guilds, text channels and DMs from READY.
    pub fn apply_ready(&mut self, d: &Value) {
        let users: HashMap<&str, &Value> = d.get("users").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|u| Some((text(u, "id")?, u))).collect();
        self.me = d.get("user").and_then(User::from_json);
        self.roster = Roster::default();
        self.roster.load_ready(d);
        // Read state per channel: (last read message, mention count).
        let reads: HashMap<String, (Option<String>, u32)> = d.get("read_states").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|r| Some((text(r, "id")?.to_string(), (text(r, "last_message_id").map(String::from), r.get("mention_count").and_then(Value::as_u64).unwrap_or(0) as u32)))).collect();

        self.guilds = d.get("guilds").and_then(Value::as_array).into_iter().flatten().filter_map(|g| {
            let props = g.get("properties").unwrap_or(g);
            let all = g.get("channels").and_then(Value::as_array).cloned().unwrap_or_default();
            // Categories: id -> (name, position). Channels sort by their category, then by position.
            let cats: HashMap<&str, (&str, i64)> = all.iter().filter(|c| c.get("type").and_then(Value::as_u64) == Some(4))
                .filter_map(|c| Some((text(c, "id")?, (text(c, "name").unwrap_or(""), c.get("position").and_then(Value::as_i64).unwrap_or(0))))).collect();
            let mut chans: Vec<((i64, String, i64), Channel)> = all.iter()
                .filter(|c| matches!(c.get("type").and_then(Value::as_u64), Some(0 | 5)))
                .filter_map(|c| {
                    let id = text(c, "id")?;
                    let cat = text(c, "parent_id").and_then(|p| cats.get(p).map(|(n, pos)| (p, *n, *pos)));
                    let (last_read, mentions) = reads.get(id).cloned().unwrap_or((None, 0));
                    let new = matches!((text(c, "last_message_id"), &last_read), (Some(l), Some(r)) if snowflake_cmp(l, r).is_gt());
                    Some(((cat.map_or(-1, |c| c.2), cat.map_or(String::new(), |c| c.0.to_string()), c.get("position").and_then(Value::as_i64).unwrap_or(0)),
                        Channel { id: id.into(), name: text(c, "name")?.into(), unread: mentions, peer: None,
                            category: cat.map(|c| c.1.to_string()).filter(|n| !n.is_empty()), new, last_read }))
                })
                .collect();
            chans.sort_by(|a, b| a.0.cmp(&b.0));
            Some(Guild {
                id: text(g, "id")?.into(),
                name: text(props, "name").unwrap_or("Servidor").into(),
                icon: text(props, "icon").map(String::from),
                channels: chans.into_iter().map(|(_, c)| c).collect(),
            })
        }).collect();

        let user_of = |r: &Value| -> Option<User> {
            match r.as_str() {
                Some(id) => users.get(id).and_then(|u| User::from_json(u)),
                None => User::from_json(r),
            }
        };
        let my_id = self.me.as_ref().map(|m| m.id.clone());
        let mut dms: Vec<(String, Channel)> = d.get("private_channels").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|c| {
                let kind = c.get("type").and_then(Value::as_u64)?;
                if kind != 1 && kind != 3 { return None }
                let peers: Vec<User> = c.get("recipients").and_then(Value::as_array).into_iter().flatten().filter_map(user_of)
                    .filter(|u| Some(&u.id) != my_id.as_ref()).collect();
                let name = text(c, "name").map(String::from)
                    .unwrap_or_else(|| peers.iter().map(|u| u.display()).collect::<Vec<_>>().join(", "));
                let id = text(c, "id")?;
                let (last_read, mentions) = reads.get(id).cloned().unwrap_or((None, 0));
                let new = matches!((text(c, "last_message_id"), &last_read), (Some(l), Some(r)) if snowflake_cmp(l, r).is_gt());
                Some((text(c, "last_message_id").unwrap_or("0").to_string(), Channel {
                    id: id.into(),
                    name: if name.is_empty() { "Conversa".into() } else { name },
                    unread: mentions, new, last_read,
                    peer: if kind == 1 { peers.into_iter().next() } else { None },
                    ..Channel::default()
                }))
            }).collect();
        // Most recent conversation first.
        dms.sort_by(|a, b| snowflake_cmp(&b.0, &a.0));
        self.dms = dms.into_iter().map(|(_, c)| c).collect();

        self.nav = Nav { guild: if self.guilds.is_empty() { None } else { Some(0) }, channel: 0 };
        self.logs.clear();
    }

    pub fn find_channel_mut(&mut self, id: &str) -> Option<&mut Channel> {
        self.guilds.iter_mut().flat_map(|g| g.channels.iter_mut()).chain(self.dms.iter_mut()).find(|c| c.id == id)
    }

    /// Inserts `m` in its log: real messages in snowflake order, pending or failed local ones at the
    /// end. A message already there (the gateway echo of something we sent, or a page overlapping
    /// live traffic) is replaced instead of repeated.
    fn insert(log: &mut Log, m: Msg) {
        if let Some(i) = log.msgs.iter().position(|x| x.id == m.id) { log.msgs[i] = m; return }
        if m.delivery != Delivery::Sent { log.msgs.push(m); return }
        let sent_end = log.msgs.iter().rposition(|x| x.delivery == Delivery::Sent).map_or(0, |p| p + 1);
        let at = log.msgs[..sent_end].partition_point(|x| snowflake_cmp(&x.id, &m.id).is_lt());
        log.msgs.insert(at, m);
    }

    /// A message from the gateway.
    pub fn on_message(&mut self, m: Message) {
        let cid = m.channel_id.clone();
        let mine = self.me.as_ref().is_some_and(|u| u.id == m.author.id);
        let open = self.channel_id() == Some(cid.as_str());
        self.roster.remember(&m.author);
        self.roster.stop_typing(&cid, &m.author.id);
        let log = self.logs.entry(cid.clone()).or_default();
        // The pending local copy of this very message gives way to the real one.
        if mine && let Some(i) = log.msgs.iter().position(|x| x.delivery == Delivery::Pending && x.body == m.content) {
            log.msgs.remove(i);
        }
        Self::insert(log, m.into());
        if !open && !mine && let Some(c) = self.find_channel_mut(&cid) { c.unread += 1; c.new = true }
    }

    /// The account read up to `message_id` somewhere else (`MESSAGE_ACK`).
    pub fn on_ack(&mut self, channel_id: &str, message_id: &str) -> bool {
        let Some(c) = self.find_channel_mut(channel_id) else { return false };
        c.last_read = Some(message_id.into());
        c.unread = 0;
        c.new = false;
        true
    }

    pub fn on_message_deleted(&mut self, channel_id: &str, id: &str) -> bool {
        let Some(log) = self.logs.get_mut(channel_id) else { return false };
        let before = log.msgs.len();
        log.msgs.retain(|m| m.id != id);
        log.msgs.len() != before
    }

    pub fn on_message_updated(&mut self, m: Message) -> bool {
        let Some(log) = self.logs.get_mut(&m.channel_id) else { return false };
        let Some(i) = log.msgs.iter().position(|x| x.id == m.id) else { return false };
        log.msgs[i] = m.into();
        true
    }

    /// A page of history, merged with whatever live traffic already arrived.
    pub fn on_history(&mut self, channel_id: &str, msgs: Vec<Message>) {
        let log = self.logs.entry(channel_id.to_string()).or_default();
        log.loading = false;
        log.loaded = true;
        log.has_more = msgs.len() >= PAGE;
        for m in &msgs { self.roster.remember(&m.author) }
        let log = self.logs.entry(channel_id.to_string()).or_default();
        for m in msgs { Self::insert(log, m.into()) }
    }

    pub fn on_history_failed(&mut self, channel_id: &str) {
        self.logs.entry(channel_id.to_string()).or_default().loading = false;
    }

    /// Adds our own message at once, marked pending until the server confirms it.
    pub fn push_pending(&mut self, channel_id: &str, body: String) -> Option<String> {
        let me = self.me.clone()?;
        self.next_local += 1;
        let id = format!("local-{}", self.next_local);
        let log = self.logs.entry(channel_id.to_string()).or_default();
        log.msgs.push({ let mut m = Msg::local(id.clone(), me, body, Delivery::Pending); m.ts = Some(crate::clock::now_ms()); m });
        Some(id)
    }

    pub fn on_sent(&mut self, channel_id: &str, local: &str, res: Result<Message, String>) {
        let log = self.logs.entry(channel_id.to_string()).or_default();
        let Some(i) = log.msgs.iter().position(|m| m.id == local) else { return };
        match res {
            Ok(m) => {
                log.msgs.remove(i);
                Self::insert(log, m.into());
            }
            Err(e) => log.msgs[i].delivery = Delivery::Failed(e),
        }
    }

    /// Every image the screen can show right now: guild icons, DM avatars, and avatars plus image
    /// attachments of the last `recent` messages of the open channel.
    pub fn wanted_images(&self, recent: usize) -> Vec<String> {
        let Some(media) = self.media.as_deref() else { return vec![] };
        let mut urls: Vec<String> = self.guilds.iter()
            .filter_map(|g| g.icon.as_deref().map(|h| fluxer_client::api::guild_icon_url(media, &g.id, h, 128))).collect();
        urls.extend(self.dms.iter().filter_map(|c| fluxer_client::api::avatar_url(media, c.peer.as_ref()?, 128)));
        if let Some(rows) = self.nav.guild.and_then(|g| self.guilds.get(g)).and_then(|g| self.roster.lists.get(&g.id)) {
            urls.extend(rows.iter().filter_map(|r| match r { crate::roster::Row::Member { user, .. } => fluxer_client::api::avatar_url(media, user, 128), _ => None }));
        }
        if let Some(log) = self.log() {
            for m in log.msgs.iter().rev().take(recent) {
                urls.extend(fluxer_client::api::avatar_url(media, &m.author, 128));
                urls.extend(m.reply.iter().filter_map(|r| fluxer_client::api::avatar_url(media, &r.author, 128)));
                urls.extend(m.embeds.iter().filter_map(|e| e.image.as_deref())
                    .map(|u| fluxer_client::api::attachment_image_url(u, EMBED_FETCH_WIDTH)));
                urls.extend(m.attachments.iter().filter(|a| a.is_image()).filter_map(|a| a.url.as_deref())
                    .map(|u| fluxer_client::api::attachment_image_url(u, ATTACHMENT_FETCH_WIDTH)));
            }
        }
        urls
    }

    /// Puts a failed message back to pending and returns its text for another attempt.
    pub fn retry_failed(&mut self, channel_id: &str, local: &str) -> Option<String> {
        let m = self.logs.get_mut(channel_id)?.msgs.iter_mut().find(|m| m.id == local && matches!(m.delivery, Delivery::Failed(_)))?;
        m.delivery = Delivery::Pending;
        Some(m.body.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(id: &str, name: &str) -> Value { json!({"id": id, "username": name, "avatar": null}) }

    fn ready() -> Value {
        json!({
            "user": user("1", "eu"),
            "users": [user("2", "ana")],
            "guilds": [
                {"id": "g1", "properties": {"name": "Casa", "icon": "ic"}, "channels": [
                    {"id": "c2", "name": "dev", "type": 0, "position": 2}, {"id": "c1", "name": "geral", "type": 0, "position": 1},
                    {"id": "v1", "name": "voz", "type": 2, "position": 0}, {"id": "cat", "name": "Texto", "type": 4}]},
                {"id": "g2", "properties": {"name": "Outro"}, "channels": []}],
            "private_channels": [
                {"id": "d1", "type": 1, "recipients": [user("2", "ana")], "last_message_id": "50"},
                {"id": "d2", "type": 3, "name": "Grupo", "recipients": ["2"], "last_message_id": "90"},
                {"id": "d3", "type": 999}]
        })
    }

    fn msg(id: &str, ch: &str, who: &str, body: &str) -> Message {
        Message::plain(id, ch, body, User { id: who.into(), username: who.into(), global_name: None, avatar: None })
    }

    #[test]
    fn ready_builds_guilds_text_channels_in_order_and_dms_newest_first() {
        let mut c = Chat::default();
        c.apply_ready(&ready());
        assert_eq!(c.guilds.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Casa", "Outro"]);
        assert_eq!(c.guilds[0].icon.as_deref(), Some("ic"));
        assert_eq!(c.guilds[0].channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["geral", "dev"], "text only, by position");
        assert_eq!(c.dms.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Grupo", "ana"]);
        assert_eq!(c.dms[1].peer.as_ref().map(|u| u.id.as_str()), Some("2"));
        assert_eq!(c.nav, Nav { guild: Some(0), channel: 0 });
        assert_eq!(c.channel_id(), Some("c1"));
    }

    #[test]
    fn messages_go_to_their_channel_count_unread_and_clear_on_open() {
        let mut c = Chat::default();
        c.apply_ready(&ready());
        c.on_message(msg("10", "c1", "2", "oi"));
        c.on_message(msg("11", "c2", "2", "a"));
        c.on_message(msg("12", "c2", "2", "b"));
        c.on_message(msg("13", "c2", "1", "meu"));
        assert_eq!((c.guilds[0].channels[0].unread, c.guilds[0].channels[1].unread), (0, 2), "own messages are not unread");
        assert_eq!(c.unread(Some(0)), 2);
        c.open_channel(1);
        assert_eq!(c.unread(Some(0)), 0);
        assert_eq!(c.log().unwrap().msgs.len(), 3);
    }

    #[test]
    fn history_merges_with_live_traffic_without_duplicates_and_in_numeric_order() {
        let mut c = Chat::default();
        c.apply_ready(&ready());
        c.on_message(msg("100", "c1", "2", "ao vivo"));
        c.on_history("c1", vec![msg("9", "c1", "2", "velha"), msg("20", "c1", "2", "média"), msg("100", "c1", "2", "ao vivo")]);
        let ids: Vec<_> = c.log().unwrap().msgs.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["9", "20", "100"]);
        assert!(c.log().unwrap().loaded && !c.log().unwrap().has_more);
        c.on_history("c1", (0..PAGE).map(|i| msg(&format!("{}", 1000 + i), "c1", "2", "x")).collect());
        assert!(c.log().unwrap().has_more, "a full page may have older ones");
    }

    #[test]
    fn sending_shows_a_pending_message_then_swaps_in_the_servers_copy() {
        let mut c = Chat::default();
        c.apply_ready(&ready());
        let local = c.push_pending("c1", "olá".into()).unwrap();
        assert_eq!(c.log().unwrap().msgs[0].delivery, Delivery::Pending);
        // The gateway echo arrives before the REST reply: it replaces the pending copy.
        c.on_message(msg("77", "c1", "1", "olá"));
        assert_eq!(c.log().unwrap().msgs.len(), 1);
        assert_eq!(c.log().unwrap().msgs[0].delivery, Delivery::Sent);
        c.on_sent("c1", &local, Ok(msg("77", "c1", "1", "olá")));
        assert_eq!(c.log().unwrap().msgs.len(), 1, "no duplicate");
    }

    #[test]
    fn a_failed_send_stays_visible_and_can_be_retried() {
        let mut c = Chat::default();
        c.apply_ready(&ready());
        let local = c.push_pending("c1", "x".into()).unwrap();
        c.on_sent("c1", &local, Err("Sem permissão".into()));
        assert_eq!(c.log().unwrap().msgs[0].delivery, Delivery::Failed("Sem permissão".into()));
        assert_eq!(c.retry_failed("c1", &local).as_deref(), Some("x"));
        assert_eq!(c.log().unwrap().msgs[0].delivery, Delivery::Pending);
        c.on_sent("c1", &local, Ok(msg("5", "c1", "1", "x")));
        assert_eq!(c.log().unwrap().msgs[0].id, "5");
    }

    #[test]
    fn wanted_images_cover_icons_dm_avatars_and_recent_messages() {
        let mut c = Chat::default();
        c.media = Some("https://media".into());
        let mut r = ready();
        r["users"] = json!([{"id": "2", "username": "ana", "avatar": "av2"}]);
        r["private_channels"][0]["recipients"] = json!([{"id": "2", "username": "ana", "avatar": "av2"}]);
        c.apply_ready(&r);
        let mut m = msg("1", "c1", "2", "foto");
        m.author.avatar = Some("av2".into());
        m.attachments.push(Attachment { id: "a".into(), filename: "f.png".into(), content_type: Some("image/png".into()), url: Some("https://m/f.png".into()), width: Some(10), height: Some(10), size: 1 });
        m.attachments.push(Attachment { id: "b".into(), filename: "x.zip".into(), content_type: Some("application/zip".into()), url: Some("https://m/x.zip".into()), width: None, height: None, size: 1 });
        m.embeds.push(Embed { image: Some("https://m/e.jpg".into()), title: Some("t".into()), ..Embed::default() });
        c.on_message(m);
        let urls = c.wanted_images(80);
        assert!(urls.contains(&"https://m/e.jpg?format=png&width=480".to_string()), "embed pictures are fetched");
        assert!(urls.contains(&"https://media/icons/g1/ic.png?size=128".to_string()));
        assert!(urls.contains(&"https://media/avatars/2/av2.png?size=128".to_string()));
        assert!(urls.contains(&"https://m/f.png?format=png&width=720".to_string()));
        assert!(!urls.iter().any(|u| u.contains("x.zip")), "only images are fetched");
        c.media = None;
        assert!(c.wanted_images(80).is_empty(), "without a media host there is nothing to ask");
    }

    #[test]
    fn edits_and_deletes_touch_only_known_messages() {
        let mut c = Chat::default();
        c.apply_ready(&ready());
        c.on_message(msg("1", "c1", "2", "a"));
        assert!(c.on_message_updated(msg("1", "c1", "2", "editada")));
        assert_eq!(c.log().unwrap().msgs[0].body, "editada");
        assert!(!c.on_message_updated(msg("9", "c1", "2", "?")));
        assert!(c.on_message_deleted("c1", "1") && !c.on_message_deleted("c1", "1"));
    }

    #[test]
    fn categories_order_channels_and_read_state_marks_new() {
        let mut c = Chat::default();
        c.apply_ready(&json!({
            "user": {"id": "1", "username": "eu"},
            "read_states": [{"id": "c2", "last_message_id": "100", "mention_count": 2}, {"id": "c3", "last_message_id": "300", "mention_count": 0}],
            "guilds": [{"id": "g", "properties": {"name": "G"}, "channels": [
                {"id": "k2", "type": 4, "name": "Comunidade", "position": 2},
                {"id": "k1", "type": 4, "name": "Texto", "position": 1},
                {"id": "c1", "type": 0, "name": "solto", "position": 0},
                {"id": "c2", "type": 0, "name": "geral", "position": 0, "parent_id": "k1", "last_message_id": "150"},
                {"id": "c3", "type": 0, "name": "off", "position": 0, "parent_id": "k2", "last_message_id": "300"},
                {"id": "v", "type": 2, "name": "voz", "position": 1, "parent_id": "k1"}]}],
            "private_channels": []}));
        let ch = &c.guilds[0].channels;
        assert_eq!(ch.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["solto", "geral", "off"], "uncategorised first, then by category position; voice is left out");
        assert_eq!(ch[1].category.as_deref(), Some("Texto"));
        assert!(ch[1].new && ch[1].unread == 2 && ch[1].last_read.as_deref() == Some("100"));
        assert!(!ch[2].new, "read up to the last message");
        assert!(!ch[0].new, "no read state, nothing to flag");
        assert!(c.has_new(Some(0)));
        c.on_ack("c2", "150");
        assert!(!c.has_new(Some(0)));
    }
}

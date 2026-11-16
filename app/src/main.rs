//! Fluxer on Rune: sign-in, guilds and DMs, live messages, history, sending, avatars and images.
//! `cargo run --release -p fluxer-app` opens the window (`FLUXER_DEMO=1` for fake data);
//! `-- --snap out.png [--scene login|code|ip]` renders it headless.

mod anim;
mod animsize;
mod auth;
mod audio;
mod call;
mod clock;
mod gateway;
mod model;
mod net;
mod notify;
mod picker;
mod prefs;
mod richtext;
mod roster;
mod sounds;
mod store;
mod upload;
mod voice;
mod ui;

use fluxer_client::api::{Api, Embed, Message, Quoted, User};
use fluxer_client::connection::Event;
use fluxer_client::{Opcode, Payload};
use model::{Channel, Chat, Delivery, Guild, Log, Msg, Nav};
use rune::{AppSpec, TextEdit, Tx, Waker};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen { Login, Main }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status { Demo, Connecting, Connected, Reconnecting, Failed }

/// The right-click menu of a person: who, and the key of the element it opens beside.
pub struct Ctx { pub user: User, pub x: f32, pub y: f32 }

/// The profile card of a person, opened beside the element (by key) that was clicked; `at` is
/// where the pointer was, for a mention with no element of its own to anchor beside.
pub struct Prof { pub user: User, pub anchor: String, pub at: Option<(f32, f32)> }

/// The menu of one message (right click): the message, and where the pointer was.
pub struct MsgMenu { pub msg: Msg, pub x: f32, pub y: f32 }

/// What the message box is doing besides writing a new message: answering one, or rewriting one.
#[derive(Clone, Debug)]
pub struct Draft {
    /// The message being answered (`Quoted` is what the reply bar and the server payload use).
    pub reply: Option<Quoted>,
    /// The id of this account's message being edited; the box holds its text.
    pub editing: Option<String>,
}

/// The channel search: the box, the query of the answer on screen, and the results.
pub struct Search {
    pub open: bool,
    pub input: TextEdit,
    pub query: String,
    pub results: Vec<fluxer_client::api::Message>,
    pub loading: bool,
    pub indexing: bool,
    pub error: Option<String>,
}

impl Default for Search {
    fn default() -> Self { Search { open: false, input: TextEdit::new(""), query: String::new(), results: vec![], loading: false, indexing: false, error: None } }
}

pub fn run_search(s: &mut State) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    let q = s.search.input.text.trim().to_string();
    if q.is_empty() { return }
    let guild = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)).map(|g| g.id.clone());
    s.search.query = q.clone();
    s.search.loading = true;
    s.search.indexing = false;
    s.search.error = None;
    // A server is searched as a whole; a DM channel has no server, so it searches itself.
    if guild.is_some() { s.net.search(guild.as_deref(), None, &q) } else { s.net.search(None, Some(&cid), &q) }
}

fn on_search(s: &mut State, query: String, res: Result<fluxer_client::api::SearchOutcome, String>) {
    if s.search.query != query { return }
    s.search.loading = false;
    match res {
        Ok(fluxer_client::api::SearchOutcome::Indexing) => { s.search.indexing = true; s.search.results.clear() }
        Ok(fluxer_client::api::SearchOutcome::Hits(msgs)) => s.search.results = msgs,
        Err(e) => s.search.error = Some(e),
    }
}

/// A search hit was pressed: open its channel and put it in the middle of the list.
pub fn open_search_hit(s: &mut State, tx: &mut Tx, channel: &str, id: &str) {
    s.search.open = false;
    if s.chat.channel_id() != Some(channel) && !s.chat.open_channel_by_id(channel) { return }
    s.visible = 0..0;
    reconcile(s);
    if s.chat.logs.get(channel).is_some_and(|l| l.msgs.iter().any(|m| m.id == id)) {
        jump_to(s, tx, id);
    } else {
        s.pending_jump = Some((channel.to_string(), id.to_string()));
        s.net.history_around(channel, id);
    }
}

pub struct State {
    pub rail_hover: Option<String>,
    pub settings_open: bool,
    /// Profile form of the settings dialog.
    pub prof: [TextEdit; 4],
    pub pf: ui::settings::Form,
    pub picker: picker::Picker,
    pub prefs: prefs::Prefs,
    /// Message box popups: the "+" menu and its Customize submenu.
    pub plus_open: bool,
    /// The message the pointer is over (its reaction bar shows), and the one whose emoji picker is open.
    pub msg_hover: Option<String>,
    pub react_picker: Option<String>,
    /// The message jumped to from a quote click (id, until when its blue flash lasts).
    pub highlighted: Option<(String, std::time::Instant)>,
    /// Where the pointer last was, tracked live so a mention can open its card beside it.
    pub pointer: rune::Live<[f32; 2]>,
    /// The members panel's filter (names and nicknames).
    pub member_search: TextEdit,
    /// The status this account shows (online/idle/dnd/invisible), and its little menu.
    pub my_status: String,
    pub status_menu: bool,
    /// The video attachment playing inline, if any (attachment id, player).
    pub video: Option<(String, rune::Player)>,
    pub customize_open: bool,
    /// A short note about something that failed (a send), shown above the message box.
    pub notice: Option<String>,
    pub ctx: Option<Ctx>,
    /// The message whose right-click menu is open, and where.
    pub msg_menu: Option<MsgMenu>,
    /// Answering a message, or rewriting one.
    pub draft_mode: Draft,
    /// The (channel, message) already acknowledged to the server, so one ack goes per message.
    acked: Option<(String, String)>,
    /// The channel search (header magnifier).
    pub search: Search,
    /// A message to center once its around-page arrives (a search hit outside the loaded history).
    pending_jump: Option<(String, String)>,
    /// A DM call ringing for this account (channel id).
    pub incoming_call: Option<String>,
    pub profile: Option<Prof>,
    /// Profiles fetched so far, by `net::profile_key`.
    pub profiles: std::collections::HashMap<String, fluxer_client::api::Profile>,
    /// Progress (0..1) of the login card's entrance, driven by the poll clock.
    pub intro: f32,
    intro_t0: Option<f64>,
    pub screen: Screen,
    pub auth: auth::Auth,
    pub session: Option<store::Saved>,
    pub wake: Option<Waker>,
    pub status: Status,
    pub chat: Chat,
    pub net: net::Net,
    pub draft: TextEdit,
    pub gateway: Option<gateway::Handle>,
    /// The (server, channel) whose member list the gateway is sending us.
    pub subscribed: Option<(String, String)>,
    pub synced_guilds: std::collections::HashSet<String>,
    pub show_members: bool,
    pub call: call::Call,
    /// The window has the focus; GIFs and the banner only move while it does.
    pub focused: bool,
    /// Which messages of the open channel are on screen (indexes into its log): only those get
    /// their animated pictures loaded.
    pub visible: std::ops::Range<usize>,
    /// A typing notice arrived and its fade-out still has to be scheduled.
    typing_wake: bool,
    /// The scroll thumb of the message list.
    pub thumb: ui::thumb::Thumb,
}

fn demo_chat() -> Chat {
    let people: Vec<User> = ["ana", "bruno", "carla", "dudu"].iter().enumerate()
        .map(|(i, n)| User { id: (i + 1).to_string(), username: (*n).into(), global_name: None, avatar: None, discriminator: None }).collect();
    let mut c = Chat::default();
    c.me = Some(User { id: "0".into(), username: "você".into(), global_name: None, avatar: None, discriminator: None });
    let chan = |i: usize, n: &str| Channel { id: format!("demo-{i}"), name: n.into(), category: Some(if i < 2 { "Texto" } else { "Comunidade" }.into()),
        new: i == 1, last_read: Some("9990".into()), ..Channel::default() };
    c.guilds = vec![Guild {
        id: "demo".into(), name: "Demo".into(), icon: None, banner: None, owner_id: None, voice: vec![], stickers: vec![],
        channels: ["geral", "dev", "música", "off-topic"].iter().enumerate().map(|(i, n)| chan(i, n)).collect(),
    }];
    c.dms = vec![Channel { id: "demo-dm".into(), name: "ana".into(), unread: 2, peer: Some(people[0].clone()), ..Channel::default() }];
    for p in &people { c.roster.remember(p) }
    c.roster.on_typing("demo-0", "2", clock::now_ms());
    c.roster.on_typing("demo-0", "3", clock::now_ms());
    c.roster.on_member_list(&serde_json::json!({"guild_id": "demo", "member_count": 5, "online_count": 3, "ops": [{"op": "SYNC", "range": [0, 99], "items": [
        {"group": {"id": "online", "count": 3}},
        {"member": {"user": {"id": "1", "username": "ana"}, "presence": {"status": "online"}}},
        {"member": {"user": {"id": "2", "username": "bruno"}, "nick": "Bruno Silva", "presence": {"status": "idle"}}},
        {"member": {"user": {"id": "3", "username": "carla"}, "presence": {"status": "dnd"}}},
        {"group": {"id": "offline", "count": 2}},
        {"member": {"user": {"id": "4", "username": "dudu"}}},
    ]}]}));
    c.nav = Nav { guild: Some(0), channel: 0 };
    let base = clock::now_ms() - 10_000 * 40 * 60_000;
    let msgs = (0..10_000usize).map(|i| {
        let mut m = Msg::local(i.to_string(), people[i / 3 % 4].clone(),
            format!("Mensagem #{i} — **olá**, teste de _acentuação_: ção, ñ, 日本語 e emoji 🙂 com `código`"), Delivery::Sent);
        m.ts = Some(base + (i / 3) as i64 * 120 * 60_000 + (i % 3) as i64 * 2 * 60_000);
        if i % 7 == 3 { m.reply = Some(Quoted { id: Some(i.to_string()), author: people[(i + 1) % 4].clone(), content: "mensagem original citada".into(), has_attachments: false }) }
        if i % 11 == 5 { m.embeds = vec![Embed { url: Some("https://exemplo.com".into()), title: Some("Título do link".into()),
            description: Some("Descrição curta do conteúdo vinculado.".into()), provider: Some("Exemplo".into()), ..Embed::default() }] }
        m
    }).collect();
    c.logs.insert("demo-0".into(), Log { msgs, loaded: true, loading: false, has_more: false });
    c
}

impl Default for State {
    fn default() -> Self {
        State {
            rail_hover: None, settings_open: false, pf: Default::default(), prof: [TextEdit::new(""), TextEdit::new(""), TextEdit::new(""), TextEdit::new("")],
            picker: Default::default(), prefs: prefs::load(), plus_open: false, msg_hover: None, react_picker: None, highlighted: None, pointer: rune::Live::new([0.0, 0.0]), member_search: TextEdit::new(""), my_status: "online".into(), status_menu: false, video: None, customize_open: false, notice: None,
            ctx: None, msg_menu: None, draft_mode: Draft { reply: None, editing: None }, acked: None, search: Default::default(), pending_jump: None, incoming_call: None, profile: None, profiles: Default::default(), screen: Screen::Main, auth: auth::Auth::default(), session: None, wake: None, status: Status::Demo,
            intro: 0.0, intro_t0: None, chat: demo_chat(), net: net::Net::default(), draft: TextEdit::new(""), gateway: None, subscribed: None, synced_guilds: Default::default(), show_members: true, call: Default::default(), focused: true, visible: 0..0, typing_wake: false, thumb: Default::default(),
        }
    }
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(*k))?.as_str()
}

/// Applies one gateway event to the state; true when something visible changed.
fn apply(s: &mut State, e: Event) -> bool {
    match e {
        Event::Disconnected { fatal, .. } => {
            s.status = if fatal { Status::Failed } else { Status::Reconnecting };
            true
        }
        Event::ConnectFailed(why) => {
            eprintln!("fluxer: gateway connection failed: {why}");
            s.status = Status::Reconnecting;
            true
        }
        Event::Payload(Payload { op: Opcode::DISPATCH, t: Some(t), d: Some(d), .. }) => match t.as_str() {
            "READY" => {
                s.status = Status::Connected;
                s.subscribed = None;
                s.synced_guilds.clear();
                s.chat.apply_ready(&d);
                if let Some(u) = &s.chat.me { s.call.set_me(&u.id) }
                let api = s.net.api().cloned();
                s.picker.prefetch(&api);
                true
            }
            "RESUMED" => { s.status = Status::Connected; true }
            "TYPING_START" => match (str_at(&d, &["channel_id"]), str_at(&d, &["user_id"])) {
                (Some(c), Some(u)) => {
                    s.chat.roster.on_typing(c, u, clock::now_ms());
                    // The notice fades by itself; `poll` asks Rune to wake once it is due.
                    s.typing_wake = true;
                    true
                }
                _ => false,
            },
            "VOICE_STATE_UPDATE" => {
                let uid = str_at(&d, &["user_id"]).unwrap_or("").to_string();
                let here = |c: &Chat, ch: &Option<String>| ch.as_ref().is_some_and(|ch| c.voice.get(ch).is_some_and(|v| v.iter().any(|p| p.user_id == uid)));
                let channel = s.call.channel_id.clone();
                let was_here = here(&s.chat, &channel);
                if Some(uid.as_str()) == s.chat.me.as_ref().map(|u| u.id.as_str()) { s.call.on_my_state(&d) }
                s.chat.on_voice_state(&d);
                if Some(uid.as_str()) != s.chat.me.as_ref().map(|u| u.id.as_str()) { s.call.on_other_presence(was_here, here(&s.chat, &channel)) }
                true
            }
            "VOICE_SERVER_UPDATE" => { s.call.on_server_update(&d); true }
            "PRESENCE_UPDATE" => s.chat.roster.on_presence(&d),
            "GUILD_MEMBER_LIST_UPDATE" => { s.chat.roster.on_member_list(&d); true }
            "MESSAGE_ACK" => match (str_at(&d, &["channel_id"]), str_at(&d, &["message_id"])) {
                (Some(c), Some(m)) => s.chat.on_ack(c, m),
                _ => false,
            },
            "MESSAGE_CREATE" => match Message::from_json(&d) {
                Some(m) => {
                    let copy = m.clone();
                    s.chat.on_message(m);
                    if !s.focused { crate::notify::incoming(s, &copy) }
                    true
                }
                None => false,
            },
            "MESSAGE_UPDATE" => Message::from_json(&d).is_some_and(|m| s.chat.on_message_updated(m)),
            "MESSAGE_REACTION_ADD" | "MESSAGE_REACTION_REMOVE" => {
                let me = s.chat.me.as_ref().map(|u| u.id.as_str());
                match (str_at(&d, &["channel_id"]), str_at(&d, &["message_id"]), d.get("emoji")) {
                    (Some(c), Some(m), Some(e)) => s.chat.on_reaction(c, m, e, str_at(&d, &["user_id"]) == me && me.is_some(), t == "MESSAGE_REACTION_ADD"),
                    _ => false,
                }
            }
            "MESSAGE_REACTION_REMOVE_ALL" | "MESSAGE_REACTION_REMOVE_EMOJI" => match (str_at(&d, &["channel_id"]), str_at(&d, &["message_id"])) {
                (Some(c), Some(m)) => s.chat.on_reactions_cleared(c, m, d.get("emoji")),
                _ => false,
            },
            "MESSAGE_DELETE" => match (str_at(&d, &["channel_id"]), str_at(&d, &["id"])) {
                (Some(c), Some(id)) => s.chat.on_message_deleted(c, id),
                _ => false,
            },
            "CALL_CREATE" => {
                let me = s.chat.me.as_ref().map(|u| u.id.clone());
                let ringing = d.get("ringing").and_then(Value::as_array).into_iter().flatten()
                    .filter_map(Value::as_str).any(|id| Some(id) == me.as_deref());
                if ringing && let Some(c) = str_at(&d, &["channel_id"]) { s.incoming_call = Some(c.into()); true } else { false }
            }
            "CALL_DELETE" => {
                if let Some(c) = str_at(&d, &["channel_id"]) {
                    if s.incoming_call.as_deref() == Some(c) { s.incoming_call = None }
                    if s.call.active() && s.call.channel_id.as_deref() == Some(c) { s.call.leave(s.gateway.as_ref()); }
                    true
                } else { false }
            }
            "CALL_UPDATE" => false,
            _ => false,
        },
        Event::Payload(_) => false,
    }
}

fn setup(s: &mut State, wake: Waker) {
    s.auth.set_waker(wake.clone());
    s.net.set_waker(wake.clone());
    s.call.start(wake.clone());
    s.picker.set_waker(wake.clone());
    s.wake = Some(wake.clone());
    if std::env::var_os("FLUXER_DEMO").is_some() { return }
    match store::load() {
        Some(saved) => start_session(s, saved, wake),
        None => s.screen = Screen::Login,
    }
}

fn api_for(saved: &store::Saved) -> Arc<Api> {
    Arc::new(Api::new(
        fluxer_client::rest::Endpoints {
            api_client: saved.api_client.clone(), gateway: saved.gateway.clone(),
            media: saved.media.clone(), origin: saved.origin.clone(),
        },
        saved.token.clone(),
    ))
}

fn start_session(s: &mut State, saved: store::Saved, wake: Waker) {
    s.screen = Screen::Main;
    s.status = Status::Connecting;
    s.chat = Chat { media: saved.media.clone(), restore: store::load_nav(), ..Chat::default() };
    s.net.set_api(Some(api_for(&saved)));
    s.net.refresh_endpoints(&saved.instance);
    s.gateway = Some(gateway::start(gateway::Settings { gateway: saved.gateway.clone(), token: saved.token.clone() }, wake));
    s.session = Some(saved);
}

/// Back to the login screen, keeping the instance the user typed.
pub fn sign_out(s: &mut State, reason: Option<&str>) {
    s.call.leave(s.gateway.as_ref());
    store::clear();
    s.gateway = None;
    s.session = None;
    s.net.set_api(None);
    s.chat = Chat::default();
    s.screen = Screen::Login;
    s.auth.cancel_polling();
    let instance = s.auth.instance.text.clone();
    s.auth = auth::Auth::default();
    s.auth.instance = TextEdit::new(instance);
    if let Some(w) = &s.wake { s.auth.set_waker(w.clone()) }
    s.auth.error = reason.map(String::from);
}

fn on_auth(s: &mut State, m: auth::Msg) {
    match m {
        auth::Msg::Failed(e) => {
            if matches!(s.auth.step, auth::Step::Waiting { .. }) { s.auth.cancel_polling(); s.auth.step = auth::Step::Credentials }
            s.auth.info = None;
            s.auth.error = Some(e);
        }
        auth::Msg::Notice(n) => s.auth.info = Some(n),
        auth::Msg::Waiting { instance, ep, ticket, email, resend_in } => {
            s.auth.resolved = Some((instance, ep));
            s.auth.step = auth::Step::Waiting {
                ticket: ticket.clone(), email, resend_ready_at: std::time::Instant::now() + std::time::Duration::from_secs(resend_in),
            };
            s.auth.error = None;
            s.auth.info = None;
            s.auth.start_polling(ticket);
        }
        auth::Msg::Mfa { instance, ep, ticket } => {
            s.auth.resolved = Some((instance, ep));
            s.auth.step = auth::Step::Code { ticket };
            s.auth.error = None;
        }
        auth::Msg::Signed { instance, ep, creds } => {
            let saved = store::Saved {
                instance, gateway: ep.gateway, api_client: ep.api_client, media: ep.media, origin: ep.origin,
                token: creds.token, user_id: creds.user_id, username: creds.username,
            };
            if let Err(e) = store::save(&saved) { eprintln!("fluxer: could not save the session: {e}") }
            s.auth.cancel_polling();
            s.auth.password = TextEdit::new("");
            s.auth.code = TextEdit::new("");
            s.auth.step = auth::Step::Credentials;
            if let Some(w) = s.wake.clone() { start_session(s, saved, w) }
        }
    }
}

/// Applies what the network jobs finished. True when the screen needs a redraw.
fn on_net(s: &mut State, d: net::Done) -> bool {
    match d {
        net::Done::History { channel, res: Ok(msgs) } => { s.chat.on_history(&channel, msgs); true }
        net::Done::History { channel, res: Err(e) } => {
            eprintln!("fluxer: history of {channel} failed: {e}");
            s.chat.on_history_failed(&channel);
            true
        }
        net::Done::Sent { channel, local, res } => { s.chat.on_sent(&channel, &local, res); true }
        net::Done::Endpoints(Ok(ep)) => {
            if let Some(saved) = &mut s.session {
                saved.api_client = ep.api_client.clone();
                saved.gateway = ep.gateway.clone();
                saved.media = ep.media.clone();
                saved.origin = ep.origin.clone();
                let _ = store::save(saved);
                s.net.set_api(Some(api_for(saved)));
            }
            s.chat.media = ep.media;
            true
        }
        net::Done::Endpoints(Err(e)) => { eprintln!("fluxer: endpoint refresh failed: {e}"); false }
        net::Done::Posted(Err(e)) => { s.notice = Some(format!("Não foi possível enviar: {e}")); true }
        net::Done::Posted(Ok(())) => false,
        net::Done::Reacted(Err(e)) => { s.notice = Some(format!("Não foi possível reagir: {e}")); true }
        net::Done::Reacted(Ok(())) => false,
        net::Done::ProfileSaved { guild, res } => { ui::settings::on_saved(s, guild, res); true }
        net::Done::Edited(Ok(m)) => s.chat.on_message_updated(m),
        net::Done::Edited(Err(e)) => { s.notice = Some(format!("Não foi possível editar: {e}")); true }
        net::Done::Deleted { channel, id, res: Ok(()) } => s.chat.on_message_deleted(&channel, &id),
        net::Done::Deleted { res: Err(e), .. } => { s.notice = Some(format!("Não foi possível apagar: {e}")); true }
        net::Done::Acked(_) => false,
        net::Done::AvatarSaved(res) => { ui::settings::on_avatar(s, res); true }
        net::Done::TimezoneSaved(res) => { ui::settings::on_timezone(s, res); true }
        net::Done::Search { query, res } => { on_search(s, query, res); true }
        net::Done::HistoryAround { channel, target, res: Ok(msgs) } => {
            s.chat.load_around(&channel, msgs);
            s.pending_jump = Some((channel, target));
            true
        }
        net::Done::HistoryAround { res: Err(e), .. } => { s.notice = Some(format!("Não foi possível abrir a mensagem: {e}")); true }
        net::Done::Profile { key, res: Ok(p) } => {
            if let (Some(media), Some(h)) = (s.chat.media.clone(), p.banner.as_deref()) {
                if let Some(uid) = key.split('@').next() { s.net.want_image_first(&fluxer_client::api::banner_still_url(&media, uid, h, 600)) }
            }
            s.profiles.insert(key.clone(), p);
            ui::settings::on_profile_loaded(s, &key);
            true
        }
        net::Done::Profile { res: Err(e), .. } => { eprintln!("fluxer: profile failed: {e}"); false }
        net::Done::Image { .. } => false,
        // An animation showed its first frame: the view swaps its still for the moving picture.
        net::Done::Animated { .. } => true,
    }
}

/// Asks for whatever the open channel is missing: its history, and the pictures on screen.
/// Idempotent, so it runs after every change of channel or of incoming data.
fn reconcile(s: &mut State) {
    if s.net.has_api() && let Some(cid) = s.chat.channel_id().map(String::from) {
        let log = s.chat.logs.entry(cid.clone()).or_default();
        if !log.loaded && !log.loading {
            log.loading = true;
            s.net.history(&cid, None);
        }
    }
    // Reading is acknowledging: the newest real message of the open channel, once, while the
    // window has the focus. The server keeps the unread badges and the "new messages" line.
    if s.focused && s.net.has_api() && let Some(cid) = s.chat.channel_id().map(String::from)
        && let Some(mid) = s.chat.logs.get(&cid).and_then(|l| l.msgs.iter().rev()
            .find(|m| m.delivery == Delivery::Sent && !m.id.starts_with("local-")).map(|m| m.id.clone()))
        && s.acked.as_ref() != Some(&(cid.clone(), mid.clone()))
    {
        s.net.ack(&cid, &mid);
        s.acked = Some((cid, mid));
    }
    let wanted = wanted_all(s);
    for url in &wanted { s.net.want_image(url) }
    s.net.trim(&wanted.into_iter().collect());
    subscribe_members(s);
}

/// Every picture the screen shows or is about to show: the open channel's (animated ones only for
/// the messages on screen, and only while the window is focused), the GIF picker's, and the open
/// profile card's banner.
fn wanted_all(s: &State) -> Vec<String> {
    // Until the list reports what is on screen nothing is known to move; the window not focused, nothing moves.
    let animate = (s.focused && !s.visible.is_empty()).then(|| s.visible.clone());
    let mut urls = s.chat.wanted_images(80, animate, s.picker.open == Some(picker::Tab::Stickers));
    urls.extend(s.picker.home_images());
    urls.extend(s.picker.wanted_images());
    urls.extend(s.picker.animated_images());
    if let (Some(p), Some(media)) = (&s.profile, s.chat.media.as_deref()) {
        let guild = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)).map(|g| g.id.as_str());
        if let Some(h) = s.profiles.get(&net::profile_key(&p.user.id, guild)).and_then(|i| i.banner.as_deref()) {
            urls.push(fluxer_client::api::banner_still_url(media, &p.user.id, h, 600));
        }
    }
    urls
}

/// Asks the gateway for the open server's member list (and its typing events). Read-only; resent
/// after every new session because subscriptions do not outlive one.
fn subscribe_members(s: &mut State) {
    if s.status != Status::Connected { return }
    let want = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)).zip(s.chat.channel_id()).map(|(g, c)| (g.id.clone(), c.to_string()));
    if want == s.subscribed { return }
    let Some(gw) = &s.gateway else { return };
    let Some((guild, channel)) = &want else { s.subscribed = None; return };
    let mut sub = serde_json::json!({ "active": true, "typing": true,
        "member_list_channels": { channel: [[roster::LIST_RANGE.0, roster::LIST_RANGE.1]] } });
    if !s.synced_guilds.contains(guild) { sub["sync"] = true.into() }
    let frame = Payload { op: Opcode::LAZY_REQUEST, d: Some(serde_json::json!({ "subscriptions": { guild: sub } })), s: None, t: None };
    if gw.send(frame) {
        s.synced_guilds.insert(guild.clone());
        s.subscribed = want;
    }
}

fn poll(s: &mut State, t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    if s.intro < 1.0 {
        let t0 = *s.intro_t0.get_or_insert(t);
        s.intro = (((t - t0) / INTRO_SECS) as f32).clamp(0.0, 1.0);
        changed = true;
    }
    // A jumped-to message's blue flash fades out on its own after its three seconds.
    if s.highlighted.as_ref().is_some_and(|(_, until)| std::time::Instant::now() >= *until) {
        s.highlighted = None;
        changed = true;
    }
    // While a clip plays, the frame re-draws so its progress bar moves.
    if s.video.as_ref().is_some_and(|(_, p)| p.is_playing()) { changed = true }
    for m in s.auth.drain() { on_auth(s, m); changed = true }
    let mut events = Vec::new();
    if let Some(g) = &s.gateway { events.extend(g.inbox.try_iter()) }
    changed |= events.into_iter().fold(false, |c, e| apply(s, e) | c);
    if std::mem::take(&mut s.typing_wake) { tx.wake_after(std::time::Duration::from_millis(roster::TYPING_TTL_MS as u64 + 250)) }
    let images_before = s.net.busy_with_images();
    for d in s.net.drain() { changed |= on_net(s, d) }
    // A search hit's around-page arrived: center it now that its row exists.
    if let Some((channel, id)) = s.pending_jump.take() {
        if s.chat.channel_id() == Some(channel.as_str()) { tx.scroll_center(format!("rows/{id}")) }
        else { s.pending_jump = Some((channel, id)) }
    }
    changed |= s.call.poll(s.gateway.as_ref());
    if s.picker.poll() { picker_images(s); changed = true }
    // A finished download changes what draws; so does a queue that just emptied.
    changed |= images_before || s.net.busy_with_images();
    if s.status == Status::Failed && s.session.is_some() {
        sign_out(s, Some("A sessão expirou ou foi recusada. Entre de novo."));
        changed = true;
    }
    if s.screen == Screen::Main { reconcile(s) }
    changed
}

const INTRO_SECS: f64 = 0.8;

/// Toggles the inline clip player of a video attachment: opens and plays, or closes.
pub fn toggle_video(s: &mut State, id: &str, url: &str) {
    if s.video.as_ref().is_some_and(|(v, _)| v == id) {
        s.video = None;
        return;
    }
    let p = rune::Player::url(url);
    if let Some(w) = s.wake.clone() { p.set_waker(w) }
    p.preload();
    p.play();
    s.video = Some((id.to_string(), p));
}

/// Pauses or resumes the open clip.
pub fn toggle_video_play(s: &mut State) {
    if let Some((_, p)) = &s.video {
        if p.is_playing() { p.pause() } else { p.play() }
    }
}

/// A quote click lands on the original message and flashes it blue for three seconds.
pub fn jump_to(s: &mut State, tx: &mut rune::Tx, id: &str) {
    s.highlighted = Some((id.to_string(), std::time::Instant::now() + std::time::Duration::from_secs(3)));
    tx.scroll_center(format!("rows/{id}"));
}

/// Clicking a voice channel joins it (unless already in it) and shows the call's mosaic.
pub fn join_voice(s: &mut State, channel: &str) {
    let Some(guild) = s.chat.voice_channel(channel).map(|(g, _)| g.id.clone()) else { return };
    s.call.join(s.gateway.as_ref(), Some(&guild), channel);
    if s.call.channel_id.as_deref() == Some(channel) {
        s.call.view = true;
        s.chat.voice_view = None;
    }
}

/// Starts (or joins) the DM call of the open direct-message channel.
pub fn start_dm_call(s: &mut State, channel: &str) {
    let peer = s.chat.dms.iter().find(|c| c.id == channel).and_then(|c| c.peer.as_ref()).map(|p| p.id.clone());
    if let Some(p) = peer { s.net.ring_call(channel, vec![p]) }
    s.incoming_call = None;
    s.call.join(s.gateway.as_ref(), None, channel);
    s.call.view = true;
}

pub fn watch_stream(s: &mut State, connection: &str, on: bool) { s.call.set_watching(s.gateway.as_ref(), connection, on) }

pub fn leave_voice(s: &mut State) { s.call.leave(s.gateway.as_ref()) }
pub fn toggle_mute(s: &mut State) { s.call.toggle_mute(s.gateway.as_ref()) }
pub fn toggle_deafen(s: &mut State) { s.call.toggle_deafen(s.gateway.as_ref()) }

/// Changes what status this account shows (the gateway tells everyone else; invisible looks offline).
pub fn set_status(s: &mut State, status: &str) {
    s.my_status = status.to_string();
    if let Some(me) = &s.chat.me { s.chat.roster.status.insert(me.id.clone(), status.to_string()); }
    if let Some(gw) = &s.gateway {
        gw.send(Payload { op: Opcode::PRESENCE_UPDATE, d: Some(serde_json::json!({ "status": status })), s: None, t: None });
    }
}

pub fn set_focus(s: &mut State, focused: bool) {    s.focused = focused;
    s.net.set_focused(focused);
    // Losing the focus stops the animations (their pictures are let go in time); gaining it asks for those on screen again.
    reconcile(s);
}

pub fn submit(s: &mut State) { s.auth.submit(); }

/// Remembers what is open so the next launch comes back to it.
fn remember(s: &State) {
    if s.session.is_some() && let Some(pos) = s.chat.position() { store::save_nav(&pos) }
}

pub fn open_guild(s: &mut State, g: Option<usize>) {
    s.call.view = false;
    s.visible = 0..0;
    s.chat.open_guild(g);
    reconcile(s);
    remember(s);
}

pub fn open_channel(s: &mut State, i: usize) {
    s.call.view = false;
    s.visible = 0..0;
    s.draft_mode.reply = None;
    if s.draft_mode.editing.take().is_some() { s.draft.clear() }
    s.chat.open_channel(i);
    reconcile(s);
    remember(s);
}

pub fn open_voice_chat(s: &mut State, i: usize) {
    s.call.view = false;
    s.visible = 0..0;
    s.chat.open_voice_chat(i);
    reconcile(s);
}

/// Reacts to a message of the open channel with `emoji` (an `emoji_key`), or takes the reaction back
/// when this account already used it. Shown at once; the server's echo changes nothing more.
pub fn toggle_reaction(s: &mut State, message: &str, emoji: &str) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    let (name, id) = match emoji.split_once(':') { Some((n, i)) => (n, Some(i)), None => (emoji, None) };
    let add = !s.chat.logs.get(&cid).and_then(|l| l.msgs.iter().find(|m| m.id == message))
        .is_some_and(|m| m.reactions.iter().any(|r| r.key == emoji && r.me));
    let json = serde_json::json!({ "name": name, "id": id });
    s.chat.on_reaction(&cid, message, &json, true, add);
    s.net.react(&cid, message, emoji, add);
}

/// Opens the profile card of `user` beside the element keyed `anchor`, and asks for the full profile.
pub fn show_profile(s: &mut State, user: &User, anchor: &str) {
    let guild = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)).map(|g| g.id.clone());
    s.net.profile(&user.id, guild.as_deref());
    s.profile = Some(Prof { user: user.clone(), anchor: anchor.to_string(), at: None });
}

/// Opens the profile card of someone mentioned at `at` (window points), so it can sit to the
/// right of the mention even though a mention has no element of its own to anchor beside.
pub fn show_profile_at(s: &mut State, user: &User, anchor: &str, at: (f32, f32)) {
    show_profile(s, user, anchor);
    if let Some(p) = &mut s.profile { p.at = Some(at) }
}

pub fn load_older(s: &mut State) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    let Some(log) = s.chat.logs.get_mut(&cid) else { return };
    if log.loading { return }
    let Some(first) = log.msgs.iter().find(|m| m.delivery == Delivery::Sent).map(|m| m.id.clone()) else { return };
    log.loading = true;
    s.net.history(&cid, Some(&first));
}

pub fn retry(s: &mut State, local: &str) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    if let Some(body) = s.chat.retry_failed(&cid, local) { s.net.send(&cid, local, &body) }
}

/// Sends the draft to the open channel: shown at once as pending, confirmed by the REST reply or
/// the gateway echo, whichever comes first. In demo mode it just appears.
pub fn send(s: &mut State) {
    let body = s.draft.text.trim().to_string();
    if body.is_empty() { return }
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    if let Some(id) = s.draft_mode.editing.clone() {
        s.notice = None;
        if s.net.has_api() { s.net.edit_message(&cid, &id, &body) }
        else if let Some(m) = s.chat.logs.get_mut(&cid).and_then(|l| l.msgs.iter_mut().find(|m| m.id == id)) { m.body = body; m.edited = true }
        s.draft_mode.editing = None;
        s.draft.clear();
        return;
    }
    if let Some(reply) = s.draft_mode.reply.take() {
        send_reply_body(s, body, reply);
        s.draft.clear();
        return;
    }
    send_body(s, body);
    // Cleared in place, not replaced: a fresh TextEdit would drop the caret and the next message
    // would need another click on the box.
    s.draft.clear();
}

/// Sends `body` answering `reply`: shown at once with its quote, confirmed by the server.
fn send_reply_body(s: &mut State, body: String, reply: Quoted) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    s.notice = None;
    if s.net.has_api() {
        if let Some(local) = s.chat.push_pending(&cid, body.clone()) {
            if let Some(m) = s.chat.logs.get_mut(&cid).and_then(|l| l.msgs.iter_mut().find(|m| m.id == local)) { m.reply = Some(reply.clone()) }
            match reply.id.as_deref() {
                Some(id) => s.net.send_reply(&cid, &local, &body, id),
                None => s.net.send(&cid, &local, &body),
            }
        }
    } else if let Some(local) = s.chat.push_pending(&cid, body) {
        if let Some(m) = s.chat.logs.get_mut(&cid).and_then(|l| l.msgs.iter_mut().find(|m| m.id == local)) {
            m.reply = Some(reply);
            m.delivery = Delivery::Sent;
        }
    }
}

/// Sends `body` to the open channel: shown at once as pending, confirmed by the REST reply or the
/// gateway echo, whichever comes first. In demo mode it just appears.
pub fn send_body(s: &mut State, body: String) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    s.notice = None;
    s.draft_mode.reply = None;
    s.draft_mode.editing = None;
    if s.net.has_api() {
        if let Some(local) = s.chat.push_pending(&cid, body.clone()) { s.net.send(&cid, &local, &body) }
    } else if let Some(local) = s.chat.push_pending(&cid, body) {
        s.chat.on_sent(&cid, &local, Ok(Message::plain(format!("sent-{local}"), cid.clone(), s.chat.logs[&cid].msgs.last().map(|m| m.body.clone()).unwrap_or_default(),
            s.chat.me.clone().expect("demo user"))));
    }
}

pub fn send_sticker(s: &mut State, id: &str) {
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    s.notice = None;
    s.net.send_sticker(&cid, id);
    s.picker.close();
    reconcile(s);
}

/// A GIF or saved picture goes out as its link; the server turns it into the picture.
pub fn send_link(s: &mut State, url: String) { send_body(s, url); s.picker.close(); reconcile(s) }

pub fn insert_text(s: &mut State, text: &str) { let t = format!("{}{text}", s.draft.text); s.draft.replace_text(t) }

/// Asks the image cache for what the open picker tab shows.
pub fn picker_images(s: &mut State) {
    // Behind the message box's own pictures, the GIF home page loads ahead of time.
    for url in s.picker.home_images() { s.net.want_image(&url) }
    // What the open tab shows goes first.
    for url in s.picker.wanted_images() { s.net.want_image_first(&url) }
    // The moving versions follow once the still ones are on their way.
    for url in s.picker.animated_images() { s.net.want_image(&url) }
    for url in s.chat.wanted_images(0, None, s.picker.open == Some(picker::Tab::Stickers)) { s.net.want_image(&url) }
    // What the picker asks for is part of what is wanted: pictures it no longer shows are let go.
    reconcile(s);
}

/// A connected call in the demo server: two people, one sharing a screen, and the mosaic open.
fn demo_call(s: &mut State) {
    use crate::model::VState;
    let mut vch = Channel::default();
    vch.id = "demo-v0".into();
    vch.name = "Geral".into();
    if s.chat.guilds[0].voice.is_empty() { s.chat.guilds[0].voice.push(vch) }
    let st = |id: &str, conn: &str, stream: bool| VState { user_id: id.into(), connection_id: conn.into(), self_stream: stream, ..Default::default() };
    s.chat.voice.insert("demo-v0".into(), vec![st("0", "c0", false), st("1", "c1", true), st("2", "c2", false), st("3", "c3", false)]);
    s.call.phase = call::Phase::Connected;
    s.call.guild_id = Some("demo".into());
    s.call.channel_id = Some("demo-v0".into());
    s.call.view = true;
}

fn main() {
    let spec = AppSpec::<State> {
        scenes: &[
            ("login", |s| { s.intro = 1.0; s.screen = Screen::Login }),
            ("ip", |s| {
                s.intro = 1.0;
                s.screen = Screen::Login;
                s.auth.step = auth::Step::Waiting { ticket: "t".into(), email: "voce@exemplo.com".into(), resend_ready_at: std::time::Instant::now() };
            }),
            ("code", |s| {
                s.intro = 1.0;
                s.screen = Screen::Login;
                s.auth.step = auth::Step::Code { ticket: "t".into() };
                s.auth.error = Some("Código inválido.".into());
            }),
            // The call's mosaic with a screen share, for looking at it without a server.
            ("call", |s| { demo_call(s); }),
            ("call-watch", |s| { demo_call(s); s.call.watching.push("demo:demo-v0:c1".into()); }),
            ("call-ping", |s| { demo_call(s); s.call.ping = Some(42); s.call.ping_tip = true; }),
            ("dm", |s| { s.chat.open_guild(None); }),
            // The message list's scroll thumb, shown as with the pointer near it (see `--scene scroll`).
            ("scroll", |s| { s.thumb.pin(); }),
            // The settings dialog, for looking at it without clicking (see `--scene settings`).
            ("settings", |s| {
                s.settings_open = true;
                if let Some(me) = &mut s.chat.me {
                    me.id = "1567191623270400000".into();
                    me.username = "muniz".into();
                    me.global_name = Some("Muniz".into());
                    me.discriminator = Some("9205".into());
                }
                s.prof[0] = TextEdit::new("Muniz");
                s.prof[3] = TextEdit::new("#4641D9");
                s.pf.base = ["Muniz".into(), String::new(), String::new(), "#4641D9".into()];
            }),
            // The per-community profile dialog (see `--scene settings-guild`).
            ("settings-guild", |s| {
                s.settings_open = true;
                if let Some(me) = &mut s.chat.me {
                    me.id = "1567191623270400000".into();
                    me.username = "muniz".into();
                    me.global_name = Some("Muniz".into());
                    me.discriminator = Some("9205".into());
                }
                s.pf.kind = Some(0);
                s.prof[0] = TextEdit::new("Muniz");
                s.prof[2] = TextEdit::new("Doc, I'm from the future.");
                s.prof[3] = TextEdit::new("#4641D9");
                s.pf.base = ["Muniz".into(), String::new(), "Doc, I'm from the future.".into(), "#4641D9".into()];
            }),
            // The settings dialog's voice page, with a made-up device list (see `--scene settings-voice`).
            ("settings-voice", |s| {
                s.settings_open = true;
                if let Some(me) = &mut s.chat.me {
                    me.id = "1567191623270400000".into();
                    me.username = "muniz".into();
                    me.global_name = Some("Muniz".into());
                    me.discriminator = Some("9205".into());
                }
                let dev = |id: &str, label: &str| crate::audio::Device { id: id.into(), label: label.into() };
                s.call.devices = Some(crate::audio::Devices {
                    mics: vec![dev("coreaudio:BuiltInMicrophoneDevice", "Microfone (MacBook Air)"),
                        dev("coreaudio:AppleUSBAudioEngine:Kingston:HyperX", "HyperX Virtual Surround Sound")],
                    speakers: vec![dev("coreaudio:BuiltInSpeakerDevice", "Alto-falantes (MacBook Air)"),
                        dev("coreaudio:05E30224-0000-0000-071E-010380351E78", "24G2W1G4")],
                });
                s.call.settings.mic = Some("coreaudio:AppleUSBAudioEngine:Kingston:HyperX".into());
                s.pf.page = crate::ui::settings::Page::Voice;
            }),
            // The right-click menu of a message (see `--scene msg-menu`).
            ("msg-menu", |s| {
                let me = s.chat.me.clone().expect("demo user");
                let m = Msg::local("mine".into(), me, "olha esse menu".into(), Delivery::Sent);
                s.msg_menu = Some(MsgMenu { msg: m, x: 640.0, y: 360.0 });
            }),
            // The message box answering an earlier message (see `--scene reply-bar`).
            ("reply-bar", |s| {
                let me = s.chat.me.clone().expect("demo user");
                s.draft_mode.reply = Some(Quoted { id: Some("9998".into()), author: me, content: "mensagem original".into(), has_attachments: false });
            }),
            // Embed + video + role icon proofs (see `--scene extras`).
            ("extras", |s| {
                use fluxer_client::api::User as U;
                let now = crate::clock::now_ms();
                let ana = U { id: "2".into(), username: "ana".into(), global_name: None, avatar: None, discriminator: None };
                s.chat.roster.remember(&ana);
                s.chat.roster.roles.insert("r1".into(), "pai de santo".into());
                s.chat.roster.role_icon.insert("r1".into(), (None, Some("⭐".into())));
                s.chat.roster.set_roles("demo", "2", vec!["r1".into()]);
                let mut msgs = Vec::new();
                let mut m1 = Msg::local("e1".into(), ana.clone(), "https://github.com/fluxerapp/fluxer".into(), Delivery::Sent);
                m1.ts = Some(now - 30_000);
                m1.embeds = vec![Embed { url: Some("https://github.com/fluxerapp/fluxer".into()),
                    title: Some("GitHub - fluxerapp/fluxer: A free and open source instant messaging app".into()),
                    description: Some("A free and open source instant messaging and VoIP chat app built for friends, groups, and communities.".into()),
                    provider: Some("GitHub".into()), author: None, image: None, thumbnail: Some("https://example.com/thumb.png".into()),
                    video: false, gif: false, image_size: None, color: Some(0x24292e) }];
                msgs.push(m1);
                let mut m2 = Msg::local("e2".into(), ana.clone(), "shows, primeira mensagem é do pai".into(), Delivery::Sent);
                m2.ts = Some(now);
                m2.attachments = vec![fluxer_client::api::Attachment { id: "vid1".into(), filename: "10_segundos_de_CHUPA_CORINTHIANS.mp4".into(),
                    content_type: Some("video/mp4".into()), url: Some("https://example.com/clip.mp4".into()), width: Some(1280), height: Some(720), size: 565_000 }];
                msgs.push(m2);
                s.chat.logs.insert("demo-0".into(), Log { msgs, loaded: true, loading: false, has_more: false });
            }),
            // The chat with nobody typing, for checking the message box sits right under the
            // last message (see `--scene quiet`).
            ("quiet", |s| {
                s.chat.roster.stop_typing("demo-0", "2");
                s.chat.roster.stop_typing("demo-0", "3");
            }),
            // A far-off reply target, for checking the jump scrolls (see `--scene farjump`).
            ("farjump", |s| {
                let now = crate::clock::now_ms();
                let mut msgs = vec![Msg::local("old".into(),
                    fluxer_client::api::User { id: "2".into(), username: "ana".into(), global_name: None, avatar: None, discriminator: None },
                    "mensagem original bem antiga".into(), Delivery::Sent)];
                msgs[0].ts = Some(now - 300_000);
                for i in 1..298 {
                    let mut m = Msg::local(format!("x{i}"),
                        fluxer_client::api::User { id: "3".into(), username: "bruno".into(), global_name: None, avatar: None, discriminator: None },
                        format!("filler {i}"), Delivery::Sent);
                    m.ts = Some(now - 297_000 + i * 1000);
                    msgs.push(m);
                }
                let mut last = Msg::local("new".into(),
                    fluxer_client::api::User { id: "3".into(), username: "bruno".into(), global_name: None, avatar: None, discriminator: None },
                    "Respondendo à antiga".into(), Delivery::Sent);
                last.ts = Some(now);
                last.reply = Some(Quoted { id: Some("old".into()),
                    author: fluxer_client::api::User { id: "2".into(), username: "ana".into(), global_name: None, avatar: None, discriminator: None },
                    content: "mensagem original bem antiga".into(), has_attachments: false });
                msgs.push(last);
                s.chat.logs.insert("demo-0".into(), Log { msgs, loaded: true, loading: false, has_more: false });
            }),
            // A long bio with no spaces, for checking the line breaking (see `--scene settings-longbio`).
            ("settings-longbio", |s| {
                s.settings_open = true;
                if let Some(me) = &mut s.chat.me {
                    me.id = "1567191623270400000".into();
                    me.username = "muniz".into();
                    me.global_name = Some("Muniz".into());
                    me.discriminator = Some("9205".into());
                }
                s.pf.kind = Some(0);
                let bio = "a".repeat(131);
                s.prof[0] = TextEdit::new("Muniz");
                s.prof[2] = TextEdit::new(bio.clone());
                s.prof[3] = TextEdit::new("#4641D9");
                s.pf.base = ["Muniz".into(), String::new(), bio, "#4641D9".into()];
            }),
            // The colour picker open (see `--scene settings-picker`).
            ("settings-picker", |s| {
                s.settings_open = true;
                if let Some(me) = &mut s.chat.me {
                    me.id = "1567191623270400000".into();
                    me.username = "muniz".into();
                    me.global_name = Some("Muniz".into());
                    me.discriminator = Some("9205".into());
                }
                s.prof[0] = TextEdit::new("Muniz");
                s.prof[3] = TextEdit::new("#4641D9");
                s.pf.base = ["Muniz".into(), String::new(), String::new(), "#4641D9".into()];
                s.pf.color_open = true;
            }),
            // Mention / reply / welcome rendering (see `--scene mentions`).
            ("mentions", |s| {
                use fluxer_client::api::User as U;
                let now = crate::clock::now_ms();
                let lion = U { id: "1".into(), username: "lion".into(), global_name: None, avatar: None, discriminator: None };
                let namorada = U { id: "2".into(), username: "namorada".into(), global_name: None, avatar: None, discriminator: None };
                let fluxer = U { id: "sys".into(), username: "fluxer".into(), global_name: Some("Fluxer".into()), avatar: None, discriminator: None };
                s.chat.me = Some(lion.clone());
                s.chat.roster.remember(&lion);
                s.chat.roster.remember(&namorada);
                s.chat.roster.set_nick("demo", "2", Some("Namorada do Ronaldinho"));
                let mk = |id: &str, a: U, body: &str, ts: i64| {
                    let mut m = Msg::local(id.into(), a, body.into(), Delivery::Sent);
                    m.ts = Some(ts);
                    m
                };
                let mut msgs = vec![
                    mk("m1", fluxer, "Welcome, Lion!", now - 50_000),
                    mk("m2", namorada.clone(), "@everyone", now - 40_000),
                    mk("m3", lion.clone(), "Eu só consigo falar, não consigo ouvir", now - 30_000),
                ];
                let mut r = mk("m4", lion.clone(), "<@2> oi", now - 20_000);
                r.reply = Some(Quoted { id: Some("m3".into()), author: lion.clone(), content: "Eu só consigo falar, não consigo ouvir".into(), has_attachments: false });
                r.mentions = vec![namorada.clone()];
                msgs.push(r);
                let mut ping = mk("m5", namorada.clone(), "<@1> viu?", now - 10_000);
                ping.mentions = vec![lion.clone()];
                msgs.push(ping);
                s.chat.logs.insert("demo-0".into(), Log { msgs, loaded: true, loading: false, has_more: false });
                // Someone in the voice channel, for the rail's call badge.
                let mut vch = Channel::default();
                vch.id = "demo-v0".into();
                vch.name = "Geral".into();
                s.chat.guilds[0].voice.push(vch);
                s.chat.voice.insert("demo-v0".into(), vec![crate::model::VState { user_id: "2".into(), self_mute: false, self_deaf: false, mute: false, deaf: false, ..Default::default() }]);
            }),
        ],
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("Fluxer", (1100.0, 700.0), State::default, ui::view)
    };
    rune::run(spec);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxer_client::api::Attachment;
    use rune::{Env, Runtime};
    use serde_json::json;
    use std::time::Instant;

    /// Keeps tests away from the real saved session (`sign_out` deletes it).
    fn isolate() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let d = std::env::temp_dir().join(format!("fluxer-app-test-{}", std::process::id()));
            // SAFETY: runs once, before any test reads the variable.
            unsafe { std::env::set_var("FLUXER_RUX_CONFIG_DIR", d) };
        });
    }

    fn dispatch(json: &str) -> Event { Event::Payload(Payload::decode(json).unwrap()) }

    fn rt() -> Runtime<State> {
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(State::default(), ui::view, (1100.0, 700.0), env);
        rt.frame(0.0);
        rt
    }

    fn ready_event() -> Event {
        dispatch(r#"{"op":0,"s":1,"t":"READY","d":{"session_id":"x","user":{"id":"1","username":"eu"},
            "guilds":[{"id":"g","properties":{"name":"Casa"},"channels":[{"id":"c1","name":"geral","type":0},{"id":"c2","name":"dev","type":0}]}],
            "private_channels":[{"id":"d1","type":1,"recipients":[{"id":"2","username":"ana"}],"last_message_id":"5"}]}}"#)
    }

    fn msg_event(id: &str, channel: &str, text: &str) -> Event {
        dispatch(&format!(r#"{{"op":0,"t":"MESSAGE_CREATE","d":{{"id":"{id}","channel_id":"{channel}","content":"{text}","author":{{"id":"2","username":"ana"}}}}}}"#))
    }

    /// CPU time of this thread, so tests running beside this one don't count against it.
    fn thread_cpu() -> std::time::Duration {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: a valid timespec is handed to clock_gettime, which fills it in.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
        std::time::Duration::new(ts.tv_sec.max(0) as u64, ts.tv_nsec.max(0) as u32)
    }

    #[test]
    fn ten_thousand_messages_lay_out_only_the_visible_rows() {
        let mut rt = rt();
        let t = thread_cpu();
        for _ in 0..120 { rt.frame(1.0 / 60.0); }
        let per_frame = (thread_cpu() - t).as_secs_f64() * 1000.0 / 120.0;
        eprintln!("idle frame cpu: {per_frame:.2} ms");
        assert!(rt.find("9999").is_some(), "stick_bottom opens on the newest message");
        assert!(rt.find("0").is_none(), "rows far from the viewport are not built");
        assert!(per_frame < 8.0, "{per_frame} ms per frame");
    }

    #[test]
    fn demo_send_appears_at_once_and_clears_the_draft() {
        let mut s = State::default();
        s.draft = TextEdit::new("oi  ");
        s.draft.focused = true;
        send(&mut s);
        let log = s.chat.log().unwrap();
        assert_eq!((log.msgs.last().unwrap().body.as_str(), &log.msgs.last().unwrap().delivery), ("oi", &Delivery::Sent));
        assert!(s.draft.text.is_empty());
        assert!(s.draft.focused, "the box keeps the caret for the next message");
        send(&mut s);
        assert_eq!(s.chat.log().unwrap().msgs.len(), 10_001, "an empty draft sends nothing");
    }

    #[test]
    fn a_reply_carries_its_quote_and_an_edit_rewrites_in_place() {
        let mut s = State::default();
        let quoted = fluxer_client::api::Quoted { id: Some("5".into()), author: s.chat.me.clone().unwrap(), content: "original".into(), has_attachments: false };
        s.draft = TextEdit::new("respondendo");
        s.draft_mode.reply = Some(quoted);
        send(&mut s);
        let last = s.chat.log().unwrap().msgs.last().unwrap().clone();
        assert_eq!(last.body, "respondendo");
        assert_eq!(last.reply.and_then(|r| r.id).as_deref(), Some("5"), "the reply keeps what it answers");
        assert!(s.draft_mode.reply.is_none());

        let id = last.id;
        s.draft = TextEdit::new("editada");
        s.draft_mode.editing = Some(id.clone());
        send(&mut s);
        assert_eq!(s.chat.log().unwrap().msgs.iter().find(|m| m.id == id).unwrap().body, "editada");
        assert!(s.draft_mode.editing.is_none());
    }

    #[test]
    fn ready_then_live_messages_fill_the_chat_and_the_view() {
        let mut rt = rt();
        rt.update(|s, _| {
            apply(s, ready_event());
            apply(s, msg_event("10", "c1", "ao vivo"));
            apply(s, msg_event("11", "c2", "noutro canal"));
        });
        for _ in 0..30 { rt.frame(1.0 / 60.0); }
        assert_eq!(rt.state.chat.guilds[0].name, "Casa");
        assert!(rt.find("10").is_some(), "the open channel shows its message");
        assert!(rt.find("11").is_none());
        assert_eq!(rt.state.chat.unread(Some(0)), 1);
    }

    #[test]
    fn switching_between_servers_dms_and_channels_changes_what_is_shown() {
        let mut rt = rt();
        rt.update(|s, _| {
            apply(s, ready_event());
            apply(s, msg_event("10", "c1", "geral"));
            apply(s, msg_event("11", "c2", "dev"));
            apply(s, msg_event("12", "d1", "privado"));
        });
        for _ in 0..10 { rt.frame(1.0 / 60.0); }
        assert_eq!(rt.state.chat.unread(None), 1, "the DM counts as unread");
        let (x, y) = rt.find("ch-1").unwrap();
        rt.pointer_down(x, y); rt.pointer_up(x, y);
        for _ in 0..10 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("11").is_some() && rt.find("10").is_none());
        let (x, y) = rt.find("rail-dm").unwrap();
        rt.pointer_down(x, y); rt.pointer_up(x, y);
        for _ in 0..10 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("12").is_some(), "the DM opens");
        assert_eq!(rt.state.chat.unread(None), 0);
    }

    #[test]
    fn history_and_image_requests_wait_for_an_api_and_are_made_once() {
        let mut s = State::default();
        apply(&mut s, ready_event());
        reconcile(&mut s);
        assert!(!s.chat.log().is_some_and(|l| l.loading), "no api, no request");
        // With an api (pointing nowhere) the open channel is marked loading exactly once.
        s.net.set_api(Some(Arc::new(Api::new(fluxer_client::rest::Endpoints {
            api_client: "http://127.0.0.1:9".into(), gateway: "wss://g".into(), media: None, origin: None }, "t"))));
        s.net.set_waker(Arc::new(|| {}));
        reconcile(&mut s);
        assert!(s.chat.log().unwrap().loading);
        s.chat.on_history_failed("c1");
        assert!(s.chat.log().unwrap().loaded == false);
    }

    #[test]
    fn a_pending_message_is_shown_dimmed_and_a_failed_one_offers_a_retry() {
        let mut rt = rt();
        rt.update(|s, _| {
            apply(s, ready_event());
            s.chat.push_pending("c1", "tentando".into());
            let id = s.chat.push_pending("c1", "quebrou".into()).unwrap();
            s.chat.on_sent("c1", &id, Err("Sem permissão".into()));
        });
        for _ in 0..10 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("local-1").is_some());
        assert!(rt.find("retry-local-2").is_some());
    }

    #[test]
    fn image_attachments_show_a_placeholder_and_other_files_a_chip() {
        let mut rt = rt();
        rt.update(|s, _| {
            apply(s, ready_event());
            apply(s, dispatch(r#"{"op":0,"t":"MESSAGE_CREATE","d":{"id":"20","channel_id":"c1","content":"","author":{"id":"2","username":"ana"},
                "attachments":[{"id":"a1","filename":"foto.png","content_type":"image/png","url":"https://m/foto.png","width":1200,"height":800,"size":5},
                               {"id":"a2","filename":"nota.pdf","content_type":"application/pdf","url":"https://m/nota.pdf","size":2048}]}}"#));
        });
        for _ in 0..10 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("att-a1").is_some() && rt.find("att-a2").is_some());
        let _ = Attachment { id: String::new(), filename: String::new(), content_type: None, url: None, width: None, height: None, size: 0 };
    }

    #[test]
    fn two_factor_moves_to_the_code_step_and_back() {
        isolate();
        let mut s = State::default();
        s.screen = Screen::Login;
        on_auth(&mut s, auth::Msg::Mfa {
            instance: "https://i".into(),
            ep: fluxer_client::rest::Endpoints { api_client: "https://a".into(), gateway: "wss://g".into(), media: None, origin: None },
            ticket: "tk".into(),
        });
        assert!(matches!(&s.auth.step, auth::Step::Code { ticket } if ticket == "tk"));
        s.auth.back_to_credentials();
        assert!(matches!(s.auth.step, auth::Step::Credentials));
    }

    #[test]
    fn failures_are_shown_and_empty_forms_are_not_submitted() {
        isolate();
        let mut s = State::default();
        s.screen = Screen::Login;
        assert!(!s.auth.submit());
        assert!(s.auth.error.as_deref().is_some_and(|e| e.contains("Preencha")));
        on_auth(&mut s, auth::Msg::Failed("Senha inválida".into()));
        assert_eq!(s.auth.error.as_deref(), Some("Senha inválida"));
    }

    #[test]
    fn a_refused_session_returns_to_login_and_forgets_the_token() {
        isolate();
        let mut s = State::default();
        s.session = Some(store::Saved { instance: "https://i".into(), gateway: "wss://g".into(), api_client: "https://a".into(), media: None,
            origin: None, token: "t".into(), user_id: "1".into(), username: None });
        s.status = Status::Failed;
        assert!(poll(&mut s, 0.0, &mut Tx::default()));
        assert!(s.screen == Screen::Login && s.session.is_none());
        assert!(s.auth.error.as_deref().is_some_and(|e| e.contains("sessão")));
    }

    #[test]
    fn the_voice_signal_explains_the_ping_on_hover() {
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(State::default(), ui::view, (1100.0, 700.0), env);
        rt.update(|s, _| {
            s.call.phase = crate::call::Phase::Connected;
            s.call.channel_id = Some("demo-v0".into());
            s.call.ping = Some(42);
            s.call.ping_tip = true;
        });
        rt.frame(0.0);
        assert!(rt.find("voice-signal").is_some());
        assert!(rt.find("voice-ping").is_some(), "hovering the signal shows the round-trip time");
        rt.update(|s, _| s.call.ping_tip = false);
        rt.frame(0.0);
        assert!(rt.find("voice-ping").is_none(), "the tip goes with the pointer");
    }

    #[test]
    fn the_settings_dialog_lists_the_audio_devices() {
        isolate();
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(State::default(), ui::view, (1100.0, 700.0), env);
        rt.update(|s, _| {
            s.chat.me = Some(fluxer_client::api::User { id: "1".into(), username: "eu".into(), global_name: None, avatar: None, discriminator: None });
            s.settings_open = true;
            s.pf.page = crate::ui::settings::Page::Voice;
            s.call.devices = Some(crate::audio::Devices {
                mics: vec![crate::audio::Device { id: "m1".into(), label: "Microfone".into() }],
                speakers: vec![crate::audio::Device { id: "s1".into(), label: "Alto-falantes".into() }],
            });
        });
        rt.frame(0.0);
        assert!(rt.find("set-page-voice").is_some(), "the sidebar has the audio page");
        assert!(rt.find("set-mic-default").is_some(), "microphone choices are listed");
        assert!(rt.find("set-mic-0").is_some());
        assert!(rt.find("set-spk-default").is_some(), "speaker choices are listed");
        assert!(rt.find("set-spk-0").is_some());
        assert!(rt.find("set-in-vol").is_some(), "the voice page has the input volume");
        assert!(rt.find("set-out-vol").is_some());
        assert!(rt.find("set-denoise").is_some());
        assert!(rt.find("set-audio-refresh").is_some());
    }

    #[test]
    fn a_message_menu_offers_reply_and_the_author_s_edit_and_delete() {
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(State::default(), ui::view, (1100.0, 700.0), env);
        rt.update(|s, _| {
            let me = s.chat.me.clone().expect("demo user");
            let m = Msg::local("mine".into(), me, "oi".into(), Delivery::Sent);
            s.msg_menu = Some(crate::MsgMenu { msg: m, x: 200.0, y: 200.0 });
        });
        rt.frame(0.0);
        assert!(rt.find("msg-menu").is_some());
        assert!(rt.find("mm-reply").is_some() && rt.find("mm-copy").is_some());
        assert!(rt.find("mm-edit").is_some() && rt.find("mm-delete").is_some(), "own messages can be changed");
    }

    #[test]
    fn the_channel_search_has_a_box_and_the_matching_messages() {
        let mut rt = rt();
        rt.update(|s, _| {
            s.search.open = true;
            let me = s.chat.me.clone().expect("demo user");
            s.search.results = vec![fluxer_client::api::Message::plain("found", "demo-0", "achado", me)];
        });
        rt.frame(0.0);
        assert!(rt.find("search-input").is_some());
        assert!(rt.find("search-go").is_some());
        assert!(rt.find("sr-found").is_some());
    }

    #[test]
    fn the_members_panel_has_a_name_filter() {
        let mut rt = rt();
        rt.update(|s, _| s.show_members = true);
        rt.frame(0.0);
        assert!(rt.find("member-search").is_some());
    }

    #[test]
    fn the_login_screen_renders_its_fields_and_the_error() {
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(State::default(), ui::view, (960.0, 640.0), env);
        rt.update(|s, _| { s.screen = Screen::Login; s.auth.error = Some("Senha inválida".into()); });
        for _ in 0..5 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("login-error").is_some());
        assert!(rt.find("composer").is_none(), "the chat is not mounted behind the login");
    }

    #[test]
    fn an_ip_authorization_request_waits_instead_of_asking_to_log_in_again() {
        isolate();
        let mut s = State::default();
        s.screen = Screen::Login;
        s.auth.password = TextEdit::new("segredo");
        on_auth(&mut s, auth::Msg::Waiting {
            instance: "https://i".into(),
            ep: fluxer_client::rest::Endpoints { api_client: "http://127.0.0.1:9".into(), gateway: "wss://g".into(), media: None, origin: None },
            ticket: "tk".into(), email: "a@b.c".into(), resend_in: 30,
        });
        assert!(matches!(&s.auth.step, auth::Step::Waiting { email, .. } if email == "a@b.c"));
        assert_eq!(s.auth.password.text, "segredo", "the typed credentials are kept");
        on_auth(&mut s, auth::Msg::Failed("Perdi a conexão".into()));
        assert!(matches!(s.auth.step, auth::Step::Credentials));
        assert_eq!(s.auth.error.as_deref(), Some("Perdi a conexão"));
    }

    #[test]
    fn the_waiting_screen_shows_the_address() {
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(State::default(), ui::view, (960.0, 640.0), env);
        rt.update(|s, _| {
            s.screen = Screen::Login;
            s.auth.step = auth::Step::Waiting { ticket: "t".into(), email: "a@b.c".into(), resend_ready_at: Instant::now() };
        });
        for _ in 0..5 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("ip-waiting").is_some());
    }

    #[test]
    fn a_failed_connection_shows_as_reconnecting_not_as_connecting_forever() {
        let mut s = State::default();
        s.status = Status::Connecting;
        assert!(apply(&mut s, Event::ConnectFailed("HTTP error: 400 Bad Request".into())));
        assert_eq!(s.status, Status::Reconnecting);
        apply(&mut s, ready_event());
        assert_eq!(s.status, Status::Connected);
    }

    #[test]
    fn the_call_mosaic_offers_a_stream_and_watching_it_is_tracked() {
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(State::default(), ui::view, (1200.0, 800.0), env);
        rt.update(|s, _| demo_call(s));
        for _ in 0..5 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("ms-watch-c1").is_some(), "a stream is offered, not auto-played");
        assert!(rt.find("mt-2").is_some() && rt.find("mt-3").is_some(), "everyone has a tile");
        assert!(rt.find("composer").is_none(), "the mosaic replaces the text chat");
        rt.update(|s, _| watch_stream(s, "c1", true));
        for _ in 0..5 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("ms-stop-c1").is_some() && rt.find("ms-watch-c1").is_none());
        assert_eq!(rt.state.call.watching, vec!["demo:demo-v0:c1".to_string()]);
        rt.update(|s, _| watch_stream(s, "c1", false));
        assert!(rt.state.call.watching.is_empty());
    }

    /// Serves `body` to every request, forever, and returns the address.
    fn serve_forever(body: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || while let Ok((mut c, _)) = l.accept() {
            let mut buf = [0u8; 2048];
            let _ = c.read(&mut buf);
            let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = c.write_all(&body);
        });
        base
    }

    #[test]
    fn a_gif_on_screen_plays_by_streaming_and_one_off_screen_does_not() {
        isolate();
        unsafe { std::env::set_var("FLUXER_RUX_CACHE_DIR", std::env::temp_dir().join(format!("fluxer-anim-cache-{}", std::process::id()))) };
        let base = serve_forever(anim::test_gif(60, 300, 200));
        let mut s = State::default();
        s.chat.media = Some(base.clone());
        let gif_at = |id: &str| fluxer_client::api::Attachment { id: id.into(), filename: "a.gif".into(), content_type: Some("image/gif".into()),
            url: Some(format!("{base}/{id}.gif")), width: Some(300), height: Some(200), size: 1 };
        // The newest message (on screen, the list sticks to the bottom) and one far up (not on screen).
        let log = s.chat.logs.get_mut("demo-0").unwrap();
        log.msgs[9_999].attachments = vec![gif_at("near")];
        log.msgs[2_000].attachments = vec![gif_at("far")];
        let animated = |id: &str| model::attachment_fetch_url(&format!("{base}/{id}.gif"), true);
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(s, ui::view, (1100.0, 700.0), env);
        rt.update(|s, _| s.net.set_waker(Arc::new(|| {})));
        let started = Instant::now();
        while rt.state.net.shown(&animated("near")).is_none() {
            rt.update(|s, tx| { poll(s, 0.0, tx); });
            rt.frame(1.0 / 60.0);
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "the visible GIF never showed a frame");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for _ in 0..30 { rt.update(|s, tx| { poll(s, 0.0, tx); }); rt.frame(1.0 / 60.0); }
        assert!(rt.state.net.shown(&animated("far")).is_none(), "an animation off screen is never asked for, let alone decoded");
        assert_eq!(rt.state.net.playing_count(), 1, "the one on screen plays");
        // 60 frames of 300x200 would be 60 * 320 KB * 4/3 decoded by the cache; streaming holds the bytes and a few frames.
        assert!(rt.state.net.held_bytes() < 2 << 20, "held {} bytes", rt.state.net.held_bytes());
        rt.update(|s, tx| { set_focus(s, false); poll(s, 0.0, tx); });
        assert_eq!(rt.state.net.playing_count(), 0, "a window in the background pauses its animations");
    }

    #[test]
    fn every_gif_tile_of_the_open_picker_plays_not_only_the_one_under_the_pointer() {
        isolate();
        unsafe { std::env::set_var("FLUXER_RUX_CACHE_DIR", std::env::temp_dir().join(format!("fluxer-picker-cache-{}", std::process::id()))) };
        let base = serve_forever(anim::test_gif(20, 200, 150));
        let mut s = State::default();
        s.picker.open = Some(picker::Tab::Gifs);
        s.picker.view = picker::GifView::Trending;
        s.picker.trending = (0..6).map(|i| picker::Gif { id: i.to_string(), url: format!("https://k/{i}"), src: format!("{base}/still{i}.png"),
            width: 200, height: 150, anim: Some(format!("{base}/anim{i}.gif")) }).collect();
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(s, ui::view, (1100.0, 900.0), env);
        rt.update(|s, _| s.net.set_waker(Arc::new(|| {})));
        let started = Instant::now();
        while rt.state.net.playing_count() < 6 {
            rt.update(|s, tx| { poll(s, 0.0, tx); });
            rt.frame(1.0 / 60.0);
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "only {} of 6 tiles play", rt.state.net.playing_count());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        rt.update(|s, tx| { s.picker.close(); crate::reconcile(s); poll(s, 0.0, tx); });
        for _ in 0..3 { rt.frame(1.0 / 60.0); }
        assert_eq!(rt.state.net.playing_count(), 0, "closing the picker stops what it played");
    }

    #[test]
    fn dragging_the_scroll_thumb_scrolls_the_message_list() {
        let mut rt = rt();
        for _ in 0..5 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("9999").is_some() && rt.find("0").is_none(), "the list opens at its newest message");
        // The thumb sits at the end of its track at the right edge of the list; pull it to the top.
        let (x, y) = rt.find("msg-scroll-track").expect("the track is mounted");
        rt.pointer_move(x, y);
        for _ in 0..3 { rt.frame(1.0 / 60.0); }
        rt.pointer_down(x, y);
        rt.pointer_move(x, 0.0);
        rt.pointer_up(x, 0.0);
        for _ in 0..30 { rt.frame(1.0 / 60.0); }
        assert!(rt.find("0").is_some(), "dragged to the top, the oldest message is shown");
        assert!(rt.find("9999").is_none());
    }

    #[test]
    fn the_scroll_thumb_is_mid_width_while_scrolling_wide_near_the_edge_and_gone_after_the_hold() {
        let (rest, shown, near) = ui::thumb::WIDTHS;
        let mut rt = rt();
        for _ in 0..5 { rt.frame(1.0 / 60.0); }
        assert_eq!(rt.state.thumb.look(), (0.0, 0.0), "nothing shows before anything happens");
        // The wheel over the middle of the list: shown at its middle width.
        rt.wheel(500.0, 300.0, 120.0);
        for _ in 0..5 { rt.frame(1.0 / 60.0); }
        assert_eq!(rt.state.thumb.look(), (shown, 1.0), "scrolling shows the thumb at its middle width");
        // The pointer near the right edge: wide.
        let (x, y) = rt.find("msg-scroll-track").unwrap();
        rt.pointer_move(x, y);
        for _ in 0..3 { rt.frame(1.0 / 60.0); }
        assert_eq!(rt.state.thumb.look(), (near, 1.0), "near the edge it is wide");
        // Away from it, and then nothing for longer than the hold: narrow and gone.
        rt.pointer_move(500.0, 300.0);
        for _ in 0..3 { rt.frame(1.0 / 60.0); }
        assert_eq!(rt.state.thumb.look().0, shown, "leaving the edge brings it back to its middle width");
        std::thread::sleep(ui::thumb::HOLD + std::time::Duration::from_millis(400));
        assert_eq!(rt.state.thumb.look(), (rest, 0.0), "left alone it narrows and fades out");
    }

    #[test]
    fn json_helpers() {
        assert_eq!(str_at(&json!({"a": {"b": "c"}}), &["a", "b"]), Some("c"));
    }

    #[test]
    fn a_jump_flash_fades_after_three_seconds() {
        let mut s = State::default();
        s.highlighted = Some(("m3".into(), Instant::now() - std::time::Duration::from_secs(1)));
        poll(&mut s, 0.0, &mut rune::Tx::default());
        assert!(s.highlighted.is_none(), "an expired flash clears on its own");
        s.highlighted = Some(("m3".into(), Instant::now() + std::time::Duration::from_secs(3)));
        poll(&mut s, 0.0, &mut rune::Tx::default());
        assert!(s.highlighted.is_some(), "a fresh flash survives");
    }
}

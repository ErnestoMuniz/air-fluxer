//! Fluxer on Rune: sign-in, guilds and DMs, live messages, history, sending, avatars and images.
//! `cargo run --release -p fluxer-app` opens the window (`FLUXER_DEMO=1` for fake data);
//! `-- --snap out.png [--scene login|code|ip]` renders it headless.

mod auth;
mod clock;
mod gateway;
mod model;
mod net;
mod richtext;
mod roster;
mod store;
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

pub struct State {
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
}

fn demo_chat() -> Chat {
    let people: Vec<User> = ["ana", "bruno", "carla", "dudu"].iter().enumerate()
        .map(|(i, n)| User { id: (i + 1).to_string(), username: (*n).into(), global_name: None, avatar: None }).collect();
    let mut c = Chat::default();
    c.me = Some(User { id: "0".into(), username: "você".into(), global_name: None, avatar: None });
    let chan = |i: usize, n: &str| Channel { id: format!("demo-{i}"), name: n.into(), category: Some(if i < 2 { "Texto" } else { "Comunidade" }.into()),
        new: i == 1, last_read: Some("9990".into()), ..Channel::default() };
    c.guilds = vec![Guild {
        id: "demo".into(), name: "Demo".into(), icon: None,
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
        if i % 7 == 3 { m.reply = Some(Quoted { author: people[(i + 1) % 4].clone(), content: "mensagem original citada".into(), has_attachments: false }) }
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
            screen: Screen::Main, auth: auth::Auth::default(), session: None, wake: None, status: Status::Demo,
            chat: demo_chat(), net: net::Net::default(), draft: TextEdit::new(""), gateway: None, subscribed: None, synced_guilds: Default::default(), show_members: true,
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
                true
            }
            "RESUMED" => { s.status = Status::Connected; true }
            "TYPING_START" => match (str_at(&d, &["channel_id"]), str_at(&d, &["user_id"])) {
                (Some(c), Some(u)) => {
                    s.chat.roster.on_typing(c, u, clock::now_ms());
                    // The notice fades by itself; wake Rune once it is due.
                    if let Some(w) = s.wake.clone() {
                        std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(roster::TYPING_TTL_MS as u64 + 250)); w() });
                    }
                    true
                }
                _ => false,
            },
            "PRESENCE_UPDATE" => { s.chat.roster.on_presence(&d); true }
            "GUILD_MEMBER_LIST_UPDATE" => { s.chat.roster.on_member_list(&d); true }
            "MESSAGE_ACK" => match (str_at(&d, &["channel_id"]), str_at(&d, &["message_id"])) {
                (Some(c), Some(m)) => s.chat.on_ack(c, m),
                _ => false,
            },
            "MESSAGE_CREATE" => Message::from_json(&d).map(|m| s.chat.on_message(m)).is_some(),
            "MESSAGE_UPDATE" => Message::from_json(&d).is_some_and(|m| s.chat.on_message_updated(m)),
            "MESSAGE_DELETE" => match (str_at(&d, &["channel_id"]), str_at(&d, &["id"])) {
                (Some(c), Some(id)) => s.chat.on_message_deleted(c, id),
                _ => false,
            },
            _ => false,
        },
        Event::Payload(_) => false,
    }
}

fn setup(s: &mut State, wake: Waker) {
    s.auth.set_waker(wake.clone());
    s.net.set_waker(wake.clone());
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
    s.chat = Chat { media: saved.media.clone(), ..Chat::default() };
    s.net.set_api(Some(api_for(&saved)));
    s.net.refresh_endpoints(&saved.instance);
    s.gateway = Some(gateway::start(gateway::Settings { gateway: saved.gateway.clone(), token: saved.token.clone() }, wake));
    s.session = Some(saved);
}

/// Back to the login screen, keeping the instance the user typed.
pub fn sign_out(s: &mut State, reason: Option<&str>) {
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
        net::Done::Image { .. } => false,
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
    for url in s.chat.wanted_images(80) { s.net.want_image(&url) }
    subscribe_members(s);
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

fn poll(s: &mut State, _t: f64, _tx: &mut Tx) -> bool {
    let mut changed = false;
    for m in s.auth.drain() { on_auth(s, m); changed = true }
    let mut events = Vec::new();
    if let Some(g) = &s.gateway { events.extend(g.inbox.try_iter()) }
    changed |= events.into_iter().fold(false, |c, e| apply(s, e) | c);
    let images_before = s.net.busy_with_images();
    for d in s.net.drain() { changed |= on_net(s, d) }
    // A finished download changes what draws; so does a queue that just emptied.
    changed |= images_before || s.net.busy_with_images();
    if s.status == Status::Failed && s.session.is_some() {
        sign_out(s, Some("A sessão expirou ou foi recusada. Entre de novo."));
        changed = true;
    }
    if s.screen == Screen::Main { reconcile(s) }
    changed
}

pub fn submit(s: &mut State) { s.auth.submit(); }

pub fn open_guild(s: &mut State, g: Option<usize>) {
    s.chat.open_guild(g);
    reconcile(s);
}

pub fn open_channel(s: &mut State, i: usize) {
    s.chat.open_channel(i);
    reconcile(s);
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
    let Some(cid) = s.chat.channel_id().map(String::from) else { return };
    if body.is_empty() { return }
    if s.net.has_api() {
        if let Some(local) = s.chat.push_pending(&cid, body.clone()) { s.net.send(&cid, &local, &body) }
    } else if let Some(local) = s.chat.push_pending(&cid, body) {
        s.chat.on_sent(&cid, &local, Ok(Message::plain(format!("sent-{local}"), cid.clone(), s.chat.logs[&cid].msgs.last().map(|m| m.body.clone()).unwrap_or_default(),
            s.chat.me.clone().expect("demo user"))));
    }
    s.draft = TextEdit::new("");
}

fn main() {
    rune::run(AppSpec::<State> {
        scenes: &[
            ("login", |s| s.screen = Screen::Login),
            ("ip", |s| {
                s.screen = Screen::Login;
                s.auth.step = auth::Step::Waiting { ticket: "t".into(), email: "voce@exemplo.com".into(), resend_ready_at: std::time::Instant::now() };
            }),
            ("code", |s| {
                s.screen = Screen::Login;
                s.auth.step = auth::Step::Code { ticket: "t".into() };
                s.auth.error = Some("Código inválido.".into());
            }),
            ("dm", |s| { s.chat.open_guild(None); }),
        ],
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("Fluxer", (1100.0, 700.0), State::default, ui::view)
    });
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

    #[test]
    fn ten_thousand_messages_lay_out_only_the_visible_rows() {
        let mut rt = rt();
        let t = Instant::now();
        for _ in 0..120 { rt.frame(1.0 / 60.0); }
        let per_frame = t.elapsed().as_secs_f64() * 1000.0 / 120.0;
        eprintln!("idle frame cpu: {per_frame:.2} ms");
        assert!(rt.find("9999").is_some(), "stick_bottom opens on the newest message");
        assert!(rt.find("0").is_none(), "rows far from the viewport are not built");
        assert!(per_frame < 8.0, "{per_frame} ms per frame");
    }

    #[test]
    fn demo_send_appears_at_once_and_clears_the_draft() {
        let mut s = State::default();
        s.draft = TextEdit::new("oi  ");
        send(&mut s);
        let log = s.chat.log().unwrap();
        assert_eq!((log.msgs.last().unwrap().body.as_str(), &log.msgs.last().unwrap().delivery), ("oi", &Delivery::Sent));
        assert!(s.draft.text.is_empty());
        send(&mut s);
        assert_eq!(s.chat.log().unwrap().msgs.len(), 10_001, "an empty draft sends nothing");
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
    fn json_helpers() {
        assert_eq!(str_at(&json!({"a": {"b": "c"}}), &["a", "b"]), Some("c"));
    }
}

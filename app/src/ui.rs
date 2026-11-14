//! Views: login, the guild rail, the channel list, the virtualized message list and the composer.

use crate::model::{Channel, Delivery, Msg};
use crate::{State, auth, set_focus, sign_out, submit, send, join_voice, leave_voice, toggle_mute, toggle_deafen, open_guild, open_channel, open_voice_chat, toggle_reaction, retry, load_older, jump_to};
use fluxer_client::api::{self, Attachment, User};
use rune::tok::{self, radius, type_};
use rune::{Color, Cx, El, GrowthSpec, ScrollSpec, col, el, oklch, oklcha, row, text, text_input};
use fluxer_client::api::{Embed, Quoted, snowflake_cmp};
use crate::roster::{self, Row};
use rune_kit::{Button, ButtonVariant, Kit, Markdown, MarkdownStyle, TextArea, TextInput};

mod compose;
mod mosaic;
pub mod settings;

const RAIL_WIDTH: f32 = 72.0;

fn list_w(s: &State) -> f32 { s.prefs.list_w.clamp(220, 420) as f32 }
fn members_w(s: &State) -> f32 { s.prefs.members_w.clamp(200, 420) as f32 }
/// The floating bar spans the rail and the channel list, less its margins.
/// The banner keeps its 248 x 140 proportions however wide the list is dragged.
fn banner_h(s: &State) -> f32 { list_w(s) * 140.0 / 248.0 }
fn dock_w(s: &State) -> f32 { RAIL_WIDTH + list_w(s) - 16.0 }

/// A thin strip on a panel's edge that resizes it by dragging. `grow_right`: dragging right widens.
fn resize_handle(key: &str, grow_right: bool, current: f32, set: fn(&mut State, f32)) -> El {
    el().key(key.to_string()).w(8.0).h_full().cursor(rune::Cursor::ResizeColumn)
        .on_drag(move |s: &mut State, d: &mut rune::Drag, _| {
            if matches!(d.phase, rune::Phase::Down) { d.data = current }
            let w = if grow_right { d.data + d.dx } else { d.data - d.dx };
            set(s, w);
            if matches!(d.phase, rune::Phase::Up) { crate::prefs::save(&s.prefs) }
        })
}


const STROKE: &str = r#"stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" fill="none""#;

static SPEAKER: rune::Icon = rune::Icon { name: "app-speaker", view: 24.0, body: r#"<path d="M4 9v6h4l5 4V5L8 9z" fill="currentColor"/><path d="M16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"# };
static MIC: rune::Icon = rune::Icon { name: "app-mic", view: 24.0, body: r#"<rect x="9" y="3" width="6" height="12" rx="3" fill="currentColor"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"# };
static MIC_OFF: rune::Icon = rune::Icon { name: "app-mic-off", view: 24.0, body: r#"<rect x="9" y="3" width="6" height="12" rx="3" fill="currentColor"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M4 4l16 16" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" fill="none"/>"# };
static HEADSET: rune::Icon = rune::Icon { name: "app-headset", view: 24.0, body: r#"<path d="M4 15v-3a8 8 0 0 1 16 0v3" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/><rect x="3" y="13" width="4.5" height="7" rx="2" fill="currentColor"/><rect x="16.5" y="13" width="4.5" height="7" rx="2" fill="currentColor"/>"# };
static HEADSET_OFF: rune::Icon = rune::Icon { name: "app-headset-off", view: 24.0, body: r#"<path d="M4 15v-3a8 8 0 0 1 16 0v3" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/><rect x="3" y="13" width="4.5" height="7" rx="2" fill="currentColor"/><rect x="16.5" y="13" width="4.5" height="7" rx="2" fill="currentColor"/><path d="M4 4l16 16" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>"# };
static GEAR: rune::Icon = rune::Icon { name: "app-gear", view: 24.0, body: r#"<circle cx="12" cy="12" r="3" stroke="currentColor" stroke-width="2" fill="none"/><circle cx="12" cy="12" r="7" stroke="currentColor" stroke-width="2" fill="none"/><path d="M12 2.5v2.5M12 19v2.5M2.5 12H5M19 12h2.5M5.3 5.3l1.8 1.8M16.9 16.9l1.8 1.8M5.3 18.7l1.8-1.8M16.9 7.1l1.8-1.8" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>"# };
static SCREEN: rune::Icon = rune::Icon { name: "app-screen", view: 24.0, body: r#"<rect x="3" y="4" width="18" height="12" rx="2" stroke="currentColor" stroke-width="2" fill="none"/><path d="M8 20h8M12 16v4" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"# };
static EYE: rune::Icon = rune::Icon { name: "app-eye", view: 24.0, body: r#"<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" fill="currentColor"/><circle cx="12" cy="12" r="3" fill="black"/>"# };
static GRID: rune::Icon = rune::Icon { name: "app-grid", view: 24.0, body: r#"<rect x="4" y="4" width="7" height="7" rx="1.5" fill="currentColor"/><rect x="13" y="4" width="7" height="7" rx="1.5" fill="currentColor"/><rect x="4" y="13" width="7" height="7" rx="1.5" fill="currentColor"/><rect x="13" y="13" width="7" height="7" rx="1.5" fill="currentColor"/>"# };
static EXPAND: rune::Icon = rune::Icon { name: "app-expand", view: 24.0, body: r#"<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static CLOSE: rune::Icon = rune::Icon { name: "app-close", view: 24.0, body: r#"<path d="M6 6l12 12M18 6L6 18" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" fill="none"/>"# };
static PLAY: rune::Icon = rune::Icon { name: "app-play", view: 24.0, body: r#"<path d="M7 4.5v15l13-7.5z" fill="currentColor"/>"# };
static HANG_UP: rune::Icon = rune::Icon { name: "app-hang-up", view: 24.0, body: r#"<path d="M3 14.5c5-5 13-5 18 0l-2.5 3-3-1.5v-2.5c-2-.7-4-.7-6 0V16l-3 1.5z" fill="currentColor"/>"# };

/// The mark beside a server owner's name.
static CROWN: rune::Icon = rune::Icon {
    name: "app-crown",
    view: 256.0,
    body: r#"<path d="M24 80l56 48 48-80 48 80 56-48-24 120H48z" fill="currentColor" stroke="currentColor" stroke-width="16" stroke-linejoin="round"/>"#,
};

/// The speech bubble of the direct messages button (Phosphor "chat circle", filled).
static CHAT_CIRCLE: rune::Icon = rune::Icon {
    name: "app-chat-circle",
    view: 256.0,
    body: r#"<path d="M128,24A104,104,0,0,0,36.18,176.88L24.83,210.93a16,16,0,0,0,20.24,20.24l34.05-11.35A104,104,0,1,0,128,24Z" fill="currentColor"/>"#,
};

/// Surfaces, darkest to lightest: rail, side panel, chat. Everything else comes from the Rune theme.
struct Pal { link: Color, rail: Color, side: Color, chat: Color, input: Color, line: Color, select: Color, hover: Color, brand: Color, unread: Color }

fn pal() -> Pal {
    Pal {
        link: oklch(0.74, 0.14, 245.0),
        rail: oklch(0.155, 0.006, 265.0),
        side: oklch(0.19, 0.006, 265.0),
        chat: oklch(0.225, 0.006, 265.0),
        input: oklch(0.275, 0.008, 265.0),
        line: oklcha(1.0, 0.0, 0.0, 0.07),
        select: oklcha(1.0, 0.0, 0.0, 0.055),
        hover: oklcha(1.0, 0.0, 0.0, 0.03),
        brand: tok::accent(0, true),
        unread: oklch(0.64, 0.2, 25.0),
    }
}
const ATTACHMENT_MAX: (f32, f32) = (360.0, 280.0);

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    match s.screen { crate::Screen::Login => login_view(s), crate::Screen::Main => main_view(s, cx.layout_size("mosaic-stage")) }
}

fn login_view(s: &State) -> El {
    let t = tok::theme(true);
    let k = &Kit::neutral(true);
    let a = &s.auth;
    const W: f32 = 430.0;
    let panel = oklch(0.2, 0.01, 300.0);
    let line = oklcha(1.0, 0.0, 0.0, 0.1);
    let labelled = |label: &str, input: El| col().gap(8.0)
        .child(text(label).font(type_::LABEL).color(t.text)).child(input);
    let title = |s: &str| row().w(W).center().child(text(s.to_string()).font(rune::tok::TextStyle::new(20.0, 600.0)).color(t.text));
    let in_code_step = matches!(a.step, auth::Step::Code { .. });
    let waiting = if let auth::Step::Waiting { email, .. } = &a.step { Some(email.clone()) } else { None };

    let mut form = col().gap(20.0).w(W);
    if let Some(email) = &waiting {
        form = form
            .child(title("Check your email"))
            .child(text(format!("We sent an authorization link to {email}. Once you authorize it, Fluxer signs you in automatically."))
                .key("ip-waiting").font(type_::BODY).color(t.text_secondary).wrap().w(W))
            .child(text("Waiting for authorization…").font(type_::LABEL).color(tok::accent(0, true)));
    } else if in_code_step {
        form = form
            .child(title("Two-factor authentication"))
            .child(text("Enter the code from your authenticator app.").font(type_::BODY).color(t.text_secondary).wrap().w(W))
            .child(labelled("Code", TextInput::new(&a.code).height(44.0).placeholder("000000").label("Code")
                .width(Some(W)).on_edit(|s: &mut State| &mut s.auth.code)
                .on_submit(|s: &mut State, _| submit(s)).build(k)));
    } else {
        form = form
            .child(title("Welcome back"))
            .child(labelled("Email", TextInput::new(&a.identifier).height(44.0).label("Email").width(Some(W))
                .on_edit(|s: &mut State| &mut s.auth.identifier).on_submit(|s: &mut State, _| submit(s)).build(k)))
            .child(labelled("Password", TextInput::new(&a.password).height(44.0).label("Password").secure().width(Some(W))
                .on_edit(|s: &mut State| &mut s.auth.password).on_submit(|s: &mut State, _| submit(s)).build(k)));
    }
    if let Some(e) = &a.error {
        form = form.child(text(e.clone()).key("login-error").font(type_::BODY).color(k.c.danger.fg).wrap().w(W));
    }
    if let Some(n) = &a.info {
        form = form.child(text(n.clone()).key("login-info").font(type_::BODY).color(t.text_secondary).wrap().w(W));
    }
    let action = if waiting.is_some() {
        Button::new("Resend email").variant(ButtonVariant::Neutral).on_press(|s: &mut State| s.auth.resend()).build(k).cursor(rune::Cursor::Pointer)
    } else {
        let label = match (a.busy, in_code_step) { (true, _) => "Signing in…", (_, true) => "Verify", _ => "Sign in" };
        let b = row().key("login-submit").h(36.0).px(22.0).radius(radius::SM).fill(oklch(0.5, 0.2, 277.0)).center()
            .child(text(label).font(rune::tok::TextStyle::new(13.0, 600.0)).color(t.text));
        if a.busy { b.opacity(0.6) } else { b.cursor(rune::Cursor::Pointer).on_press(|s: &mut State| submit(s)) }
    };
    form = form.child(el().w(W).h(1.0).fill(line))
        .child(row().w(W).gap(16.0).justify_end().center_y()
            .child(if in_code_step || waiting.is_some() {
                text("Back").key("login-back").font(type_::LABEL).color(t.text_secondary)
                    .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| s.auth.back_to_credentials())
            } else { el().size(0.0, 0.0) })
            .child(action));

    let e = 1.0 - (1.0 - s.intro).powi(3);
    let brand = match s.net.logo() {
        Some(img) => el().size(128.0, 128.0).pixels(img.pixels()),
        None => el().size(128.0, 128.0).radius(64.0).fill(oklch(0.5, 0.2, 277.0)),
    };
    let left = col().w(288.0).h_full().center().gap(14.0).child(brand)
        .child(text("Fluxer").font(rune::tok::TextStyle::new(40.0, 800.0)).color(t.text));
    let right = col().grow(1.0).h_full().center().child(form);
    let card = row().key("login-card").opacity(e).opacity_motion(rune::Anim::INSTANT).offset(0.0, 40.0 * (1.0 - e)).offset_motion(rune::Anim::INSTANT)
        .w(800.0).h(447.0).radius(radius::LG).fill(panel).border(1.0, line).shadow(12.0, 40.0, t.shadow)
        .child(left).child(el().w(1.0).h_full().fill(line)).child(right);
    col().w_full().h_full().center()
        .fill(oklch(0.13, 0.03, 277.0)).gradient(oklch(0.115, 0.03, 277.0), oklch(0.15, 0.03, 280.0))
        .child(card)
}


fn tint(id: &str) -> usize { id.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize)) % 4 }

fn initial(name: &str) -> String { name.chars().next().map_or("?".into(), |c| c.to_uppercase().to_string()) }

/// A round avatar: the downloaded picture, or a tinted circle with the initial until it arrives.
fn avatar(s: &State, user: &User, size: f32) -> El {
    let t = tok::theme(true);
    let img = s.chat.media.as_deref().and_then(|m| api::avatar_url(m, user, 128)).and_then(|u| s.net.image(&u).cloned());
    // Nobody without a picture of their own: the Fluxer logo, like the web app's default avatar.
    let img = img.or_else(|| if user.avatar.is_none() { s.net.logo().cloned() } else { None });
    match img {
        Some(img) => el().size(size, size).radius(size / 2.0).pixels(img.pixels()),
        None => row().size(size, size).radius(size / 2.0).fill(tok::accent(tint(&user.id), true)).center()
            .child(text(initial(user.display())).font(type_::LABEL).color(t.knob)),
    }
}

fn guild_icon(s: &State, i: usize, size: f32, r: f32) -> El {
    let t = tok::theme(true);
    let g = &s.chat.guilds[i];
    let img = s.chat.media.as_deref().zip(g.icon.as_deref())
        .map(|(m, h)| api::guild_icon_url(m, &g.id, h, 128)).and_then(|u| s.net.image(&u).cloned());
    match img {
        Some(img) => el().size(size, size).radius(r).pixels(img.pixels()),
        None => row().size(size, size).radius(r).fill(tok::accent(tint(&g.id), true)).center()
            .child(text(initial(&g.name)).font(type_::TITLE).color(t.knob)),
    }
}

/// Phosphor's `SpeakerHigh`, filled: a solid cone with the two sound arcs, as the web app's badge draws it.
static SPEAKER_HIGH: rune::Icon = rune::Icon { name: "app-speaker-high", view: 256.0, body: r#"<path d="M160,32.25V223.69a8.29,8.29,0,0,1-3.91,7.18,8,8,0,0,1-9-.56l-65.57-51A4,4,0,0,1,80,176.16V79.84a4,4,0,0,1,1.55-3.15l65.57-51a8,8,0,0,1,10,.16A8.27,8.27,0,0,1,160,32.25ZM60,80H32A16,16,0,0,0,16,96v64a16,16,0,0,0,16,16H60a4,4,0,0,0,4-4V84A4,4,0,0,0,60,80Zm126.77,20.84a8,8,0,0,0-.72,11.3,24,24,0,0,1,0,31.72,8,8,0,1,0,12,10.58,40,40,0,0,0,0-52.88A8,8,0,0,0,186.74,100.84Zm40.89-26.17a8,8,0,1,0-11.92,10.66,64,64,0,0,1,0,85.34,8,8,0,1,0,11.92,10.66,80,80,0,0,0,0-106.66Z"/>"# };

/// The green call bubble on a server tile while someone is in one of its voice channels: a 20 px
/// speaker at the top-right corner, cut out of the tile by a 3 px ring of the rail's colour, as
/// the web app's `guildVoiceBadge` draws it.
fn call_badge(in_call: bool) -> El {
    if !in_call { return el().size(0.0, 0.0) }
    row().size(26.0, 26.0).radius(13.0).fill(pal().rail).center()
        .child(row().size(20.0, 20.0).radius(10.0).fill(oklch(0.666, 0.154, 156.0)).center()
            .child(rune::icon(&SPEAKER_HIGH, 12.0).color(tok::theme(true).knob)))
}

fn badge(n: u32) -> El {
    if n == 0 { return el().size(0.0, 0.0) }
    let p = pal();
    row().h(18.0).px(5.0).radius(9.0).fill(p.unread).border(2.0, p.rail).center()
        .child(text(if n > 99 { "99+".into() } else { n.to_string() }).font(type_::LABEL).color(tok::theme(true).knob))
}

fn guild_rail(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let nav = s.chat.nav;
    let tile = |key: String, selected: bool, unread: u32, new: bool, in_call: bool, body: El, on: Box<dyn Fn(&mut State)>| {
        let hovered = s.rail_hover.as_deref() == Some(key.as_str());
        let pill = if selected { 28.0 } else if hovered && !(unread > 0 || new) { 8.0 } else if unread > 0 || new { 8.0 } else { 0.0 };
        let k = key.clone();
        row().key(key).w(RAIL_WIDTH).h(52.0).center()
            .child(el().absolute(0.0, (52.0 - pill) / 2.0).size(4.0, pill).radius(2.0).fill(t.text))
            .child(row().size(48.0, 48.0).center().child(body)
                .child(el().absolute(30.0, 30.0).child(badge(unread)))
                .child(el().absolute(29.0, -7.0).child(call_badge(in_call))))
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| on(s))
            .on_hover_change(move |s: &mut State, inside, _| {
                if inside { s.rail_hover = Some(k.clone()) } else if s.rail_hover.as_deref() == Some(k.as_str()) { s.rail_hover = None }
            })
    };
    let hov = |key: &str| s.rail_hover.as_deref() == Some(key);
    let radius_of = |selected: bool, key: &str| if selected || hov(key) { 15.0 } else { 24.0 };
    let dm_sel = nav.guild.is_none();
    let dm_lit = dm_sel || hov("rail-dm");
    let dms = row().size(48.0, 48.0).radius(radius_of(dm_sel, "rail-dm")).fill(if dm_lit { p.brand } else { p.input }).center()
        .child(rune::icon(&CHAT_CIRCLE, 26.0).color(if dm_lit { t.knob } else { t.text }));
    let mut rail = col().w(RAIL_WIDTH).h_full().pt(12.0).pb(DOCK_USER_H + 24.0 + if s.call.active() { DOCK_CALL_H } else { 0.0 }).gap(8.0).fill(p.rail)
        .child(tile("rail-dm".into(), dm_sel, s.chat.unread(None), s.chat.has_new(None), false, dms, Box::new(|s| open_guild(s, None))))
        .child(row().w_full().h(2.0).px(24.0).child(el().w_full().h(2.0).radius(1.0).fill(p.line)));
    for i in 0..s.chat.guilds.len() {
        let sel = nav.guild == Some(i);
        let g = &s.chat.guilds[i];
        let in_call = g.voice.iter().any(|ch| s.chat.voice.get(&ch.id).is_some_and(|v| !v.is_empty()));
        rail = rail.child(tile(format!("rail-{i}"), sel, s.chat.unread(Some(i)), s.chat.has_new(Some(i)), in_call, guild_icon(s, i, 48.0, radius_of(sel, &format!("rail-{i}"))),
            Box::new(move |s| open_guild(s, Some(i)))));
    }
    rail.scroll(ScrollSpec { stick_bottom: false })
}

fn channel_row(s: &State, i: usize, ch: &Channel) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let active = s.chat.voice_view.is_none() && i == s.chat.nav.channel;
    let lead = match &ch.peer {
        Some(u) => avatar(s, u, 24.0),
        None => text(if s.chat.nav.guild.is_some() { "#" } else { "@" }).font(type_::TITLE).color(t.text_secondary).w(24.0),
    };
    let name_color = if active || ch.unread > 0 || ch.new { t.text } else { t.text_secondary };
    row().key(format!("ch-{i}")).h(36.0).px(10.0).gap(10.0).center_y().radius(radius::SM)
        .fill(if active { p.select } else { oklcha(0.0, 0.0, 0.0, 0.0) }).hover(if active { p.select } else { p.hover })
        .child(lead)
        .child(text(ch.name.clone()).font(if ch.unread > 0 || ch.new { type_::TITLE } else { type_::BODY }).color(name_color).grow(1.0))
        .child(badge_on(ch.unread, p.side))
        .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| open_channel(s, i))
}

/// A count pill whose outline matches the surface it sits on.
fn badge_on(n: u32, surface: Color) -> El {
    if n == 0 { return el().size(0.0, 0.0) }
    row().h(18.0).px(6.0).radius(9.0).fill(pal().unread).border(0.0, surface).center()
        .child(text(if n > 99 { "99+".into() } else { n.to_string() }).font(type_::LABEL).color(tok::theme(true).knob))
}

fn channel_list(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let (title, sub) = match s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)) {
        Some(g) => (g.name.clone(), "CANAIS"),
        None => ("Mensagens diretas".to_string(), "CONVERSAS"),
    };
    let mut list = col().gap(2.0).px(8.0).pt(8.0).pb(DOCK_USER_H + 24.0 + if s.call.active() { DOCK_CALL_H } else { 0.0 });
    if s.chat.nav.guild.is_none() {
        list = list.child(row().h(24.0).px(10.0).center_y().child(text(sub).font(type_::SECTION).color(t.text_secondary)));
    }
    let mut last_cat: Option<&Option<String>> = None;
    for (i, ch) in s.chat.channels().iter().enumerate() {
        if s.chat.nav.guild.is_some() && last_cat != Some(&ch.category) {
            if let Some(c) = &ch.category {
                list = list.child(row().h(28.0).px(10.0).center_y().child(text(c.to_uppercase()).font(type_::SECTION).color(t.text_secondary)));
            }
        }
        last_cat = Some(&ch.category);
        list = list.child(channel_row(s, i, ch));
    }
    if let Some(g) = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)) && !g.voice.is_empty() {
        list = list.child(row().h(28.0).px(10.0).pt(8.0).center_y().child(text("VOICE CHANNELS").font(type_::SECTION).color(t.text_secondary)));
        for (vi, ch) in g.voice.iter().enumerate() { list = voice_block(s, list, ch, vi) }
    }
    let banner = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)).and_then(|g| {
        let (media, hash) = (s.chat.media.as_deref()?, g.banner.as_deref()?);
        let still = api::banner_still_url(media, &g.id, hash, crate::model::BANNER_SIZE);
        let animated = api::banner_url(media, &g.id, hash, crate::model::BANNER_SIZE);
        Some((playing(s, &animated, &still), animated))
    });
    let header = match banner {
        // A banner replaces the plain title bar: the picture (animated if it is) with the name over it.
        Some((img, animated)) => {
            let pic = el().key("guild-banner").size(list_w(s), banner_h(s)).on_visible(on_screen(animated));
            let pic = match img { Some(i) => pic.pixels(&i.pixels), None => pic.fill(p.rail) };
            col().w(list_w(s)).h(banner_h(s)).child(pic.child(el().absolute(0.0, 0.0).w(list_w(s)).h(banner_h(s) * 0.4)
                .linear_gradient(std::f32::consts::PI, [(0.0, oklcha(0.0, 0.0, 0.0, 0.55)), (1.0, oklcha(0.0, 0.0, 0.0, 0.0))]))
                .child(el().absolute(16.0, 14.0).child(text(title).font(type_::TITLE).color(rune::tok::theme(true).knob))))
        }
        None => col().child(row().h(48.0).px(16.0).center_y().child(text(title).font(type_::TITLE).color(t.text)))
            .child(el().w_full().h(1.0).fill(p.line)),
    };
    col().w(list_w(s)).h_full().fill(p.side)
        .child(header)
        .child(list.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
        .child(resize_handle("resize-list", true, list_w(s), |s, w| s.prefs.list_w = w.clamp(220.0, 420.0) as u32).pin(None, Some(0.0), Some(0.0), Some(0.0)))
}

/// People in a voice channel in a fixed order (by name, then id), so someone who leaves and comes
/// back lands in the same place instead of at the end.
fn sorted_voice<'a>(s: &State, people: Option<&'a Vec<crate::model::VState>>) -> Vec<&'a crate::model::VState> {
    let mut v: Vec<&crate::model::VState> = people.into_iter().flatten().collect();
    // By the name shown in the open server (the nickname there, else the display name).
    let key = |p: &crate::model::VState| (s.chat.roster.users.get(&p.user_id).map_or(String::new(), |u| name_in(s, u).to_lowercase()), p.user_id.clone());
    v.sort_by_key(|p| key(p));
    v
}

/// A voice channel and the people in it.
fn voice_block(s: &State, mut list: El, ch: &Channel, vi: usize) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let here = s.call.channel_id.as_deref() == Some(ch.id.as_str());
    let id = ch.id.clone();
    let chat_open = s.chat.voice_view == Some(vi);
    let chat_btn = row().key(format!("vc-chat-{}", ch.id)).size(24.0, 24.0).radius(radius::SM).center().hover(p.select)
        .child(rune::icon(&CHAT_CIRCLE, 16.0).color(if chat_open { t.text } else { t.text_secondary }))
        .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| open_voice_chat(s, vi));
    list = list.child(row().key(format!("vc-{}", ch.id)).h(36.0).px(10.0).gap(10.0).center_y().radius(radius::SM)
        .fill(if here { p.select } else { oklcha(0.0, 0.0, 0.0, 0.0) }).hover(if here { p.select } else { p.hover })
        .child(rune::icon(&SPEAKER, 18.0).color(if here { oklch(0.72, 0.17, 150.0) } else { t.text_secondary }))
        .child(text(ch.name.clone()).font(type_::BODY).color(if here { t.text } else { t.text_secondary }).grow(1.0))
        .child(chat_btn)
        .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| join_voice(s, &id)));
    for v in sorted_voice(s, s.chat.voice.get(&ch.id)) {
        let user = s.chat.roster.users.get(&v.user_id);
        let name = user.map_or("…".to_string(), |u| name_in(s, u));
        let speaking = s.call.speaking.contains(&v.user_id);
        let face = match user {
            Some(u) => avatar(s, u, 24.0),
            None => el().size(24.0, 24.0).radius(12.0).fill(p.input),
        };
        let ring = row().size(28.0, 28.0).radius(14.0).center()
            .border(2.0, if speaking { oklch(0.72, 0.17, 150.0) } else { oklcha(0.0, 0.0, 0.0, 0.0) }).instant_changes().child(face);
        let mut line = row().h(32.0).pl(30.0).pr(10.0).gap(8.0).center_y().radius(radius::SM).hover(p.select).cursor(rune::Cursor::Pointer)
            .child(ring)
            .child(text(name).font(type_::BODY).color(t.text).truncate().min_w(0.0));
        if let Some(u) = user && let Some(ic) = top_role_el(s, u) { line = line.child(ic).child(el().grow(1.0)) } else { line = line.child(el().grow(1.0)) }
        if let Some(u) = user.cloned() { let key = format!("vp-{}-{}", ch.id, v.user_id); line = line.on_secondary_press(move |s: &mut State, p, _| open_user_menu(s, &u, p)) }
        if v.self_stream {
            line = line.child(row().h(16.0).px(5.0).radius(4.0).center().fill(p.unread).child(text("LIVE").font(type_::SECTION).color(t.knob)));
        }
        if v.deafened() { line = line.child(rune::icon(&HEADSET_OFF, 16.0).color(t.text_secondary)) }
        else if v.muted() { line = line.child(rune::icon(&MIC_OFF, 16.0).color(t.text_secondary)) }
        if let Some(u) = user { line = opens_profile(line, u, format!("vp-{}-{}", ch.id, v.user_id)) }
        list = list.child(line);
    }
    list
}

const GREEN: fn() -> Color = || oklch(0.72, 0.17, 150.0);
const DOCK_USER_H: f32 = 56.0;
const DOCK_CALL_H: f32 = 108.0;

static WAVEFORM: rune::Icon = rune::Icon { name: "app-waveform", view: 24.0, body: r#"<path d="M4 10v4M8 6v12M12 3v18M16 7v10M20 10v4" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" fill="none"/>"# };
static SIGNAL: rune::Icon = rune::Icon { name: "app-signal", view: 24.0, body: r#"<path d="M4 4a16 16 0 0 1 16 16M4 10a10 10 0 0 1 10 10M4 16a4 4 0 0 1 4 4" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" fill="none"/>"# };
static CHEVRON: rune::Icon = rune::Icon { name: "app-chevron", view: 24.0, body: r#"<path d="M9 5l7 7-7 7" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static CHECK: rune::Icon = rune::Icon { name: "app-check", view: 24.0, body: r#"<path d="M5 12.5l4.5 4.5L19 7.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };

/// The floating bar at the bottom left: the voice call (when in one) above the account bar. It
/// sits over the server rail and the channel list, as in the web app.
fn dock(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let Some(u) = s.chat.me.as_ref() else { return el().size(0.0, 0.0) };
    let clear = oklcha(0.0, 0.0, 0.0, 0.0);
    let c = &s.call;
    let icon_btn = |key: &str, on: bool, danger: bool, icon: &'static rune::Icon, f: fn(&mut State)| {
        let b = row().key(key.to_string()).size(32.0, 30.0).radius(radius::SM).center()
            .fill(if danger { oklcha(0.64, 0.2, 25.0, 0.22) } else if on { p.select } else { clear }).hover(p.select)
            .child(rune::icon(icon, 19.0).color(if danger { p.unread } else if on { t.text } else { t.text_secondary }))
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s));
        b
    };
    let mut card = col().key("dock").w(dock_w(s)).radius(12.0).fill(oklch(0.2, 0.008, 265.0)).border(1.0, p.line).shadow(6.0, 24.0, t.shadow);
    if c.active() {
        let place = c.channel_id.as_deref().and_then(|id| s.chat.voice_channel(id)).map_or(String::new(), |(g, ch)| format!("{} / {}", ch.name, g.name));
        let connected = c.phase == crate::call::Phase::Connected;
        let people = c.channel_id.as_deref().and_then(|id| s.chat.voice.get(id));
        let mut faces = row().gap(4.0).h(32.0).center_y();
        for v in sorted_voice(s, people) {
            let Some(user) = s.chat.roster.users.get(&v.user_id) else { continue };
            let speaking = c.speaking.contains(&v.user_id);
            // The ring around the picture shows at once when sound comes through, and goes with it.
            let mut f = row().size(32.0, 32.0).radius(16.0).center().instant_changes()
                .border(2.0, if speaking { GREEN() } else { clear })
                .child(avatar(s, user, 26.0));
            // What is off, said with its icon: the microphone, or everything for deafened.
            if v.muted() {
                let icon = if v.deafened() { &HEADSET_OFF } else { &MIC_OFF };
                f = f.child(row().absolute(18.0, 18.0).size(16.0, 16.0).radius(8.0).fill(p.unread).border(2.0, oklch(0.2, 0.008, 265.0)).center()
                    .child(rune::icon(icon, 9.0).color(t.knob)));
            }
            faces = faces.child(opens_profile(f, user, format!("df-{}", v.user_id)));
        }
        card = card.child(col().key("voice-section").h(DOCK_CALL_H).px(14.0).py(10.0).gap(6.0)
            .child(row().h(30.0).center_y().gap(8.0)
                .child(rune::icon(&SIGNAL, 16.0).color(GREEN()))
                .child(text(if connected { "In voice chat" } else { "Connecting…" }).font(type_::TITLE).color(GREEN()).grow(1.0))
                .child(icon_btn("noise-toggle", s.call.settings.noise_suppression, false, &WAVEFORM, |s| { let on = !s.call.settings.noise_suppression; s.call.set_noise_suppression(on) }))
                .child(icon_btn("voice-leave", false, false, &HANG_UP, leave_voice)))
            .child(text(place).font(type_::LABEL).color(t.text_secondary).truncate().w(dock_w(s) - 28.0))
            .child(faces));
        card = card.child(el().w_full().h(1.0).fill(p.line));
    } else if let Some(e) = &c.error {
        card = card.child(text(e.clone()).key("voice-error").font(type_::LABEL).color(p.unread).wrap().w(dock_w(s) - 28.0).px(14.0).py(8.0));
    }
    let muted = c.muted || c.deafened;
    let mic = icon_btn("mic-toggle", false, muted, if muted { &MIC_OFF } else { &MIC }, toggle_mute)
        .on_secondary_press(|s: &mut State, _, _| s.call.open_menu_at("mic-toggle"));
    let deaf = icon_btn("deafen-toggle", false, c.deafened, if c.deafened { &HEADSET_OFF } else { &HEADSET }, toggle_deafen)
        .on_secondary_press(|s: &mut State, _, _| s.call.open_menu_at("deafen-toggle"));
    let status = s.chat.roster.status_of(&u.id);
    let face = opens_profile(row().size(36.0, 36.0).child(avatar(s, u, 36.0))
        .child(el().absolute(25.0, 25.0).size(12.0, 12.0).radius(6.0).fill(status_color(if status == "offline" { "online" } else { status })).border(2.0, oklch(0.2, 0.008, 265.0))), u, "account-face".into());
    card.child(row().key("account-bar").h(DOCK_USER_H).px(12.0).gap(10.0).center_y()
        .child(face)
        .child(col().grow(1.0).min_w(0.0)
            .child(text(u.display().to_string()).font(type_::TITLE).color(t.text).truncate())
            .child(text(match status { "idle" => "Idle", "dnd" => "Do not disturb", _ => "Online" }).font(type_::LABEL).color(t.text_secondary)))
        .child(mic).child(deaf)
        .child(icon_btn("audio-settings", false, false, &GEAR, settings::open)))
        .pin(Some(8.0), None, None, Some(8.0))
}

/// A horizontal slider, 0..=1: drag or click anywhere on it.
fn slider(key: &str, value: f32, set: fn(&mut State, f32)) -> El {
    let (p, t) = (pal(), tok::theme(true));
    const W: f32 = 238.0;
    let v = value.clamp(0.0, 1.0);
    el().key(key.to_string()).w(W).h(22.0)
        .child(el().absolute(0.0, 9.0).size(W, 4.0).radius(2.0).fill(p.input))
        .child(el().absolute(0.0, 9.0).size((W * v).max(2.0), 4.0).radius(2.0).fill(oklch(0.58, 0.19, 277.0)))
        .child(el().absolute((W * v - 8.0).clamp(0.0, W - 16.0), 3.0).size(16.0, 16.0).radius(8.0).fill(t.text))
        .cursor(rune::Cursor::Pointer).on_drag(move |s: &mut State, d: &mut rune::Drag, _| set(s, (d.x / d.w.max(1.0)).clamp(0.0, 1.0)))
}

/// The audio menu: devices, profile, volumes, the microphone level and deafening.
fn audio_menu(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    if !s.call.menu_open { return el().size(0.0, 0.0) }
    let clear = oklcha(0.0, 0.0, 0.0, 0.0);
    let cfg = &s.call.settings;
    let dev = s.call.devices.as_ref();
    let label_of = |list: Option<&Vec<crate::audio::Device>>, id: &Option<String>| match (id, list) {
        (Some(i), Some(l)) => l.iter().find(|d| &d.id == i).map_or("System default".to_string(), |d| d.label.clone()),
        _ => "System default".to_string(),
    };
    let folded = s.call.fold;
    let header = |key: &str, title: &str, sub: String, fold: crate::call::Fold| row().key(key.to_string()).h(46.0).px(12.0).gap(8.0).center_y().radius(radius::SM).hover(p.hover)
        .child(col().grow(1.0).min_w(0.0)
            .child(text(title.to_string()).font(type_::TITLE).color(t.text))
            .child(text(sub).font(type_::LABEL).color(t.text_secondary).truncate()))
        .child(rune::icon(&CHEVRON, 14.0).color(t.text_secondary).rotate(if folded == fold { std::f32::consts::FRAC_PI_2 } else { 0.0 }))
        .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| s.call.toggle_fold(fold));
    let choice = |key: String, label: String, on: bool, f: Box<dyn Fn(&mut State)>| row().key(key).h(30.0).pl(22.0).pr(12.0).gap(8.0).center_y().radius(radius::SM).hover(p.hover)
        .child(text(label).font(type_::BODY).color(if on { t.text } else { t.text_secondary }).truncate().grow(1.0).min_w(0.0))
        .child(if on { rune::icon(&CHECK, 14.0).color(GREEN()) } else { el().size(14.0, 14.0) })
        .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s));
    let row_of = |left: &str, right: String| row().h(24.0).px(12.0).center_y()
        .child(text(left.to_string()).font(type_::TITLE).color(t.text).grow(1.0)).child(text(right).font(type_::LABEL).color(t.text_secondary));
    let divider = || el().w_full().h(1.0).fill(p.line).my(6.0);

    let mut body = col().gap(1.0).py(8.0).px(6.0);
    body = body.child(header("m-in", "Input device", label_of(dev.map(|d| &d.mics), &cfg.mic), crate::call::Fold::Input));
    if folded == crate::call::Fold::Input {
        body = body.child(choice("mi-default".into(), "System default".into(), cfg.mic.is_none(), Box::new(|s| s.call.set_mic(None))));
        for (i, d) in dev.into_iter().flat_map(|d| d.mics.iter()).enumerate() {
            let id = d.id.clone();
            body = body.child(choice(format!("mi-{i}"), d.label.clone(), cfg.mic.as_deref() == Some(d.id.as_str()), Box::new(move |s| s.call.set_mic(Some(id.clone())))));
        }
    }
    body = body.child(header("m-prof", "Input profile", (if cfg.noise_suppression { "Focused voice" } else { "Studio" }).to_string(), crate::call::Fold::Profile));
    if folded == crate::call::Fold::Profile {
        body = body.child(choice("mp-focus".into(), "Focused voice (RNNoise)".into(), cfg.noise_suppression, Box::new(|s| s.call.set_noise_suppression(true))))
            .child(choice("mp-studio".into(), "Studio (no processing)".into(), !cfg.noise_suppression, Box::new(|s| s.call.set_noise_suppression(false))));
    }
    body = body.child(header("m-out", "Output device", label_of(dev.map(|d| &d.speakers), &cfg.speaker), crate::call::Fold::Output));
    if folded == crate::call::Fold::Output {
        body = body.child(choice("mo-default".into(), "System default".into(), cfg.speaker.is_none(), Box::new(|s| s.call.set_speaker(None))));
        for (i, d) in dev.into_iter().flat_map(|d| d.speakers.iter()).enumerate() {
            let id = d.id.clone();
            body = body.child(choice(format!("mo-{i}"), d.label.clone(), cfg.speaker.as_deref() == Some(d.id.as_str()), Box::new(move |s| s.call.set_speaker(Some(id.clone())))));
        }
    }
    // The level bar: 22 small bars, lit up to the microphone's level.
    let level = s.call.level();
    let mut meter = row().gap(3.0).h(18.0).px(12.0).center_y();
    for i in 0..22 {
        let on = (i as f32 + 0.5) / 22.0 <= level;
        meter = meter.child(el().size(6.0, 6.0 + i as f32 * 0.5).radius(1.5).fill(if on { GREEN() } else { p.input }));
    }
    body = body.child(divider())
        .child(row_of("Input volume", format!("{}%", (cfg.input_volume * 100.0).round())))
        .child(row().px(12.0).child(slider("vol-in", cfg.input_volume, |s, v| s.call.set_input_volume(v))))
        .child(row_of("Input level", String::new()))
        .child(meter)
        .child(row_of("Output volume", format!("{}%", (cfg.output_volume * 100.0).round())))
        .child(row().px(12.0).child(slider("vol-out", cfg.output_volume, |s, v| s.call.set_output_volume(v))))
        .child(divider())
        .child(row().key("m-deafen").h(34.0).px(12.0).center_y().radius(radius::SM).hover(p.hover)
            .child(text("Deafen").font(type_::TITLE).color(t.text).grow(1.0))
            .child(row().size(18.0, 18.0).radius(4.0).border(2.0, if s.call.deafened { GREEN() } else { t.text_secondary }).center()
                .fill(if s.call.deafened { GREEN() } else { clear }).child(if s.call.deafened { rune::icon(&CHECK, 12.0).color(t.knob) } else { el().size(0.0, 0.0) }))
            .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| toggle_deafen(s)));
    col().key("audio-menu").w(dock_w(s) - 34.0).max_h(560.0).radius(10.0).fill(oklch(0.22, 0.008, 265.0)).border(1.0, p.line).shadow(6.0, 24.0, t.shadow).on_press(|_: &mut State| {})
        .child(body.scroll(ScrollSpec { stick_bottom: false }))
        .anchored(s.call.menu_anchor.clone(), rune::Side::Top, rune::Align::Center, 12.0)
}

/// The menu of a person (right click): mention, message, how loud they are for me, mute for me, copy id.
fn user_menu(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let Some(c) = &s.ctx else { return el().size(0.0, 0.0) };
    let me = s.chat.me.as_ref().is_some_and(|m| m.id == c.user.id);
    let clear = oklcha(0.0, 0.0, 0.0, 0.0);
    let id = c.user.id.clone();
    let item = |key: &str, label: &str, f: Box<dyn Fn(&mut State, &mut rune::Tx)>| row().key(key.to_string()).h(34.0).px(12.0).center_y().radius(radius::SM).hover(p.hover)
        .child(text(label.to_string()).font(type_::BODY).color(t.text).grow(1.0))
        .cursor(rune::Cursor::Pointer).on_press_tx(move |s: &mut State, tx: &mut rune::Tx| { f(s, tx); s.ctx = None });
    let divider = || el().w_full().h(1.0).fill(p.line).my(6.0);
    let mut body = col().gap(1.0).py(8.0).px(6.0);
    if s.chat.nav.guild.is_some() {
        let u = c.user.clone();
        body = body.child(item("cm-mention", "Mention", Box::new(move |s, _| { let t = format!("{}<@{}> ", s.draft.text, u.id); s.draft = rune::TextEdit::new(t) })));
    }
    // A direct message that already exists with this person.
    if let Some(i) = s.chat.dms.iter().position(|d| d.peer.as_ref().is_some_and(|p| p.id == c.user.id)) && !me {
        body = body.child(item("cm-message", "Message", Box::new(move |s, _| { open_guild(s, None); open_channel(s, i) })));
    }
    if !me {
        let (vol, muted) = (s.call.user_volume(&c.user.id), s.call.user_muted(&c.user.id));
        let gid = id.clone();
        body = body.child(divider())
            .child(row().h(24.0).px(12.0).center_y()
                .child(text("User volume").font(type_::TITLE).color(t.text).grow(1.0))
                .child(text(format!("{}%", (vol * 100.0).round())).font(type_::LABEL).color(t.text_secondary)))
            .child(row().px(12.0).child(slider("cm-vol", vol / 2.0, |s, v| if let Some(c) = &s.ctx { let id = c.user.id.clone(); s.call.set_user_volume(&id, v * 2.0) })))
            .child(divider())
            .child(row().key("cm-mute").h(34.0).px(12.0).center_y().radius(radius::SM).hover(p.hover)
                .child(text("Mute").font(type_::BODY).color(t.text).grow(1.0))
                .child(row().size(18.0, 18.0).radius(4.0).border(2.0, if muted { GREEN() } else { t.text_secondary }).center()
                    .fill(if muted { GREEN() } else { clear }).child(if muted { rune::icon(&CHECK, 12.0).color(t.knob) } else { el().size(0.0, 0.0) }))
                .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| s.call.toggle_user_mute(&gid)));
    }
    let copy = id.clone();
    body = body.child(divider()).child(item("cm-copy", "Copy user ID", Box::new(move |_, tx| tx.copy(copy.clone()))));
    col().key("user-menu").w(250.0).radius(10.0).fill(oklch(0.22, 0.008, 265.0)).border(1.0, p.line).shadow(6.0, 24.0, t.shadow).on_press(|_: &mut State| {})
        .child(body)
        .anchored("ctx-point", rune::Side::Bottom, rune::Align::Start, 2.0)
}

/// A point with no size where the right click landed; the menu opens at its lower right.
fn ctx_point(s: &State) -> El {
    match &s.ctx { Some(c) => el().key("ctx-point").absolute(c.x, c.y).size(0.0, 0.0), None => el().size(0.0, 0.0) }
}

/// "Nov 3, 2026" for a moment in Unix milliseconds.
fn short_date(ms: i64) -> String {
    let (y, m, d) = crate::clock::ymd(ms);
    let names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {d}, {y}", names[((m - 1).clamp(0, 11)) as usize])
}

/// The profile card: banner (the picture, else the colour they chose), picture, names, pronouns, bio,
/// where they are in voice, when they joined, and a way to write to them.
fn profile_card(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let Some(pr) = &s.profile else { return el().size(0.0, 0.0) };
    let u = &pr.user;
    let me = s.chat.me.as_ref().is_some_and(|m| m.id == u.id);
    let guild = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g));
    let info = s.profiles.get(&crate::net::profile_key(&u.id, guild.map(|g| g.id.as_str())));
    let status = s.chat.roster.status_of(&u.id);
    let card_bg = oklch(0.17, 0.008, 265.0);
    let tag = match &u.discriminator { Some(d) => format!("{}#{d}", u.username), None => u.username.clone() };
    // The picture they set if it has arrived; otherwise their colour; otherwise a tint of their id.
    let image = s.chat.media.as_deref().zip(info.and_then(|i| i.banner.as_deref()))
        .and_then(|(m, h)| s.net.image(&api::banner_still_url(m, &u.id, h, 600)));
    let colour = info.and_then(|i| i.color).and_then(|c| rune::color::hex(&format!("#{c:06X}"))).unwrap_or_else(|| tok::accent(tint(&u.id), true));
    let banner = match image {
        Some(img) => el().w_full().h(100.0).pixels(img.pixels()),
        None => el().w_full().h(100.0).fill(colour),
    };
    let face = row().size(80.0, 80.0).radius(40.0).fill(card_bg).border(6.0, card_bg).center().child(avatar(s, u, 68.0))
        .child(el().absolute(56.0, 56.0).size(18.0, 18.0).radius(9.0).fill(status_color(if status == "offline" && me { "online" } else { status })).border(3.0, card_bg));
    let mut body = col().px(16.0).pb(16.0).gap(4.0)
        .child(text(name_in(s, u)).font(type_::TITLE).color(t.text).truncate())
        .child(text(tag).font(type_::BODY).color(t.text_secondary).truncate());
    if let Some(pn) = info.and_then(|i| i.pronouns.clone()) { body = body.child(text(pn).font(type_::LABEL).color(t.text_secondary).truncate()) }
    if let Some(bio) = info.and_then(|i| i.bio.clone()) { body = body.child(text(bio).font(type_::LABEL).color(t.text).wrap().w(268.0).mt(6.0)) }
    // Where they are in voice, if anywhere.
    let here = s.chat.voice.iter().find(|(_, v)| v.iter().any(|x| x.user_id == u.id)).map(|(c, _)| c.clone());
    if let Some(cid) = here && let Some((g, ch)) = s.chat.voice_channel(&cid) {
        let (gi, vi) = (s.chat.guilds.iter().position(|x| x.id == g.id), g.voice.iter().position(|c| c.id == cid));
        let mut faces = row().gap(4.0).h(24.0).center_y();
        for v in sorted_voice(s, s.chat.voice.get(&cid)) {
            if let Some(user) = s.chat.roster.users.get(&v.user_id) { faces = faces.child(avatar(s, user, 22.0)) }
        }
        body = body.child(col().mt(10.0).px(12.0).py(12.0).gap(8.0).radius(8.0).fill(oklch(0.2, 0.008, 265.0)).border(1.0, p.line)
            .child(text("IN VOICE").font(type_::SECTION).color(t.text))
            .child(row().gap(6.0).center_y().child(rune::icon(&SPEAKER, 14.0).color(t.text_secondary))
                .child(text(format!("{} / {}", ch.name, g.name)).font(type_::LABEL).color(t.text_secondary).truncate()))
            .child(faces)
            .child(row().key("prof-voice").h(34.0).radius(radius::SM).center().fill(p.brand).hover(p.brand)
                .child(text("Open voice").font(type_::TITLE).color(t.knob)).cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    s.profile = None;
                    if let (Some(gi), Some(vi)) = (gi, vi) { open_guild(s, Some(gi)); open_voice_chat(s, vi) }
                })));
    }
    // When they made the account, and when they joined this server.
    let made = (u.id.parse::<u64>().unwrap_or(0) >> 22) as i64 + 1_420_070_400_000;
    let logo = match s.net.logo() { Some(l) => el().size(16.0, 16.0).pixels(l.pixels()), None => el().size(16.0, 16.0) };
    let mut dates = row().gap(14.0).center_y().child(row().gap(6.0).center_y().child(logo).child(text(short_date(made)).font(type_::LABEL).color(t.text)));
    if let (Some(ms), Some(gi)) = (info.and_then(|i| i.joined_ms), s.chat.nav.guild) {
        dates = dates.child(row().gap(6.0).center_y().child(guild_icon(s, gi, 16.0, 8.0)).child(text(short_date(ms)).font(type_::LABEL).color(t.text)));
    }
    body = body.child(text("Member since").font(type_::TITLE).color(t.text).mt(10.0)).child(dates);
    // Their roles, highest first: each a chip with the role's icon, or just its name.
    if let Some(g) = guild {
        let roles = s.chat.roster.member_roles_sorted(&g.id, &u.id);
        if !roles.is_empty() {
            body = body.child(text("Roles").font(type_::TITLE).color(t.text).mt(10.0));
            let mut chips = row().gap(6.0).wrap();
            for (rid, name, _color, icon, emoji) in roles {
                let mut chip = row().h(26.0).px(8.0).gap(6.0).center_y().radius(13.0).fill(oklch(0.2, 0.008, 265.0)).border(1.0, p.line);
                if let Some(pic) = role_icon_el(s, &rid, &icon, &emoji, &name) { chip = chip.child(pic) }
                chip = chip.child(text(name.clone()).font(type_::LABEL).color(t.text)).help(name.clone());
                chips = chips.child(chip);
            }
            body = body.child(chips);
        }
    }
    if !me && let Some(i) = s.chat.dms.iter().position(|d| d.peer.as_ref().is_some_and(|q| q.id == u.id)) {
        body = body.child(row().key("prof-message").mt(10.0).h(36.0).radius(radius::SM).center().gap(8.0).fill(p.brand).hover(p.brand)
            .child(rune::icon(&CHAT_CIRCLE, 14.0).color(t.knob)).child(text("Message").font(type_::TITLE).color(t.knob)).cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| { s.profile = None; open_guild(s, None); open_channel(s, i) }));
    }
    // The shadow lives on the outer box and the rounded clip on the inner one, so the banner follows the corners.
    col().key("profile-card").w(300.0).radius(10.0).shadow(6.0, 24.0, t.shadow).on_press(|_: &mut State| {})
        .child(col().w_full().radius(10.0).fill(card_bg).border(1.0, p.line).clip_rounded(10.0)
            .child(banner)
            .child(row().px(12.0).mt(-40.0).child(face))
            .child(body))
        .anchored(pr.anchor.clone(), rune::Side::Right, rune::Align::Start, 8.0)
}

fn profile_scrim(s: &State) -> El {
    if s.profile.is_none() { return el().size(0.0, 0.0) }
    el().key("profile-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).on_press(|s: &mut State| s.profile = None)
}

fn user_menu_scrim(s: &State) -> El {
    if s.ctx.is_none() { return el().size(0.0, 0.0) }
    el().key("ctx-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).on_press(|s: &mut State| s.ctx = None)
        .on_secondary_press(|s: &mut State, _, _| s.ctx = None)
}

/// A full-window layer under the menu: a click anywhere else closes it.
fn menu_scrim(s: &State) -> El {
    if !s.call.menu_open { return el().size(0.0, 0.0) }
    el().key("menu-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).on_press(|s: &mut State| s.call.close_menu())
}

fn human_size(b: u64) -> String {
    match b { 0..=1023 => format!("{b} B"), 1024..=1_048_575 => format!("{:.0} KB", b as f64 / 1024.0), _ => format!("{:.1} MB", b as f64 / 1_048_576.0) }
}

const GIF_MAX: f32 = 300.0;

/// The animated picture once its first frame is decoded (it holds the last one shown while paused),
/// else the still, which is small and arrives first.
fn playing(s: &State, animated: &str, still: &str) -> Option<crate::net::Shown> {
    s.net.shown(animated).or_else(|| s.net.shown(still))
}

/// The view's report that the element showing animated picture `url` is on screen or not: the
/// animation plays only while it is. Moves no state the view reads, so it asks for no new view.
fn on_screen(url: String) -> impl Fn(&mut State, bool, &mut rune::Tx) + 'static {
    move |s: &mut State, on: bool, tx: &mut rune::Tx| { s.net.set_on_screen(&url, on); tx.live_only() }
}

/// An animated picture with the "GIF" tag in its corner; the grey box stands in until it arrives.
/// `animated` is the address of the moving version, so it plays while this is on screen.
fn gif_view(key: String, w: f32, h: f32, img: Option<&crate::net::Shown>, animated: String) -> El {
    let t = tok::theme(true);
    let base = el().key(key).size(w, h).radius(radius::SM).on_visible(on_screen(animated));
    let base = match img { Some(i) => base.pixels(&i.pixels), None => base.fill(t.control_off) };
    base.child(el().absolute(8.0, 8.0).child(
        row().px(6.0).py(2.0).radius(4.0).fill(oklcha(0.0, 0.0, 0.0, 0.62)).center()
            .child(text("GIF").font(rune::tok::TextStyle::new(11.0, 800.0)).color(t.text))))
}

fn attachment(s: &State, a: &Attachment) -> El {
    let t = tok::theme(true);
    let url = a.url.clone();
    let open = move |_: &mut State, tx: &mut rune::Tx| { if let Some(u) = &url { let _ = tx.open_url(u); } };
    if a.is_gif() {
        let (w, h) = (a.width.unwrap_or(300) as f32, a.height.unwrap_or(300) as f32);
        let k = (GIF_MAX / w).min(GIF_MAX / h).min(1.0);
        let (dw, dh) = ((w * k).max(40.0), (h * k).max(40.0));
        let animated = a.url.as_deref().map(|u| crate::model::attachment_fetch_url(u, true)).unwrap_or_default();
        let img = a.url.as_deref().and_then(|u| playing(s, &animated, &crate::model::attachment_fetch_url(u, false)));
        return gif_view(format!("att-{}", a.id), dw, dh, img.as_ref(), animated).cursor(rune::Cursor::Pointer).on_press_tx(open);
    }
    if a.is_image() {
        let (w, h) = (a.width.unwrap_or(320) as f32, a.height.unwrap_or(200) as f32);
        let k = (ATTACHMENT_MAX.0 / w).min(ATTACHMENT_MAX.1 / h).min(1.0);
        let (dw, dh) = ((w * k).max(40.0), (h * k).max(40.0));
        let fetched = a.url.as_deref().map(|u| api::attachment_image_url(u, crate::model::ATTACHMENT_FETCH_WIDTH));
        let base = el().key(format!("att-{}", a.id)).size(dw, dh).radius(radius::SM);
        return match fetched.and_then(|u| s.net.image(&u).cloned()) {
            Some(img) => base.pixels(img.pixels()).cursor(rune::Cursor::Pointer).on_press_tx(open),
            None => base.fill(t.control_off),
        };
    }
    if a.is_video() {
        let (id, u) = (a.id.clone(), a.url.clone().unwrap_or_default());
        if s.video.as_ref().is_some_and(|(v, _)| v == &a.id) {
            let pp = pal();
            let (_, pl) = s.video.clone().unwrap();
            let playing_now = pl.is_playing();
            let (vw, vh) = pl.natural_size().map_or((360.0, 202.0), |(w, h)| (w as f32, h as f32));
            let k = (360.0 / vw).min(260.0 / vh).min(1.0);
            let (dw, dh) = ((vw * k).max(80.0), (vh * k).max(45.0));
            let dur = pl.duration();
            let frac = dur.map_or(0.0, |d| if d > 0.0 { (pl.time() / d).clamp(0.0, 1.0) } else { 0.0 });
            let mmss = |x: f64| { let x = x.max(0.0) as u64; format!("{}:{:02}", x / 60, x % 60) };
            let face = el().key(format!("vid-{}", a.id)).size(dw, dh).radius(radius::SM).video(&pl, rune::SurfaceFit::Fit)
                .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| crate::toggle_video_play(s));
            return col().key(format!("att-{}", a.id)).w(360.0).gap(6.0)
                .child(face)
                .child(row().gap(10.0).center_y()
                    .child(row().key(format!("vid-play-{}", a.id)).size(28.0, 28.0).radius(14.0).center().hover(pp.select)
                        .child(text(if playing_now { "⏸" } else { "▶" }).font(type_::BODY).color(t.text))
                        .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| crate::toggle_video_play(s)))
                    .child(el().grow(1.0).h(4.0).radius(2.0).fill(pp.input)
                        .child(el().h(4.0).radius(2.0).fill(pp.brand).w((300.0 * frac as f32).max(4.0))))
                    .child(text(format!("{} / {}", mmss(pl.time()), dur.map_or("--:--".into(), mmss))).font(type_::LABEL).color(t.text_secondary)))
                .child(row().gap(6.0).center_y()
                    .child(text(format!("▶ {}", a.filename)).font(type_::LABEL).color(t.text_secondary).truncate().min_w(0.0))
                    .child(row().key(format!("vid-close-{}", a.id)).px(6.0).py(2.0).radius(4.0).center().hover(pp.select)
                        .child(text("Fechar").font(type_::LABEL).color(t.text_secondary))
                        .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| crate::toggle_video(s, &id, &u))));
        }
        let (id, u) = (a.id.clone(), a.url.clone().unwrap_or_default());
        return row().key(format!("att-{}", a.id)).px(10.0).py(8.0).gap(8.0).radius(radius::SM).fill(t.card)
            .child(text(format!("▶ {}", a.filename)).font(type_::BODY).color(tok::accent(0, true)))
            .child(text(human_size(a.size)).font(type_::LABEL).color(t.text_secondary))
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| crate::toggle_video(s, &id, &u));
    }
    row().key(format!("att-{}", a.id)).px(10.0).py(8.0).gap(8.0).radius(radius::SM).fill(t.card)
        .child(text(format!("📎 {}", a.filename)).font(type_::BODY).color(tok::accent(0, true)))
        .child(text(human_size(a.size)).font(type_::LABEL).color(t.text_secondary))
        .cursor(rune::Cursor::Pointer).on_press_tx(open)
}

const FOLLOW_WINDOW_MS: i64 = 7 * 60_000;

static ARROW_IN: rune::Icon = rune::Icon { name: "app-arrow-in", view: 24.0, body: r#"<path d="M3 12h15M13 5.5L19.5 12 13 18.5" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };

fn open_link(_: &mut State, url: &str, tx: &mut rune::Tx) { let _ = tx.open_url(url); }

/// Fluxer's own welcome line ("Welcome, Lion!"), posted by the system account on join.
fn is_welcome(m: &Msg) -> bool {
    (m.author.username.eq_ignore_ascii_case("fluxer") || m.author.display().eq_ignore_ascii_case("fluxer"))
        && m.body.starts_with("Welcome, ") && m.body.ends_with('!')
}

/// The message text: a plain label when it has no markup, else Markdown with mentions rewritten.
fn message_text(s: &State, m: &Msg, dim: bool) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let names: Vec<(String, String)> = m.mentions.iter().map(|u| (u.id.clone(), name_in(s, u))).collect();
    let src = crate::richtext::prepare(&m.body, &names);
    let color = if dim { t.text_secondary } else { t.text };
    if !crate::richtext::needs_markdown(&src) {
        return text(src.into_owned()).font(type_::BODY).color(color).wrap();
    }
    let style = MarkdownStyle::neutral(true).text(color).muted(t.text_secondary).link(p.link)
        .body_font(rune::tok::TextStyle { line_height: 20.0, ..type_::BODY })
        .code_block(p.rail, p.line).inline_code_bg(p.input);
    // A mention draws as a translucent blue pill with periwinkle text, the way Fluxer's web app
    // marks them: the message behind stays visible through the background.
    let pill_text = oklch(0.87, 0.06, 275.0);
    let pill_bg = oklch(0.55, 0.20, 262.0).alpha(0.30);
    let mid = m.id.clone();
    let pill = Markdown::new(src.into_owned()).selectable(true)
        .on_link_tx(move |s: &mut State, url: &str, tx: &mut rune::Tx| open_mention(s, tx, &mid, url))
        .link_style(move |url: &str| url.strip_prefix("fluxer-mention:").map(|_| (
            rune::Span { color: Some(pill_text), weight: Some(500.0), bg: Some(pill_bg), bg_pad: 3.0, ..Default::default() },
            rune::LinkPaint::new().bg(oklch(0.60, 0.20, 262.0).alpha(0.45)).color(pill_text),
        )));
    pill.render(&style).key(format!("md-{}", m.id))
}

/// A link press: a mention opens the person's profile card beside this message,
/// anything else opens in the browser.
fn open_mention(s: &mut State, tx: &mut rune::Tx, mid: &str, url: &str) {
    if let Some(id) = url.strip_prefix("fluxer-mention:") {
        if let Some(u) = s.chat.roster.users.get(id).cloned() {
            // A mention is a span inside the text, with no element to anchor beside: put the
            // card just past the pill's right edge, from the click and the name's width.
            let p = s.pointer.get();
            if p == [0.0, 0.0] { crate::show_profile(s, &u, &format!("md-{mid}")) }
            else {
                let width = 7.2 * format!("@{}", name_in(s, &u)).chars().count() as f32 + 8.0;
                crate::show_profile_at(s, &u, "mention-anchor", (p[0] + width * 0.5, p[1] - 10.0))
            }
        }
        return;
    }
    let _ = tx.open_url(url);
}

/// What someone is called here: their nickname in the open server, else their own name.
fn name_in(s: &State, user: &User) -> String {
    s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g))
        .and_then(|g| s.chat.roster.nick(&g.id, &user.id)).map_or_else(|| user.display().to_string(), String::from)
}

/// Makes `e` open the profile card of `user` beside itself when pressed.
fn opens_profile(e: El, user: &User, key: String) -> El {
    let u = user.clone();
    let k = key.clone();
    e.key(key).cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| crate::show_profile(s, &u, &k))
}

fn open_user_menu(s: &mut State, user: &User, p: rune::PointerPress) { s.ctx = Some(crate::Ctx { user: user.clone(), x: p.x, y: p.y }) }

/// Which loaded message a quote points at: its own id when the server sent one, else the
/// loaded message by the same author with the same text (older payloads carry no id).
fn quote_target(s: &State, r: &Quoted) -> Option<String> {
    if let Some(id) = &r.id { return Some(id.clone()) }
    s.chat.log()?.msgs.iter().find(|q| q.author.id == r.author.id && q.body == r.content).map(|q| q.id.clone())
}

fn jump_at(id: String) -> impl Fn(&mut State, &mut rune::Tx) {
    move |s: &mut State, tx: &mut rune::Tx| jump_to(s, tx, &id)
}

/// The little icon of a role, with the role's name as its hover chip.
fn role_icon_el(s: &State, role_id: &str, hash: &Option<String>, emoji: &Option<String>, name: &str) -> Option<El> {
    let pic = if let Some(h) = hash {
        let url = s.chat.media.as_deref().map(|m| api::role_icon_url(m, role_id, h, 64))?;
        el().size(16.0, 16.0).radius(4.0).pixels(s.net.image(&url)?.pixels())
    } else {
        text(emoji.clone()?).font(rune::tok::TextStyle::new(15.0, 400.0))
    };
    Some(pic.help(name.to_string()))
}

/// Their topmost role icon in the open server, or nothing.
fn top_role_el(s: &State, user: &User) -> Option<El> {
    let g = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g))?;
    let (role_id, name, icon, emoji) = s.chat.roster.top_role_icon(&g.id, &user.id)?;
    role_icon_el(s, &role_id, &icon, &emoji, &name)
}

/// The colour of a name: their role's colour in this server, plain text otherwise.
fn name_color(s: &State, user_id: &str) -> Color {
    let guild = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g));
    guild.and_then(|g| s.chat.roster.name_color(&g.id, user_id))
        .and_then(|c| rune::color::hex(&format!("#{:06x}", c & 0xFF_FFFF)))
        .unwrap_or(tok::theme(true).text)
}

fn reply_bar(s: &State, m: &Msg) -> Option<El> {
    let (p, t) = (pal(), tok::theme(true));
    // The elbow that ties the quote to the message below: it turns from the quote
    // line and drops under the avatar column of the message beneath.
    let arc = el().size(26.0, 22.0)
        .shape(rune::Shape::new().stroke(vec![[26.0, 11.0], [16.0, 11.0], [11.0, 11.0], [7.0, 13.0], [5.0, 17.0], [5.0, 22.0]], 1.5, p.line));
    let inner = match &m.reply {
        Some(r) => {
            // Only the people the quote mentions need a name, not everyone the roster has ever seen.
            let names: Vec<(String, String)> = crate::richtext::mentioned_ids(&r.content).into_iter()
                .filter_map(|id| s.chat.roster.users.get(id)).map(|u| (u.id.clone(), name_in(s, u))).collect();
            let what = if r.content.trim().is_empty() && r.has_attachments { "Anexo".to_string() }
                else { crate::richtext::preview(&crate::richtext::prepare(&r.content, &names).replace('*', ""), 120) };
            row().gap(6.0).center_y()
                .child(rune::icon(&CHAT_CIRCLE, 14.0).color(t.text_secondary))
                .child(text(name_in(s, &r.author)).font(type_::LABEL).color(name_color(s, &r.author.id)))
                .child(text(what).font(type_::LABEL).color(t.text_secondary).truncate().grow(1.0).min_w(0.0))
        }
        None if m.kind == 19 => row().center_y().child(text("Mensagem original indisponível").font(type_::LABEL).color(t.text_secondary)),
        None => return None,
    };
    let bar = row().key(format!("reply-{}", m.id)).w_full().h(22.0).center_y().child(arc).child(inner.px(8.0));
    match m.reply.as_ref().and_then(|r| quote_target(s, r)) {
        Some(id) => Some(bar.cursor(rune::Cursor::Pointer).on_press_tx(jump_at(id))),
        None => Some(bar),
    }
}

fn embed_card(s: &State, key: String, e: &Embed) -> El {
    let (p, t) = (pal(), tok::theme(true));
    if e.gif && let Some(u) = &e.image {
        let animated = crate::model::embed_image_url(u, true);
        let img = playing(s, &animated, &crate::model::embed_image_url(u, false));
        let (w, h) = img.as_ref().and_then(|i| i.size).map(|[w, h]| (w as f32, h as f32))
            .or(e.image_size.map(|(w, h)| (w as f32, h as f32))).unwrap_or((GIF_MAX, 200.0));
        let k = (GIF_MAX / w).min(GIF_MAX / h).min(1.0);
        let gif = gif_view(key, (w * k).max(40.0), (h * k).max(40.0), img.as_ref(), animated);
        return match e.url.clone() { Some(u) => gif.cursor(rune::Cursor::Pointer).on_press_tx(move |_: &mut State, tx: &mut rune::Tx| { let _ = tx.open_url(&u); }), None => gif };
    }
    // A link's thumbnail is the small square on the right; a big picture or a video takes the
    // whole width instead, so the two never show together.
    let thumb = e.thumbnail.as_deref().filter(|_| e.image.is_none()).map(|u| {
        let square = el().pin(None, Some(14.0), Some(12.0), None).size(80.0, 80.0).radius(radius::SM).clip_rounded(radius::SM);
        match s.net.image(&api::attachment_image_url(u, 160)) {
            Some(img) => square.pixels(img.pixels()), None => square.fill(p.input),
        }
    });
    let mut body = col().gap(4.0).pl(16.0).pr(if thumb.is_some() { 108.0 } else { 12.0 }).py(12.0);
    if let Some(v) = e.provider.as_ref().or(e.author.as_ref()) { body = body.child(text(v.clone()).font(type_::LABEL).color(t.text_secondary)) }
    if let Some(v) = &e.title { body = body.child(text(v.clone()).font(type_::TITLE).color(p.link).wrap()) }
    if let Some(v) = &e.description { body = body.child(text(crate::richtext::preview(v, 280)).font(type_::BODY).color(t.text).wrap()) }
    if let Some(u) = &e.image {
        let img = s.net.image(&api::attachment_image_url(u, crate::model::EMBED_FETCH_WIDTH));
        let (w, h) = img.and_then(|i| i.size()).map_or((492.0, 260.0), |[w, h]| (w as f32, h as f32));
        let pic_h = (492.0 * h / w.max(1.0)).clamp(80.0, 320.0);
        let pic = el().w_full().h(pic_h).radius(radius::SM);
        let pic = match img { Some(i) => pic.pixels(i.pixels()), None => pic.fill(p.input) };
        body = body.child(if e.video { col().child(pic).child(text("▶ vídeo").font(type_::LABEL).color(t.text_secondary)) } else { col().child(pic) });
    }
    // The 4-point rail on the left is the embed's colour, like the web app's.
    let accent = e.color.and_then(|c| rune::color::hex(&format!("#{:06X}", c & 0xFF_FFFF))).unwrap_or(p.line);
    let url = e.url.clone();
    let mut card = col().key(key).w_full().max_w(516.0).radius(radius::SM).fill(p.side).border(1.0, p.line).clip_rounded(radius::SM)
        .child(el().pin(Some(0.0), Some(0.0), None, Some(0.0)).w(4.0).fill(accent))
        .child(body);
    if let Some(t) = thumb { card = card.child(t) }
    match url { Some(u) => card.cursor(rune::Cursor::Pointer).on_press_tx(move |_: &mut State, tx: &mut rune::Tx| { let _ = tx.open_url(&u); }), None => card }
}

/// `prev` is the message above, used to group consecutive messages and to place day separators.
fn message(s: &State, m: &Msg, prev: Option<&Msg>) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let now = crate::clock::now_ms();
    let new_day = m.ts.is_some() && prev.is_none_or(|q| q.ts.map(crate::clock::day_of) != m.ts.map(crate::clock::day_of));
    let mut out = col().key(format!("m-{}", m.id)).w_full();
    if new_day {
        let label = crate::clock::day_label(m.ts.unwrap_or(now), now);
        out = out.child(row().w_full().px(16.0).py(8.0).gap(12.0).center_y()
            .child(el().grow(1.0).h(1.0).fill(p.line)).child(text(label).font(type_::SECTION).color(t.text_secondary)).child(el().grow(1.0).h(1.0).fill(p.line)));
    }
    if m.kind == 7 {
        return out.child(row().w_full().px(16.0).py(6.0).gap(10.0).center_y()
            .child(el().size(8.0, 8.0).radius(4.0).fill(tok::accent(4, true)))
            .child(text(format!("{} entrou no servidor", m.author.display())).font(type_::BODY).color(t.text_secondary))
            .child(text(m.ts.map(crate::clock::hhmm).unwrap_or_default()).font(type_::LABEL).color(t.text_secondary.alpha(0.6))));
    }
    if is_welcome(m) {
        return out.child(row().w_full().px(16.0).py(6.0).gap(10.0).center_y()
            .child(rune::icon(&ARROW_IN, 16.0).color(oklch(0.72, 0.17, 150.0)))
            .child(text(m.body.trim().to_string()).font(type_::BODY).color(t.text_secondary))
            .child(text(m.ts.map(|ts| crate::clock::full(ts, now)).unwrap_or_default()).font(type_::LABEL).color(t.text_secondary.alpha(0.75))));
    }
    let marker = s.chat.channel().and_then(|c| c.last_read.as_deref()).is_some_and(|r| {
        m.delivery == Delivery::Sent && snowflake_cmp(&m.id, r).is_gt() && prev.is_some_and(|q| snowflake_cmp(&q.id, r).is_le())
    }) && s.chat.me.as_ref().is_none_or(|me| me.id != m.author.id);
    if marker {
        out = out.child(row().w_full().px(16.0).py(6.0).gap(8.0).center_y()
            .child(el().grow(1.0).h(1.0).fill(p.unread))
            .child(row().h(18.0).px(8.0).radius(9.0).fill(p.unread).center().child(text("NOVAS").font(type_::SECTION).color(t.knob))));
    }
    let follow = !new_day && !marker && prev.is_some_and(|q| {
        q.author.id == m.author.id && q.kind == 0 && m.kind == 0 && m.reply.is_none()
            && q.ts.zip(m.ts).is_none_or(|(a, b)| (b - a).abs() < FOLLOW_WINDOW_MS)
    });
    // A message that pings this account (or everyone) gets the yellow bar and wash.
    let pinged = s.chat.me.as_ref().is_some_and(|me| m.mentions.iter().any(|u| u.id == me.id)) || m.body.contains("@everyone");
    // The message jumped to from a quote click flashes blue for three seconds.
    let flashed = s.highlighted.as_ref().is_some_and(|(id, until)| id == &m.id && std::time::Instant::now() < *until);
    let mut body = col().gap(2.0).grow(1.0).min_w(0.0);
    if !follow {
        let mut head = row().gap(8.0).center_y()
            .child({ let (u, key) = (m.author.clone(), format!("av-{}", m.id)); opens_profile(text(name_in(s, &m.author)).font(type_::TITLE).color(name_color(s, &m.author.id)).on_secondary_press(move |s: &mut State, p, _| open_user_menu(s, &u, p)), &m.author, format!("pn-{}", m.id)) });
        if let Some(ic) = top_role_el(s, &m.author) { head = head.child(ic) }
        if let Some(ts) = m.ts { head = head.child(text(crate::clock::full(ts, now)).font(type_::LABEL).color(t.text_secondary.alpha(0.75))) }
        if m.delivery == Delivery::Pending { head = head.child(text("enviando…").font(type_::LABEL).color(t.text_secondary)) }
        body = body.child(head);
    } else if m.delivery == Delivery::Pending {
        body = body.child(text("enviando…").font(type_::LABEL).color(t.text_secondary));
    }
    if !m.body.is_empty() && !is_bare_gif_link(m) {
        let mut line = row().gap(6.0).child(message_text(s, m, m.delivery != Delivery::Sent).grow(1.0).min_w(0.0));
        if m.edited { line = line.child(text("(editado)").font(type_::LABEL).color(t.text_secondary.alpha(0.6))) }
        body = body.child(line);
    } else if m.attachments.is_empty() && m.embeds.is_empty() && m.kind != 0 && m.kind != 19 {
        body = body.child(text("Mensagem do sistema").font(type_::LABEL).color(t.text_secondary));
    }
    for a in &m.attachments { body = body.child(attachment(s, a)) }
    for (i, e) in m.embeds.iter().enumerate() { body = body.child(embed_card(s, format!("emb-{}-{i}", m.id), e)) }
    if let Delivery::Failed(why) = &m.delivery {
        let id = m.id.clone();
        body = body.child(text(format!("Falhou: {why} — clique para tentar de novo")).key(format!("retry-{}", m.id))
            .font(type_::LABEL).color(p.unread).wrap()
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| retry(s, &id)));
    }
    let lead = match (follow, m.ts) {
        (true, Some(ts)) => row().w(40.0).h(20.0).center().child(text(crate::clock::hhmm(ts)).font(type_::LABEL).color(t.text_secondary.alpha(0.0))),
        (true, None) => el().size(40.0, 1.0),
        (false, _) => { let (u, key) = (m.author.clone(), format!("av-{}", m.id)); opens_profile(row().child(avatar(s, &m.author, 40.0)).on_secondary_press(move |s: &mut State, p, _| open_user_menu(s, &u, p)), &m.author, key.clone()) }
    };
    // The elbow sits under the avatar column (which spans 16..56 on this row).
    if let Some(bar) = reply_bar(s, m) { out = out.child(row().w_full().px(16.0).py(4.0).child(el().w(15.0)).child(bar)) }
    // Reactions under the text, one pill per emoji; pressing one adds or takes back this account's.
    if !m.reactions.is_empty() {
        let mut pills = row().gap(6.0).wrap();
        for r in &m.reactions {
            let (id, key) = (m.id.clone(), r.key.clone());
            let glyph = if r.id.is_some() { format!(":{}:", r.name) } else { r.name.clone() };
            pills = pills.child(row().key(format!("rx-{}-{}", m.id, r.key)).h(26.0).px(8.0).gap(6.0).center_y().radius(8.0)
                .fill(if r.me { oklcha(0.58, 0.19, 277.0, 0.25) } else { p.input })
                .border(1.0, if r.me { oklch(0.58, 0.19, 277.0) } else { oklcha(0.0, 0.0, 0.0, 0.0) })
                .hover(if r.me { oklcha(0.58, 0.19, 277.0, 0.35) } else { p.select })
                .child(text(glyph).font(rune::tok::TextStyle::new(16.0, 400.0)).color(t.text))
                .child(text(r.count.to_string()).font(type_::LABEL).color(if r.me { t.text } else { t.text_secondary }))
                .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| toggle_reaction(s, &id, &key)));
        }
        body = body.child(pills);
    }
    let reactable = m.kind == 0 && m.delivery == Delivery::Sent;
    let over = reactable && (s.msg_hover.as_deref() == Some(m.id.as_str()) || s.react_picker.as_deref() == Some(m.id.as_str()));
    let id = m.id.clone();
    let mut line = row().w_full().px(16.0).pt(if follow { 1.0 } else { 12.0 }).pb(if follow { 1.0 } else { 3.0 }).gap(14.0).hover(p.hover).child(lead).child(body)
        .on_hover_change(move |s: &mut State, inside, _| {
            if inside { s.msg_hover = Some(id.clone()) } else if s.msg_hover.as_deref() == Some(id.as_str()) { s.msg_hover = None }
        });
    if flashed {
        line = line.fill(oklcha(0.62, 0.20, 255.0, 0.12))
            .child(el().pin(Some(0.0), Some(0.0), None, Some(0.0)).w(4.0).fill(oklch(0.62, 0.20, 255.0)));
    } else if pinged {
        line = line.fill(oklcha(0.82, 0.16, 90.0, 0.10))
            .child(el().pin(Some(0.0), Some(0.0), None, Some(0.0)).w(4.0).fill(oklch(0.85, 0.19, 90.0)));
    }
    if over { line = line.child(el().pin(None, Some(0.0), Some(16.0), None).child(compose::reaction_bar(s, &m.id))) }
    out.child(line)
}

/// A message that is only the link of a GIF it embeds: the picture alone is shown, not the URL.
fn is_bare_gif_link(m: &Msg) -> bool {
    m.embeds.iter().any(|e| e.gif && e.image.is_some())
        && m.body.split_whitespace().next().is_some()
        && m.body.split_whitespace().all(|w| w.starts_with("http://") || w.starts_with("https://"))
}

/// Asks for the next older page when the first visible row gets within three of the top of
/// the loaded history. `loading` keeps one page in flight at a time.
fn maybe_load_older(s: &mut State, first_visible: usize) {
    if first_visible > 3 { return }
    let Some(log) = s.chat.log() else { return };
    if log.has_more && !log.loading { load_older(s) }
}

fn message_list(s: &State) -> El {
    let t = tok::theme(true);
    let n = s.chat.log().map_or(0, |l| l.msgs.len());
    let cid = s.chat.channel_id().unwrap_or("none").to_string();
    let first = s.chat.log().and_then(|l| l.msgs.first()).map(|m| m.id.clone());
    let rows = col().key("rows").growth(GrowthSpec::default())
        .virtual_rows(n, 56.0,
            |s: &State, i| s.chat.log().map_or("", |l| l.msgs[i].id.as_str()).into(),
            |s: &State, i| match s.chat.log() {
                Some(l) => message(s, &l.msgs[i], i.checked_sub(1).map(|j| &l.msgs[j])),
                None => el(),
            })
        .virtual_rows_version((cid.clone(), first))
        // Scrolling up near the first message asks for the next page by itself.
        .on_visible_rows(|s: &mut State, rows| {
            maybe_load_older(s, rows.start);
            // The animated pictures of what is on screen are asked for now, and those that left it let go.
            if s.visible != rows { s.visible = rows; crate::reconcile(s) }
            false
        });
    let mut top = col();
    if let Some(l) = s.chat.log() {
        if !l.has_more && l.loaded && n == 0 {
            top = top.child(row().w_full().h(60.0).center().child(text("Nenhuma mensagem aqui ainda.").font(type_::BODY).color(t.text_secondary)));
        }
    }
    // The message box floats over the list, so a spacer under the rows keeps its height clear at
    // the bottom and the last message still scrolls up above it.
    let composer_room = if typing_notice(s).is_some() { 94.0 } else { 72.0 };
    let content = col().w_full().child(rows).child(el().w_full().h(composer_room));
    // A fresh scroll area per channel, so each one opens at its newest message.
    col().grow(1.0).min_h(0.0).child(top)
        .child(col().key(format!("scroll-{cid}")).scroll(ScrollSpec { stick_bottom: true }).grow(1.0).min_h(0.0).child(content))
}

fn status_color(status: &str) -> Color {
    match status {
        "online" => tok::accent(4, true),
        "idle" => tok::accent(3, true),
        "dnd" => pal().unread,
        _ => oklch(0.5, 0.01, 265.0),
    }
}

fn member_row(s: &State, user: &api::User, nick: Option<&str>, status: &str, owner: bool) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let offline = status == "offline";
    let face = row().size(32.0, 32.0).child(avatar(s, user, 32.0))
        .child(el().absolute(21.0, 21.0).size(12.0, 12.0).radius(6.0).fill(status_color(status)).border(2.0, p.side));
    let name = text(nick.unwrap_or_else(|| user.display()).to_string()).font(rune::tok::TextStyle::new(15.0, 500.0))
        .color(if offline { t.text_secondary } else { t.text }).truncate().min_w(0.0);
    let mut line = row().grow(1.0).min_w(0.0).gap(6.0).center_y().child(name);
    if let Some(ic) = top_role_el(s, user) { line = line.child(ic) }
    if owner { line = line.child(rune::icon(&CROWN, 14.0).color(oklch(0.78, 0.13, 85.0))) }
    let (u, key) = (user.clone(), format!("mem-{}", user.id));
    opens_profile(row().h(44.0).px(8.0).gap(12.0).center_y().radius(radius::SM).hover(p.hover)
        .child(face.opacity(if offline { 0.5 } else { 1.0 }))
        .child(line)
        .on_secondary_press({ let u = u.clone(); move |s: &mut State, p, _| open_user_menu(s, &u, p) }), user, key.clone())
}

fn members_panel(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let guild = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g));
    let owner = guild.and_then(|g| g.owner_id.as_deref());
    let mut list = col().px(12.0).py(12.0).gap(2.0);
    match guild.and_then(|g| s.chat.roster.lists.get(&g.id)) {
        Some(rows) if !rows.is_empty() => for r in rows {
            list = list.child(match r {
                Row::Group { id, count } => row().h(28.0).px(8.0).pt(8.0).center_y()
                    .child(text(format!("{}—{count}", s.chat.roster.group_label(id))).font(rune::tok::TextStyle::new(13.0, 600.0)).color(t.text_secondary)),
                Row::Member { user, nick, status } => member_row(s, user, nick.as_deref(),
                    s.chat.roster.status.get(&user.id).map_or(status.as_str(), String::as_str), owner == Some(user.id.as_str())),
            });
        },
        _ => list = list.child(row().h(40.0).px(10.0).center_y().child(text("Carregando membros…").font(type_::LABEL).color(t.text_secondary))),
    }
    col().w(members_w(s)).h_full().fill(p.side).child(list.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
        .child(resize_handle("resize-members", false, members_w(s), |s, w| s.prefs.members_w = w.clamp(200.0, 420.0) as u32).pin(Some(0.0), Some(0.0), None, Some(0.0)))
}

/// The "someone is typing" line, or nothing.
fn typing_notice(s: &State) -> Option<String> {
    let me = s.chat.me.as_ref().map(|u| u.id.as_str());
    s.chat.channel_id().and_then(|c| roster::typing_text(&s.chat.roster.typing_names(c, me, crate::clock::now_ms())))
}

fn typing_bar(s: &State) -> El {
    let t = tok::theme(true);
    let Some(msg) = typing_notice(s) else {
        // Nobody typing takes no room at all.
        return el().size(0.0, 0.0);
    };
    row().w_full().h(22.0).px(20.0).center_y().child(text(msg).key("typing").font(type_::LABEL).color(t.text_secondary))
}

/// The stage above the text chat of a voice channel: a "join" card, or, once in the call, everyone's
/// picture and the call controls.
fn voice_panel(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let Some(ch) = s.chat.voice_view.and_then(|_| s.chat.channel()) else { return el().size(0.0, 0.0) };
    let clear = oklcha(0.0, 0.0, 0.0, 0.0);
    let stage = col().w_full().h(300.0).fill(oklch(0.0, 0.0, 0.0)).center().gap(24.0);
    let here = s.call.channel_id.as_deref() == Some(ch.id.as_str()) && s.call.active();
    if !here {
        let id = ch.id.clone();
        return stage.child(col().center().gap(10.0)
            .child(row().size(48.0, 48.0).radius(24.0).fill(p.select).center().child(rune::icon(&SPEAKER, 24.0).color(t.text)))
            .child(text(ch.name.clone()).font(type_::TITLE).color(t.text))
            .child(row().key("voice-join").h(36.0).px(14.0).radius(radius::SM).center().fill(p.brand).hover(p.brand)
                .child(text("Join voice channel").font(type_::TITLE).color(t.knob))
                .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| join_voice(s, &id))));
    }
    let mut faces = row().gap(12.0).center_y();
    for v in sorted_voice(s, s.chat.voice.get(&ch.id)) {
        let Some(user) = s.chat.roster.users.get(&v.user_id) else { continue };
        let speaking = s.call.speaking.contains(&v.user_id);
        let mut f = row().size(76.0, 76.0).radius(38.0).center().instant_changes()
            .border(3.0, if speaking { GREEN() } else { clear }).child(avatar(s, user, 68.0));
        if v.muted() {
            let icon = if v.deafened() { &HEADSET_OFF } else { &MIC_OFF };
            f = f.child(row().absolute(52.0, 52.0).size(24.0, 24.0).radius(12.0).fill(p.unread).border(2.0, oklch(0.0, 0.0, 0.0)).center()
                .child(rune::icon(icon, 13.0).color(t.knob)));
        }
        faces = faces.child(opens_profile(f, user, format!("vf-{}", v.user_id)));
    }
    let btn = |key: &str, danger: bool, icon: &'static rune::Icon, f: fn(&mut State)| {
        row().key(key.to_string()).size(44.0, 40.0).radius(radius::SM).center()
            .fill(if danger { oklcha(0.64, 0.2, 25.0, 0.22) } else { p.select }).hover(p.hover)
            .child(rune::icon(icon, 20.0).color(if danger { p.unread } else { t.text }))
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s))
    };
    let c = &s.call;
    let bar = row().gap(6.0).px(6.0).py(6.0).radius(14.0).fill(oklch(0.2, 0.008, 265.0)).center_y()
        .child(btn("vp-mic", c.muted || c.deafened, if c.muted || c.deafened { &MIC_OFF } else { &MIC }, toggle_mute))
        .child(btn("vp-deafen", c.deafened, if c.deafened { &HEADSET_OFF } else { &HEADSET }, toggle_deafen))
        .child(row().key("vp-leave").size(44.0, 40.0).radius(radius::SM).center().fill(p.unread).hover(p.unread)
            .child(rune::icon(&HANG_UP, 20.0).color(t.knob)).cursor(rune::Cursor::Pointer).on_press(|s: &mut State| leave_voice(s)));
    stage.child(faces).child(bar)
}

fn main_view(s: &State, stage: Option<[f32; 2]>) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let prefix = if s.chat.nav.guild.is_some() { "#" } else { "@" };
    let mosaic_channel = if s.call.view && s.call.active() { s.call.channel_id.as_deref().and_then(|id| s.chat.voice_channel(id)).map(|(_, c)| c.name.clone()) } else { None };
    let name = mosaic_channel.clone().unwrap_or_else(|| s.chat.channel().map_or(String::new(), |c| c.name.clone()));
    let in_guild = s.chat.nav.guild.is_some();
    let mut header = row().h(48.0).px(16.0).gap(8.0).center_y()
        .child(if s.chat.voice_view.is_some() || mosaic_channel.is_some() { rune::icon(&SPEAKER, 20.0).color(t.text_secondary) } else { text(prefix).font(type_::TITLE).color(t.text_secondary) })
        .child(text(name.clone()).font(type_::TITLE).color(t.text).grow(1.0));
    if in_guild {
        header = header.child(text("Membros").key("toggle-members").font(type_::LABEL).color(if s.show_members { t.text } else { t.text_secondary })
            .px(10.0).py(6.0).radius(radius::SM).fill(if s.show_members { p.select } else { oklcha(0.0, 0.0, 0.0, 0.0) }).hover(p.select)
            .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| s.show_members = !s.show_members));
    }
    // The message box floats over the bottom of the chat, so the messages scroll behind its
    // frosted glass instead of stopping dead above a solid strip.
    let composer = compose::composer(s).pin(Some(0.0), None, Some(0.0), Some(0.0));
    let mosaic = s.call.view && s.call.active();
    let main = if mosaic {
        col().grow(1.0).h_full().min_w(0.0).fill(p.chat).child(header).child(el().w_full().h(1.0).fill(p.line)).child(mosaic::view(s, stage))
    } else {
        col().grow(1.0).h_full().min_w(0.0).fill(p.chat).child(header).child(el().w_full().h(1.0).fill(p.line))
            .child(voice_panel(s)).child(message_list(s)).child(composer)
    };
    let mut all = row().w_full().h_full().fill(p.rail).child(guild_rail(s)).child(channel_list(s)).child(main);
    if in_guild && s.show_members { all = all.child(members_panel(s)) }
    for e in compose::layers(s) { all = all.child(e) }
    // A mention's card anchors beside this marker at the point the pointer was on.
    let mention_anchor = s.profile.as_ref().and_then(|p| p.at)
        .map_or_else(|| el().size(0.0, 0.0), |(x, y)| el().key("mention-anchor").absolute(x, y).size(1.0, 1.0));
    all.child(menu_scrim(s)).child(user_menu_scrim(s)).child(profile_scrim(s)).child(dock(s)).child(audio_menu(s)).child(ctx_point(s)).child(user_menu(s)).child(mention_anchor).child(profile_card(s)).child(settings::dialog(s))
        .on_pointer_move_local(|s: &mut State, p, tx| { s.pointer.set([p.window_x, p.window_y]); tx.live_only() })
        .on_window_focus(|s: &mut State, focused, _| set_focus(s, focused))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Log;

    fn msg(author: User, body: &str) -> Msg {
        Msg::local("x".into(), author, body.into(), crate::model::Delivery::Sent)
    }

    #[test]
    fn a_quote_without_an_id_falls_back_to_author_and_text() {
        let ana = User { id: "2".into(), username: "ana".into(), global_name: None, avatar: None, discriminator: None };
        let mut s = State::default();
        s.chat.logs.insert("demo-0".into(), Log { msgs: vec![{ let mut m = msg(ana.clone(), "oi"); m.id = "old".into(); m }], loaded: true, loading: false, has_more: false });
        let with_id = Quoted { id: Some("new".into()), author: ana.clone(), content: "oi".into(), has_attachments: false };
        assert_eq!(quote_target(&s, &with_id).as_deref(), Some("new"), "the server's own id wins");
        let without_id = Quoted { id: None, author: ana.clone(), content: "oi".into(), has_attachments: false };
        assert_eq!(quote_target(&s, &without_id).as_deref(), Some("old"), "older payloads match by author and text");
        let missing = Quoted { id: None, author: ana.clone(), content: "não está carregado".into(), has_attachments: false };
        assert_eq!(quote_target(&s, &missing), None, "an unloaded original has nowhere to jump to");
    }

    #[test]
    fn nearing_the_top_asks_for_older_messages_once() {
        let mut s = State::default();
        let cid = s.chat.channel_id().expect("a demo channel is open").to_string();
        let l = s.chat.logs.get_mut(&cid).unwrap();
        l.loaded = true;
        l.has_more = true;
        l.loading = false;
        maybe_load_older(&mut s, 10);
        assert!(!s.chat.log().unwrap().loading, "far from the top, no page is asked for");
        maybe_load_older(&mut s, 2);
        assert!(s.chat.log().unwrap().loading, "near the top, the older page is asked for");
        maybe_load_older(&mut s, 0);
        assert!(s.chat.log().unwrap().loading, "a page already in flight is not asked for twice");
    }

    #[test]
    fn only_fluxers_welcome_line_is_a_welcome() {
        let fluxer = User { id: "sys".into(), username: "fluxer".into(), global_name: Some("Fluxer".into()), avatar: None, discriminator: None };
        let lion = User { id: "1".into(), username: "lion".into(), global_name: None, avatar: None, discriminator: None };
        assert!(is_welcome(&msg(fluxer.clone(), "Welcome, Lion!")));
        assert!(!is_welcome(&msg(lion.clone(), "Welcome, Lion!")), "a person saying the same words is a normal message");
        assert!(!is_welcome(&msg(fluxer, "hello")), "other Fluxer text is not the welcome line");
        assert!(!is_welcome(&msg(lion, "Welcome back")), "no trailing batch mark, no welcome");
    }
}

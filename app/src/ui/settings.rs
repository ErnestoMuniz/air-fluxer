//! The settings dialog (opened by the gear in the dock). Only the Profile page exists so far, so
//! the sidebar holds just the profile card and the sign-out/footer block.
//!
//! The look follows Fluxer's own settings page: a wide card over a dimmed app, a quiet sidebar
//! with its own icons, and a centred 730 point column holding the form and the live preview.

use super::*;

// ---------------------------------------------------------------- icons

static X: rune::Icon = rune::Icon { name: "app-set-x", view: 24.0, body: r#"<path d="M6 6l12 12M18 6L6 18" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"# };
static CHEV_DOWN: rune::Icon = rune::Icon { name: "app-set-chev-down", view: 24.0, body: r#"<path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static LINK: rune::Icon = rune::Icon { name: "app-set-link", view: 24.0, body: r#"<path d="M10 14a4.5 4.5 0 0 0 6.36 0l2.64-2.64a4.5 4.5 0 0 0-6.36-6.36L11.5 6.14" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/><path d="M14 10a4.5 4.5 0 0 0-6.36 0L5 12.64a4.5 4.5 0 0 0 6.36 6.36l1.14-1.14" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"# };
static ATOM: rune::Icon = rune::Icon { name: "app-set-atom", view: 24.0, body: r#"<ellipse cx="12" cy="12" rx="9" ry="3.8" stroke="currentColor" stroke-width="1.8" fill="none"/><ellipse cx="12" cy="12" rx="3.8" ry="9" stroke="currentColor" stroke-width="1.8" fill="none"/><circle cx="12" cy="12" r="1.9" fill="currentColor"/>"# };
static PENCIL: rune::Icon = rune::Icon { name: "app-set-pencil", view: 24.0, body: r#"<path d="M4 20.1l4.4-1.1L19.6 7.8a2.2 2.2 0 0 0-3.1-3.1L5.3 15.9z" fill="currentColor"/><path d="M14.6 6.5l3.1 3.1" stroke="currentColor" stroke-width="1.7" fill="none"/>"# };
static SMILE: rune::Icon = rune::Icon { name: "app-set-smile", view: 24.0, body: r#"<circle cx="12" cy="12" r="8.6" stroke="currentColor" stroke-width="1.8" fill="none"/><circle cx="9" cy="10" r="1.3" fill="currentColor"/><circle cx="15" cy="10" r="1.3" fill="currentColor"/><path d="M8.4 14.2c1 1.9 6.2 1.9 7.2 0" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" fill="none"/>"# };
static CHECK: rune::Icon = rune::Icon { name: "app-set-check", view: 24.0, body: r#"<path d="M5 12.5l4.5 4.5L19 7.5" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static EYEDROP: rune::Icon = rune::Icon { name: "app-set-eyedrop", view: 24.0, body: r#"<path d="M17.8 2.9a2.6 2.6 0 0 1 3.7 3.7l-1.9 1.9 1 1-2.2 2.2-1-1-6.6 6.6-3.9 1 1-3.9 6.6-6.6-1-1 2.2-2.2 1 1z" fill="currentColor"/>"# };
static BUBBLE: rune::Icon = rune::Icon { name: "app-set-bubble", view: 24.0, body: r#"<path d="M4 5.4A2.6 2.6 0 0 1 6.6 2.8h10.8A2.6 2.6 0 0 1 20 5.4v8.2a2.6 2.6 0 0 1-2.6 2.6h-6.9l-4.4 3.6v-3.6h-.1A2.6 2.6 0 0 1 4 13.6z" fill="currentColor"/>"# };

// ---------------------------------------------------------------- palette

fn hx(s: &str) -> Color { rune::color::hex(s).unwrap_or(Color::BLACK) }
fn panel() -> Color { hx("#1A181E") }
fn sidebar_bg() -> Color { hx("#2A2730") }
fn field_bg() -> Color { hx("#2A2730") }
fn ink() -> Color { hx("#E0E0E2") }
fn dim() -> Color { hx("#9C98A5") }
fn accent() -> Color { hx("#413CDD") }
fn banner() -> Color { hx("#5F3C30") }
fn card_bg() -> Color { hx("#0C0B0E") }
fn line() -> Color { hx("#34313C") }
fn hover() -> Color { hx("#332F3B") }
fn pressed_row() -> Color { hx("#403D45") }

/// The kit for the form fields: the page's own greys instead of the theme's.
fn fields() -> Kit {
    let mut k = Kit::neutral(true);
    k.c.bg_base = field_bg();
    k.c.border_base = hx("#3A3642");
    k.c.border_focus = accent();
    k.c.text_base = ink();
    k.c.text_muted = dim();
    k.c.text_faint = dim();
    k.c.text_accent = accent();
    k.c.hover = hx("#322E3A");
    k
}

fn title(size: f32, weight: f32) -> rune::tok::TextStyle { rune::tok::TextStyle::new(size, weight) }

// ---------------------------------------------------------------- pieces

/// How the per-community avatar or banner is shown: the account's own, or none at all.
/// The custom-image choice is a Plutonium upsell and stays disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuildMedia {
    #[default]
    Global,
    Hide,
}

/// Which settings page the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Profile,
    Voice,
}

/// What the profile page keeps besides the text fields (`State::prof`).
#[derive(Default)]
pub struct Form {
    /// The server whose profile is edited (index into the guilds); None is the account's own.
    pub kind: Option<usize>,
    /// Which page the sidebar has selected.
    pub page: Page,
    pub type_open: bool,
    /// The values as saved: the fields differ from these exactly when there is something to save.
    pub base: [String; 4],
    pub color_open: bool,
    /// Hue (0..360), saturation and value (0..1) behind the colour picker.
    pub hsv: (f32, f32, f32),
    pub emoji_open: bool,
    pub saving: bool,
    /// An avatar upload or removal is in flight, and the time zone save, so the buttons say so.
    pub avatar_saving: bool,
    pub tz_saving: bool,
    pub tz_saved: bool,
    pub error: Option<String>,
    /// The person tried to leave with changes pending: the bar says so.
    pub nag: bool,
    /// Per-community avatar/banner choice (preview only; the server has no field for it yet).
    pub avatar_mode: GuildMedia,
    pub banner_mode: GuildMedia,
}

pub const BIO_MAX: i64 = 320;
const NAME_MAX: usize = 32;
const PRONOUNS_MAX: usize = 40;

fn guild_id(s: &State) -> Option<String> { s.pf.kind.and_then(|g| s.chat.guilds.get(g)).map(|g| g.id.clone()) }

fn profile_key(s: &State) -> Option<String> {
    let me = s.chat.me.as_ref()?;
    Some(crate::net::profile_key(&me.id, guild_id(s).as_deref()))
}

pub fn dirty(s: &State) -> bool { (0..4).any(|i| s.prof[i].text != s.pf.base[i]) }

/// "#RRGGBB" (the # is optional) as packed RGB.
fn parse_hex(t: &str) -> Option<u32> {
    let t = t.trim().trim_start_matches('#');
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_hexdigit()) { return None }
    u32::from_str_radix(t, 16).ok()
}

fn hex_of(c: u32) -> String { format!("#{c:06X}") }

fn hsv_to_rgb((h, sat, v): (f32, f32, f32)) -> u32 {
    let h = h.rem_euclid(360.0) / 60.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - sat), v * (1.0 - sat * f), v * (1.0 - sat * (1.0 - f)));
    let (r, g, b) = match i as i32 % 6 { 0 => (v, t, p), 1 => (q, v, p), 2 => (p, v, t), 3 => (p, q, v), 4 => (t, p, v), _ => (v, p, q) };
    let to = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u32;
    to(r) << 16 | to(g) << 8 | to(b)
}

fn rgb_to_hsv(c: u32, keep_hue: f32) -> (f32, f32, f32) {
    let (r, g, b) = ((c >> 16 & 255) as f32 / 255.0, (c >> 8 & 255) as f32 / 255.0, (c & 255) as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 { keep_hue } else if max == r { 60.0 * ((g - b) / d).rem_euclid(6.0) } else if max == g { 60.0 * ((b - r) / d + 2.0) } else { 60.0 * ((r - g) / d + 4.0) };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

/// The picker's colour: the state's own when the field still holds it, otherwise what the field says.
fn shown_hsv(s: &State) -> (f32, f32, f32) {
    match parse_hex(&s.prof[3].text) {
        Some(c) if c == hsv_to_rgb(s.pf.hsv) => s.pf.hsv,
        Some(c) => rgb_to_hsv(c, s.pf.hsv.0),
        None => s.pf.hsv,
    }
}

fn set_hsv(s: &mut State, hsv: (f32, f32, f32)) {
    s.pf.hsv = hsv;
    s.prof[3] = rune::TextEdit::new(hex_of(hsv_to_rgb(hsv)));
}

/// Why the form cannot be saved, if it cannot.
fn problem(s: &State) -> Option<String> {
    let bio = BIO_MAX - s.prof[2].text.chars().count() as i64;
    if bio < 0 { return Some(format!("About me is {} characters too long.", -bio)) }
    if s.prof[0].text.chars().count() > NAME_MAX { return Some(format!("The name can have up to {NAME_MAX} characters.")) }
    if s.prof[1].text.chars().count() > PRONOUNS_MAX { return Some(format!("Pronouns can have up to {PRONOUNS_MAX} characters.")) }
    if !s.prof[3].text.trim().is_empty() && parse_hex(&s.prof[3].text).is_none() { return Some("The accent colour must look like #4641D9.".into()) }
    None
}

fn set_form(s: &mut State, vals: [String; 4]) {
    for i in 0..4 { s.prof[i] = rune::TextEdit::new(vals[i].clone()) }
    s.pf.base = vals;
    s.pf.error = None;
}

/// Fills the form with what is saved for the profile being edited, and asks for it again.
fn load_values(s: &mut State) {
    let Some(key) = profile_key(s) else { return };
    let guild = guild_id(s);
    if let Some(me) = s.chat.me.clone() { s.net.profile(&me.id, guild.as_deref()) }
    fill_from(s, &key);
}

fn fill_from(s: &mut State, key: &str) {
    let Some(me) = s.chat.me.clone() else { return };
    let info = s.profiles.get(key).cloned();
    let vals = if s.pf.kind.is_none() {
        let part = info.map(|i| i.user_part).unwrap_or_default();
        [me.global_name.clone().unwrap_or_default(), part.pronouns.unwrap_or_default(), part.bio.unwrap_or_default(), part.accent.map(hex_of).unwrap_or_default()]
    } else {
        let (part, nick) = info.map(|i| (i.guild_part, i.nick.unwrap_or_default())).unwrap_or_default();
        [nick, part.pronouns.unwrap_or_default(), part.bio.unwrap_or_default(), part.accent.map(hex_of).unwrap_or_default()]
    };
    set_form(s, vals);
}

/// A profile arrived; it fills the form when it is the one being edited and nothing was typed yet.
pub fn on_profile_loaded(s: &mut State, key: &str) {
    if s.settings_open && !dirty(s) && profile_key(s).as_deref() == Some(key) { fill_from(s, key) }
}

/// Opens the dialog on the account's own profile.
pub fn open(s: &mut State) {
    if !s.settings_open && s.chat.me.is_some() {
        s.pf = Form::default();
        load_values(s);
        s.call.refresh_devices();
        s.call.meter(false);
    }
    s.settings_open = true;
}

/// Closing needs the changes saved or thrown away first; otherwise the bar points at its buttons.
fn close(s: &mut State) {
    if dirty(s) { s.pf.page = Page::Profile; s.pf.nag = true; return }
    s.settings_open = false;
    s.call.meter(false);
    s.pf = Form::default();
}

fn reset(s: &mut State) {
    let base = s.pf.base.clone();
    for i in 0..4 { s.prof[i] = rune::TextEdit::new(base[i].clone()) }
    s.pf.nag = false;
    s.pf.error = None;
}

fn save(s: &mut State) {
    if !dirty(s) || s.pf.saving { return }
    if let Some(e) = problem(s) { s.pf.error = Some(e); return }
    let opt = |t: &str| { let t = t.trim(); if t.is_empty() { serde_json::Value::Null } else { t.into() } };
    let color = parse_hex(&s.prof[3].text).map_or(serde_json::Value::Null, |c| c.into());
    let name_key = if s.pf.kind.is_none() { "global_name" } else { "nick" };
    let body = serde_json::json!({ name_key: opt(&s.prof[0].text), "pronouns": opt(&s.prof[1].text), "bio": opt(&s.prof[2].text), "accent_color": color });
    s.pf.saving = true;
    s.pf.error = None;
    let guild = guild_id(s);
    s.net.save_profile(guild.as_deref(), body);
}

/// The server answered the save.
pub fn on_saved(s: &mut State, guild: Option<String>, res: Result<serde_json::Value, String>) {    s.pf.saving = false;
    if guild != guild_id(s) { return }
    match res {
        Err(e) => s.pf.error = Some(e),
        Ok(v) => {
            s.pf.nag = false;
            if guild.is_none() && let Some(u) = fluxer_client::api::User::from_json(&v) {
                s.chat.roster.users.insert(u.id.clone(), u.clone());
                s.chat.me = Some(u);
            }
            if let (Some(g), Some(me)) = (&guild, &s.chat.me) {
                let nick = s.prof[0].text.trim().to_string();
                let id = me.id.clone();
                s.chat.roster.set_nick(g, &id, Some(&nick));
            }
            // What is shown next time is what the server has now.
            if let Some(key) = profile_key(s) { s.profiles.remove(&key); }
            s.pf.base = [s.prof[0].text.clone(), s.prof[1].text.clone(), s.prof[2].text.clone(), s.prof[3].text.clone()];
            if let Some(me) = s.chat.me.clone() { s.net.profile(&me.id, guild_id(s).as_deref()) }
        }
    }
}

/// The avatar upload/removal finished: the new picture comes from the returned user.
pub fn on_avatar(s: &mut State, res: Result<serde_json::Value, String>) {
    s.pf.avatar_saving = false;
    match res {
        Err(e) => s.pf.error = Some(format!("Não foi possível mudar o avatar: {e}")),
        Ok(v) => {
            s.pf.error = None;
            if let Some(u) = fluxer_client::api::User::from_json(&v) {
                s.chat.roster.users.insert(u.id.clone(), u.clone());
                s.chat.me = Some(u);
            }
            if let Some(me) = s.chat.me.clone() { s.net.profile(&me.id, guild_id(s).as_deref()) }
        }
    }
}

/// The profile time zone was saved.
pub fn on_timezone(s: &mut State, res: Result<serde_json::Value, String>) {
    s.pf.tz_saving = false;
    match res {
        Ok(_) => { s.pf.tz_saved = true; s.pf.error = None }
        Err(e) => { s.pf.tz_saved = false; s.pf.error = Some(format!("Não foi possível salvar o fuso: {e}")) }
    }
}

fn choose_type(s: &mut State, kind: Option<usize>) {
    s.pf.type_open = false;
    if s.pf.kind == kind { return }
    s.pf.kind = kind;
    s.pf.color_open = false;
    s.pf.emoji_open = false;
    s.pf.avatar_mode = GuildMedia::Global;
    s.pf.banner_mode = GuildMedia::Global;
    load_values(s);
}

fn label(s: &str) -> El { text(s.to_string()).font(title(15.0, 600.0)).color(ink()) }
fn note(s: &str, w: f32) -> El { text(s.to_string()).font(title(12.5, 400.0)).color(dim()).wrap().w(w) }

fn section_head(s: &str) -> El {
    row().gap(8.0).center_y()
        .child(text(s.to_string()).font(title(20.0, 700.0)).color(ink()))
        .child(rune::icon(&LINK, 14.0).color(dim()))
}

/// A button that does something; greyed out and inert when `enabled` is false.
fn action(key: &str, label: &str, primary: bool, enabled: bool, f: fn(&mut State)) -> El {
    let b = row().key(key.to_string()).h(36.0).px(14.0).radius(radius::SM).center()
        .fill(if primary { accent() } else { field_bg() })
        .border(1.0, if primary { accent() } else { hx("#3A3642") })
        .child(text(label.to_string()).font(title(14.0, 600.0)).color(if primary { Color::WHITE } else { ink() }));
    if enabled { b.cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s)) } else { b.opacity(0.45) }
}

fn input(edit: &rune::TextEdit, name: &str, place: &str, k: &Kit, f: fn(&mut State) -> &mut rune::TextEdit) -> El {
    TextInput::new(edit).width(Some(374.0)).height(42.0).label(name).placeholder(place).on_edit(f).build(k)
}

/// The card's live preview of the profile, following what is typed in the form.
fn preview(s: &State, me: &User) -> El {
    let status = s.chat.roster.status_of(&me.id);
    let typed = s.prof[0].text.trim();
    let name = if typed.is_empty() { me.display().to_string() } else { typed.to_string() };
    let tag = match &me.discriminator {
        Some(d) => format!("{}#{d}", me.username),
        None => me.username.clone(),
    };
    let color = parse_hex(&s.prof[3].text).and_then(|c| rune::color::hex(&hex_of(c))).unwrap_or_else(banner);
    let hide_avatar = s.pf.kind.is_some() && s.pf.avatar_mode == GuildMedia::Hide;
    let hide_banner = s.pf.kind.is_some() && s.pf.banner_mode == GuildMedia::Hide;
    let face = row().size(84.0, 84.0).radius(42.0).fill(color).center()
        .child(if hide_avatar {
            // "Don't show": the default avatar, ignoring the account's picture.
            let img = s.net.logo().cloned();
            match img {
                Some(img) => el().size(76.0, 76.0).radius(38.0).pixels(img.pixels()),
                None => row().size(76.0, 76.0).radius(38.0).fill(field_bg()).center()
                    .child(text(initial(me.display())).font(title(28.0, 700.0)).color(ink())),
            }
        } else { avatar(s, me, 76.0) });
    let dot = el().absolute(62.0, 62.0).size(22.0, 22.0).radius(11.0)
        .fill(status_color(if status == "offline" { "online" } else { status })).border(4.0, card_bg());
    let mut text_col = col().px(24.0).pt(44.0).pb(20.0).gap(2.0)
        .child(text(name).font(title(20.0, 700.0)).color(ink()).truncate().w(252.0))
        .child(text(tag).font(title(14.0, 400.0)).color(dim()).truncate().w(252.0));
    if !s.prof[1].text.trim().is_empty() { text_col = text_col.child(text(s.prof[1].text.trim().to_string()).font(title(13.0, 400.0)).color(dim()).truncate().w(252.0)) }
    if !s.prof[2].text.trim().is_empty() { text_col = text_col.child(row().h(6.0)).child(text(s.prof[2].text.trim().to_string()).font(title(13.0, 400.0)).color(ink()).wrap_anywhere().w(252.0).max_h(80.0)) }
    let key = profile_key(s);
    let joined = key.as_deref().and_then(|k| s.profiles.get(k)).and_then(|p| p.joined_ms);
    let mut since = row().gap(6.0).center_y()
        .child(el().size(16.0, 16.0).radius(8.0).fill(accent()))
        .child(text(member_since(&me.id)).font(title(14.0, 400.0)).color(ink()));
    if s.pf.kind.is_some() && let Some(ms) = joined {
        since = row().gap(6.0).center_y()
            .child(el().size(16.0, 16.0).radius(8.0).fill(accent()))
            .child(text(member_since(&me.id)).font(title(14.0, 400.0)).color(ink()))
            .child(el().w(12.0))
            .child(guild_icon(s, s.pf.kind.unwrap(), 16.0, 8.0))
            .child(text(fmt_ms(ms)).font(title(14.0, 400.0)).color(ink()));
    }
    text_col = text_col
        .child(row().h(12.0))
        .child(text("Member since").font(title(14.0, 700.0)).color(ink()))
        .child(since)
        .child(row().h(12.0))
        .child(row().h(38.0).radius(radius::SM).fill(hx("#272376")).center().gap(8.0)
            .child(rune::icon(&BUBBLE, 16.0).color(hx("#8E8AA8")))
            .child(text("Message").font(title(13.0, 600.0)).color(hx("#8E8AA8"))));
    let _ = hide_banner; // "Don't show" still shows the accent colour; only the banner picture goes.
    let card = col().w(300.0)
        .child(col().w_full().radius(radius::MD).border(2.0, color).fill(card_bg()).clip_rounded(radius::MD)
            .child(el().w_full().h(102.0).fill(color))
            .child(text_col))
        .child(el().absolute(24.0, 60.0).child(face).child(dot));
    col().w(300.0).gap(12.0)
        .child(row().w(300.0).center().child(text("Profile preview").font(title(14.0, 500.0)).color(dim())))
        .child(card)
}

/// The colour picker: a square for saturation and brightness, a bar for the hue, and a way to clear it.
fn color_picker(s: &State) -> El {
    const W: f32 = 218.0;
    const H: f32 = 160.0;
    let (h, sat, v) = shown_hsv(s);
    let hue_col = |h: f32| rune::color::hex(&hex_of(hsv_to_rgb((h, 1.0, 1.0)))).unwrap_or(Color::WHITE);
    let white = Color::WHITE;
    let black = Color::BLACK;
    let square = el().key("cp-square").w(W).h(H).radius(6.0).fill(hue_col(h)).clip_rounded(6.0)
        .child(el().absolute(0.0, 0.0).size(W, H).linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, white), (1.0, white.alpha(0.0))]))
        .child(el().absolute(0.0, 0.0).size(W, H).linear_gradient(std::f32::consts::PI, [(0.0, black.alpha(0.0)), (1.0, black)]))
        .child(el().absolute((sat * W - 8.0).clamp(-8.0, W - 8.0), ((1.0 - v) * H - 8.0).clamp(-8.0, H - 8.0)).size(16.0, 16.0).radius(8.0).border(2.0, white))
        .cursor(rune::Cursor::Pointer)
        .on_drag(move |s: &mut State, d: &mut rune::Drag, _| {
            let hue = shown_hsv(s).0;
            set_hsv(s, (hue, (d.x / d.w.max(1.0)).clamp(0.0, 1.0), 1.0 - (d.y / d.h.max(1.0)).clamp(0.0, 1.0)))
        });
    let stops: Vec<(f32, Color)> = (0..=6).map(|i| (i as f32 / 6.0, hue_col(i as f32 * 60.0))).collect();
    let bar = el().key("cp-hue").w(W).h(12.0).radius(6.0).linear_gradient(std::f32::consts::FRAC_PI_2, stops)
        .child(el().absolute((h / 360.0 * W - 10.0).clamp(0.0, W - 20.0), -4.0).size(20.0, 20.0).radius(10.0).border(3.0, white).fill(hue_col(h)))
        .cursor(rune::Cursor::Pointer)
        .on_drag(move |s: &mut State, d: &mut rune::Drag, _| {
            let (_, sat, v) = shown_hsv(s);
            set_hsv(s, ((d.x / d.w.max(1.0)).clamp(0.0, 1.0) * 360.0, sat, v))
        });
    col().key("color-picker").w(W + 36.0).px(18.0).py(18.0).gap(16.0).radius(10.0).fill(hx("#0C0B0E")).border(1.0, line())
        .shadow(6.0, 24.0, tok::theme(true).shadow).on_press(|_: &mut State| {})
        .child(square).child(bar)
        .child(row().key("cp-reset").h(28.0).center().radius(radius::SM).hover(hover())
            .child(text("Reset").font(title(14.0, 400.0)).color(dim())).cursor(rune::Cursor::Pointer)
            .on_press(|s: &mut State| s.prof[3] = rune::TextEdit::new("")))
        .anchored("pf-eyedrop", rune::Side::Right, rune::Align::Start, 8.0)
}

/// The list under the profile type field: the account's own profile, then one per server.
fn type_menu(s: &State) -> El {
    let row_of = |key: String, selected: bool, icon: El, name: String, kind: Option<usize>| {
        row().key(key).h(52.0).px(16.0).gap(12.0).center_y().radius(radius::MD)
            .fill(if selected { accent() } else { Color::TRANSPARENT }).hover(if selected { accent() } else { hover() })
            .child(icon)
            .child(text(name).font(title(14.0, 400.0)).color(if selected { Color::WHITE } else { ink() }).grow(1.0))
            .child(if selected { rune::icon(&CHECK, 16.0).color(Color::WHITE) } else { el().size(0.0, 0.0) })
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| choose_type(s, kind))
    };
    let mut list = col().px(6.0).py(6.0).gap(2.0)
        .child(row_of("pt-global".into(), s.pf.kind.is_none(), el().size(0.0, 0.0), "Global profile".into(), None));
    for (i, g) in s.chat.guilds.iter().enumerate() {
        list = list.child(row_of(format!("pt-{i}"), s.pf.kind == Some(i), guild_icon(s, i, 28.0, 14.0), g.name.clone(), Some(i)));
    }
    col().key("type-menu").w(730.0).max_h(360.0).radius(radius::MD).fill(sidebar_bg()).border(1.0, hx("#3A3642"))
        .shadow(6.0, 24.0, tok::theme(true).shadow).on_press(|_: &mut State| {})
        .child(list.scroll(ScrollSpec { stick_bottom: false }))
        .anchored("pf-type", rune::Side::Bottom, rune::Align::Start, 4.0)
}

// ---------------------------------------------------------------- the dialog

fn set_avatar_global(s: &mut State) { s.pf.avatar_mode = GuildMedia::Global }
fn set_avatar_hide(s: &mut State) { s.pf.avatar_mode = GuildMedia::Hide }
fn set_banner_global(s: &mut State) { s.pf.banner_mode = GuildMedia::Global }
fn set_banner_hide(s: &mut State) { s.pf.banner_mode = GuildMedia::Hide }
fn no_media(s: &mut State) { let _ = s.pf.avatar_mode; }

/// One radio option in the per-community avatar/banner groups.
fn media_radio(key: &str, on: bool, disabled: bool, heading: &str, sub: &str, f: fn(&mut State)) -> El {
    let dot = row().size(20.0, 20.0).radius(10.0).center()
        .border(2.0, if on { accent() } else { hx("#5A5566") })
        .fill(if on { accent() } else { Color::TRANSPARENT })
        .child(if on { el().size(8.0, 8.0).radius(4.0).fill(Color::WHITE) } else { el().size(0.0, 0.0) });
    let col = col().gap(1.0)
        .child(text(heading.to_string()).font(title(14.0, 600.0)).color(if disabled { dim() } else { ink() }))
        .child(text(sub.to_string()).font(title(12.5, 400.0)).color(dim()).w(320.0));
    let r = row().key(key).gap(12.0).center_y().py(4.0)
        .child(dot).child(col);
    if disabled { r.opacity(0.5) } else { r.cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s)) }
}

/// The blue Plutonium upsell shown over the custom-media choices.
fn plutonium_box(s: &State) -> El {
    let site = s.session.as_ref().and_then(|x| x.origin.clone().or_else(|| Some(x.instance.clone())))
        .unwrap_or_else(|| "https://fluxer.com".into());
    let url = format!("{}/premium", site.trim_end_matches('/'));
    col().w(374.0).px(16.0).py(16.0).gap(12.0).radius(radius::MD).fill(accent())
        .child(row().gap(10.0).items_start()
            .child(rune::icon(&ATOM, 20.0).color(Color::WHITE))
            .child(text("Customizing your avatar, banner, accent color, and bio for individual communities requires Plutonium. Community nicknames and pronouns are free for everyone.".to_string())
                .font(title(13.0, 400.0)).color(Color::WHITE).wrap().w(310.0)))
        .child(row().child(row().key("pf-plutonium").h(32.0).px(14.0).radius(radius::SM).center().fill(Color::WHITE).cursor(rune::Cursor::Pointer)
            .child(text("Get Plutonium".to_string()).font(title(13.0, 600.0)).color(hx("#1A181E")))
            .on_press_tx(move |_: &mut State, tx: &mut rune::Tx| { let _ = tx.open_url(&url); })))
}

/// The accent colour field, shared by the global and per-community forms.
fn accent_field(s: &State, k: &Kit) -> El {
    col().gap(8.0)
        .child(label("Accent color"))
        .child(TextInput::new(&s.prof[3]).width(Some(374.0)).height(42.0).label("Accent color")
            .placeholder("#4641D9").on_edit(|s: &mut State| &mut s.prof[3])
            .trailing(row().key("pf-eyedrop").size(38.0, 38.0).radius(radius::SM).center().mr(-8.0)
                .fill(parse_hex(&s.prof[3].text).and_then(|c| rune::color::hex(&hex_of(c))).unwrap_or_else(accent))
                .child(rune::icon(&EYEDROP, 18.0).color(Color::WHITE)).cursor(rune::Cursor::Pointer)
                .on_press(|s: &mut State| { s.pf.color_open = !s.pf.color_open; s.pf.emoji_open = false }))
            .build(k))
        .child(note("Customizes the border and banner color on your profile", 374.0))
}

/// The "About me" bio. Multiline: it grows with the text up to eight lines, then scrolls inside.
/// The emoji picker button and the counter ride pinned to the top-right / bottom-right corners.
fn about_field(s: &State, k: &Kit) -> El {
    let left = BIO_MAX - s.prof[2].text.chars().count() as i64;
    col().gap(8.0)
        .child(label("About me"))
        .child(el().w(374.0)
            // pr(38) reserves the emoji column inside the box, so wrapped text never slides under it.
            .child(TextArea::new(&s.prof[2]).width(Some(374.0)).rows(4).auto_grow(4, 8).max_height(160.0).wrap_anywhere()
                .label("About me").placeholder("Doc, I'm from the future. I came here in a time\nmachine that you invented. Now, I need your help\nto get back to the year 1985.")
                .on_edit(|s: &mut State| &mut s.prof[2]).build(k).pr(38.0))
            // Solid fill so the wrapped text never bleeds through the buttons.
            .child(el().key("pf-emoji").pin(None, Some(10.0), Some(10.0), None).size(22.0, 22.0).center().radius(4.0).fill(field_bg()).cursor(rune::Cursor::Pointer)
                .child(rune::icon(&SMILE, 22.0).color(if s.pf.emoji_open { ink() } else { dim() }))
                .on_press(|s: &mut State| { s.pf.emoji_open = !s.pf.emoji_open; s.pf.color_open = false; s.picker.query = rune::TextEdit::new("") }))
            .child(el().pin(None, None, Some(10.0), Some(6.0)).radius(4.0).fill(field_bg()).px(4.0).py(1.0).child(text(left.to_string()).font(title(12.0, 500.0)).color(if left < 0 { hx("#F0707A") } else { dim() }))))
        .child(note("You can use links, emoji, and Markdown.", 374.0))
}

static EXIT: rune::Icon = rune::Icon { name: "app-set-exit", view: 24.0, body: r#"<path d="M14 3.5h-9A1.5 1.5 0 0 0 3.5 5v14a1.5 1.5 0 0 0 1.5 1.5h9" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/><path d="M9.5 12h11M17 8l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };

/// The IANA time zone of this computer (`America/Sao_Paulo`), from `/etc/localtime` or `TZ`.
fn system_timezone() -> Option<String> {
    if let Some(tz) = std::env::var("TZ").ok().filter(|t| !t.is_empty() && !t.starts_with(':')) { return Some(tz) }
    let link = std::fs::read_link("/etc/localtime").ok()?;
    let text = link.to_string_lossy();
    text.find("zoneinfo/").map(|i| text[i + "zoneinfo/".len()..].to_string()).filter(|t| !t.is_empty())
}

/// "macOS 26.2 (arm64)" or "Linux 7.2.15-1-omarchy (x64)"; read once, not on every frame.
fn os_info() -> String {
    static INFO: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INFO.get_or_init(|| {
        let arch = match std::env::consts::ARCH { "x86_64" => "x64", "aarch64" => "arm64", a => a };
        #[cfg(target_os = "macos")]
        {
            let version = std::process::Command::new("sw_vers").arg("-productVersion").output().ok()
                .and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()).unwrap_or_default();
            if version.is_empty() { format!("macOS ({arch})") } else { format!("macOS {version} ({arch})") }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default();
            if kernel.is_empty() { format!("Linux ({arch})") } else { format!("Linux {kernel} ({arch})") }
        }
    }).clone()
}

/// The sidebar's bottom: sign out, build/technology/OS facts, and the legal links.
fn side_footer() -> El {
    let red = hx("#F0707A");
    let ver = title(12.0, 400.0);
    let facts = col().gap(1.0)
        .child(text(format!("Stable Desktop {}", env!("CARGO_PKG_VERSION"))).font(ver).color(dim()))
        .child(text("Rune (Rust)".to_string()).font(ver).color(dim()))
        .child(text(os_info()).font(ver).color(dim()));
    let links = col().gap(1.0)
        .child(row().key("pf-terms").center_y().cursor(rune::Cursor::Pointer)
            .child(text("Terms of service").font(ver).color(oklch(0.74, 0.14, 245.0))))
        .child(row().key("pf-privacy").center_y().cursor(rune::Cursor::Pointer)
            .child(text("Privacy policy").font(ver).color(oklch(0.74, 0.14, 245.0))));
    col().w_full().px(16.0).pb(16.0).gap(12.0)
        .child(row().key("pf-sign-out").h(40.0).px(10.0).gap(12.0).center_y().radius(radius::SM).hover(hover())
            .child(rune::icon(&EXIT, 18.0).color(red))
            .child(text("Sign out").font(title(15.0, 400.0)).color(red))
            .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| { s.settings_open = false; s.pf = Form::default(); sign_out(s, None); }))
        .child(facts)
        .child(links)
}

// ---------------------------------------------------------------- voice & audio

/// One device in the settings list: a radio row that saves the choice at once.
fn device_row(key: String, label: String, hint: String, on: bool, f: Box<dyn Fn(&mut State)>) -> El {
    let dot = row().size(18.0, 18.0).radius(9.0).border(2.0, if on { accent() } else { hx("#3A3642") }).center()
        .child(if on { el().size(8.0, 8.0).radius(4.0).fill(accent()) } else { el().size(0.0, 0.0) });
    let mut words = col().gap(1.0).grow(1.0).min_w(0.0)
        .child(text(label).font(title(14.0, 500.0)).color(ink()).truncate());
    let tall = !hint.is_empty();
    if tall {
        words = words.child(text(hint).font(title(12.0, 400.0)).color(dim()).truncate());
    }
    let mut item = row().key(key).w_full().h(if tall { 56.0 } else { 44.0 }).px(12.0).gap(12.0).center_y()
        .radius(radius::SM).fill(if on { pressed_row() } else { oklcha(0.0, 0.0, 0.0, 0.0) })
        .hover(if on { pressed_row() } else { hover() })
        .child(dot).child(words);
    if on { item = item.child(rune::icon(&CHECK, 16.0).color(accent())) }
    item.cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s))
}

/// "System default" then every device the audio system knows, with the chosen one marked.
fn device_list(prefix: &str, devices: Option<&Vec<crate::audio::Device>>, chosen: &Option<String>,
    default_hint: &str, choose: fn(&mut State, Option<String>)) -> El {
    let mut list = col().gap(2.0)
        .child(device_row(format!("{prefix}-default"), "System default".into(), default_hint.into(), chosen.is_none(),
            Box::new(move |s| choose(s, None))));
    match devices {
        Some(ds) => for (i, d) in ds.iter().enumerate() {
            let id = d.id.clone();
            list = list.child(device_row(format!("{prefix}-{i}"), d.label.clone(), String::new(),
                chosen.as_deref() == Some(d.id.as_str()), Box::new(move |s| choose(s, Some(id.clone())))));
        },
        None => list = list.child(row().h(44.0).px(12.0).center_y()
            .child(text("Looking for devices…").font(title(13.0, 400.0)).color(dim()))),
    }
    list
}

fn choose_mic(s: &mut State, id: Option<String>) { s.call.set_mic(id) }
fn choose_speaker(s: &mut State, id: Option<String>) { s.call.set_speaker(id) }

fn voice_page(s: &State) -> El {
    let cfg = &s.call.settings;
    let dev = s.call.devices.as_ref();
    // The settings column is narrower than 730 at the app's default window: let it shrink.
    let hint = |words: &str| text(words.to_string()).font(title(12.5, 400.0)).color(dim()).wrap().w_full();
    let mut inner = col().w_full().max_w(730.0).gap(0.0)
        .child(section_head("Microphone"))
        .child(row().h(8.0))
        .child(hint("Your voice is sent from this device. A change takes effect at once, including in a call."))
        .child(row().h(14.0))
        .child(device_list("set-mic", dev.map(|d| &d.mics), &cfg.mic,
            "Whatever input macOS is using; the choice follows it", choose_mic))
        .child(el().w_full().h(1.0).fill(line()).my(28.0))
        .child(section_head("Speakers"))
        .child(row().h(8.0))
        .child(hint("Calls, the join and leave sounds and the microphone monitor play here."))
        .child(row().h(14.0))
        .child(device_list("set-spk", dev.map(|d| &d.speakers), &cfg.speaker,
            "Whatever output macOS is using; the choice follows it", choose_speaker))
        .child(el().w_full().h(1.0).fill(line()).my(28.0))
        .child(section_head("Levels"))
        .child(row().h(8.0))
        .child(hint("The same volumes as the microphone menu, saved with the session."))
        .child(row().h(14.0))
        .child(row().center_y()
            .child(text("Input volume").font(title(14.0, 500.0)).color(ink()).grow(1.0))
            .child(text(format!("{}%", (cfg.input_volume * 100.0).round())).font(title(13.0, 400.0)).color(dim())))
        .child(row().w_full().px(4.0).child(super::slider("set-in-vol", cfg.input_volume, |s, v| s.call.set_input_volume(v))))
        .child(row().center_y().pt(14.0)
            .child(text("Input level").font(title(14.0, 500.0)).color(ink()).grow(1.0))
            .child(text(if s.call.settings_audio { "live" } else { "" }).font(title(12.0, 400.0)).color(dim())))
        .child({ let level = s.call.level();
            let mut meter = row().gap(3.0).h(18.0).px(4.0).center_y();
            for i in 0..22 {
                let on = (i as f32 + 0.5) / 22.0 <= level;
                meter = meter.child(el().size(6.0, 6.0 + i as f32 * 0.5).radius(1.5).fill(if on { super::GREEN() } else { field_bg() }));
            }
            meter })
        .child(row().center_y().pt(14.0)
            .child(text("Output volume").font(title(14.0, 500.0)).color(ink()).grow(1.0))
            .child(text(format!("{}%", (cfg.output_volume * 100.0).round())).font(title(13.0, 400.0)).color(dim())))
        .child(row().w_full().px(4.0).child(super::slider("set-out-vol", cfg.output_volume, |s, v| s.call.set_output_volume(v))))
        .child(row().key("set-denoise").h(40.0).mt(16.0).px(12.0).center_y().radius(radius::SM).hover(hover())
            .child(col().gap(1.0).grow(1.0)
                .child(text("Noise suppression").font(title(14.0, 500.0)).color(ink()))
                .child(text("RNNoise cleans the microphone (off is studio quality)").font(title(12.0, 400.0)).color(dim())))
            .child(row().size(18.0, 18.0).radius(4.0).border(2.0, if cfg.noise_suppression { accent() } else { hx("#3A3642") }).center()
                .fill(if cfg.noise_suppression { accent() } else { oklcha(0.0, 0.0, 0.0, 0.0) })
                .child(if cfg.noise_suppression { rune::icon(&CHECK, 12.0).color(Color::WHITE) } else { el().size(0.0, 0.0) }))
            .cursor(rune::Cursor::Pointer)
            .on_press(|s: &mut State| { let on = !s.call.settings.noise_suppression; s.call.set_noise_suppression(on) }))
        .child(row().h(14.0))
        .child(row().center_y()
            .child(col().gap(1.0).grow(1.0)
                .child(text("Screen share quality").font(title(14.0, 500.0)).color(ink()))
                .child(text("4K screens are scaled down to this").font(title(12.0, 400.0)).color(dim())))
            .child({ let pill = |h: u32| {
                let on = cfg.share_height == h;
                row().key(format!("share-{h}")).h(28.0).px(10.0).radius(radius::SM).center()
                    .fill(if on { accent() } else { field_bg() })
                    .child(text(format!("{h}p")).font(title(13.0, 600.0)).color(if on { Color::WHITE } else { ink() }))
                    .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| s.call.set_share_height(h))
            };
            row().gap(6.0).child(pill(720)).child(pill(1080)) }))
        .child(row().h(28.0))
        .child(row().gap(12.0).center_y()
            .child(action("set-audio-refresh", "Refresh devices", false, true, |s| s.call.refresh_devices()))
            .child(text("Devices plugged in or out appear here.").font(title(12.5, 400.0)).color(dim())));
    if let Some(e) = &s.call.error {
        inner = inner.child(row().h(16.0))
            .child(text(e.clone()).font(title(13.0, 400.0)).color(hx("#F0707A")).wrap().w_full());
    }
    col().w_full().py(16.0)
        .child(row().w_full().justify_center().px(16.0).child(inner))
        .scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0)
}

/// A sidebar row: which settings page is open.
fn nav_row(key: &str, label: &str, icon: &'static rune::Icon, on: bool, f: fn(&mut State)) -> El {
    row().key(key.to_string()).h(38.0).px(8.0).gap(10.0).center_y().radius(radius::SM)
        .fill(if on { pressed_row() } else { oklcha(0.0, 0.0, 0.0, 0.0) })
        .hover(if on { pressed_row() } else { hover() })
        .child(rune::icon(icon, 17.0).color(if on { ink() } else { dim() }))
        .child(text(label.to_string()).font(title(14.0, if on { 600.0 } else { 400.0 })).color(if on { ink() } else { dim() }))
        .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s))
}

/// The profile page: type, form and live preview.
fn profile_page(s: &State, me: &User, k: &Kit) -> El {
    // ---- profile type ----
    let kind_guild = s.pf.kind.and_then(|g| s.chat.guilds.get(g));
    let type_label = kind_guild.map_or("Global profile".to_string(), |g| g.name.clone());
    let mut profile_type = row().key("pf-type").w_full().h(42.0).px(16.0).gap(10.0).radius(radius::MD).fill(field_bg()).border(1.0, hx("#3A3642"))
        .center_y().cursor(rune::Cursor::Pointer);
    if let Some(g) = s.pf.kind { profile_type = profile_type.child(guild_icon(s, g, 24.0, 12.0)) }
    let profile_type = profile_type
        .child(text(type_label).font(title(14.0, 400.0)).color(ink()).grow(1.0))
        .child(rune::icon(&CHEV_DOWN, 18.0).color(dim()))
        .on_press(|s: &mut State| { if dirty(s) { s.pf.nag = true } else { s.pf.type_open = !s.pf.type_open } });

    // ---- form ----
    let username = row().h(42.0).px(14.0).radius(radius::MD).fill(field_bg()).border(1.0, hx("#3A3642")).center_y()
        .child(text(format!("@{}", me.username)).font(title(14.0, 400.0)).color(ink()));
    let field = |name: &str, e: El| col().gap(8.0).child(label(name)).child(e);
    let global = s.pf.kind.is_none();
    let form = if global {
        col().w(374.0)
            .child(field("Username", username))
            .child(note("Changing your username is not available in this client yet; the web app still does it.", 374.0).mt(6.0))
            .child(field("Display name", input(&s.prof[0], "Display name", me.display(), k, |s| &mut s.prof[0])).pt(32.0))
            .child(field("Pronouns", input(&s.prof[1], "Pronouns", "", k, |s| &mut s.prof[1])).pt(24.0))
            .child(col().gap(10.0).pt(24.0)
                .child(label("Profile local time"))
                .child(note("Other people can only see your UTC offset, not your exact time zone identifier. The server keeps the IANA name.", 374.0))
                .child(row().gap(8.0).center_y()
                    .child(text(format!("This computer: {}", system_timezone().unwrap_or_else(|| "unknown".into())))
                        .font(title(12.5, 400.0)).color(dim()).grow(1.0).truncate())
                    .child(action("pf-time", if s.pf.tz_saving { "Saving…" } else if s.pf.tz_saved { "Saved ✓" } else { "Use this time zone" },
                        false, !s.pf.tz_saving, |s| {
                            if let Some(tz) = system_timezone() {
                                s.pf.tz_saving = true;
                                s.pf.tz_saved = false;
                                s.pf.error = None;
                                s.net.save_timezone(tz);
                            }
                        }))))
            .child(col().gap(8.0).pt(24.0)
                .child(label("Avatar"))
                .child(row().gap(8.0).center_y()
                    .child({ let b = row().key("pf-avatar").h(38.0).px(14.0).radius(radius::SM).center().fill(accent())
                        .child(text(if s.pf.avatar_saving { "Uploading…" } else { "Change avatar" }).font(title(14.0, 600.0)).color(Color::WHITE))
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(|s: &mut State, tx: &mut rune::Tx| {
                            s.pf.error = None;
                            let options = rune::FileDialogOptions { title: "Choose an avatar".into(), ..Default::default() };
                            tx.open_file(options, |s: &mut State, result, _| match result {
                                Ok(Some(path)) => { s.pf.avatar_saving = true; s.net.upload_avatar(path) }
                                Ok(None) => {}
                                Err(e) => { s.pf.avatar_saving = false; s.pf.error = Some(format!("Não foi possível abrir o seletor de arquivos: {e}")) }
                            });
                        });
                        if s.pf.avatar_saving { b.opacity(0.5) } else { b } })
                    .child(action("pf-avatar-rm", "Remove avatar", false, !s.pf.avatar_saving, |s| {
                        s.pf.error = None;
                        s.pf.avatar_saving = true;
                        s.net.set_avatar(None);
                    }))))
            .child(col().gap(10.0).pt(36.0)
                .child(label("Banner"))
                .child(note("Profile banners require Plutonium.", 374.0)))
            .child(col().gap(8.0).pt(30.0).child(accent_field(s, k)))
            .child(col().gap(8.0).pt(24.0).child(about_field(s, k)))
    } else {
        // Editing the profile for one server: nickname + pronouns are free; avatar, banner,
        // accent and bio are Plutonium, so the media choices are radios with the custom
        // image disabled, like the web app.
        let avatar = col().gap(6.0)
            .child(label("Avatar"))
            .child(media_radio("pf-avatar-global", s.pf.avatar_mode == GuildMedia::Global, false, "Use global profile", "Show your global profile avatar in this community", set_avatar_global))
            .child(media_radio("pf-avatar-custom", false, true, "Use custom image", "Upload a custom avatar for this community", no_media))
            .child(media_radio("pf-avatar-hide", s.pf.avatar_mode == GuildMedia::Hide, false, "Don't show", "Show default avatar, ignoring your global profile", set_avatar_hide));
        let banner_mode = col().gap(6.0)
            .child(label("Banner"))
            .child(media_radio("pf-banner-global", s.pf.banner_mode == GuildMedia::Global, false, "Use global profile", "Show your global profile banner in this community", set_banner_global))
            .child(media_radio("pf-banner-custom", false, true, "Use custom image", "Upload a custom banner for this community", no_media))
            .child(media_radio("pf-banner-hide", s.pf.banner_mode == GuildMedia::Hide, false, "Don't show", "Show accent color only, ignoring your global profile", set_banner_hide));
        col().w(374.0)
            .child(col().gap(8.0)
                .child(label("Community nickname"))
                .child(input(&s.prof[0], "Community nickname", me.display(), k, |s| &mut s.prof[0]))
                .child(note("This nickname will only be visible in this community", 374.0)))
            .child(field("Pronouns", input(&s.prof[1], "Pronouns", "", k, |s| &mut s.prof[1])).pt(24.0))
            .child(col().pt(24.0).child(plutonium_box(s)))
            .child(col().gap(8.0).pt(24.0).child(avatar))
            .child(col().gap(8.0).pt(24.0).child(banner_mode))
            .child(col().gap(8.0).pt(24.0).child(accent_field(s, k)))
            .child(col().gap(8.0).pt(24.0).child(about_field(s, k)))
    };

    let customize = row().w_full().gap(58.0).items_start().flex_wrap()
        .child(form)
        .child(preview(s, me));

    let customize_title = if global { "Profile customization".to_string() }
        else { format!("Profile customization for {}", kind_guild.map_or("this server", |g| g.name.as_str())) };
    let wide = |words: &str| text(words.to_string()).font(title(12.5, 400.0)).color(dim()).wrap().w_full();
    let mut inner = col().w_full().max_w(730.0).gap(0.0)
        .child(label("Profile type"))
        .child(row().h(10.0))
        .child(profile_type);
    if !global {
        inner = inner.child(row().h(10.0))
            .child(wide("You are editing your per-community profile. This profile will only be visible in this community and will override your global profile."));
    }
    let inner = inner
        .child(el().w_full().h(1.0).fill(line()).my(28.0))
        .child(section_head(&customize_title))
        .child(row().h(8.0))
        .child(wide("Edit your profile appearance and see a live preview"))
        .child(row().h(20.0))
        .child(customize)
        .child(el().w_full().h(1.0).fill(line()).mt(36.0).mb(24.0))
        .child(section_head("Badges"))
        .child(row().h(8.0))
        .child(wide("Choose which badges other people see on your profile."))
        .child(row().h(18.0))
        .child(text("You don't have any badges yet.").font(title(14.0, 400.0)).color(ink()))
        .child(row().h(40.0));

    col().w_full().py(16.0)
        .child(row().w_full().justify_center().px(16.0).child(inner))
        .scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0)
}

pub fn dialog(s: &State) -> El {
    if !s.settings_open || s.chat.me.is_none() { return el().size(0.0, 0.0) }
    let Some(me) = s.chat.me.clone() else { return el().size(0.0, 0.0) };
    let k = fields();

    // ---- sidebar ----
    let face = row().size(40.0, 40.0)
        .child(avatar(s, &me, 40.0))
        .child(el().absolute(28.0, 28.0).size(12.0, 12.0).radius(6.0)
            .fill(status_color(&s.chat.roster.status_of(&me.id))).border(2.0, pressed_row()));
    let side_body = col().w_full().px(12.0).py(16.0).gap(2.0)
        .child(row().h(10.0))
        .child(row().h(44.0).px(8.0).gap(10.0).center_y().radius(radius::SM)
            .fill(if s.pf.page == Page::Profile { pressed_row() } else { oklcha(0.0, 0.0, 0.0, 0.0) })
            .child(face)
            .child(col().gap(1.0)
                .child(text(me.display().to_string()).font(title(15.0, 600.0)).color(ink()))
                .child(row().gap(4.0).center_y()
                    .child(rune::icon(&PENCIL, 11.0).color(dim()))
                    .child(text("Edit your profile").font(title(13.0, 400.0)).color(dim())))
            )
            .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| { s.pf.page = Page::Profile; s.call.meter(false) }))
        .child(row().h(12.0))
        .child(nav_row("set-page-profile", "Profile", &PENCIL, s.pf.page == Page::Profile, |s| { s.pf.page = Page::Profile; s.call.meter(false) }))
        .child(nav_row("set-page-voice", "Voice & audio", &MIC, s.pf.page == Page::Voice, |s| {
            s.pf.page = Page::Voice;
            s.pf.type_open = false;
            s.pf.color_open = false;
            s.pf.emoji_open = false;
            s.call.refresh_devices();
            s.call.meter(true);
        }));
    let side = col().w(320.0).h_full().fill(sidebar_bg())
        .child(side_body.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
        .child(side_footer());

    // ---- header: the title, or the bar about unsaved changes ----
    let changed = dirty(s) && s.pf.page == Page::Profile;
    let blocked = problem(s);
    let top = if changed {
        let msg = if s.pf.nag { "Save or reset your changes before leaving.".to_string() }
            else if let Some(e) = s.pf.error.clone().or(blocked.clone()) { e } else { "You have unsaved changes.".to_string() };
        row().h(70.0).px(16.0).center_y().gap(8.0).fill(hx("#0C0B0E")).border(1.0, line())
            .child(text(msg).font(title(14.0, 500.0)).color(if s.pf.nag || blocked.is_some() || s.pf.error.is_some() { hx("#F0707A") } else { ink() }).grow(1.0).truncate())
            .child(action("pf-reset", "Reset", false, !s.pf.saving, reset))
            .child(action("pf-save", if s.pf.saving { "Saving…" } else { "Save changes" }, true, blocked.is_none() && !s.pf.saving, save))
    } else {
        let (heading, icon) = match s.pf.page {
            Page::Profile => ("Profile", &LINK),
            Page::Voice => ("Voice & audio", &MIC),
        };
        row().h(70.0).px(16.0).center_y().gap(8.0)
            .child(text(heading).font(title(20.0, 600.0)).color(ink()))
            .child(rune::icon(icon, 14.0).color(dim()))
            .child(el().grow(1.0))
            .child(row().key("settings-close").size(32.0, 32.0).radius(radius::SM).center().hover(hover())
                .cursor(rune::Cursor::Pointer)
                .child(rune::icon(&X, 20.0).color(dim())).on_press(close))
    };

    // ---- the page ----
    let page = match s.pf.page {
        Page::Profile => profile_page(s, &me, &k),
        Page::Voice => voice_page(s),
    };

    let card = row().key("settings-card").w_full().h_full().max_w(1400.0).radius(radius::LG).fill(panel())
        .border(1.0, hx("#2E2B35")).shadow(8.0, 32.0, tok::theme(true).shadow).clip_rounded(radius::LG)
        .on_press(|_: &mut State| {})
        .child(side)
        .child(col().grow(1.0).h_full().min_w(0.0).child(top).child(page));

    let mut scrim = row().key("settings-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).center().px(32.0).py(32.0)
        .fill(oklcha(0.0, 0.0, 0.0, 0.82))
        .on_press(close).on_secondary_press(|s: &mut State, _, _| close(s))
        .child(card);
    // Floating pieces: each has a layer under it that closes it on a press elsewhere.
    let layer = |key: &str, f: fn(&mut State)| el().key(key.to_string()).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).on_press(f);
    if s.pf.page == Page::Profile && s.pf.type_open {
        scrim = scrim.child(layer("pf-type-layer", |s| s.pf.type_open = false)).child(type_menu(s));
    }
    if s.pf.page == Page::Profile && s.pf.color_open {
        scrim = scrim.child(layer("pf-color-layer", |s| s.pf.color_open = false)).child(color_picker(s));
    }
    if s.pf.page == Page::Profile && s.pf.emoji_open {
        scrim = scrim.child(layer("pf-emoji-layer", |s| s.pf.emoji_open = false))
            .child(compose::emoji_popover(s, "pf-emoji", |s, glyph| { let t = format!("{}{glyph}", s.prof[2].text); s.prof[2] = rune::TextEdit::new(t) }));
    }
    scrim
}

/// "Nov 3, 2026" from the account id (a snowflake: milliseconds since 2015-01-01 in the high bits).
/// The day is the local one, as the web app shows it (the snowflake itself is UTC).
fn member_since(id: &str) -> String {
    let ms = (id.parse::<u64>().unwrap_or(0) >> 22) as i64 + 1_420_070_400_000;
    fmt_ms(ms)
}

fn fmt_ms(ms: i64) -> String {
    let (y, m, d) = crate::clock::ymd(ms);
    let names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {d}, {y}", names[((m - 1).clamp(0, 11)) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_survive_the_round_trip_through_the_picker() {
        for c in [0x4641D9u32, 0xFF0000, 0x00FF00, 0x0000FF, 0x123456, 0xFFFFFF, 0x000000, 0x808080] {
            assert_eq!(hsv_to_rgb(rgb_to_hsv(c, 0.0)), c, "{c:06X}");
        }
        assert_eq!(parse_hex("#4641d9"), Some(0x4641D9));
        assert_eq!(parse_hex("4641D9"), Some(0x4641D9));
        assert_eq!(parse_hex("#46"), None);
        assert_eq!(parse_hex("#GGGGGG"), None);
    }

    #[test]
    fn the_hue_bar_runs_through_the_full_rainbow_and_wraps() {
        // The bar's stops: red at both ends, the other hues between.
        assert_eq!(hsv_to_rgb((0.0, 1.0, 1.0)), 0xFF0000);
        assert_eq!(hsv_to_rgb((60.0, 1.0, 1.0)), 0xFFFF00);
        assert_eq!(hsv_to_rgb((120.0, 1.0, 1.0)), 0x00FF00);
        assert_eq!(hsv_to_rgb((240.0, 1.0, 1.0)), 0x0000FF);
        assert_eq!(hsv_to_rgb((360.0, 1.0, 1.0)), 0xFF0000);
        assert_eq!(hsv_to_rgb((-60.0, 1.0, 1.0)), 0xFF00FF);
    }

    #[test]
    fn the_per_community_media_starts_on_the_global_profile_and_switches() {
        let mut s = State::default();
        assert_eq!(s.pf.avatar_mode, GuildMedia::Global);
        assert_eq!(s.pf.banner_mode, GuildMedia::Global);
        set_avatar_hide(&mut s);
        set_banner_hide(&mut s);
        assert_eq!(s.pf.avatar_mode, GuildMedia::Hide);
        assert_eq!(s.pf.banner_mode, GuildMedia::Hide);
        set_avatar_global(&mut s);
        set_banner_global(&mut s);
        assert_eq!(s.pf.avatar_mode, GuildMedia::Global);
        assert_eq!(s.pf.banner_mode, GuildMedia::Global);
        assert_eq!(fmt_ms(43_200_000), "Jan 1, 1970");
    }

    #[test]
    fn changes_are_pending_until_they_match_what_is_saved_and_a_long_bio_cannot_be_saved() {
        let mut s = State::default();
        assert!(!dirty(&s));
        s.prof[2] = rune::TextEdit::new("x".repeat(321));
        assert!(dirty(&s) && problem(&s).is_some());
        s.prof[2] = rune::TextEdit::new("x".repeat(320));
        assert!(problem(&s).is_none());
        reset(&mut s);
        assert!(!dirty(&s), "reset puts the saved values back");
        s.prof[3] = rune::TextEdit::new("nope");
        assert!(problem(&s).is_some());
        s.settings_open = true;
        close(&mut s);
        assert!(s.settings_open && s.pf.nag, "the dialog stays open while there are changes");
    }
}

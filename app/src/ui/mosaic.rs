//! The call's mosaic: one 16:9 tile per person in the voice channel you are in, plus a tile for
//! each screen share, which you watch on request. While you watch a stream it takes most of the
//! stage and everyone else shrinks to a strip below; "grid view" brings back equal tiles.
//! Shown in place of the text chat when you click the voice channel you are connected to.

use super::*;
use crate::model::VState;
use crate::watch_stream;

const RATIO: f32 = 16.0 / 9.0;

const GAP: f32 = 4.0;

/// Columns and tile width for `n` tiles in a `w` x `h` stage: the choice that makes the biggest 16:9 tiles.
fn best_fit(n: usize, w: f32, h: f32) -> (usize, f32) {
    let width = |c: usize| {
        let rows = n.div_ceil(c) as f32;
        ((w - GAP * (c as f32 - 1.0)) / c as f32).min((h - GAP * (rows - 1.0)) / rows * RATIO)
    };
    let c = (1..=n.max(1)).max_by(|a, b| width(*a).partial_cmp(&width(*b)).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(1);
    (c, width(c).max(40.0))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind { Person, Stream }

pub fn view(s: &State, size: Option<[f32; 2]>) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let people = s.call.channel_id.as_deref().and_then(|id| s.chat.voice.get(id));
    let sorted = sorted_voice(s, people);
    // Screen shares come first, then everyone.
    let mut tiles: Vec<(Kind, &VState)> = sorted.iter().filter(|v| v.self_stream && !v.connection_id.is_empty()).map(|v| (Kind::Stream, *v)).collect();
    // Someone sharing their screen has just that tile; their own comes back when they stop.
    tiles.extend(sorted.iter().filter(|v| !(v.self_stream && !v.connection_id.is_empty())).map(|v| (Kind::Person, *v)));
    let focus = if s.call.grid { None } else { tiles.iter().position(|(k, v)| *k == Kind::Stream && s.call.is_watching(&v.connection_id)) };

    // Until the stage has been measured, a typical window.
    let [sw, sh] = size.unwrap_or([880.0, 600.0]);
    let stage = match focus {
        Some(i) => focused(s, &tiles, i, &sorted, sw),
        None => grid(s, &tiles, &sorted, (sw, sh)),
    };
    let stage = col().key("mosaic-stage").grow(1.0).min_h(0.0).w_full().child(stage);

    let c = &s.call;
    let danger = oklcha(0.64, 0.2, 25.0, 0.22);
    // Square-ish buttons like the ones in the account bar, only larger.
    let btn = |key: &str, off: bool, icon: &'static rune::Icon, f: fn(&mut State)| {
        row().key(key.to_string()).size(46.0, 40.0).radius(radius::SM).center()
            .fill(if off { danger } else { p.select }).hover(if off { oklcha(0.64, 0.2, 25.0, 0.32) } else { oklcha(1.0, 0.0, 0.0, 0.1) })
            .child(rune::icon(icon, 20.0).color(if off { p.unread } else { t.text }))
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| f(s))
    };
    let bar = row().gap(6.0).px(6.0).py(6.0).radius(12.0).center_y()
        .fill(oklch(0.2, 0.008, 265.0)).border(1.0, p.line).shadow(6.0, 24.0, t.shadow)
        .child(btn("mv-mic", c.muted || c.deafened, if c.muted || c.deafened { &MIC_OFF } else { &MIC }, crate::toggle_mute))
        .child(btn("mv-deafen", c.deafened, if c.deafened { &HEADSET_OFF } else { &HEADSET }, crate::toggle_deafen))
        .child(row().key("mv-leave").size(46.0, 40.0).radius(radius::SM).center().fill(p.unread).hover(oklch(0.7, 0.2, 25.0))
            .child(rune::icon(&HANG_UP, 20.0).color(t.knob)).cursor(rune::Cursor::Pointer).on_press(|s: &mut State| leave_voice(s)));
    // The voice channel's own text chat, as the bubble at the bottom left.
    let vi = s.call.channel_id.as_deref().and_then(|id| s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g)).and_then(|g| g.voice.iter().position(|ch| ch.id == id)));
    let chat_btn = match vi {
        Some(vi) => row().key("mv-chat").size(46.0, 40.0).radius(radius::SM).center().fill(p.select).hover(oklcha(1.0, 0.0, 0.0, 0.1))
            .child(rune::icon(&CHAT_CIRCLE, 20.0).color(t.text)).cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| open_voice_chat(s, vi)),
        None => el().size(46.0, 40.0),
    };
    // Well clear of the window's bottom edge.
    let footer = row().w_full().h(100.0).px(18.0).pb(28.0).center_y()
        .child(row().w(80.0).child(chat_btn))
        .child(row().grow(1.0).justify_center().child(bar))
        .child(el().w(80.0));
    col().grow(1.0).min_h(0.0).w_full().fill(oklch(0.0, 0.0, 0.0)).child(stage).child(footer)
}

/// Equal 16:9 tiles packed tight and centred; a short last row sits in the middle.
fn grid(s: &State, tiles: &[(Kind, &VState)], all: &[&VState], (w, h): (f32, f32)) -> El {
    let (aw, ah) = (w - 2.0 * GAP, h - 2.0 * GAP);
    let (cols, tw) = best_fit(tiles.len(), aw, ah);
    let th = tw / RATIO;
    let mut g = col().grow(1.0).min_h(0.0).w_full().center().gap(GAP);
    for chunk in tiles.chunks(cols) {
        let mut line = row().gap(GAP);
        for (kind, v) in chunk { line = line.child(tile(s, *kind, v, all, tw < 300.0).w(tw).h(th)) }
        g = g.child(line);
    }
    g
}

/// The watched stream big, everyone else in a strip of small tiles under it.
fn focused(s: &State, tiles: &[(Kind, &VState)], at: usize, all: &[&VState], width: f32) -> El {
    let (_, v) = tiles[at];
    let big = tile(s, Kind::Stream, v, all, false).aspect_fit(RATIO);
    // Small enough that they all fit side by side.
    let others = (tiles.len() - 1).max(1) as f32;
    let h = (((width - 2.0 * GAP) / others - GAP) / RATIO).clamp(52.0, 96.0);
    let mut strip = row().w_full().min_w(0.0).h(h + 2.0 * GAP).gap(GAP).px(GAP).pb(GAP).justify_center().center_y().clip_rounded(0.0);
    for (i, (kind, v)) in tiles.iter().enumerate() {
        if i == at { continue }
        strip = strip.child(tile(s, *kind, v, all, true).h(h).aspect_ratio(RATIO));
    }
    col().grow(1.0).min_h(0.0).w_full()
        .child(row().grow(1.0).min_h(0.0).w_full().px(GAP).py(GAP).center().child(big))
        .child(strip)
}

fn tile(s: &State, kind: Kind, v: &VState, all: &[&VState], small: bool) -> El {
    match kind { Kind::Person => person_tile(s, v, small), Kind::Stream => stream_tile(s, v, all, small) }
}

/// A pill over the corner of a tile: the name, with what is off beside it.
fn name_pill(s: &State, v: &VState, suffix: &str, small: bool) -> El {
    let t = tok::theme(true);
    let user = s.chat.roster.users.get(&v.user_id);
    let name = user.map_or("…".to_string(), |u| name_in(s, u));
    let mut pill = row().h(if small { 20.0 } else { 24.0 }).px(8.0).gap(6.0).radius(6.0).center_y().fill(oklcha(0.0, 0.0, 0.0, 0.6));
    if suffix.is_empty() {
        if v.deafened() { pill = pill.child(rune::icon(&HEADSET_OFF, 14.0).color(t.text)) }
        else if v.muted() { pill = pill.child(rune::icon(&MIC_OFF, 14.0).color(t.text)) }
    } else {
        pill = pill.child(rune::icon(&SCREEN, 14.0).color(t.text));
    }
    pill.child(text(format!("{name}{suffix}")).font(type_::LABEL).color(t.text).truncate())
}

fn person_tile(s: &State, v: &VState, small: bool) -> El {
    let speaking = s.call.speaking.contains(&v.user_id);
    let user = s.chat.roster.users.get(&v.user_id);
    let base = oklch(0.3, 0.05, 60.0 + 80.0 * tint(&v.user_id) as f32);
    let cam = if v.self_video { s.call.picture(&format!("cam:{}", v.connection_id)) } else { None };
    let mut tile = el().key(format!("mt-{}", v.user_id)).radius(12.0).fill(base).clip_rounded(12.0);
    let face = if small { 30.0 } else { 88.0 };
    tile = match cam {
        Some(px) => tile.child(el().absolute(0.0, 0.0).w_full().h_full().pixels_fit(&px, rune::SurfaceFit::Fit).fill(oklch(0.0, 0.0, 0.0))),
        None => tile.child(row().absolute(0.0, 0.0).w_full().h_full().center().child(match user {
            Some(u) => opens_profile(row().size(face + 8.0, face + 8.0).radius((face + 8.0) / 2.0).center().child(avatar(s, u, face)), u, format!("mp-{}", v.user_id)),
            None => el().size(face, face).radius(face / 2.0).fill(pal().input),
        })),
    };
    tile = tile.child(el().absolute(0.0, 0.0).w_full().h_full().radius(12.0).border(if speaking { 3.0 } else { 0.0 }, GREEN()).instant_changes());
    tile.child(el().pin(Some(8.0), None, None, Some(8.0)).child(name_pill(s, v, "", small)))
}

fn stream_tile(s: &State, v: &VState, all: &[&VState], small: bool) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let conn = v.connection_id.clone();
    let key = s.call.stream_key(&conn).unwrap_or_default();
    let watching = s.call.is_watching(&conn);
    let viewers: Vec<&&VState> = all.iter().filter(|o| o.viewing.contains(&key)).collect();
    let mut tile = el().key(format!("ms-{conn}")).radius(12.0).fill(oklch(0.08, 0.005, 265.0)).clip_rounded(12.0);
    if watching {
        match s.call.picture(&format!("screen:{conn}")) {
            Some(px) => tile = tile.child(el().absolute(0.0, 0.0).w_full().h_full().pixels_fit(&px, rune::SurfaceFit::Fit)),
            None => tile = tile.child(row().absolute(0.0, 0.0).w_full().h_full().center()
                .child(text("Connecting to the stream…").font(type_::BODY).color(t.text_secondary))),
        }
        // Icon-only actions in one row: switch between the big and the equal-tiles layouts, stop.
        let c = conn.clone();
        let (bw, bh) = if small { (26.0, 24.0) } else { (34.0, 30.0) };
        let action = |key: String, icon: &'static rune::Icon, bad: bool| row().key(key).size(bw, bh).radius(radius::SM).center()
            .fill(if bad { oklcha(0.64, 0.2, 25.0, 0.9) } else { oklcha(0.0, 0.0, 0.0, 0.65) })
            .hover(if bad { oklch(0.7, 0.2, 25.0) } else { oklcha(0.3, 0.0, 0.0, 0.85) })
            .child(rune::icon(icon, if small { 13.0 } else { 16.0 }).color(if bad { t.knob } else { t.text })).cursor(rune::Cursor::Pointer);
        let mut actions = row().gap(6.0);
        if !small {
            actions = actions.child(action(format!("ms-layout-{conn}"), if s.call.grid { &EXPAND } else { &GRID }, false)
                .on_press(|s: &mut State| s.call.grid = !s.call.grid));
        }
        actions = actions.child(action(format!("ms-stop-{conn}"), &CLOSE, true).on_press(move |s: &mut State| watch_stream(s, &c, false)));
        tile = tile.child(actions.pin(None, None, Some(8.0), Some(8.0)));
    } else {
        let c = conn.clone();
        tile = tile.child(row().absolute(0.0, 0.0).w_full().h_full().center().child(
            row().key(format!("ms-watch-{conn}")).size(if small { 34.0 } else { 52.0 }, if small { 28.0 } else { 38.0 }).radius(radius::SM).center()
                .fill(oklch(0.52, 0.2, 277.0)).hover(oklch(0.58, 0.2, 277.0))
                .child(rune::icon(&PLAY, if small { 14.0 } else { 20.0 }).color(t.knob))
                .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| watch_stream(s, &c, true))));
    }
    // Viewers at the top left (hover for who), LIVE at the top right.
    let mut pill = row().key(format!("ms-viewers-{conn}")).h(22.0).px(8.0).gap(5.0).radius(6.0).center_y().fill(oklcha(0.0, 0.0, 0.0, 0.6))
        .child(rune::icon(&EYE, 14.0).color(t.text)).child(text(viewers.len().to_string()).font(type_::LABEL).color(t.text));
    let k = key.clone();
    pill = pill.on_hover_change(move |s: &mut State, inside, _| {
        if inside { s.call.viewer_tip = Some(k.clone()) } else if s.call.viewer_tip.as_deref() == Some(k.as_str()) { s.call.viewer_tip = None }
    });
    tile = tile.child(pill.pin(Some(8.0), Some(8.0), None, None));
    if s.call.viewer_tip.as_deref() == Some(key.as_str()) && !viewers.is_empty() {
        let mut list = col().gap(6.0).px(10.0).py(8.0).radius(10.0).fill(oklch(0.22, 0.008, 265.0)).border(1.0, p.line).shadow(6.0, 18.0, t.shadow);
        for o in viewers.iter().take(12) {
            let Some(u) = s.chat.roster.users.get(&o.user_id) else { continue };
            list = list.child(row().gap(8.0).center_y().child(avatar(s, u, 20.0)).child(text(name_in(s, u)).font(type_::LABEL).color(t.text)));
        }
        if viewers.len() > 12 { list = list.child(text(format!("+{} more", viewers.len() - 12)).font(type_::LABEL).color(t.text_secondary)) }
        tile = tile.child(list.pin(Some(8.0), Some(36.0), None, None));
    }
    tile = tile.child(row().h(20.0).px(6.0).radius(4.0).center().fill(p.unread)
        .child(text("LIVE").font(type_::SECTION).color(t.knob)).pin(None, Some(8.0), Some(8.0), None));
    // The sharer's voice still shows: a green ring while they speak.
    let speaking = s.call.speaking.contains(&v.user_id);
    tile = tile.child(el().absolute(0.0, 0.0).w_full().h_full().radius(12.0).border(if speaking { 3.0 } else { 0.0 }, GREEN()).instant_changes());
    tile.child(el().pin(Some(8.0), None, None, Some(8.0)).child(name_pill(s, v, "'s screen", small)))
}

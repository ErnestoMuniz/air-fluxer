//! The message box: the "+" menu, the buttons on its right and the picker they open (GIFs, saved
//! media, stickers and emoji).

use super::*;
use crate::picker::{GifView, Tab};
use crate::{insert_text, send_link, send_sticker};

static PLUS: rune::Icon = rune::Icon { name: "app-plus", view: 24.0, body: r#"<path d="M12 4v16M4 12h16" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" fill="none"/>"# };
static IMAGE: rune::Icon = rune::Icon { name: "app-image", view: 24.0, body: r#"<rect x="3.5" y="4.5" width="17" height="15" rx="3" stroke="currentColor" stroke-width="2" fill="none"/><circle cx="9" cy="10" r="1.6" fill="currentColor"/><path d="M4.5 17.5l4.5-4.5 3 3 3-3 4.5 4.5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static STICKER: rune::Icon = rune::Icon { name: "app-sticker", view: 24.0, body: r#"<path d="M4 7a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v6.5L13.5 20H7a3 3 0 0 1-3-3z" fill="currentColor"/><path d="M20 13.5h-3.5a3 3 0 0 0-3 3V20" stroke="currentColor" stroke-width="2" stroke-linejoin="round" fill="none" opacity=".5"/>"# };
static SMILE: rune::Icon = rune::Icon { name: "app-smile", view: 24.0, body: r#"<circle cx="12" cy="12" r="8.5" stroke="currentColor" stroke-width="2" fill="none"/><circle cx="9" cy="10" r="1.3" fill="currentColor"/><circle cx="15" cy="10" r="1.3" fill="currentColor"/><path d="M8.5 14c1 1.8 5.9 1.8 7 0" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"# };
static SAD: rune::Icon = rune::Icon { name: "app-sad", view: 24.0, body: r#"<circle cx="12" cy="12" r="9.5" stroke="currentColor" stroke-width="2" fill="none"/><circle cx="8.8" cy="10" r="1.3" fill="currentColor"/><circle cx="15.2" cy="10" r="1.3" fill="currentColor"/><path d="M8 16.5c1.2-1.8 6.8-1.8 8 0" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"# };
static SEARCH: rune::Icon = rune::Icon { name: "app-search", view: 24.0, body: r#"<circle cx="10.5" cy="10.5" r="6.5" stroke="currentColor" stroke-width="2.2" fill="none"/><path d="M15.5 15.5L20 20" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>"# };
static PAPERCLIP: rune::Icon = rune::Icon { name: "app-paperclip", view: 24.0, body: r#"<path d="M19 11.5l-7 7a4.5 4.5 0 0 1-6.4-6.4l8-8a3 3 0 0 1 4.3 4.3l-8 8a1.5 1.5 0 0 1-2.1-2.1l6.5-6.5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static ARROW_LEFT: rune::Icon = rune::Icon { name: "app-arrow-left", view: 24.0, body: r#"<path d="M19 12H5M11 6l-6 6 6 6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static SEND: rune::Icon = rune::Icon { name: "app-send", view: 24.0, body: r#"<path d="M3 11.5L21 3l-6.5 18-3-7.5z" fill="currentColor"/>"# };

const PICKER_W: f32 = 440.0;
const PICKER_H: f32 = 520.0;
const MENU_FILL: fn() -> Color = || oklch(0.22, 0.008, 265.0);

fn clear() -> Color { oklcha(0.0, 0.0, 0.0, 0.0) }

fn any_open(s: &State) -> bool { s.plus_open || s.picker.open.is_some() || s.react_picker.is_some() }

fn close_all(s: &mut State) { s.react_picker = None; s.plus_open = false; s.customize_open = false; s.picker.close() }

/// The box at the bottom of the chat.
pub fn composer(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let prefs = &s.prefs;
    let place = match (s.chat.nav.guild.is_some(), s.chat.channel()) {
        (true, Some(c)) => format!("Message #{}", c.name),
        (false, Some(c)) => format!("Message @{}", c.name),
        _ => "Message".to_string(),
    };
    let btn = |key: &str, tab: Tab, shown: bool, icon: El| if !shown { el().size(0.0, 0.0) } else {
        let on = s.picker.open == Some(tab);
        row().key(key.to_string()).size(34.0, 34.0).radius(radius::SM).center().fill(if on { p.select } else { clear() }).hover(p.select)
            .child(icon)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| { s.plus_open = false; s.customize_open = false; let api = s.net.api().cloned(); s.picker.toggle(tab, &api); crate::picker_images(s) })
    };
    let tint = |on: bool| if on { t.text } else { t.text_secondary };
    let gif_icon = row().px(4.0).h(18.0).radius(4.0).border(1.5, tint(s.picker.open == Some(Tab::Gifs))).center()
        .child(text("GIF").font(rune::tok::TextStyle::new(10.0, 800.0)).color(tint(s.picker.open == Some(Tab::Gifs))));
    // As tall as the account bar at the bottom left, and frosted: a little translucent over the
    // chat with the blur behind it.
    let mut bar = row().key("composer-box").h(56.0).px(8.0).gap(2.0).center_y().radius(10.0)
        .fill(oklcha(0.32, 0.008, 265.0, 0.35)).backdrop_blur(16.0)
        .child(row().key("plus-btn").size(34.0, 34.0).radius(radius::SM).center().fill(if s.plus_open { p.select } else { clear() }).hover(p.select)
            .child(rune::icon(&PLUS, 22.0).color(tint(s.plus_open)))
            .cursor(rune::Cursor::Pointer)
            .on_press(|s: &mut State| { let open = !s.plus_open; close_all(s); s.plus_open = open }))
        .child(text_input(&s.draft, type_::BODY)
            .key("composer")
            .placeholder(place, t.text_secondary)
            .on_edit(|s: &mut State| &mut s.draft)
            .on_submit(|s: &mut State, _| send(s))
            .grow(1.0).min_w(0.0).px(8.0).py(12.0))
        .child(btn("btn-gifs", Tab::Gifs, prefs.gifs_button, gif_icon))
        .child(btn("btn-media", Tab::Media, prefs.media_button, rune::icon(&IMAGE, 22.0).color(tint(s.picker.open == Some(Tab::Media)))))
        .child(btn("btn-stickers", Tab::Stickers, prefs.stickers_button, rune::icon(&STICKER, 22.0).color(tint(s.picker.open == Some(Tab::Stickers)))))
        .child(btn("btn-emoji", Tab::Emojis, prefs.emoji_button, rune::icon(&SMILE, 22.0).color(tint(s.picker.open == Some(Tab::Emojis)))));
    if prefs.send_button {
        bar = bar.child(row().key("btn-send").size(34.0, 34.0).radius(radius::SM).center().hover(p.select)
            .child(rune::icon(&SEND, 20.0).color(tint(!s.draft.text.trim().is_empty())))
            .cursor(rune::Cursor::Pointer).on_press(|s: &mut State| send(s)));
    }
    col().px(16.0).py(8.0).child(super::typing_bar(s)).child(bar)
}

/// Popups and the layer that closes them; added at the root so they draw above everything.
pub fn layers(s: &State) -> Vec<El> {
    let mut v = Vec::new();
    if any_open(s) {
        v.push(el().key("compose-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).on_press(|s: &mut State| close_all(s)));
    }
    if s.plus_open { v.push(plus_menu(s)); if s.customize_open { v.push(customize_menu(s)) } }
    if let Some(tab) = s.picker.open { v.push(picker(s, tab)) }
    if let Some(id) = &s.react_picker {
        v.push(menu_card("react-picker").w(PICKER_W).h(PICKER_H).fill(oklch(0.18, 0.008, 265.0))
            .child(emojis_body(s, |s, glyph| {
                if let Some(id) = s.react_picker.take() { crate::toggle_reaction(s, &id, glyph) }
            }))
            .anchored(format!("rx-more-{id}"), rune::Side::Left, rune::Align::Start, 8.0));
    }
    v
}

/// The bar that floats over a message under the pointer: three quick reactions and the emoji picker.
pub fn reaction_bar(s: &State, id: &str) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let quick = |glyph: &'static str| {
        let id = id.to_string();
        row().key(format!("rxq-{id}-{glyph}")).size(30.0, 30.0).radius(radius::SM).center().hover(p.select)
            .child(text(glyph.to_string()).font(rune::tok::TextStyle::new(18.0, 400.0)).color(t.text))
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| crate::toggle_reaction(s, &id, glyph))
    };
    let more = { let id = id.to_string();
        row().key(format!("rx-more-{id}")).size(30.0, 30.0).radius(radius::SM).center().hover(p.select)
            .child(rune::icon(&SMILE, 18.0).color(t.text_secondary))
            .cursor(rune::Cursor::Pointer).on_press(move |s: &mut State| {
                s.picker.query = rune::TextEdit::new("");
                s.react_picker = if s.react_picker.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) }
            }) };
    row().gap(2.0).px(4.0).py(3.0).radius(8.0).fill(oklch(0.2, 0.008, 265.0)).border(1.0, p.line).shadow(2.0, 8.0, t.shadow)
        .child(quick("👍")).child(quick("👌")).child(quick("🎉")).child(more)
}

fn menu_card(key: &str) -> El {
    // A press on the card itself (not on an item) is taken here, so it does not fall through to the layer that closes the popup.
    col().key(key.to_string()).radius(10.0).fill(MENU_FILL()).border(1.0, pal().line).shadow(6.0, 24.0, tok::theme(true).shadow).on_press(|_: &mut State| {})
}

fn plus_menu(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let item = |key: &str, label: &str, icon: &'static rune::Icon, on: bool| row().key(key.to_string()).h(38.0).px(12.0).gap(8.0).center_y().radius(radius::SM)
        .fill(if on { p.select } else { clear() }).hover(p.select)
        .child(text(label.to_string()).font(type_::BODY).color(t.text).grow(1.0))
        .child(rune::icon(icon, 18.0).color(t.text_secondary));
    menu_card("plus-menu").w(260.0).px(6.0).py(6.0)
        .child(item("pm-upload", "Upload file", &PAPERCLIP, false).cursor(rune::Cursor::Pointer).on_press_tx(|s: &mut State, tx: &mut rune::Tx| { close_all(s); crate::upload::choose_file(s, tx) }))
        .child(item("pm-customize", "Customize", &super::CHEVRON, s.customize_open).cursor(rune::Cursor::Pointer).on_press(|s: &mut State| s.customize_open = !s.customize_open))
        .anchored("composer-box", rune::Side::Top, rune::Align::Start, 8.0)
}

fn customize_menu(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let prefs = &s.prefs;
    let check = |key: &str, label: &str, on: bool, f: fn(&mut crate::prefs::Prefs)| row().key(key.to_string()).h(34.0).px(12.0).center_y().radius(radius::SM).hover(p.select)
        .child(text(label.to_string()).font(type_::BODY).color(t.text).grow(1.0))
        .child(row().size(18.0, 18.0).radius(4.0).border(2.0, if on { oklch(0.58, 0.19, 277.0) } else { t.text_secondary }).center()
            .fill(if on { oklch(0.58, 0.19, 277.0) } else { clear() }).child(if on { rune::icon(&super::CHECK, 12.0).color(t.knob) } else { el().size(0.0, 0.0) }))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| { f(&mut s.prefs); crate::prefs::save(&s.prefs) });
    menu_card("customize-menu").w(240.0).px(6.0).py(6.0)
        .child(check("cu-gifs", "Show GIFs button", prefs.gifs_button, |p| p.gifs_button = !p.gifs_button))
        .child(check("cu-media", "Show media button", prefs.media_button, |p| p.media_button = !p.media_button))
        .child(check("cu-stickers", "Show stickers button", prefs.stickers_button, |p| p.stickers_button = !p.stickers_button))
        .child(check("cu-emoji", "Show emoji button", prefs.emoji_button, |p| p.emoji_button = !p.emoji_button))
        .child(check("cu-send", "Show send button", prefs.send_button, |p| p.send_button = !p.send_button))
        .anchored("pm-customize", rune::Side::Right, rune::Align::Start, 8.0)
}

fn picker(s: &State, tab: Tab) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let pill = |key: &str, label: &str, me: Tab| row().key(key.to_string()).h(28.0).px(10.0).radius(radius::SM).center()
        .fill(if tab == me { p.select } else { clear() }).hover(p.select)
        .child(text(label.to_string()).font(rune::tok::TextStyle::new(13.0, 700.0)).color(if tab == me { t.text } else { t.text_secondary }))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| { let api = s.net.api().cloned(); s.picker.open = None; s.picker.toggle(me, &api); crate::picker_images(s) });
    let tabs = row().px(12.0).py(10.0).gap(4.0)
        .child(pill("pk-gifs", "GIFs", Tab::Gifs)).child(pill("pk-media", "Media", Tab::Media))
        .child(pill("pk-stickers", "Stickers", Tab::Stickers)).child(pill("pk-emojis", "Emojis", Tab::Emojis));
    let body = match tab {
        Tab::Gifs => gifs_body(s),
        Tab::Media => media_body(s),
        Tab::Stickers => stickers_body(s),
        Tab::Emojis => emojis_body(s, insert_text),
    };
    menu_card("picker").w(PICKER_W).h(PICKER_H).fill(oklch(0.18, 0.008, 265.0))
        .child(tabs)
        .child(body)
        .anchored("composer-box", rune::Side::Top, rune::Align::End, 8.0)
}

fn search_box(s: &State, placeholder: &str, on_submit: fn(&mut State)) -> El {
    let (p, t) = (pal(), tok::theme(true));
    row().h(40.0).mx(12.0).px(12.0).gap(8.0).center_y().radius(8.0).fill(p.input).border(1.0, p.line)
        .child(rune::icon(&SEARCH, 16.0).color(t.text_secondary))
        .child(text_input(&s.picker.query, type_::BODY).key("picker-search").placeholder(placeholder.to_string(), t.text_secondary)
            .on_edit(|s: &mut State| &mut s.picker.query).on_submit(move |s: &mut State, _| on_submit(s)).grow(1.0).min_w(0.0))
}

fn empty_state(icon: &'static rune::Icon, title: &str, sub: &str) -> El {
    let t = tok::theme(true);
    col().grow(1.0).w_full().center().gap(8.0).px(40.0)
        .child(rune::icon(icon, 40.0).color(t.text_secondary))
        .child(text(title.to_string()).font(rune::tok::TextStyle::new(17.0, 700.0)).color(t.text))
        .child(text(sub.to_string()).font(type_::BODY).color(t.text_secondary).wrap().w(260.0).align(rune::Align::Center))
}

fn status_line(s: &State) -> Option<El> {
    let t = tok::theme(true);
    if s.picker.loading { return Some(row().w_full().h(40.0).center().child(text("Loading…").font(type_::LABEL).color(t.text_secondary))) }
    s.picker.error.as_ref().map(|e| row().w_full().px(16.0).py(10.0).child(text(e.clone()).font(type_::LABEL).color(pal().unread).wrap().w(PICKER_W - 32.0)))
}

/// A picture tile: the image (or a placeholder) with an optional label over it.
/// `anim` is the playing version: shown once loaded, and only while the window has the focus.
fn tile(s: &State, key: String, url: &str, anim: Option<&str>, w: f32, h: f32, label: Option<&str>) -> El {
    let p = pal();
    let mut e = el().key(key).size(w, h).radius(8.0).fill(p.input).border(1.0, p.line).cursor(rune::Cursor::Pointer);
    // Only the tile under the pointer moves: its animation is loaded when the pointer arrives.
    if let Some(a) = anim {
        let a = a.to_string();
        e = e.on_hover_change(move |s: &mut State, inside, _| {
            if inside { if s.picker.hover.as_deref() != Some(a.as_str()) { s.picker.hover = Some(a.clone()); crate::picker_images(s) } }
            else if s.picker.hover.as_deref() == Some(a.as_str()) { s.picker.hover = None }
        });
    }
    let still = s.net.image(url);
    let playing = if s.focused { anim.and_then(|a| s.net.image(a)) } else { None };
    if let Some(img) = playing.or(still) { e = e.pixels(img.pixels()) }
    if let Some(l) = label {
        e = e.child(el().absolute(0.0, 0.0).size(w, h).radius(8.0).fill(oklcha(0.0, 0.0, 0.0, 0.42)))
            .child(row().absolute(0.0, 0.0).size(w, h).center().child(text(l.to_string()).font(rune::tok::TextStyle::new(16.0, 800.0)).color(tok::theme(true).text)));
    }
    e
}

/// Two columns, each new picture going under the shorter column.
fn masonry(items: Vec<El>, heights: Vec<f32>) -> El {
    let (mut a, mut b) = (col().gap(8.0), col().gap(8.0));
    let (mut ha, mut hb) = (0.0, 0.0);
    for (e, h) in items.into_iter().zip(heights) {
        if ha <= hb { a = a.child(e); ha += h } else { b = b.child(e); hb += h }
    }
    row().gap(8.0).px(12.0).py(8.0).items_start().child(a).child(b)
}

fn gifs_body(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let pk = &s.picker;
    let w = (PICKER_W - 12.0 * 2.0 - 8.0) / 2.0;
    let search = search_box(s, "Search Klipy", |s| { let (q, api) = (s.picker.query.text.clone(), s.net.api().cloned()); s.picker.search(&q, &api); crate::picker_images(s) });
    let mut content = col().gap(8.0).px(12.0).py(8.0);
    match &pk.view {
        GifView::Home => {
            let trending_bg = pk.trending.first().map(|g| g.preview_url()).unwrap_or_default();
            let mut tiles = vec![tile(s, "gif-trending".into(), &trending_bg, pk.trending.first().and_then(|g| g.anim.as_deref()), w, 132.0, Some("↗ Trending GIFs")).on_press(|s: &mut State| { let api = s.net.api().cloned(); s.picker.show_trending(&api); crate::picker_images(s) })];
            for (i, c) in pk.categories.iter().enumerate() {
                let name = c.name.clone();
                tiles.push(tile(s, format!("gif-cat-{i}"), &c.preview_url(), c.anim.as_deref(), w, 132.0, Some(&c.name))
                    .on_press(move |s: &mut State| { let api = s.net.api().cloned(); s.picker.query = rune::TextEdit::new(name.clone()); s.picker.search(&name, &api); crate::picker_images(s) }));
            }
            for pair in tiles.chunks(2) { let _ = pair; }
            let mut it = tiles.into_iter();
            while let Some(a) = it.next() {
                let mut r = row().gap(8.0).child(a);
                if let Some(b) = it.next() { r = r.child(b) }
                content = content.child(r);
            }
        }
        view => {
            let list = if *view == GifView::Trending { &pk.trending } else { &pk.results };
            content = content.child(row().key("gif-back").h(30.0).gap(6.0).center_y().cursor(rune::Cursor::Pointer)
                .child(rune::icon(&ARROW_LEFT, 16.0).color(t.text_secondary))
                .child(text(match view { GifView::Trending => "Trending GIFs".to_string(), GifView::Search(q) => q.clone(), GifView::Home => String::new() }).font(type_::TITLE).color(t.text))
                .on_press(|s: &mut State| { s.picker.show_home(); crate::picker_images(s) }));
            if list.is_empty() && !pk.loading { content = content.child(text("No GIFs found").font(type_::BODY).color(t.text_secondary)) }
            let mut tiles = Vec::new();
            let mut heights = Vec::new();
            for (i, g) in list.iter().enumerate() {
                let h = if g.width > 0 && g.height > 0 { (w * g.height as f32 / g.width as f32).clamp(70.0, 240.0) } else { 120.0 };
                let url = g.url.clone();
                tiles.push(tile(s, format!("gif-{i}"), &g.preview_url(), g.anim.as_deref(), w, h, None).on_press(move |s: &mut State| send_link(s, url.clone())));
                heights.push(h);
            }
            content = content.child(masonry(tiles, heights).px(0.0).py(0.0));
        }
    }
    let mut body = col().grow(1.0).min_h(0.0).child(search).gap(4.0);
    if let Some(st) = status_line(s) { body = body.child(st) }
    let _ = p;
    body.child(content.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
}

fn media_body(s: &State) -> El {
    let t = tok::theme(true);
    let w = (PICKER_W - 12.0 * 2.0 - 8.0) / 2.0;
    let memes = s.picker.memes.as_deref().unwrap_or(&[]);
    if memes.is_empty() {
        return match status_line(s) { Some(l) => col().grow(1.0).child(l), None => empty_state(&SAD, "No saved media", "Save some media from messages to get started!") };
    }
    let mut tiles = Vec::new();
    let mut heights = Vec::new();
    for (i, m) in memes.iter().enumerate() {
        let h = if m.width > 0 && m.height > 0 { (w * m.height as f32 / m.width as f32).clamp(70.0, 240.0) } else { 120.0 };
        let url = m.url.clone();
        tiles.push(tile(s, format!("meme-{i}"), &m.preview_url(), None, w, h, None).on_press(move |s: &mut State| send_link(s, url.clone())));
        heights.push(h);
    }
    let _ = t;
    col().grow(1.0).min_h(0.0).child(masonry(tiles, heights).scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
}

fn stickers_body(s: &State) -> El {
    let stickers: Vec<(String, String)> = s.chat.guilds.iter().flat_map(|g| g.stickers.iter().cloned()).collect();
    let Some(media) = s.chat.media.as_deref() else { return empty_state(&SAD, "No stickers yet", "Stickers added to your communities show up here.") };
    if stickers.is_empty() { return empty_state(&SAD, "No stickers yet", "Stickers added to your communities show up here.") }
    let mut grid = col().gap(8.0).px(12.0).py(8.0);
    for chunk in stickers.chunks(3) {
        let mut r = row().gap(8.0);
        for (id, name) in chunk {
            let url = crate::model::sticker_url(media, id);
            let sid = id.clone();
            r = r.child(tile(s, format!("stk-{id}"), &url, None, 128.0, 128.0, None).on_press(move |s: &mut State| send_sticker(s, &sid)));
            let _ = name;
        }
        grid = grid.child(r);
    }
    col().grow(1.0).min_h(0.0).child(grid.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
}

fn emojis_body(s: &State, pick: fn(&mut State, &'static str)) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let groups = crate::picker::emoji_groups();
    let g = s.picker.emoji_group.min(groups.len() - 1);
    let q = s.picker.query.text.clone();
    let searching = !q.trim().is_empty();
    let list = if searching { crate::picker::emoji_search(&q) } else { crate::picker::emojis_in(&groups[g].2) };
    let mut sidebar = col().w(48.0).h_full().py(8.0).gap(4.0).items_center();
    for (i, (name, glyph, _)) in groups.iter().enumerate() {
        sidebar = sidebar.child(row().key(format!("eg-{i}")).size(34.0, 34.0).radius(radius::SM).center()
            .fill(if i == g && !searching { p.select } else { clear() }).hover(p.select)
            .child(text(glyph.to_string()).font(rune::tok::TextStyle::new(18.0, 400.0)).color(t.text))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| { s.picker.emoji_group = i; s.picker.query = rune::TextEdit::new("") }));
        let _ = name;
    }
    let mut grid = col().gap(4.0).px(10.0).py(6.0);
    grid = grid.child(row().h(28.0).px(4.0).center_y().child(text(if searching { "Results".to_string() } else { groups[g].0.to_string() }).font(rune::tok::TextStyle::new(13.0, 700.0)).color(t.text_secondary)));
    for chunk in list.chunks(9) {
        let mut r = row().gap(4.0);
        for e in chunk {
            let glyph = e.as_str();
            r = r.child(row().key(format!("em-{glyph}")).size(38.0, 38.0).radius(radius::SM).center().hover(p.select)
                .child(text(glyph.to_string()).font(rune::tok::TextStyle::new(24.0, 400.0)).color(t.text))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| pick(s, glyph)));
        }
        grid = grid.child(r);
    }
    col().grow(1.0).min_h(0.0).gap(4.0)
        .child(search_box(s, "Find the emoji of your dreams", |_| {}))
        .child(row().grow(1.0).min_h(0.0)
            .child(sidebar)
            .child(grid.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_w(0.0)))
}

/// The emoji grid in a floating card beside the element keyed `anchor`; `pick` gets each emoji pressed.
pub fn emoji_popover(s: &State, anchor: &str, pick: fn(&mut State, &'static str)) -> El {
    menu_card("emoji-popover").w(PICKER_W).h(PICKER_H).fill(oklch(0.18, 0.008, 265.0))
        .child(emojis_body(s, pick))
        .anchored(anchor.to_string(), rune::Side::Left, rune::Align::Start, 8.0)
}

//! Views: login, the guild rail, the channel list, the virtualized message list and the composer.

use crate::model::{Channel, Delivery, Msg};
use crate::{State, Status, auth, sign_out, submit, send, open_guild, open_channel, retry, load_older};
use fluxer_client::api::{self, Attachment, User};
use rune::tok::{self, radius, type_};
use rune::{Color, Cx, El, GrowthSpec, ScrollSpec, col, el, oklch, oklcha, row, text, text_input};
use fluxer_client::api::{Embed, snowflake_cmp};
use crate::roster::{self, Row};
use rune_kit::{Button, ButtonVariant, Kit, Markdown, MarkdownStyle, TextInput};

const RAIL_WIDTH: f32 = 72.0;

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

pub fn view(s: &State, _: &mut Cx<State>) -> El {
    match s.screen { crate::Screen::Login => login_view(s), crate::Screen::Main => main_view(s) }
}

fn login_view(s: &State) -> El {
    let t = tok::theme(true);
    let k = &Kit::neutral(true);
    let a = &s.auth;
    let labelled = |label: &str, input: El| col().gap(6.0).child(text(label).font(type_::LABEL).color(t.text_secondary)).child(input);
    let in_code_step = matches!(a.step, auth::Step::Code { .. });
    let waiting = if let auth::Step::Waiting { email, .. } = &a.step { Some(email.clone()) } else { None };

    let mut form = col().gap(14.0);
    if let Some(email) = &waiting {
        form = form
            .child(text("Verifique seu e-mail").font(type_::TITLE).color(t.text))
            .child(text(format!("Enviamos um link de autorização para {email}. Assim que você autorizar, o Fluxer entra na sua conta automaticamente."))
                .key("ip-waiting").font(type_::BODY).color(t.text_secondary).wrap().w(304.0))
            .child(text("Aguardando autorização…").font(type_::LABEL).color(tok::accent(0, true)));
    } else if in_code_step {
        form = form
            .child(text("Verificação em duas etapas").font(type_::TITLE).color(t.text))
            .child(text("Digite o código do seu aplicativo autenticador.").font(type_::BODY).color(t.text_secondary).wrap().w(304.0))
            .child(TextInput::new(&a.code).placeholder("000000").label("Código")
                .width(Some(304.0)).on_edit(|s: &mut State| &mut s.auth.code)
                .on_submit(|s: &mut State, _| submit(s)).build(k));
    } else {
        form = form
            .child(text("Entrar no Fluxer").font(type_::TITLE).color(t.text))
            .child(labelled("Instância", TextInput::new(&a.instance).label("Instância").width(Some(304.0))
                .on_edit(|s: &mut State| &mut s.auth.instance).on_submit(|s: &mut State, _| submit(s)).build(k)))
            .child(labelled("E-mail ou usuário", TextInput::new(&a.identifier).label("E-mail ou usuário").width(Some(304.0))
                .on_edit(|s: &mut State| &mut s.auth.identifier).on_submit(|s: &mut State, _| submit(s)).build(k)))
            .child(labelled("Senha", TextInput::new(&a.password).label("Senha").secure().width(Some(304.0))
                .on_edit(|s: &mut State| &mut s.auth.password).on_submit(|s: &mut State, _| submit(s)).build(k)));
    }
    if let Some(e) = &a.error {
        form = form.child(text(e.clone()).key("login-error").font(type_::BODY).color(k.c.danger.fg).wrap().w(304.0));
    }
    if let Some(n) = &a.info {
        form = form.child(text(n.clone()).key("login-info").font(type_::BODY).color(t.text_secondary).wrap().w(304.0));
    }
    if waiting.is_some() {
        form = form.child(Button::new("Reenviar e-mail").variant(ButtonVariant::Neutral)
            .on_press(|s: &mut State| s.auth.resend()).build(k));
    } else {
        let label = match (a.busy, in_code_step) { (true, _) => "Entrando…", (_, true) => "Verificar", _ => "Entrar" };
        form = form.child(Button::new(label).variant(ButtonVariant::Submit).disabled(a.busy)
            .on_press(|s: &mut State| submit(s)).build(k));
    }
    if in_code_step || waiting.is_some() {
        form = form.child(text("Voltar").key("login-back").font(type_::LABEL).color(t.text_secondary)
            .on_press(|s: &mut State| s.auth.back_to_credentials()));
    }
    let p = pal();
    let card = form.w(380.0).padding(32.0).radius(radius::LG).fill(p.side).border(1.0, p.line).shadow(12.0, 40.0, t.shadow);
    let brand = row().size(48.0, 48.0).radius(14.0).fill(p.brand).center()
        .child(text("F").font(rune::tok::TextStyle::new(24.0, 700.0)).color(t.knob));
    col().w_full().h_full().center().gap(20.0).fill(p.rail).child(brand).child(card)
}


fn status_label(s: Status) -> (&'static str, usize) {
    match s {
        Status::Demo => ("demo", 3),
        Status::Connecting => ("conectando…", 3),
        Status::Connected => ("conectado", 1),
        Status::Reconnecting => ("reconectando…", 2),
        Status::Failed => ("sem acesso", 4),
    }
}

fn tint(id: &str) -> usize { id.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize)) % 4 }

fn initial(name: &str) -> String { name.chars().next().map_or("?".into(), |c| c.to_uppercase().to_string()) }

/// A round avatar: the downloaded picture, or a tinted circle with the initial until it arrives.
fn avatar(s: &State, user: &User, size: f32) -> El {
    let t = tok::theme(true);
    let img = s.chat.media.as_deref().and_then(|m| api::avatar_url(m, user, 128)).and_then(|u| s.net.image(&u).cloned());
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

fn badge(n: u32) -> El {
    if n == 0 { return el().size(0.0, 0.0) }
    let p = pal();
    row().h(18.0).px(5.0).radius(9.0).fill(p.unread).border(2.0, p.rail).center()
        .child(text(if n > 99 { "99+".into() } else { n.to_string() }).font(type_::LABEL).color(tok::theme(true).knob))
}

fn guild_rail(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let nav = s.chat.nav;
    let tile = |key: String, selected: bool, unread: u32, new: bool, body: El, on: Box<dyn Fn(&mut State)>| {
        let pill = if selected { 28.0 } else if unread > 0 || new { 8.0 } else { 0.0 };
        row().key(key).w(RAIL_WIDTH).h(52.0).center()
            .child(el().absolute(0.0, (52.0 - pill) / 2.0).size(4.0, pill).radius(2.0).fill(t.text))
            .child(row().size(48.0, 48.0).center().child(body).child(el().absolute(30.0, 0.0).child(badge(unread))))
            .on_press(move |s: &mut State| on(s))
    };
    let radius_of = |selected: bool| if selected { 15.0 } else { 24.0 };
    let dm_sel = nav.guild.is_none();
    let dms = row().size(48.0, 48.0).radius(radius_of(dm_sel)).fill(if dm_sel { p.brand } else { p.input }).center()
        .child(text("DM").font(type_::TITLE).color(if dm_sel { t.knob } else { t.text }));
    let mut rail = col().w(RAIL_WIDTH).h_full().py(12.0).gap(8.0).fill(p.rail)
        .child(tile("rail-dm".into(), dm_sel, s.chat.unread(None), s.chat.has_new(None), dms, Box::new(|s| open_guild(s, None))))
        .child(row().w_full().h(2.0).px(24.0).child(el().w_full().h(2.0).radius(1.0).fill(p.line)));
    for i in 0..s.chat.guilds.len() {
        let sel = nav.guild == Some(i);
        rail = rail.child(tile(format!("rail-{i}"), sel, s.chat.unread(Some(i)), s.chat.has_new(Some(i)), guild_icon(s, i, 48.0, radius_of(sel)),
            Box::new(move |s| open_guild(s, Some(i)))));
    }
    rail.scroll(ScrollSpec { stick_bottom: false })
}

fn channel_row(s: &State, i: usize, ch: &Channel) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let active = i == s.chat.nav.channel;
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
        .on_press(move |s: &mut State| open_channel(s, i))
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
    let mut list = col().gap(2.0).px(8.0).py(8.0);
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
    col().w(248.0).h_full().fill(p.side)
        .child(row().h(48.0).px(16.0).center_y().child(text(title).font(type_::TITLE).color(t.text)))
        .child(el().w_full().h(1.0).fill(p.line))
        .child(list.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
        .child(user_panel(s))
}

fn user_panel(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let Some(u) = s.chat.me.as_ref() else { return el().size(0.0, 0.0) };
    row().h(52.0).px(10.0).gap(10.0).center_y().fill(p.rail)
        .child(avatar(s, u, 32.0))
        .child(text(u.display().to_string()).font(type_::TITLE).color(t.text).grow(1.0))
}

fn human_size(b: u64) -> String {
    match b { 0..=1023 => format!("{b} B"), 1024..=1_048_575 => format!("{:.0} KB", b as f64 / 1024.0), _ => format!("{:.1} MB", b as f64 / 1_048_576.0) }
}

fn attachment(s: &State, a: &Attachment) -> El {
    let t = tok::theme(true);
    let url = a.url.clone();
    let open = move |_: &mut State, tx: &mut rune::Tx| { if let Some(u) = &url { let _ = tx.open_url(u); } };
    if a.is_image() {
        let (w, h) = (a.width.unwrap_or(320) as f32, a.height.unwrap_or(200) as f32);
        let k = (ATTACHMENT_MAX.0 / w).min(ATTACHMENT_MAX.1 / h).min(1.0);
        let (dw, dh) = ((w * k).max(40.0), (h * k).max(40.0));
        let fetched = a.url.as_deref().map(|u| api::attachment_image_url(u, crate::model::ATTACHMENT_FETCH_WIDTH));
        let base = el().key(format!("att-{}", a.id)).size(dw, dh).radius(radius::SM);
        return match fetched.and_then(|u| s.net.image(&u).cloned()) {
            Some(img) => base.pixels(img.pixels()).on_press_tx(open),
            None => base.fill(t.control_off),
        };
    }
    row().key(format!("att-{}", a.id)).px(10.0).py(8.0).gap(8.0).radius(radius::SM).fill(t.card)
        .child(text(format!("📎 {}", a.filename)).font(type_::BODY).color(tok::accent(0, true)))
        .child(text(human_size(a.size)).font(type_::LABEL).color(t.text_secondary))
        .on_press_tx(open)
}

const FOLLOW_WINDOW_MS: i64 = 7 * 60_000;

fn open_link(_: &mut State, url: &str, tx: &mut rune::Tx) { let _ = tx.open_url(url); }

/// The message text: a plain label when it has no markup, else Markdown with mentions rewritten.
fn message_text(m: &Msg, dim: bool) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let src = crate::richtext::prepare(&m.body, &m.mentions);
    let color = if dim { t.text_secondary } else { t.text };
    if !crate::richtext::needs_markdown(&src) {
        return text(src.into_owned()).font(type_::BODY).color(color).wrap();
    }
    let style = MarkdownStyle::neutral(true).text(color).muted(t.text_secondary).link(p.link)
        .body_font(rune::tok::TextStyle { line_height: 20.0, ..type_::BODY })
        .code_block(p.rail, p.line).inline_code_bg(p.input);
    Markdown::new(src.into_owned()).selectable(true).on_link_tx(open_link).render(&style).key(format!("md-{}", m.id))
}

fn reply_bar(s: &State, m: &Msg) -> Option<El> {
    let (p, t) = (pal(), tok::theme(true));
    let inner = match &m.reply {
        Some(r) => {
            let what = if r.content.trim().is_empty() && r.has_attachments { "Anexo".to_string() } else { crate::richtext::preview(&r.content, 120) };
            row().gap(6.0).center_y()
                .child(avatar(s, &r.author, 16.0))
                .child(text(r.author.display().to_string()).font(type_::LABEL).color(tok::accent(tint(&r.author.id), true)))
                .child(text(what).font(type_::LABEL).color(t.text_secondary))
        }
        None if m.kind == 19 => row().center_y().child(text("Mensagem original indisponível").font(type_::LABEL).color(t.text_secondary)),
        None => return None,
    };
    Some(row().w_full().h(22.0).center_y().child(el().size(2.0, 14.0).radius(1.0).fill(p.line)).child(inner.px(8.0)))
}

fn embed_card(s: &State, key: String, e: &Embed) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let bar = e.color.map_or(oklch(0.4, 0.012, 265.0), |c| rune::color::hex(&format!("#{:06x}", c & 0xFF_FFFF)).unwrap_or(p.line));
    let mut body = col().gap(4.0).min_w(0.0);
    if let Some(v) = e.provider.as_ref().or(e.author.as_ref()) { body = body.child(text(v.clone()).font(type_::LABEL).color(t.text_secondary)) }
    if let Some(v) = &e.title { body = body.child(text(v.clone()).font(type_::TITLE).color(p.link).wrap()) }
    if let Some(v) = &e.description { body = body.child(text(crate::richtext::preview(v, 280)).font(type_::BODY).color(t.text).wrap()) }
    if let Some(u) = &e.image {
        let img = s.net.image(&api::attachment_image_url(u, crate::model::EMBED_FETCH_WIDTH));
        let (w, h) = img.and_then(|i| i.size()).map_or((400.0, 210.0), |[w, h]| (w as f32, h as f32));
        let k = (400.0 / w).min(260.0 / h).min(1.0);
        let pic = el().size((w * k).max(40.0), (h * k).max(40.0)).radius(radius::SM);
        let pic = match img { Some(i) => pic.pixels(i.pixels()), None => pic.fill(p.input) };
        body = body.child(if e.video { col().child(pic).child(text("▶ vídeo").font(type_::LABEL).color(t.text_secondary)) } else { col().child(pic) });
    }
    let url = e.url.clone();
    let card = row().key(key).max_w(460.0).radius(radius::SM).fill(p.side).border(1.0, p.line)
        .child(el().w(4.0).fill(bar)).child(body.padding(12.0));
    match url { Some(u) => card.on_press_tx(move |_: &mut State, tx: &mut rune::Tx| { let _ = tx.open_url(&u); }), None => card }
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
    let mut body = col().gap(2.0).grow(1.0).min_w(0.0);
    if !follow {
        let mut head = row().gap(8.0).center_y()
            .child(text(m.author.display().to_string()).font(type_::TITLE).color(tok::accent(tint(&m.author.id), true)));
        if let Some(ts) = m.ts { head = head.child(text(crate::clock::full(ts, now)).font(type_::LABEL).color(t.text_secondary.alpha(0.75))) }
        if m.delivery == Delivery::Pending { head = head.child(text("enviando…").font(type_::LABEL).color(t.text_secondary)) }
        body = body.child(head);
    } else if m.delivery == Delivery::Pending {
        body = body.child(text("enviando…").font(type_::LABEL).color(t.text_secondary));
    }
    if !m.body.is_empty() {
        let mut line = row().gap(6.0).child(message_text(m, m.delivery != Delivery::Sent).grow(1.0).min_w(0.0));
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
            .on_press(move |s: &mut State| retry(s, &id)));
    }
    let lead = match (follow, m.ts) {
        (true, Some(ts)) => row().w(40.0).h(20.0).center().child(text(crate::clock::hhmm(ts)).font(type_::LABEL).color(t.text_secondary.alpha(0.0))),
        (true, None) => el().size(40.0, 1.0),
        (false, _) => avatar(s, &m.author, 40.0),
    };
    if let Some(bar) = reply_bar(s, m) { out = out.child(row().w_full().px(16.0).py(4.0).child(el().w(54.0).h(1.0)).child(bar)) }
    out.child(row().w_full().px(16.0).py(if follow { 1.0 } else { 6.0 }).gap(14.0).hover(p.hover).child(lead).child(body))
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
        .virtual_rows_version((cid.clone(), first));
    let mut top = col();
    if let Some(l) = s.chat.log() {
        if l.has_more {
            top = top.child(row().key("older").w_full().h(32.0).center()
                .child(text(if l.loading { "Carregando…" } else { "Carregar mensagens anteriores" }).font(type_::LABEL).color(tok::accent(0, true)))
                .on_press(|s: &mut State| load_older(s)));
        } else if l.loaded && n == 0 {
            top = top.child(row().w_full().h(60.0).center().child(text("Nenhuma mensagem aqui ainda.").font(type_::BODY).color(t.text_secondary)));
        }
    }
    // A fresh scroll area per channel, so each one opens at its newest message. The virtual list
    // must be the scroll area's direct child, so the "older" bar sits above it.
    col().grow(1.0).min_h(0.0).child(top)
        .child(col().key(format!("scroll-{cid}")).scroll(ScrollSpec { stick_bottom: true }).grow(1.0).min_h(0.0).child(rows))
}

fn status_color(status: &str) -> Color {
    match status {
        "online" => tok::accent(4, true),
        "idle" => tok::accent(3, true),
        "dnd" => pal().unread,
        _ => oklch(0.5, 0.01, 265.0),
    }
}

fn member_row(s: &State, user: &api::User, nick: Option<&str>, status: &str) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let offline = status == "offline";
    let face = row().size(32.0, 32.0).child(avatar(s, user, 32.0))
        .child(el().absolute(21.0, 21.0).size(12.0, 12.0).radius(6.0).fill(status_color(status)).border(2.0, p.side));
    row().key(format!("mem-{}", user.id)).h(40.0).px(10.0).gap(10.0).center_y().radius(radius::SM).hover(p.hover)
        .child(face.opacity(if offline { 0.5 } else { 1.0 }))
        .child(text(nick.unwrap_or_else(|| user.display()).to_string()).font(type_::BODY)
            .color(if offline { t.text_secondary } else { t.text }).grow(1.0).min_w(0.0))
}

fn members_panel(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let guild = s.chat.nav.guild.and_then(|g| s.chat.guilds.get(g));
    let total = guild.and_then(|g| s.chat.roster.totals.get(&g.id)).map_or(String::new(), |(m, _)| format!(" — {m}"));
    let mut list = col().px(8.0).py(8.0).gap(1.0);
    match guild.and_then(|g| s.chat.roster.lists.get(&g.id)) {
        Some(rows) if !rows.is_empty() => for r in rows {
            list = list.child(match r {
                Row::Group { id, count } => row().h(32.0).px(10.0).center_y()
                    .child(text(format!("{} — {count}", s.chat.roster.group_label(id).to_uppercase())).font(type_::SECTION).color(t.text_secondary)),
                Row::Member { user, nick, status } => member_row(s, user, nick.as_deref(), s.chat.roster.status.get(&user.id).map_or(status.as_str(), String::as_str)),
            });
        },
        _ => list = list.child(row().h(40.0).px(10.0).center_y().child(text("Carregando membros…").font(type_::LABEL).color(t.text_secondary))),
    }
    col().w(240.0).h_full().fill(p.side)
        .child(row().h(48.0).px(16.0).center_y().child(text(format!("Membros{total}")).font(type_::TITLE).color(t.text)))
        .child(el().w_full().h(1.0).fill(p.line))
        .child(list.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
}

fn typing_bar(s: &State) -> El {
    let t = tok::theme(true);
    let me = s.chat.me.as_ref().map(|u| u.id.as_str());
    let msg = s.chat.channel_id().and_then(|c| roster::typing_text(&s.chat.roster.typing_names(c, me, crate::clock::now_ms())));
    row().w_full().h(20.0).px(20.0).center_y().child(text(msg.unwrap_or_default()).key("typing").font(type_::LABEL).color(t.text_secondary))
}

fn main_view(s: &State) -> El {
    let (p, t) = (pal(), tok::theme(true));
    let (label, tone) = status_label(s.status);
    let prefix = if s.chat.nav.guild.is_some() { "#" } else { "@" };
    let name = s.chat.channel().map_or(String::new(), |c| c.name.clone());
    let in_guild = s.chat.nav.guild.is_some();
    let mut header = row().h(48.0).px(16.0).gap(8.0).center_y()
        .child(text(prefix).font(type_::TITLE).color(t.text_secondary))
        .child(text(name.clone()).font(type_::TITLE).color(t.text).grow(1.0))
        .child(el().size(8.0, 8.0).radius(4.0).fill(tok::accent(tone, true)))
        .child(text(label).font(type_::LABEL).color(t.text_secondary));
    if in_guild {
        header = header.child(text("Membros").key("toggle-members").font(type_::LABEL).color(if s.show_members { t.text } else { t.text_secondary })
            .px(10.0).py(6.0).radius(radius::SM).fill(if s.show_members { p.select } else { oklcha(0.0, 0.0, 0.0, 0.0) }).hover(p.select)
            .on_press(|s: &mut State| s.show_members = !s.show_members));
    }
    header = header.child(text(if s.session.is_some() { "Sair" } else { "" }).key("sign-out").font(type_::LABEL).color(t.text_secondary)
        .px(10.0).py(6.0).radius(radius::SM).hover(p.select)
        .on_press(|s: &mut State| sign_out(s, None)));
    let composer = col().px(16.0).py(8.0).child(
        col().px(16.0).py(12.0).radius(10.0).fill(p.input).border(1.0, p.line)
            .child(text_input(&s.draft, type_::BODY)
                .key("composer")
                .on_edit(|s: &mut State| &mut s.draft)
                .on_submit(|s: &mut State, _| send(s))),
    );
    let main = col().grow(1.0).h_full().min_w(0.0).fill(p.chat).child(header).child(el().w_full().h(1.0).fill(p.line))
        .child(message_list(s)).child(typing_bar(s)).child(composer);
    let mut all = row().w_full().h_full().fill(p.rail).child(guild_rail(s)).child(channel_list(s)).child(main);
    if in_guild && s.show_members { all = all.child(members_panel(s)) }
    all
}

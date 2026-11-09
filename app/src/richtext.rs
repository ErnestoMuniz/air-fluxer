//! Turns Fluxer message text into Markdown Rune can draw: mentions and custom emoji become plain
//! text, and single newlines become hard line breaks (chat semantics, not CommonMark's).

use fluxer_client::api::User;
use std::borrow::Cow;

/// Replaces `<@id>`, `<@&id>`, `<#id>`, `<:name:id>` and `<a:name:id>`.
fn rewrite_tokens(src: &str, mentions: &[User]) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else { out.push_str(tail); return out };
        let inner = &tail[1..end];
        match token(inner, mentions) {
            Some(t) => { out.push_str(&t); rest = &tail[end + 1..] }
            None => { out.push('<'); rest = &tail[1..] }
        }
    }
    out.push_str(rest);
    out
}

fn token(inner: &str, mentions: &[User]) -> Option<String> {
    if let Some(id) = inner.strip_prefix("@&") {
        return id.bytes().all(|b| b.is_ascii_digit()).then(|| "**@cargo**".into());
    }
    if let Some(id) = inner.strip_prefix("@!").or_else(|| inner.strip_prefix('@')) {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) { return None }
        let name = mentions.iter().find(|u| u.id == id).map_or("alguém", |u| u.display());
        return Some(format!("**@{}**", name.replace('*', "")));
    }
    if let Some(id) = inner.strip_prefix('#') {
        return (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| "**#canal**".into());
    }
    let emoji = inner.strip_prefix("a:").or_else(|| inner.strip_prefix(':'))?;
    let (name, id) = emoji.trim_end_matches(':').rsplit_once(':')?;
    (!name.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty()).then(|| format!(":{name}:"))
}

/// Single newlines become hard breaks, except inside code fences.
fn hard_breaks(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 8);
    let mut fenced = false;
    let lines: Vec<&str> = src.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") { fenced = !fenced }
        out.push_str(line);
        if i + 1 < lines.len() {
            if fenced || line.trim_start().starts_with("```") || line.is_empty() || lines[i + 1].is_empty() { out.push('\n') } else { out.push_str("  \n") }
        }
    }
    out
}

/// True when the text needs the Markdown renderer; plain text is drawn as a simple wrapped label.
pub fn needs_markdown(s: &str) -> bool {
    s.contains(['*', '_', '~', '`', '>', '#', '[', ']', '|', '\n', '<']) || s.contains("http") || s.starts_with("- ") || s.starts_with("1.")
}

/// The Markdown source to render for a message body.
pub fn prepare<'a>(body: &'a str, mentions: &[User]) -> Cow<'a, str> {
    if !body.contains('<') && !body.contains('\n') { return Cow::Borrowed(body) }
    Cow::Owned(hard_breaks(&rewrite_tokens(body, mentions)))
}

/// A one-line preview for quoted replies: collapsed whitespace, cut at `max` characters.
pub fn preview(body: &str, max: usize) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max { flat } else { format!("{}…", flat.chars().take(max).collect::<String>().trim_end()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str, name: &str) -> User { User { id: id.into(), username: name.into(), global_name: None, avatar: None } }

    #[test]
    fn mentions_and_emoji_become_text() {
        let m = [user("12", "ana")];
        assert_eq!(prepare("oi <@12> e <@!12>!", &m), "oi **@ana** e **@ana**!");
        assert_eq!(prepare("<@99> sumiu", &m), "**@alguém** sumiu");
        assert_eq!(prepare("<:pepe:123456> <a:dance:99>", &[]), ":pepe: :dance:");
        assert_eq!(prepare("veja <#555> e <@&7>", &[]), "veja **#canal** e **@cargo**");
        assert_eq!(prepare("a < b e c > d", &[]), "a < b e c > d", "a stray angle bracket is left alone");
    }

    #[test]
    fn newlines_become_hard_breaks_but_not_inside_code() {
        assert_eq!(prepare("a\nb", &[]), "a  \nb");
        assert_eq!(prepare("a\n\nb", &[]), "a\n\nb");
        assert_eq!(prepare("```\nx\ny\n```\nz", &[]), "```\nx\ny\n```\nz");
    }

    #[test]
    fn plain_text_skips_the_markdown_renderer() {
        assert!(!needs_markdown("bom dia a todos"));
        assert!(needs_markdown("isso é **forte**"));
        assert!(needs_markdown("veja https://exemplo.com"));
        assert!(needs_markdown("duas\nlinhas"));
    }

    #[test]
    fn previews_are_one_line_and_cut() {
        assert_eq!(preview("a\n  b   c", 20), "a b c");
        assert_eq!(preview("abcdefghij", 5), "abcde…");
    }
}

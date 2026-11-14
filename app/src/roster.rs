//! Who is around: the member list of the open server, online status and "is typing" notices.
//! Everything here is read-only gateway data.

use fluxer_client::api::User;
use serde_json::Value;
use std::collections::HashMap;

/// How long a typing notice lasts without a new one (the web app uses a similar window).
pub const TYPING_TTL_MS: i64 = 8_000;
/// Member lists are requested in one page; this is the span the gateway allows.
pub const LIST_RANGE: (u32, u32) = (0, 99);

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A heading: `online`, `offline` or a role id.
    Group { id: String, count: u64 },
    Member { user: User, nick: Option<String>, status: String },
}

#[derive(Default)]
pub struct Roster {
    /// Member list rows per server.
    pub lists: HashMap<String, Vec<Row>>,
    /// Latest known status per user (`online`, `idle`, `dnd`, `offline`).
    pub status: HashMap<String, String>,
    /// Users seen anywhere (READY, messages, member lists), to name people in typing notices.
    pub users: HashMap<String, User>,
    /// Role names by id, for group headings.
    pub roles: HashMap<String, String>,
    /// A role's icon: custom picture hash, or a unicode emoji.
    pub role_icon: HashMap<String, (Option<String>, Option<String>)>,
    /// Role colour (0 = none) and position by role id.
    role_style: HashMap<String, (u32, i64)>,
    /// Role ids per (server, user), from member lists, READY and messages.
    member_roles: HashMap<(String, String), Vec<String>>,
    /// Nicknames per (server, user).
    nicks: HashMap<(String, String), String>,
    pub totals: HashMap<String, (u64, u64)>,
    typing: HashMap<String, Vec<(String, i64)>>,
}

impl Roster {
    pub fn remember(&mut self, u: &User) {
        match self.users.get_mut(&u.id) {
            Some(old) if old != u => *old = u.clone(),
            Some(_) => {}
            None => { self.users.insert(u.id.clone(), u.clone()); }
        }
    }

    pub fn load_ready(&mut self, d: &Value) {
        for u in d.get("users").and_then(Value::as_array).into_iter().flatten().filter_map(User::from_json) { self.remember(&u) }
        for g in d.get("guilds").and_then(Value::as_array).into_iter().flatten() {
            for r in g.get("roles").and_then(Value::as_array).into_iter().flatten() {
                if let (Some(id), Some(name)) = (r.get("id").and_then(Value::as_str), r.get("name").and_then(Value::as_str)) {
                    self.roles.insert(id.into(), name.into());
                    self.role_style.insert(id.into(), (r.get("color").and_then(Value::as_u64).unwrap_or(0) as u32, r.get("position").and_then(Value::as_i64).unwrap_or(0)));
                    let icon = r.get("icon").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
                    let emoji = r.get("unicode_emoji").or_else(|| r.get("unicodeEmoji")).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
                    if icon.is_some() || emoji.is_some() { self.role_icon.insert(id.into(), (icon, emoji)); }
                }
            }
            if let Some(gid) = g.get("id").and_then(Value::as_str) {
                for m in g.get("members").and_then(Value::as_array).into_iter().flatten() {
                    if let (Some(uid), Some(roles)) = (m.pointer("/user/id").and_then(Value::as_str), m.get("roles")) { self.record_roles(gid, uid, roles) }
                    if let Some(uid) = m.pointer("/user/id").and_then(Value::as_str) { self.set_nick(gid, uid, m.get("nick").and_then(Value::as_str)) }
                }
            }
        }
        for p in d.get("presences").and_then(Value::as_array).into_iter().flatten() { self.on_presence(p); }
    }

    /// Remembers a person's roles in a server from a JSON array of ids.
    pub fn record_roles(&mut self, guild: &str, user: &str, roles: &Value) {
        let ids: Vec<String> = roles.as_array().into_iter().flatten().filter_map(|r| r.as_str().map(String::from)).collect();
        self.member_roles.insert((guild.into(), user.into()), ids);
    }

    pub fn set_nick(&mut self, guild: &str, user: &str, nick: Option<&str>) {
        let key = (guild.to_string(), user.to_string());
        match nick.filter(|n| !n.is_empty()) { Some(n) => { self.nicks.insert(key, n.into()); } None => { self.nicks.remove(&key); } }
    }

    /// The name someone goes by in a server: their nickname there.
    pub fn nick(&self, guild: &str, user: &str) -> Option<&str> {
        self.nicks.get(&(guild.to_string(), user.to_string())).map(String::as_str)
    }

    pub fn set_roles(&mut self, guild: &str, user: &str, roles: Vec<String>) {
        self.member_roles.insert((guild.into(), user.into()), roles);
    }

    /// A person's roles, highest first: (id, name, colour, custom-picture hash, unicode emoji).
    pub fn member_roles_sorted(&self, guild: &str, user: &str) -> Vec<(String, String, u32, Option<String>, Option<String>)> {
        let mut roles: Vec<&String> = self.member_roles.get(&(guild.to_string(), user.to_string())).into_iter().flatten().collect();
        roles.sort_by_key(|id| std::cmp::Reverse(self.role_style.get(*id).map_or(0, |s| s.1)));
        roles.iter().map(|id| {
            let (icon, emoji) = self.role_icon.get(*id).cloned().unwrap_or_default();
            ((*id).clone(), self.roles.get(*id).cloned().unwrap_or_else(|| "Cargo".into()), self.role_style.get(*id).map_or(0, |s| s.0), icon, emoji)
        }).collect()
    }

    /// Their topmost role that has an icon at all, for the little icon beside the name.
    /// (id, name, custom-picture hash, unicode emoji)
    pub fn top_role_icon(&self, guild: &str, user: &str) -> Option<(String, String, Option<String>, Option<String>)> {
        let mut roles: Vec<&String> = self.member_roles.get(&(guild.to_string(), user.to_string())).into_iter().flatten().collect();
        roles.sort_by_key(|id| std::cmp::Reverse(self.role_style.get(*id).map_or(0, |s| s.1)));
        roles.into_iter().find(|id| self.role_icon.get(*id).is_some_and(|(icon, emoji)| icon.is_some() || emoji.is_some())).map(|id| {
            let (icon, emoji) = self.role_icon.get(id).cloned().unwrap_or_default();
            (id.clone(), self.roles.get(id).cloned().unwrap_or_else(|| "Cargo".into()), icon, emoji)
        })
    }

    /// The colour of a person's name in a server: their highest role that has one (like the web app,
    /// falling back to @everyone's). None means the default text colour.
    pub fn name_color(&self, guild: &str, user: &str) -> Option<u32> {
        let mut roles: Vec<&(u32, i64)> = self.member_roles.get(&(guild.to_string(), user.to_string())).into_iter().flatten()
            .filter_map(|id| self.role_style.get(id)).collect();
        roles.sort_by(|a, b| b.1.cmp(&a.1));
        roles.into_iter().map(|r| r.0).find(|c| *c != 0)
            .or_else(|| self.role_style.get(guild).map(|r| r.0).filter(|c| *c != 0))
    }

    /// Records a status; true when it is a change (the same status again redraws nothing).
    pub fn on_presence(&mut self, d: &Value) -> bool {
        let id = d.pointer("/user/id").and_then(Value::as_str);
        let (Some(id), Some(st)) = (id, d.get("status").and_then(Value::as_str)) else { return false };
        if self.status.get(id).is_some_and(|old| old == st) { return false }
        self.status.insert(id.into(), st.into());
        true
    }

    /// `GUILD_MEMBER_LIST_UPDATE`: the only operation the server sends is a SYNC of a range.
    pub fn on_member_list(&mut self, d: &Value) {
        let Some(guild) = d.get("guild_id").and_then(Value::as_str) else { return };
        self.totals.insert(guild.into(), (d.get("member_count").and_then(Value::as_u64).unwrap_or(0), d.get("online_count").and_then(Value::as_u64).unwrap_or(0)));
        for op in d.get("ops").and_then(Value::as_array).into_iter().flatten() {
            if op.get("op").and_then(Value::as_str) != Some("SYNC") { continue }
            let start = op.pointer("/range/0").and_then(Value::as_u64).unwrap_or(0) as usize;
            let end = op.pointer("/range/1").and_then(Value::as_u64).unwrap_or(0) as usize;
            let rows: Vec<Row> = op.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(|it| self.row(Some(guild), it)).collect();
            let list = self.lists.entry(guild.into()).or_default();
            if start >= list.len() { list.extend(rows) } else { let to = (end + 1).min(list.len()); list.splice(start..to, rows); }
        }
    }

    fn row(&mut self, guild: Option<&str>, it: &Value) -> Option<Row> {
        if let Some(g) = it.get("group") {
            return Some(Row::Group { id: g.get("id")?.as_str()?.into(), count: g.get("count").and_then(Value::as_u64).unwrap_or(0) });
        }
        let m = it.get("member")?;
        let user = User::from_json(m.get("user")?)?;
        self.remember(&user);
        if let Some(gid) = guild {
            if let Some(r) = m.get("roles") { self.record_roles(gid, &user.id, r) }
            self.set_nick(gid, &user.id, m.get("nick").and_then(Value::as_str));
        }
        let status = m.pointer("/presence/status").and_then(Value::as_str).unwrap_or("offline").to_string();
        self.status.insert(user.id.clone(), status.clone());
        Some(Row::Member { nick: m.get("nick").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from), user, status })
    }

    /// Heading text for a group id.
    pub fn group_label(&self, id: &str) -> String {
        match id {
            "online" => "Online".into(),
            "offline" => "Offline".into(),
            role => self.roles.get(role).cloned().unwrap_or_else(|| "Cargo".into()),
        }
    }

    pub fn status_of(&self, user_id: &str) -> &str { self.status.get(user_id).map_or("offline", String::as_str) }

    pub fn on_typing(&mut self, channel: &str, user_id: &str, now: i64) {
        let list = self.typing.entry(channel.into()).or_default();
        list.retain(|(u, until)| u != user_id && *until > now);
        list.push((user_id.into(), now + TYPING_TTL_MS));
    }

    /// A message from someone ends their typing notice.
    pub fn stop_typing(&mut self, channel: &str, user_id: &str) {
        if let Some(l) = self.typing.get_mut(channel) { l.retain(|(u, _)| u != user_id) }
    }

    /// Names of people typing in `channel` now, `me` excluded.
    pub fn typing_names(&self, channel: &str, me: Option<&str>, now: i64) -> Vec<String> {
        self.typing.get(channel).into_iter().flatten().filter(|(u, until)| *until > now && Some(u.as_str()) != me)
            .map(|(u, _)| self.users.get(u).map_or("Alguém".to_string(), |u| u.display().to_string())).collect()
    }
}

/// "ana está digitando…", "ana e bruno estão digitando…", "Várias pessoas estão digitando…".
pub fn typing_text(names: &[String]) -> Option<String> {
    match names {
        [] => None,
        [a] => Some(format!("{a} está digitando…")),
        [a, b] => Some(format!("{a} e {b} estão digitando…")),
        _ => Some("Várias pessoas estão digitando…".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn update() -> Value {
        json!({"guild_id": "g", "member_count": 3, "online_count": 1, "ops": [{"op": "SYNC", "range": [0, 99], "items": [
            {"group": {"id": "online", "count": 1}},
            {"member": {"user": {"id": "1", "username": "ana"}, "nick": "Aninha", "presence": {"status": "idle"}}},
            {"group": {"id": "offline", "count": 2}},
            {"member": {"user": {"id": "2", "username": "bruno"}}},
        ]}]})
    }

    #[test]
    fn a_sync_fills_the_list_with_groups_and_members() {
        let mut r = Roster::default();
        r.on_member_list(&update());
        let l = &r.lists["g"];
        assert_eq!(l.len(), 4);
        assert!(matches!(&l[0], Row::Group { id, count: 1 } if id == "online"));
        assert!(matches!(&l[1], Row::Member { nick: Some(n), status, .. } if n == "Aninha" && status == "idle"));
        assert!(matches!(&l[3], Row::Member { status, .. } if status == "offline"), "no presence means offline");
        assert_eq!(r.totals["g"], (3, 1));
        assert_eq!(r.status_of("1"), "idle");
        assert_eq!(r.users["2"].username, "bruno");
    }

    #[test]
    fn a_second_sync_replaces_the_range_and_presence_updates_stick() {
        let mut r = Roster::default();
        r.on_member_list(&update());
        r.on_member_list(&json!({"guild_id": "g", "ops": [{"op": "SYNC", "range": [0, 99], "items": [{"group": {"id": "online", "count": 0}}]}]}));
        assert_eq!(r.lists["g"].len(), 1, "the synced range is replaced, not appended");
        r.on_presence(&json!({"user": {"id": "9"}, "status": "dnd"}));
        assert_eq!(r.status_of("9"), "dnd");
        assert_eq!(r.status_of("unknown"), "offline");
    }

    #[test]
    fn group_labels_use_role_names() {
        let mut r = Roster::default();
        r.load_ready(&json!({"guilds": [{"roles": [{"id": "7", "name": "Moderadores"}]}]}));
        assert_eq!(r.group_label("online"), "Online");
        assert_eq!(r.group_label("7"), "Moderadores");
        assert_eq!(r.group_label("8"), "Cargo");
    }

    #[test]
    fn typing_expires_ignores_me_and_stops_on_message() {
        let mut r = Roster::default();
        r.remember(&User { id: "1".into(), username: "ana".into(), global_name: None, avatar: None, discriminator: None });
        r.on_typing("c", "1", 1_000);
        r.on_typing("c", "me", 1_000);
        assert_eq!(r.typing_names("c", Some("me"), 2_000), vec!["ana"]);
        assert!(r.typing_names("c", Some("me"), 1_000 + TYPING_TTL_MS + 1).is_empty(), "the notice fades");
        r.on_typing("c", "1", 5_000);
        r.stop_typing("c", "1");
        assert!(r.typing_names("c", Some("me"), 5_001).is_empty());
        assert_eq!(typing_text(&["a".into(), "b".into()]).unwrap(), "a e b estão digitando…");
        assert_eq!(typing_text(&[]), None);
    }

    #[test]
    fn role_icons_are_parsed_and_top_comes_first() {
        let mut r = Roster::default();
        r.load_ready(&json!({"guilds": [{"id": "g", "roles": [
            {"id": "a", "name": "pai de santo", "position": 1, "icon": "abc"},
            {"id": "b", "name": "mod", "position": 5, "unicode_emoji": "⭐"},
        ]}]}));
        r.set_roles("g", "u1", vec!["a".into(), "b".into()]);
        assert_eq!(r.top_role_icon("g", "u1").map(|(i, n, h, e)| (i, n)), Some(("b".into(), "mod".into())), "the highest role's icon shows");
        let roles = r.member_roles_sorted("g", "u1");
        assert_eq!(roles.len(), 2);
        assert_eq!(roles[0].0, "b".to_string());
        assert_eq!(roles[0].1, "mod".to_string());
    }

    #[test]
    fn names_take_the_colour_of_the_highest_coloured_role() {
        let mut r = Roster::default();
        r.load_ready(&json!({"guilds": [{"id": "g", "roles": [
            {"id": "g", "name": "@everyone", "color": 0, "position": 0},
            {"id": "a", "name": "A", "color": 255, "position": 1},
            {"id": "b", "name": "B", "color": 0, "position": 5},
            {"id": "c", "name": "C", "color": 65280, "position": 3}]}]}));
        r.set_roles("g", "u1", vec!["a".into(), "b".into(), "c".into()]);
        r.set_roles("g", "u2", vec!["b".into()]);
        assert_eq!(r.name_color("g", "u1"), Some(65280), "the highest role that has a colour wins");
        assert_eq!(r.name_color("g", "u2"), None, "no coloured role: plain name");
        assert_eq!(r.name_color("g", "nobody"), None);
    }

    #[test]
    fn nicknames_are_per_server_and_clear_when_removed() {
        let mut r = Roster::default();
        r.set_nick("g1", "u", Some("Chefe"));
        assert_eq!(r.nick("g1", "u"), Some("Chefe"));
        assert_eq!(r.nick("g2", "u"), None, "a nickname belongs to one server");
        r.on_member_list(&update());
        assert_eq!(r.nick("g", "1"), Some("Aninha"), "member lists carry nicknames");
        r.set_nick("g1", "u", None);
        assert_eq!(r.nick("g1", "u"), None);
    }
}

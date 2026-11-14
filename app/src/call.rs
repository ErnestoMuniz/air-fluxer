//! The voice call the user is in (or joining): the gateway handshake, the local mute/deafen state,
//! the audio settings menu and the call sounds, with the media itself in `voice.rs`.
//!
//! Joining is: send our voice state naming a channel → the gateway answers with a
//! `VOICE_SERVER_UPDATE` (a LiveKit endpoint and token) → connect the room. Leaving, mute and
//! deafen are all the same frame with other fields; every frame carries the connection id the
//! server gave us once we have one.

use crate::audio::{self, Devices, Settings, Shared};
use crate::gateway::Handle;
use crate::sounds::{self, Sound};
use crate::voice::{Cmd, Ev, Frames, Voice};
use fluxer_client::{Opcode, Payload};
use rune::Waker;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase { #[default] Idle, Connecting, Connected }

/// Which list of the audio menu is unfolded.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Fold { #[default] None, Input, Profile, Output }

/// What the audio threads share with the UI, with a default for `Call`.
pub struct SharedAudio(pub Arc<Shared>);

impl Default for SharedAudio {
    fn default() -> Self { SharedAudio(Shared::new(&Settings::default())) }
}

#[derive(Default)]
pub struct Call {
    engine: Option<Voice>,
    pub phase: Phase,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    connection_id: Option<String>,
    pub muted: bool,
    pub deafened: bool,
    /// Users speaking right now.
    pub speaking: HashSet<String>,
    pub error: Option<String>,
    /// Microphone, speakers, volumes and noise suppression; saved on every change.
    pub settings: Settings,
    pub shared: SharedAudio,
    /// The audio menu (right click on the mute or deafen button, or the gear).
    pub menu_open: bool,
    /// Key of the button the menu opens above (the one right-clicked).
    pub menu_anchor: String,
    pub fold: Fold,
    /// The call's mosaic (everyone's tile, the streams) is what the main area shows.
    pub view: bool,
    /// Every tile the same size, even while watching a stream (otherwise the stream is enlarged).
    pub grid: bool,
    /// The stream key whose viewers' list is showing (pointer over the viewer count).
    pub viewer_tip: Option<String>,
    /// Stream keys (`guild:channel:connection`) we are watching.
    pub watching: Vec<String>,
    /// Live pictures from the media thread.
    pub frames: Frames,
    pub devices: Option<Devices>,
    devices_rx: Option<std::sync::mpsc::Receiver<Result<Devices, String>>>,
    wake: Option<Waker>,
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> { v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()) }

impl Call {
    pub fn start(&mut self, wake: Waker) {
        self.settings = audio::load();
        self.shared = SharedAudio(Shared::new(&self.settings));
        self.wake = Some(wake.clone());
        self.engine = Some(Voice::start(wake, self.shared.0.clone(), self.frames.clone()));
    }

    pub fn set_me(&self, id: &str) { self.shared.0.set_me(id) }

    pub fn active(&self) -> bool { self.phase != Phase::Idle }

    pub fn level(&self) -> f32 { self.shared.0.level() }

    fn sound(&self, k: Sound) { sounds::play(k, self.settings.speaker.clone(), self.settings.output_volume) }

    /// Someone else came into or left the channel we are in.
    pub fn on_other_presence(&self, was_here: bool, is_here: bool) {
        if self.phase == Phase::Connected && was_here != is_here { self.sound(if is_here { Sound::UserJoin } else { Sound::UserLeave }) }
    }

    // ---- the audio menu ----

    /// Opens or closes the menu; opening lists the devices again (they come and go) and starts
    /// measuring the microphone for the level bar.
    pub fn toggle_menu(&mut self) {
        self.menu_open = !self.menu_open;
        if self.menu_anchor.is_empty() { self.menu_anchor = "mic-toggle".into() }
        self.fold = Fold::None;
        self.engine_send(Cmd::Meter(self.menu_open));
        if !self.menu_open { return }
        let (tx, rx) = std::sync::mpsc::channel();
        self.devices_rx = Some(rx);
        let wake = self.wake.clone();
        std::thread::spawn(move || { let _ = tx.send(audio::list_devices()); if let Some(w) = wake { w() } });
    }

    /// Opens the menu above the button with this key (moving it if already open).
    pub fn open_menu_at(&mut self, anchor: &str) {
        self.menu_anchor = anchor.into();
        if !self.menu_open { self.toggle_menu() }
    }

    pub fn close_menu(&mut self) { if self.menu_open { self.toggle_menu() } }

    pub fn toggle_fold(&mut self, f: Fold) { self.fold = if self.fold == f { Fold::None } else { f } }

    fn engine_send(&self, c: Cmd) { if let Some(e) = &self.engine { e.send(c) } }

    pub fn set_mic(&mut self, id: Option<String>) {
        self.settings.mic = id.clone();
        audio::save(&self.settings);
        self.engine_send(Cmd::SetMic(id));
    }

    pub fn set_speaker(&mut self, id: Option<String>) {
        self.settings.speaker = id.clone();
        audio::save(&self.settings);
        self.engine_send(Cmd::SetSpeaker(id));
    }

    pub fn set_noise_suppression(&mut self, on: bool) {
        self.settings.noise_suppression = on;
        self.shared.0.denoise.store(on, std::sync::atomic::Ordering::Relaxed);
        audio::save(&self.settings);
    }

    pub fn set_input_volume(&mut self, v: f32) {
        self.settings.input_volume = v.clamp(0.0, 1.0);
        self.shared.0.set_in_gain(self.settings.input_volume);
        audio::save(&self.settings);
    }

    /// 0..=2, as in the menu's slider (1 = as sent).
    pub fn user_volume(&self, id: &str) -> f32 { self.settings.user_volume.get(id).copied().unwrap_or(1.0) }
    pub fn user_muted(&self, id: &str) -> bool { self.settings.muted_users.iter().any(|u| u == id) }

    pub fn set_user_volume(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 2.0);
        self.settings.user_volume.insert(id.into(), v);
        self.shared.0.set_user(id, v, self.user_muted(id));
        audio::save(&self.settings);
    }

    pub fn toggle_user_mute(&mut self, id: &str) {
        let muted = !self.user_muted(id);
        if muted { self.settings.muted_users.push(id.into()) } else { self.settings.muted_users.retain(|u| u != id) }
        self.shared.0.set_user(id, self.user_volume(id), muted);
        audio::save(&self.settings);
    }

    pub fn set_output_volume(&mut self, v: f32) {
        self.settings.output_volume = v.clamp(0.0, 1.0);
        self.shared.0.set_out_gain(self.settings.output_volume);
        audio::save(&self.settings);
    }

    // ---- the call ----

    fn frame(&self, channel: Option<&str>) -> Payload {
        Payload { op: Opcode::VOICE_STATE_UPDATE, s: None, t: None, d: Some(json!({
            "guild_id": self.guild_id, "channel_id": channel,
            "self_mute": self.muted, "self_deaf": self.deafened, "self_video": false, "self_stream": false,
            "viewer_stream_keys": self.watching,
            "connection_id": self.connection_id,
        })) }
    }

    pub fn join(&mut self, gw: Option<&Handle>, guild: &str, channel: &str) {
        if self.channel_id.as_deref() == Some(channel) { return }
        let Some(gw) = gw else { return };
        // Moving between channels keeps the same connection; a fresh join starts without one.
        if !self.active() { self.connection_id = None }
        // Streams belong to a channel: moving to another one watches none.
        self.watching.clear();
        self.engine_send(Cmd::Watch(Vec::new()));
        self.guild_id = Some(guild.into());
        self.channel_id = Some(channel.into());
        self.error = None;
        if gw.send(self.frame(Some(channel))) { self.phase = Phase::Connecting } else { self.reset() }
    }

    pub fn leave(&mut self, gw: Option<&Handle>) {
        if !self.active() { return }
        if let Some(gw) = gw { gw.send(self.frame(None)); }
        self.sound(Sound::Leave);
        self.disconnect_media();
        self.reset();
    }

    fn disconnect_media(&self) { self.engine_send(Cmd::Disconnect) }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.guild_id = None;
        self.channel_id = None;
        self.connection_id = None;
        self.speaking.clear();
        self.view = false;
        self.grid = false;
        self.viewer_tip = None;
        self.watching.clear();
        self.engine_send(Cmd::Watch(Vec::new()));
    }

    /// The key the server knows a connection's stream by.
    pub fn stream_key(&self, connection: &str) -> Option<String> {
        Some(format!("{}:{}:{}", self.guild_id.as_deref()?, self.channel_id.as_deref()?, connection))
    }

    pub fn is_watching(&self, connection: &str) -> bool {
        self.stream_key(connection).is_some_and(|k| self.watching.contains(&k))
    }

    /// Starts or stops watching a connection's screen share: the media thread subscribes (or
    /// not) and the server hears of it so the streamer sees a viewer.
    pub fn set_watching(&mut self, gw: Option<&Handle>, connection: &str, on: bool) {
        let Some(key) = self.stream_key(connection) else { return };
        if self.watching.contains(&key) == on { return }
        if on { self.watching.push(key); self.grid = false } else { self.watching.retain(|k| *k != key) }
        let conns = self.watching.iter().filter_map(|k| k.rsplit(':').next().map(String::from)).collect();
        self.engine_send(Cmd::Watch(conns));
        if self.active() && let (Some(gw), Some(c)) = (gw, self.channel_id.as_deref()) { gw.send(self.frame(Some(c))); }
    }

    /// The picture arriving for `key` (`screen:<connection>` or `cam:<connection>`), if any.
    pub fn picture(&self, key: &str) -> Option<rune::Pixels> { self.frames.lock().ok()?.get(key).cloned() }

    fn push_flags(&self, gw: Option<&Handle>) {
        self.shared.0.deafened.store(self.deafened, std::sync::atomic::Ordering::Relaxed);
        self.shared.0.mic_off.store(self.muted || self.deafened, std::sync::atomic::Ordering::Relaxed);
        self.engine_send(Cmd::SetMuted(self.muted || self.deafened));
        if self.active() && let (Some(gw), Some(c)) = (gw, self.channel_id.as_deref()) { gw.send(self.frame(Some(c))); }
    }

    /// Unmuting while deafened undoes both, as in Discord.
    pub fn toggle_mute(&mut self, gw: Option<&Handle>) {
        let was_muted = self.muted || self.deafened;
        if self.deafened { self.deafened = false; self.muted = false } else { self.muted = !self.muted }
        self.sound(if was_muted { Sound::Unmute } else { Sound::Mute });
        self.push_flags(gw);
    }

    pub fn toggle_deafen(&mut self, gw: Option<&Handle>) {
        self.deafened = !self.deafened;
        self.sound(if self.deafened { Sound::Deafen } else { Sound::Undeafen });
        self.push_flags(gw);
    }

    /// `VOICE_SERVER_UPDATE`: where to connect.
    pub fn on_server_update(&mut self, d: &Value) {
        if !self.active() { return }
        if d.get("p2p").and_then(Value::as_bool) == Some(true) {
            self.error = Some("Chamadas diretas (P2P) ainda não são suportadas.".into());
            self.disconnect_media();
            self.reset();
            return;
        }
        let (Some(endpoint), Some(token)) = (text(d, "endpoint"), text(d, "token")) else { return };
        if let Some(c) = text(d, "connection_id") { self.connection_id = Some(c.into()) }
        self.shared.0.deafened.store(self.deafened, std::sync::atomic::Ordering::Relaxed);
        self.shared.0.mic_off.store(self.muted || self.deafened, std::sync::atomic::Ordering::Relaxed);
        self.engine_send(Cmd::Connect {
            endpoint: endpoint.into(), token: token.into(), e2ee_key: text(d, "e2ee_key").map(String::from),
            muted: self.muted || self.deafened, settings: self.settings.clone(),
        });
    }

    /// A voice state of our own account: the server may have moved or disconnected us.
    pub fn on_my_state(&mut self, d: &Value) {
        if !self.active() { return }
        match text(d, "channel_id") {
            None => { self.disconnect_media(); self.reset() }
            Some(c) => {
                if let Some(id) = text(d, "connection_id") { self.connection_id = Some(id.into()) }
                self.channel_id = Some(c.into());
            }
        }
    }

    /// Drains the media engine and the device list; true when something visible changed. Failures
    /// also leave the channel on the server so we do not stay listed as connected.
    pub fn poll(&mut self, gw: Option<&Handle>) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.devices_rx && let Ok(res) = rx.try_recv() {
            self.devices_rx = None;
            match res { Ok(d) => self.devices = Some(d), Err(e) => self.error = Some(e) }
            changed = true;
        }
        // The level bar moves while the menu is open.
        if self.menu_open { changed = true }
        let Some(engine) = &self.engine else { return changed };
        let events: Vec<Ev> = engine.inbox.try_iter().collect();
        for e in events {
            changed = true;
            match e {
                Ev::Connected => if self.active() { self.phase = Phase::Connected; self.sound(Sound::Join) },
                Ev::Disconnected => if self.active() { self.leave(gw) },
                Ev::Failed(why) => { self.error = Some(why); self.leave(gw) }
                Ev::Speaking(users) => self.speaking = users.into_iter().collect(),
                Ev::Video => {}
            }
        }
        changed
    }
}

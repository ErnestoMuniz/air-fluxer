//! Audio devices and the voice settings the user picks: which microphone and speakers, and the
//! RNNoise noise suppression (on by default). Devices are listed through PulseAudio, which
//! PipeWire serves, and the choice is saved next to the session.

use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet, State};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// PulseAudio source name; None follows the system default.
    pub mic: Option<String>,
    /// PulseAudio sink name; None follows the system default.
    pub speaker: Option<String>,
    /// RNNoise noise suppression on the microphone.
    pub noise_suppression: bool,
    /// Gain on the microphone and on everything heard, 0..=1 (the menu's sliders).
    #[serde(default = "one")]
    pub input_volume: f32,
    #[serde(default = "one")]
    pub output_volume: f32,
    /// How loud each person is for me (0..=2, 1 = as sent) and who I muted; keyed by user id.
    #[serde(default)]
    pub user_volume: std::collections::HashMap<String, f32>,
    #[serde(default)]
    pub muted_users: Vec<String>,
}

fn one() -> f32 { 1.0 }

impl Default for Settings {
    fn default() -> Self { Settings { mic: None, speaker: None, noise_suppression: true, input_volume: 1.0, output_volume: 1.0, user_volume: Default::default(), muted_users: vec![] } }
}

/// Values the audio threads read while they run, so a slider or a switch takes effect at once.
pub struct Shared {
    pub denoise: AtomicBool,
    pub deafened: AtomicBool,
    in_gain: AtomicU32,
    out_gain: AtomicU32,
    /// Microphone level, 0..=1000, after processing.
    level: AtomicU32,
    /// Per-person gain and mute, read by each playback thread.
    users: std::sync::Mutex<std::collections::HashMap<String, (f32, bool)>>,
    /// When each person last had sound above the speaking threshold (me included).
    speaking: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    me: std::sync::Mutex<Option<String>>,
    /// My microphone is not being sent (muted or deafened).
    pub mic_off: AtomicBool,
}

impl Shared {
    pub fn new(s: &Settings) -> Arc<Shared> {
        Arc::new(Shared {
            denoise: AtomicBool::new(s.noise_suppression), deafened: AtomicBool::new(false),
            in_gain: AtomicU32::new(s.input_volume.to_bits()), out_gain: AtomicU32::new(s.output_volume.to_bits()), level: AtomicU32::new(0),
            speaking: Default::default(), me: Default::default(), mic_off: AtomicBool::new(false),
            users: std::sync::Mutex::new(s.user_volume.iter().map(|(k, v)| (k.clone(), (*v, s.muted_users.contains(k)))).chain(s.muted_users.iter().map(|k| (k.clone(), (s.user_volume.get(k).copied().unwrap_or(1.0), true)))).collect()),
        })
    }
    pub fn set_in_gain(&self, v: f32) { self.in_gain.store(v.to_bits(), Ordering::Relaxed) }
    pub fn set_out_gain(&self, v: f32) { self.out_gain.store(v.to_bits(), Ordering::Relaxed) }
    pub fn in_gain(&self) -> f32 { f32::from_bits(self.in_gain.load(Ordering::Relaxed)) }
    pub fn out_gain(&self) -> f32 { f32::from_bits(self.out_gain.load(Ordering::Relaxed)) }
    pub fn set_user(&self, id: &str, gain: f32, muted: bool) { self.users.lock().unwrap().insert(id.into(), (gain, muted)); }
    /// What this person's voice is multiplied by: 0 when muted.
    pub fn user_gain(&self, id: &str) -> f32 {
        match self.users.lock().unwrap().get(id) { Some((_, true)) => 0.0, Some((g, false)) => *g, None => 1.0 }
    }
    pub fn set_me(&self, id: &str) { *self.me.lock().unwrap() = Some(id.into()) }
    pub fn me(&self) -> Option<String> { self.me.lock().unwrap().clone() }

    /// Sound from `id` just came through.
    pub fn mark_speaking(&self, id: &str) { self.speaking.lock().unwrap().insert(id.into(), std::time::Instant::now()); }

    /// Who has had sound within the last `hold`, sorted (so two snapshots compare).
    pub fn speaking_ids(&self, hold: std::time::Duration) -> Vec<String> {
        let now = std::time::Instant::now();
        let mut m = self.speaking.lock().unwrap();
        m.retain(|_, t| now.duration_since(*t) < hold * 20);
        let mut v: Vec<String> = m.iter().filter(|(_, t)| now.duration_since(**t) < hold).map(|(k, _)| k.clone()).collect();
        v.sort();
        v
    }

    pub fn set_level(&self, v: u32) { self.level.store(v, Ordering::Relaxed) }
    pub fn level(&self) -> f32 { self.level.load(Ordering::Relaxed) as f32 / 1000.0 }
}

/// Opens a 48 kHz mono playback stream on `sink` (the default output when None or when it will not open).
pub fn open_playback(sink: Option<&str>) -> Option<libpulse_simple_binding::Simple> {
    use libpulse_binding::def::BufferAttr;
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    let spec = Spec { format: Format::S16NE, channels: 1, rate: 48_000 };
    // About 60 ms of buffer: low delay, with room for the scheduler's hiccups.
    let attr = BufferAttr { maxlength: u32::MAX, tlength: 48_000 / 1000 * 60 * 2, prebuf: u32::MAX, minreq: u32::MAX, fragsize: u32::MAX };
    let open = |dev: Option<&str>| libpulse_simple_binding::Simple::new(None, "Fluxer", Direction::Playback, dev, "Voice", &spec, None, Some(&attr)).ok();
    open(sink).or_else(|| if sink.is_some() { open(None) } else { None })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// What is stored and handed to the audio system.
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct Devices { pub mics: Vec<Device>, pub speakers: Vec<Device> }

fn file() -> Option<std::path::PathBuf> { Some(crate::store::dir()?.join("audio.json")) }

pub fn load() -> Settings {
    file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(s: &Settings) {
    let Some(f) = file() else { return };
    let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
    let _ = std::fs::write(f, serde_json::to_vec_pretty(s).unwrap_or_default());
}

/// Lists microphones and speakers (blocking; run it off the UI thread).
pub fn list_devices() -> Result<Devices, String> {
    let mut ml = Mainloop::new().ok_or("PulseAudio indisponível")?;
    let mut ctx = Context::new(&ml, "Fluxer").ok_or("PulseAudio indisponível")?;
    ctx.connect(None, FlagSet::NOFLAGS, None).map_err(|e| format!("PulseAudio: {e}"))?;
    let step = |ml: &mut Mainloop| match ml.iterate(true) { IterateResult::Success(_) => Ok(()), _ => Err("PulseAudio encerrou".to_string()) };
    loop {
        step(&mut ml)?;
        match ctx.get_state() { State::Ready => break, State::Failed | State::Terminated => return Err("sem conexão com o PulseAudio".into()), _ => {} }
    }
    let mics = Rc::new(RefCell::new(Vec::new()));
    let speakers = Rc::new(RefCell::new(Vec::new()));
    let pending = Rc::new(RefCell::new(2));
    let (m, p) = (mics.clone(), pending.clone());
    ctx.introspect().get_source_info_list(move |r| match r {
        // A monitor is a copy of an output, not a microphone.
        ListResult::Item(i) => if i.monitor_of_sink.is_none() && let Some(n) = &i.name {
            m.borrow_mut().push(Device { id: n.to_string(), label: i.description.as_deref().unwrap_or(n).to_string() })
        },
        _ => *p.borrow_mut() -= 1,
    });
    let (sp, p) = (speakers.clone(), pending.clone());
    ctx.introspect().get_sink_info_list(move |r| match r {
        ListResult::Item(i) => if let Some(n) = &i.name { sp.borrow_mut().push(Device { id: n.to_string(), label: i.description.as_deref().unwrap_or(n).to_string() }) },
        _ => *p.borrow_mut() -= 1,
    });
    while *pending.borrow() > 0 { step(&mut ml)? }
    ctx.disconnect();
    let (mics, speakers) = (mics.borrow().clone(), speakers.borrow().clone());
    Ok(Devices { mics, speakers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_suppression_is_on_by_default_and_settings_roundtrip() {
        let s = Settings::default();
        assert!(s.noise_suppression && s.mic.is_none() && s.speaker.is_none());
        let again: Settings = serde_json::from_slice(&serde_json::to_vec(&Settings { mic: Some("m".into()), ..s.clone() }).unwrap()).unwrap();
        assert_eq!(again.mic.as_deref(), Some("m"));
    }

    #[test]
    fn per_user_gain_follows_volume_and_mute() {
        let mut cfg = Settings::default();
        cfg.user_volume.insert("a".into(), 1.5);
        cfg.muted_users.push("b".into());
        let shared = Shared::new(&cfg);
        assert_eq!(shared.user_gain("a"), 1.5);
        assert_eq!(shared.user_gain("b"), 0.0, "muted people are silent");
        assert_eq!(shared.user_gain("nobody"), 1.0);
        shared.set_user("b", 1.0, false);
        assert_eq!(shared.user_gain("b"), 1.0, "unmuting brings them back at their volume");
    }

    #[test]
    fn speaking_shows_at_once_and_fades_after_the_hold() {
        let shared = Shared::new(&Settings::default());
        assert!(shared.speaking_ids(std::time::Duration::from_millis(200)).is_empty());
        shared.mark_speaking("7");
        assert_eq!(shared.speaking_ids(std::time::Duration::from_millis(200)), vec!["7".to_string()], "no delay before it shows");
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(shared.speaking_ids(std::time::Duration::from_millis(10)).is_empty(), "and it goes once the hold has passed");
    }
}

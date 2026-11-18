//! Voice chat media: a LiveKit room on its own thread. Both directions go through the system's
//! audio service (CoreAudio on macOS, PulseAudio on Linux) so the app controls them: the
//! microphone is read here, scaled, cleaned with RNNoise and pushed into the room as a native
//! audio source; every remote voice is played on a stream of its own with the output volume and
//! deafening applied.
//!
//! The gateway side (joining a channel, the server's token) lives in `call.rs`; this module gets
//! an endpoint and a token and does the rest.

use crate::audio::{self, Settings, Shared};
use futures_util::StreamExt;
use livekit::e2ee::key_provider::{KeyProvider, KeyProviderOptions};
use livekit::e2ee::{E2eeOptions, EncryptionType};
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::video_frame::VideoFormatType;
use livekit::webrtc::video_stream::native::NativeVideoStream;
use livekit::webrtc::video_frame::{I420Buffer, VideoFrame, VideoRotation};
use livekit::webrtc::video_source::VideoResolution;
use livekit::webrtc::video_source::RtcVideoSource;
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::native::yuv_helper;
use nnnoiseless::DenoiseState;
use rune::{Pixels, Waker};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub enum Cmd {
    Connect { endpoint: String, token: String, e2ee_key: Option<String>, muted: bool, settings: Settings },
    SetMic(Option<String>),
    SetSpeaker(Option<String>),
    SetMuted(bool),
    /// Connections whose screen share we watch; only those are subscribed (the server sends no
    /// video nobody asked for).
    Watch(Vec<String>),
    /// Keep the microphone open only to measure its level (the settings menu), outside a call.
    Meter(bool),
    /// Publish this Mac's screen (the HUD chooses the source, quality and fps).
    ListScreens,
    ShareScreen { on: bool, height: u32, fps: u32, source: usize },
    Disconnect,
}

pub enum Ev {
    Connected,
    /// The room closed (on request or not).
    Disconnected,
    Failed(String),
    /// User ids of everyone speaking now, me included.
    Speaking(Vec<String>),
    /// Round-trip time to the room, in milliseconds (WebRTC's own measurement).
    Ping(u32),
    /// This account started (true) or stopped (false) sharing its screen.
    Sharing(bool),
    /// The screens/windows the capturer can share (titles, for the HUD).
    Screens(Vec<String>),
    /// Something failed without closing the room (a screen share denied, say).
    Notice(String),
    /// A picture appeared or went away in `Frames`.
    Video,
}

/// Live pictures by key: `screen:<connection>` for a screen share, `cam:<connection>` for a camera.
/// An entry exists while frames are arriving; the view draws the `Pixels` it holds.
pub type Frames = Arc<std::sync::Mutex<std::collections::HashMap<String, Pixels>>>;

/// The connection id at the end of a participant identity (`user_<id>_<connection>`).
pub fn connection_of_identity(identity: &str) -> Option<&str> {
    let mut parts = identity.splitn(3, '_');
    parts.next()?;
    parts.next()?;
    parts.next().filter(|c| !c.is_empty())
}

pub struct Voice {
    tx: UnboundedSender<Cmd>,
    pub inbox: mpsc::Receiver<Ev>,
}

impl Voice {
    pub fn start(wake: Waker, shared: Arc<Shared>, frames: Frames) -> Voice {
        let (tx, rx) = unbounded_channel();
        let (ev_tx, inbox) = mpsc::channel();
        std::thread::Builder::new().name("fluxer-voice".into()).spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("voice runtime");
            rt.block_on(engine(rx, ev_tx, wake, shared, frames));
        }).expect("spawn voice thread");
        Voice { tx, inbox }
    }

    pub fn send(&self, c: Cmd) { let _ = self.tx.send(c); }
}

/// `user_<id>_<connection>` is how the server names participants.
pub fn user_of_identity(identity: &str) -> Option<String> {
    let rest = identity.strip_prefix("user_")?;
    let id = rest.split('_').next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

struct Live {
    room: Arc<Room>,
    mic: LocalAudioTrack,
    events: UnboundedReceiver<RoomEvent>,
    capture: Option<Capture>,
    frames: UnboundedSender<Vec<i16>>,
    /// The microphone feed; stops with the room.
    feed: tokio::task::JoinHandle<()>,
    /// One playback per person: user id → (participant identity, track, task). A person who leaves
    /// and comes back is a new participant while the old one may linger for a moment; keying by
    /// user keeps a single stream playing, never two of the same voice.
    remote: std::collections::HashMap<String, Remote>,
    speaker: Option<String>,
    /// Decoders of the pictures on screen, by track id.
    video: std::collections::HashMap<String, tokio::task::JoinHandle<()>>,
    watch: std::collections::HashSet<String>,
    /// Our own screen share, when one is running.
    screen: Option<Screen>,
}

struct Remote { identity: String, track: String, task: tokio::task::JoinHandle<()> }

/// A screen share this account is publishing: its sid, how to stop capturing, and whether any
/// frame ever arrived (a cancelled picker must not leave the LIVE badge on).
struct Screen { sid: TrackSid, stop: Arc<AtomicBool>, frames: Arc<std::sync::atomic::AtomicUsize>, since: std::time::Instant }

impl Drop for Screen { fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed) } }

/// Screen Recording is a TCC permission; ask before capturing.
#[cfg(target_os = "macos")]
fn request_screen_permission() -> bool { crate::screen_macos::permission() }

#[cfg(target_os = "macos")]
async fn start_screen(room: &Room, height: u32, fps: u32, display: u32) -> Result<Screen, String> {
    if !request_screen_permission() {
        return Err("Permita a Gravação de Tela em Ajustes do Sistema → Privacidade e Segurança e clique de novo (o macOS pode pedir para reabrir o app).".into());
    }
    let max_h = height.clamp(360, 2160);
    let source = NativeVideoSource::new(VideoResolution { width: max_h * 16 / 9, height: max_h }, true);
    let track = LocalVideoTrack::create_video_track("screen", RtcVideoSource::Native(source.clone()));
    let publish = TrackPublishOptions { source: TrackSource::Screenshare, ..Default::default() };
    let publication = room.local_participant().publish_track(LocalTrack::Video(track), publish).await
        .map_err(|e| format!("não foi possível compartilhar a tela: {e}"))?;
    let sid = publication.sid();
    let frames = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = frames.clone();
    let max_w = (max_h * 16 / 9) as i32;
    let max_hi = max_h as i32;
    let stop = crate::screen_macos::start(display, fps, move |frame| {
        seen.fetch_add(1, Ordering::Relaxed);
        let (mut w, mut h) = (frame.width as i32, frame.height as i32);
        if w < 2 || h < 2 { return }
        w &= !1;
        h &= !1;
        let mut buffer = I420Buffer::new(w as u32, h as u32);
        let (sy, su, sv) = buffer.strides();
        let (y, u, v) = buffer.data_mut();
        yuv_helper::abgr_to_i420(&frame.data, frame.stride as u32, y, sy, u, su, v, sv, w, h);
        let buffer = if w > max_w || h > max_hi {
            let (sw, sh) = if w * max_hi >= h * max_w {
                (max_w, (h * max_w / w).max(2) & !1)
            } else { ((w * max_hi / h).max(2) & !1, max_hi) };
            buffer.scale(sw, sh)
        } else { buffer };
        source.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, buffer));
    });
    Ok(Screen { sid, stop, frames, since: std::time::Instant::now() })
}

#[cfg(not(target_os = "macos"))]
async fn start_screen(_room: &Room, _height: u32, _fps: u32, _display: u32) -> Result<Screen, String> {
    Err("compartilhamento de tela ainda não é suportado nesta plataforma".into())
}

impl Drop for Live {
    fn drop(&mut self) {
        self.feed.abort();
        for r in self.remote.values() { r.task.abort() }
        for v in self.video.values() { v.abort() }
    }
}

/// Reads the microphone on a thread of its own until dropped.
struct Capture { stop: Arc<AtomicBool> }

impl Drop for Capture { fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed) } }

/// Loudness (peak, of 32768) above which someone counts as making sound.
const SPEAK_PEAK: i32 = 700;
/// How long the green ring stays after the last sound: speech has gaps between words.
const SPEAK_HOLD: std::time::Duration = std::time::Duration::from_millis(220);

fn peak_of(pcm: &[i16]) -> i32 { pcm.iter().map(|s| (*s as i32).abs()).max().unwrap_or(0) }

const FRAME: usize = 480; // 10 ms at 48 kHz: what RNNoise and the room both work in.

/// RNNoise over one 10 ms frame, in place (it works on 16-bit-scaled floats).
fn denoise_frame(rnn: &mut DenoiseState, pcm: &mut [i16], input: &mut [f32; FRAME], output: &mut [f32; FRAME]) {
    for (o, s) in input.iter_mut().zip(pcm.iter()) { *o = *s as f32 }
    rnn.process_frame(output, input);
    for (p, o) in pcm.iter_mut().zip(output.iter()) { *p = o.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16 }
}

fn scale(pcm: &mut [i16], gain: f32) {
    if (gain - 1.0).abs() < 0.001 { return }
    for p in pcm { *p = (*p as f32 * gain).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16 }
}

/// Peak of a frame as 0..=1000, eased for a level bar (speech peaks around -20 dBFS).
fn level_of(pcm: &[i16]) -> u32 {
    let peak = pcm.iter().map(|s| (*s as i32).unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0;
    ((peak * 3.0).min(1.0).sqrt() * 1000.0) as u32
}

/// `frames` None only measures the level.
fn start_capture(device: Option<String>, shared: Arc<Shared>, frames: Option<UnboundedSender<Vec<i16>>>) -> Capture {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new().name("fluxer-mic".into()).spawn(move || {
        let mut rnn = DenoiseState::new();
        let (mut bytes, mut input, mut output) = ([0u8; FRAME * 2], [0f32; FRAME], [0f32; FRAME]);
        // When following the system default, remember which device that was so a switch (AirPods
        // plugged in, the default changed in System Settings) reopens the capture on the new one.
        let mut followed = if device.is_none() { audio::default_input() } else { None };
        while !flag.load(Ordering::Relaxed) {
            let Some(mut mic) = audio::open_mic(device.as_deref()) else {
                eprintln!("fluxer: could not open the microphone {device:?}");
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            };
            let mut ticks = 0u32;
            while !flag.load(Ordering::Relaxed) {
                if mic.read(&mut bytes).is_err() { break }
                let mut pcm: Vec<i16> = bytes.chunks_exact(2).map(|b| i16::from_ne_bytes([b[0], b[1]])).collect();
                scale(&mut pcm, shared.in_gain());
                if shared.denoise.load(Ordering::Relaxed) { denoise_frame(&mut rnn, &mut pcm, &mut input, &mut output) }
                shared.set_level(level_of(&pcm));
                if peak_of(&pcm) > SPEAK_PEAK && !shared.mic_off.load(Ordering::Relaxed) && let Some(me) = shared.me() { shared.mark_speaking(&me) }
                if let Some(tx) = &frames && tx.send(pcm).is_err() { break }
                ticks += 1;
                // About once a second: did the system default input change under us?
                if device.is_none() && ticks % 100 == 0 {
                    let now = audio::default_input();
                    if now != followed { followed = now; break }
                }
            }
            shared.set_level(0);
        }
        shared.set_level(0);
    }).ok();
    Capture { stop }
}

/// Plays one remote voice: frames from the room go to a playback stream on its own thread.
fn play_remote(mut stream: NativeAudioStream, user: String, speaker: Option<String>, shared: Arc<Shared>) -> tokio::task::JoinHandle<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<i16>>(50);
    std::thread::Builder::new().name("fluxer-playback".into()).spawn(move || {
        let Some(out) = audio::open_playback(speaker.as_deref()) else { return };
        while let Ok(mut pcm) = rx.recv() {
            if peak_of(&pcm) > SPEAK_PEAK { shared.mark_speaking(&user) }
            let gain = if shared.deafened.load(Ordering::Relaxed) { 0.0 } else { shared.out_gain() * shared.user_gain(&user) };
            scale(&mut pcm, gain);
            let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_ne_bytes()).collect();
            if out.write(&bytes).is_err() { break }
        }
    }).ok();
    tokio::spawn(async move {
        while let Some(frame) = stream.next().await {
            // A full queue means the speakers fell behind: drop rather than build up delay.
            let _ = tx.try_send(frame.data.into_owned());
        }
    })
}


/// Whether this publication should be subscribed: voices and cameras always, a screen share only
/// when watched, a screen share's own audio never (it would replace the person's voice stream).
fn wanted(source: TrackSource, conn: Option<&str>, watch: &std::collections::HashSet<String>) -> bool {
    match source {
        TrackSource::ScreenshareAudio => false,
        TrackSource::Screenshare => conn.is_some_and(|c| watch.contains(c)),
        _ => true,
    }
}

fn apply_subscription(p: &RemoteTrackPublication, identity: &str, watch: &std::collections::HashSet<String>) {
    let want = wanted(p.source(), connection_of_identity(identity), watch);
    if p.is_subscribed() != want {
        p.set_subscribed(want);
        if want && matches!(p.kind(), TrackKind::Video) {
            p.set_enabled(true);
        }
    }
}

const VIDEO_MAX_WIDTH: u32 = 1600;
const VIDEO_MIN_GAP: std::time::Duration = std::time::Duration::from_millis(33);

/// Turns a remote video track into RGBA frames in the registry (at most ~30 a second, scaled down
/// when huge: converting 4K frames would cost more than the screen can show).
fn show_video(mut stream: NativeVideoStream, key: String, frames: Frames, tell: impl Fn() + Send + 'static) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let pixels = Pixels::new();
        let mut last = std::time::Instant::now() - VIDEO_MIN_GAP;
        let mut shown = false;
        while let Some(frame) = stream.next().await {
            if last.elapsed() < VIDEO_MIN_GAP { continue }
            last = std::time::Instant::now();
            let (mut w, mut h) = (frame.buffer.width(), frame.buffer.height());
            if w == 0 || h == 0 { continue }
            let mut rgba;
            if w > VIDEO_MAX_WIDTH {
                let (sw, sh) = (VIDEO_MAX_WIDTH & !1, ((h as u64 * VIDEO_MAX_WIDTH as u64 / w as u64) as u32).max(2) & !1);
                let scaled = frame.buffer.to_i420().scale(sw as i32, sh as i32);
                (w, h) = (sw, sh);
                rgba = vec![0u8; (w * h * 4) as usize];
                let ((y, u, v), (sy, su, sv)) = (scaled.data(), scaled.strides());
                livekit::webrtc::native::yuv_helper::i420_to_abgr(y, sy, u, su, v, sv, &mut rgba, w * 4, w as i32, h as i32);
            } else {
                rgba = vec![0u8; (w * h * 4) as usize];
                frame.buffer.to_argb(VideoFormatType::ABGR, &mut rgba, w * 4, w as i32, h as i32);
            }
            pixels.set(w, h, rgba);
            if !shown {
                shown = true;
                frames.lock().unwrap().insert(key.clone(), pixels.clone());
                tell();
            }
        }
        frames.lock().unwrap().remove(&key);
        tell();
    })
}

/// The room's round-trip time in milliseconds, as the web app shows it: the connection actually
/// in use (the nominated ICE candidate pair), not any pair that was ever tried. A pair that lost
/// to the chosen one, often a relay, keeps an old and much higher measurement. The RTCP report for
/// our microphone is not used even as a fallback: it includes the server's reporting delay and
/// reads about 3x too high, so until a pair is nominated the dock says "measuring".
fn rtt_ms(stats: &[livekit::webrtc::stats::RtcStats]) -> Option<u32> {
    use livekit::webrtc::stats::RtcStats;
    stats.iter().filter_map(|s| match s {
        RtcStats::CandidatePair(c) if c.candidate_pair.nominated && c.candidate_pair.current_round_trip_time > 0.0 =>
            Some((c.candidate_pair.current_round_trip_time * 1000.0).round() as u32),
        _ => None,
    }).min()
}

async fn engine(mut cmds: UnboundedReceiver<Cmd>, out: mpsc::Sender<Ev>, wake: Waker, shared: Arc<Shared>, frames: Frames) {
    let send = |e: Ev| { let _ = out.send(e); wake(); };
    let tell = { let out = out.clone(); let wake = wake.clone(); move || { let _ = out.send(Ev::Video); wake(); } };
    let mut watching: std::collections::HashSet<String> = Default::default();
    let mut live: Option<Live> = None;
    let mut sources: Vec<u32> = Vec::new();
    // The green rings come from the sound itself, checked often so they show at once.
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(30));
    // The room's round-trip time, asked from WebRTC every few seconds; not every frame (a stats
    // report walks every stream) and not never (it changes with the network).
    let mut ping = tokio::time::interval(std::time::Duration::from_secs(3));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut shown: Vec<String> = Vec::new();
    let (mut meter, mut want_meter, mut mic_choice) = (None::<Capture>, false, None::<String>);
    loop {
        // While in a room, its events and the commands are served together.
        let in_room = live.is_some();
        let probe = live.as_ref().map(|l| l.room.clone());
        let ev = async {
            match live.as_mut() { Some(l) => l.events.recv().await, None => std::future::pending().await }
        };
        tokio::select! {
            _ = tick.tick(), if in_room => {
                let now = shared.speaking_ids(SPEAK_HOLD);
                if now != shown { shown = now.clone(); send(Ev::Speaking(now)) }
            }
            _ = ping.tick(), if in_room => {
                // The whole connection's report, not the microphone's: a track's own report has no
                // candidate pairs, so the ping would fall back to the (higher) RTCP figure.
                if let Some(room) = probe
                    && let Ok(all) = room.get_stats().await {
                    let stats: Vec<_> = all.publisher_stats.into_iter().chain(all.subscriber_stats).collect();
                    if let Some(ms) = rtt_ms(&stats) { send(Ev::Ping(ms)) }
                }
                // A cancelled picker or an empty capture must not leave LIVE on forever.
                let stale = live.as_ref().and_then(|l| l.screen.as_ref())
                    .filter(|s| s.frames.load(Ordering::Relaxed) == 0 && s.since.elapsed() > std::time::Duration::from_secs(4))
                    .map(|s| s.sid.clone());
                if let Some(sid) = stale && let Some(l) = live.as_mut() {
                    l.screen = None;
                    let _ = l.room.local_participant().unpublish_track(&sid).await;
                    send(Ev::Sharing(false));
                    send(Ev::Notice("A transmissão não produziu imagem (permissão de tela ou seletor cancelado).".into()));
                }
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Cmd::Connect { endpoint, token, e2ee_key, muted, settings } => {
                        if let Some(old) = live.take() { let _ = old.room.close().await; }
                        frames.lock().unwrap().clear();
                        meter = None;
                        mic_choice = settings.mic.clone();
                        match connect(&endpoint, &token, e2ee_key, muted, settings, shared.clone()).await {
                            Ok(mut l) => {
                                l.watch = watching.clone();
                                for (id, p) in l.room.remote_participants() {
                                    for pubn in p.track_publications().values() { apply_subscription(pubn, &id.0, &l.watch) }
                                }
                                live = Some(l);
                                send(Ev::Connected)
                            }
                            Err(e) => send(Ev::Failed(e)),
                        }
                    }
                    Cmd::Disconnect => {
                        if let Some(old) = live.take() { let _ = old.room.close().await; send(Ev::Disconnected) }
                        frames.lock().unwrap().clear();
                        if want_meter { meter = Some(start_capture(mic_choice.clone(), shared.clone(), None)) }
                    }
                    Cmd::Watch(conns) => {
                        watching = conns.into_iter().collect();
                        if let Some(l) = live.as_mut() {
                            l.watch = watching.clone();
                            for (id, p) in l.room.remote_participants() {
                                for pubn in p.track_publications().values() { apply_subscription(pubn, &id.0, &l.watch) }
                            }
                        }
                    }
                    Cmd::SetMuted(m) => if let Some(l) = &live { if m { l.mic.mute() } else { l.mic.unmute() } },
                    Cmd::SetMic(dev) => {
                        mic_choice = dev.clone();
                        if let Some(l) = live.as_mut() {
                            l.capture = None;
                            l.capture = Some(start_capture(dev, shared.clone(), Some(l.frames.clone())));
                        } else if meter.is_some() {
                            // The old capture lets go of the device before the new one opens it.
                            drop(meter.take());
                            meter = Some(start_capture(dev, shared.clone(), None));
                        }
                    }
                    Cmd::SetSpeaker(name) => if let Some(l) = live.as_mut() { l.speaker = name },
                    Cmd::Meter(on) => {
                        want_meter = on;
                        meter = None;
                        if on && live.is_none() { meter = Some(start_capture(mic_choice.clone(), shared.clone(), None)) }
                    }
                    Cmd::ListScreens => {
                        #[cfg(target_os = "macos")]
                        {
                            let _ = crate::screen_macos::permission();
                            let list = crate::screen_macos::displays();
                            sources = list.iter().map(|(id, _)| *id).collect();
                            send(Ev::Screens(list.into_iter().map(|(_, n)| n).collect()));
                        }
                        #[cfg(not(target_os = "macos"))]
                        send(Ev::Screens(Vec::new()));
                    }
                    Cmd::ShareScreen { on, height, fps, source } => {
                        let Some(l) = live.as_mut() else { continue };
                        if on {
                            if l.screen.is_none() {
                                let display = sources.get(source).copied().unwrap_or(0);
                                match start_screen(&l.room, height, fps, display).await {
                                    Ok(s) => { l.screen = Some(s); send(Ev::Sharing(true)) }
                                    Err(e) => send(Ev::Notice(e)),
                                }
                            }
                        } else if let Some(s) = l.screen.take() {
                            let _ = l.room.local_participant().unpublish_track(&s.sid).await;
                            send(Ev::Sharing(false))
                        }
                    }
                }
            }
            e = ev => match e {
                Some(RoomEvent::TrackSubscribed { track: RemoteTrack::Audio(t), publication, participant }) => {
                    if let Some(l) = live.as_mut() {
                        let identity = participant.identity().0;
                        let user = user_of_identity(&identity).unwrap_or_else(|| identity.clone());
                        // Replace whatever was playing for this person (dropping it closes its stream).
                        if let Some(old) = l.remote.remove(&user) { old.task.abort() }
                        let stream = NativeAudioStream::new(t.rtc_track(), 48_000, 1);
                        let task = play_remote(stream, user.clone(), l.speaker.clone(), shared.clone());
                        l.remote.insert(user, Remote { identity, track: publication.sid().to_string(), task });
                    }
                }
                Some(RoomEvent::TrackPublished { publication, participant }) => {
                    if let Some(l) = live.as_ref() { apply_subscription(&publication, &participant.identity().0, &l.watch) }
                }
                Some(RoomEvent::TrackSubscribed { track: RemoteTrack::Video(t), publication, participant }) => {
                    if let Some(l) = live.as_mut() && let Some(conn) = connection_of_identity(&participant.identity().0) {
                        let kind = if publication.source() == TrackSource::Screenshare { "screen" } else { "cam" };
                        let key = format!("{kind}:{conn}");
                        let stream = NativeVideoStream::new(t.rtc_track());
                        let task = show_video(stream, key, frames.clone(), tell.clone());
                        if let Some(old) = l.video.insert(publication.sid().to_string(), task) { old.abort() }
                    }
                }
                Some(RoomEvent::TrackUnsubscribed { track: RemoteTrack::Video(_), publication, participant }) => {
                    if let Some(l) = live.as_mut() {
                        if let Some(t) = l.video.remove(&publication.sid().to_string()) { t.abort() }
                        if let Some(conn) = connection_of_identity(&participant.identity().0) {
                            let kind = if publication.source() == TrackSource::Screenshare { "screen" } else { "cam" };
                            frames.lock().unwrap().remove(&format!("{kind}:{conn}"));
                            tell();
                        }
                    }
                }
                Some(RoomEvent::TrackUnsubscribed { publication, participant, .. }) => {
                    if let Some(l) = live.as_mut() {
                        let identity = participant.identity().0;
                        let user = user_of_identity(&identity).unwrap_or_else(|| identity.clone());
                        let sid = publication.sid().to_string();
                        if l.remote.get(&user).is_some_and(|r| r.track == sid) && let Some(r) = l.remote.remove(&user) { r.task.abort() }
                    }
                }
                Some(RoomEvent::ParticipantDisconnected(p)) => {
                    if let Some(l) = live.as_mut() {
                        let identity = p.identity().0;
                        let user = user_of_identity(&identity).unwrap_or_else(|| identity.clone());
                        // Only that participant's own stream: a newer one of the same person stays.
                        if l.remote.get(&user).is_some_and(|r| r.identity == identity) && let Some(r) = l.remote.remove(&user) { r.task.abort() }
                    }
                }
                Some(RoomEvent::Disconnected { .. }) | None => {
                    if live.take().is_some() { frames.lock().unwrap().clear(); send(Ev::Disconnected) }
                    if want_meter { meter = Some(start_capture(mic_choice.clone(), shared.clone(), None)) }
                }
                Some(_) => {}
            },
        }
    }
}

async fn connect(endpoint: &str, token: &str, e2ee_key: Option<String>, muted: bool, settings: Settings, shared: Arc<Shared>) -> Result<Live, String> {
    let mut options = RoomOptions::default();
    // Subscribing is ours to decide: voices and cameras always, screen shares only when watched.
    options.auto_subscribe = false;
    if let Some(key) = e2ee_key {
        options.encryption = Some(E2eeOptions {
            encryption_type: EncryptionType::Gcm,
            key_provider: KeyProvider::with_shared_key(KeyProviderOptions::default(), key.into_bytes()),
        });
    }
    let (room, events) = Room::connect(endpoint, token, options).await.map_err(|e| format!("não foi possível entrar na sala de voz: {e}"))?;
    // The room is fed 10 ms frames from the microphone thread.
    let source = NativeAudioSource::new(AudioSourceOptions { echo_cancellation: true, noise_suppression: false, auto_gain_control: false }, 48_000, 1, 200);
    let track = LocalAudioTrack::create_audio_track("microphone", RtcAudioSource::Native(source.clone()));
    if muted { track.mute() }
    let (frames, mut rx) = unbounded_channel::<Vec<i16>>();
    let feed = tokio::spawn(async move {
        while let Some(pcm) = rx.recv().await {
            let frame = AudioFrame { data: Cow::Owned(pcm), sample_rate: 48_000, num_channels: 1, samples_per_channel: FRAME as u32 };
            if source.capture_frame(&frame).await.is_err() { break }
        }
    });
    let capture = Some(start_capture(settings.mic.clone(), shared, Some(frames.clone())));
    let publish = TrackPublishOptions { source: TrackSource::Microphone, ..Default::default() };
    room.local_participant().publish_track(LocalTrack::Audio(track.clone()), publish).await
        .map_err(|e| format!("não foi possível abrir o microfone: {e}"))?;
    Ok(Live { room: Arc::new(room), mic: track, events, capture, frames, feed, remote: Default::default(), speaker: settings.speaker, video: Default::default(), watch: Default::default(), screen: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creating_the_webrtc_factory_does_not_abort_on_this_platform() {
        // macOS used to abort here: WebRTC's `+[NSString stringForStdString:]` category was not
        // linked (see app/build.rs). A dead endpoint must produce an error, not a crash.
        let shared = crate::audio::Shared::new(&Settings::default());
        let r = tokio::time::timeout(std::time::Duration::from_secs(10),
            connect("wss://127.0.0.1:9", "not-a-token", None, true, Settings::default(), shared)).await;
        match r {
            Ok(r) => assert!(r.is_err(), "a dead endpoint must not produce a room"),
            Err(_) => eprintln!("the dead endpoint kept retrying; the factory was created"),
        }
    }

    #[test]
    fn the_round_trip_time_comes_from_the_connection_in_use() {
        use livekit::webrtc::stats::RtcStats;
        let rtcp: RtcStats = serde_json::from_value(serde_json::json!({"type": "remote-inbound-rtp", "roundTripTime": 0.180})).unwrap();
        let used: RtcStats = serde_json::from_value(serde_json::json!({"type": "candidate-pair", "nominated": true, "currentRoundTripTime": 0.042})).unwrap();
        let lost: RtcStats = serde_json::from_value(serde_json::json!({"type": "candidate-pair", "nominated": false, "currentRoundTripTime": 0.400})).unwrap();
        assert_eq!(rtt_ms(&[lost.clone(), rtcp.clone(), used]), Some(42), "the nominated pair wins over the RTCP report and over pairs not in use");
        assert_eq!(rtt_ms(&[lost, rtcp]), None, "the RTCP report is not a stand-in");
        assert_eq!(rtt_ms(&[]), None);
    }

    #[test]
    fn rnnoise_processes_frames_and_takes_energy_out_of_hiss() {
        let mut rnn = DenoiseState::new();
        let (mut input, mut output) = ([0f32; FRAME], [0f32; FRAME]);
        // Deterministic hiss: a cheap LCG, quiet like room noise, 100 frames (1 s) so the model settles.
        let mut x = 12345u32;
        let (mut before, mut after) = (0f64, 0f64);
        for f in 0..100 {
            let mut pcm: Vec<i16> = (0..FRAME).map(|_| { x = x.wrapping_mul(1664525).wrapping_add(1013904223); ((x >> 16) as i16) / 100 }).collect();
            let e0: f64 = pcm.iter().map(|s| (*s as f64).powi(2)).sum();
            denoise_frame(&mut rnn, &mut pcm, &mut input, &mut output);
            if f >= 50 { before += e0; after += pcm.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() }
        }
        // White hiss is not what RNNoise is best at (it is trained on speech and everyday noise), so
        // only the wiring is checked here: frames change and lose energy.
        assert!(after < before * 0.9, "noise energy {after} should be under {before}");
    }

    #[test]
    fn gain_and_level_behave() {
        let mut pcm = vec![1000i16, -1000, 30000];
        scale(&mut pcm, 0.5);
        assert_eq!(pcm, vec![500, -500, 15000]);
        scale(&mut pcm, 4.0);
        assert_eq!(pcm[2], i16::MAX, "clips instead of wrapping");
        assert_eq!(level_of(&[0; 480]), 0);
        assert!(level_of(&[16000; 480]) > 900);
    }

    #[test]
    fn participants_map_back_to_users() {
        assert_eq!(user_of_identity("user_123456_conn-abc"), Some("123456".into()));
        assert_eq!(user_of_identity("user_9_x_y"), Some("9".into()));
        assert_eq!(user_of_identity("bot"), None);
        assert_eq!(user_of_identity("user__x"), None);
    }
}

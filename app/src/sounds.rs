//! The short sounds of a voice call, synthesized (soft sine blips) so no audio files ship with the
//! app. Each plays on its own thread through PulseAudio, on the chosen output and at its volume.

use crate::audio;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound { Join, Leave, UserJoin, UserLeave, Mute, Unmute, Deafen, Undeafen }

/// Notes as (frequency in Hz, length in ms).
fn notes(s: Sound) -> &'static [(f32, u32)] {
    match s {
        Sound::Join => &[(523.0, 90), (784.0, 90), (1047.0, 140)],
        Sound::Leave => &[(784.0, 90), (523.0, 90), (392.0, 150)],
        Sound::UserJoin => &[(660.0, 80), (880.0, 130)],
        Sound::UserLeave => &[(880.0, 80), (660.0, 130)],
        Sound::Mute => &[(520.0, 70), (340.0, 110)],
        Sound::Unmute => &[(340.0, 70), (560.0, 110)],
        Sound::Deafen => &[(440.0, 70), (300.0, 70), (200.0, 120)],
        Sound::Undeafen => &[(300.0, 70), (440.0, 70), (620.0, 120)],
    }
}

/// 48 kHz mono samples of a sound at `volume` (0..=1): each note has a quick attack and a decay,
/// and the whole is kept quiet so it never startles.
pub fn render(s: Sound, volume: f32) -> Vec<i16> {
    const RATE: f32 = 48_000.0;
    let amp = 0.28 * volume.clamp(0.0, 1.0) * i16::MAX as f32;
    let mut out = Vec::new();
    for &(freq, ms) in notes(s) {
        let n = (RATE * ms as f32 / 1000.0) as usize;
        for i in 0..n {
            let t = i as f32 / RATE;
            let attack = (i as f32 / (RATE * 0.005)).min(1.0);
            let decay = (1.0 - i as f32 / n as f32).powi(2);
            out.push((amp * attack * decay * (std::f32::consts::TAU * freq * t).sin()) as i16);
        }
    }
    out
}

pub fn play(s: Sound, sink: Option<String>, volume: f32) {
    if volume <= 0.0 { return }
    std::thread::Builder::new().name("fluxer-sound".into()).spawn(move || {
        let Some(out) = audio::open_playback(sink.as_deref()) else { return };
        let mut pcm = render(s, volume);
        // A little silence so the last note is not cut by the stream closing.
        pcm.extend(std::iter::repeat_n(0, 2400));
        let bytes: Vec<u8> = pcm.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let _ = out.write(&bytes);
        let _ = out.drain();
    }).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_renders_audible_but_quiet_samples() {
        for s in [Sound::Join, Sound::Leave, Sound::UserJoin, Sound::UserLeave, Sound::Mute, Sound::Unmute, Sound::Deafen, Sound::Undeafen] {
            let pcm = render(s, 1.0);
            let peak = pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
            assert!(peak > 3000 && peak < 12000, "{s:?} peaks at {peak}");
        }
        assert!(render(Sound::Join, 0.0).iter().all(|v| *v == 0), "volume 0 is silent");
    }
}

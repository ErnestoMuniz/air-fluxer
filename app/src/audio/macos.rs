//! CoreAudio backend for macOS, through cpal. The rest of the app speaks 48 kHz mono signed
//! 16-bit; a device that only offers another rate is resampled linearly on the way in or out.

use super::{Device, Devices};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SampleRate, SizedSample};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What the rest of the app speaks.
const RATE: u32 = 48_000;

type Queue = Arc<Mutex<VecDeque<f32>>>;

/// Linear resampling between the device's rate and 48 kHz. Voice is not music: the cheap
/// interpolation is inaudible and keeps the timing right on a device that offers only 44.1 kHz.
struct Resampler {
    buf: VecDeque<f32>,
    /// Read position inside `buf`, in source samples.
    pos: f64,
    /// Source samples per output sample.
    step: f64,
}

impl Resampler {
    fn new(src: u32, dst: u32) -> Resampler { Resampler { buf: VecDeque::new(), pos: 0.0, step: src as f64 / dst as f64 } }

    fn push(&mut self, samples: impl IntoIterator<Item = f32>) { self.buf.extend(samples) }

    /// Whether `out` samples can be produced (the last one needs the sample after its position).
    fn ready(&self, out: usize) -> bool {
        out == 0 || self.buf.len() as f64 >= self.pos + (out - 1) as f64 * self.step + 1.0
    }

    /// Fills `out`; false when not enough samples have arrived yet.
    fn read(&mut self, out: &mut [f32]) -> bool {
        if out.is_empty() { return true }
        if !self.ready(out.len()) { return false }
        for o in out.iter_mut() {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            let a = self.buf.get(i).copied().unwrap_or(0.0);
            let b = self.buf.get(i + 1).copied().unwrap_or(a);
            *o = a + (b - a) * frac;
            self.pos += self.step;
        }
        let used = (self.pos.floor() as usize).min(self.buf.len());
        self.buf.drain(..used);
        self.pos -= used as f64;
        true
    }

    /// Everything buffered so far, resampled (playback writes a whole sound at once).
    fn all(&mut self) -> Vec<f32> {
        let room = self.buf.len() as f64 - 1.0;
        if room < 0.0 { return Vec::new() }
        let mut out = vec![0.0; (room / self.step).floor() as usize + 1];
        if self.read(&mut out) { out } else { Vec::new() }
    }
}

pub struct Playback {
    /// Kept alive for as long as we play.
    _stream: cpal::Stream,
    queue: Queue,
    /// Samples handed to the stream and not yet played (for `drain`), at the device's rate.
    pending: Arc<AtomicUsize>,
    resampler: Mutex<Resampler>,
    failed: Arc<AtomicBool>,
}

impl Playback {
    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if self.failed.load(Ordering::Relaxed) { return Err("CoreAudio parou".into()) }
        let mut resampler = self.resampler.lock().unwrap();
        resampler.push(bytes.as_chunks::<2>().0.iter().map(|b| i16::from_ne_bytes(*b) as f32 / 32768.0));
        let out = resampler.all();
        let n = out.len();
        let mut q = self.queue.lock().unwrap();
        // A consumer that stopped must not grow memory without bound.
        if q.len() + n > RATE as usize * 5 {
            q.clear();
            self.pending.store(n, Ordering::Relaxed);
        } else {
            self.pending.fetch_add(n, Ordering::Relaxed);
        }
        q.extend(out);
        Ok(())
    }

    /// Waits until what was written has been heard (a sound's last note must not be cut off).
    pub fn drain(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.pending.load(Ordering::Relaxed) > 0 && !self.failed.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

pub struct Mic {
    /// Kept alive for as long as we record.
    _stream: cpal::Stream,
    queue: Queue,
    failed: Arc<AtomicBool>,
    resampler: Resampler,
    /// Scratch space for one read, reused across frames.
    scratch: Vec<f32>,
}

impl Mic {
    pub fn read(&mut self, bytes: &mut [u8]) -> Result<(), String> {
        if !bytes.len().is_multiple_of(2) { return Err("odd microphone buffer".into()) }
        let frames = bytes.len() / 2;
        if self.scratch.len() < frames { self.scratch.resize(frames, 0.0) }
        while !self.resampler.read(&mut self.scratch[..frames]) {
            if self.failed.load(Ordering::Relaxed) { return Err("CoreAudio parou".into()) }
            {
                let mut q = self.queue.lock().unwrap();
                if !q.is_empty() { self.resampler.push(q.drain(..)) }
            }
            if !self.resampler.ready(frames) { std::thread::sleep(std::time::Duration::from_millis(2)) }
        }
        for (b, s) in bytes.as_chunks_mut::<2>().0.iter_mut().zip(self.scratch[..frames].iter()) {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            *b = v.to_ne_bytes();
        }
        Ok(())
    }
}

/// Picks the cheapest configuration that runs at `rate`: f32 before integer formats, mono or
/// stereo before more channels.
fn pick(configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>, rate: SampleRate) -> Option<cpal::SupportedStreamConfig> {
    let mut best: Option<(u32, cpal::SupportedStreamConfig)> = None;
    for r in configs {
        if !(r.min_sample_rate()..=r.max_sample_rate()).contains(&rate) { continue }
        let format = match r.sample_format() {
            SampleFormat::F32 => 0,
            SampleFormat::I16 => 1,
            SampleFormat::U16 => 2,
            _ => continue,
        };
        let score = format * 100 + r.channels().min(2) as u32;
        if best.as_ref().is_none_or(|(b, _)| score < *b) {
            best = Some((score, r.with_sample_rate(rate)));
        }
    }
    best.map(|(_, c)| c)
}

fn output_config(device: &cpal::Device) -> Option<cpal::SupportedStreamConfig> {
    pick(device.supported_output_configs().ok()?, RATE).or_else(|| device.default_output_config().ok())
}

fn input_config(device: &cpal::Device) -> Option<cpal::SupportedStreamConfig> {
    pick(device.supported_input_configs().ok()?, RATE).or_else(|| device.default_input_config().ok())
}

fn build_output<T>(device: &cpal::Device, config: cpal::StreamConfig, queue: Queue, pending: Arc<AtomicUsize>,
    on_error: impl FnMut(cpal::Error) + Send + 'static) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device.build_output_stream(config, move |data: &mut [T], _| {
        let mut q = queue.lock().unwrap();
        let mut played = 0;
        for frame in data.chunks_mut(channels) {
            let v = if let Some(v) = q.pop_front() {
                played += 1;
                v
            } else {
                0.0
            };
            let sample: T = T::from_sample(v);
            for s in frame.iter_mut() { *s = sample }
        }
        if played > 0 { let _ = pending.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| Some(n.saturating_sub(played))); }
    }, on_error, None)
}

fn build_input<T>(device: &cpal::Device, config: cpal::StreamConfig, queue: Queue,
    on_error: impl FnMut(cpal::Error) + Send + 'static) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    device.build_input_stream(config, move |data: &[T], _| {
        let mut q = queue.lock().unwrap();
        for frame in data.chunks(channels) {
            // Downmix to mono; the voice pipeline has no use for separate channels.
            let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
            q.push_back(sum / channels as f32);
        }
        // A reader that fell behind must not build up delay (or memory).
        while q.len() > RATE as usize * 2 { q.pop_front(); }
    }, on_error, None)
}

fn stream_error(failed: &Arc<AtomicBool>, side: &'static str) -> impl FnMut(cpal::Error) + Send + 'static {
    let failed = failed.clone();
    move |e| { eprintln!("fluxer: audio {side}: {e}"); failed.store(true, Ordering::Relaxed) }
}

/// Opens a 48 kHz mono playback stream on `sink` (the default output when None or when it will not open).
pub fn open_playback(sink: Option<&str>) -> Option<Playback> {
    open_playback_on(sink).or_else(|| if sink.is_some() { open_playback_on(None) } else { None })
}

fn open_playback_on(sink: Option<&str>) -> Option<Playback> {
    let host = cpal::default_host();
    let device = sink.and_then(|id| id.parse().ok()).and_then(|id| host.device_by_id(&id))
        .or_else(|| host.default_output_device())?;
    let config = output_config(&device)?;
    let format = config.sample_format();
    let stream_config = config.config();
    let queue: Queue = Default::default();
    let pending = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicBool::new(false));
    let (q, p) = (queue.clone(), pending.clone());
    let err = stream_error(&failed, "out");
    let stream = match format {
        SampleFormat::F32 => build_output::<f32>(&device, stream_config, q, p, err),
        SampleFormat::I16 => build_output::<i16>(&device, stream_config, q, p, err),
        SampleFormat::U16 => build_output::<u16>(&device, stream_config, q, p, err),
        _ => return None,
    };
    let stream = match stream { Ok(s) => s, Err(e) => { eprintln!("fluxer: could not open the speakers {sink:?}: {e}"); return None } };
    if let Err(e) = stream.play() { eprintln!("fluxer: could not start the speakers: {e}"); return None }
    Some(Playback { _stream: stream, queue, pending, failed, resampler: Mutex::new(Resampler::new(RATE, stream_config.sample_rate)) })
}

/// Opens the microphone `device` (the default input when None or when it will not open).
pub fn open_mic(device: Option<&str>) -> Option<Mic> {
    open_mic_on(device).or_else(|| if device.is_some() { open_mic_on(None) } else { None })
}

fn open_mic_on(device: Option<&str>) -> Option<Mic> {
    let host = cpal::default_host();
    let device = device.and_then(|id| id.parse().ok()).and_then(|id| host.device_by_id(&id))
        .or_else(|| host.default_input_device())?;
    let config = input_config(&device)?;
    let format = config.sample_format();
    let stream_config = config.config();
    let queue: Queue = Default::default();
    let failed = Arc::new(AtomicBool::new(false));
    let q = queue.clone();
    let err = stream_error(&failed, "in");
    let stream = match format {
        SampleFormat::F32 => build_input::<f32>(&device, stream_config, q, err),
        SampleFormat::I16 => build_input::<i16>(&device, stream_config, q, err),
        SampleFormat::U16 => build_input::<u16>(&device, stream_config, q, err),
        _ => return None,
    };
    let stream = match stream { Ok(s) => s, Err(e) => { eprintln!("fluxer: could not open the microphone {device:?}: {e}"); return None } };
    if let Err(e) = stream.play() { eprintln!("fluxer: could not start the microphone: {e}"); return None }
    Some(Mic { _stream: stream, queue, failed, resampler: Resampler::new(stream_config.sample_rate, RATE), scratch: Vec::new() })
}

/// The current default input device's id (what "System default" resolves to right now).
pub fn default_input() -> Option<String> {
    cpal::default_host().default_input_device()?.id().ok().map(|id| id.to_string())
}

/// Lists microphones and speakers (blocking; run it off the UI thread).
pub fn list_devices() -> Result<Devices, String> {    fn collect(devices: impl Iterator<Item = cpal::Device>) -> Vec<Device> {
        let mut out: Vec<Device> = devices.map(|d| {
            let label = d.description().map(|i| i.name().to_string()).unwrap_or_else(|_| d.to_string());
            let id = d.id().map(|i| i.to_string()).unwrap_or_else(|_| label.clone());
            Device { id, label }
        }).collect();
        out.sort_by_key(|d| d.label.to_lowercase());
        out.dedup_by(|a, b| a.id == b.id);
        out
    }
    let host = cpal::default_host();
    let mics = collect(host.input_devices().into_iter().flatten());
    let speakers = collect(host.output_devices().into_iter().flatten());
    if mics.is_empty() && speakers.is_empty() { Err("nenhum dispositivo de áudio".into()) } else { Ok(Devices { mics, speakers }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_passes_a_constant_through_at_every_rate() {
        for (src, dst) in [(48_000, 48_000), (44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
            let mut r = Resampler::new(src, dst);
            r.push(std::iter::repeat_n(0.25, 10_000));
            let out = r.all();
            assert!(!out.is_empty(), "{src} -> {dst}");
            assert!(out.iter().all(|s| (*s - 0.25).abs() < 1e-6), "{src} -> {dst}");
        }
    }

    #[test]
    fn resampler_keeps_48k_frames_at_their_length_and_values() {
        let mut r = Resampler::new(48_000, 48_000);
        let input: Vec<f32> = (0..480).map(|i| (i as f32 / 480.0).sin()).collect();
        r.push(input.clone());
        assert_eq!(r.all(), input);
        // A partially read stream keeps the sample the next frame starts from.
        let mut r = Resampler::new(48_000, 48_000);
        r.push(input.clone());
        let mut part = vec![0.0; 479];
        assert!(r.read(&mut part));
        assert_eq!(part, input[..479]);
        r.push([1.0]);
        assert_eq!(r.all(), vec![input[479], 1.0]);
    }
}

//! PulseAudio backend (PipeWire serves it) for Linux: a blocking playback stream and a blocking
//! microphone stream, 48 kHz mono signed 16-bit, plus device enumeration.

use super::{Device, Devices};
use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet, State};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use std::cell::RefCell;
use std::rc::Rc;

/// One 10 ms frame at 48 kHz: what the microphone reads at a time.
const FRAME: usize = 480;

pub struct Playback(libpulse_simple_binding::Simple);

impl Playback {
    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.write(bytes).map_err(|e| format!("PulseAudio: {e}"))
    }

    /// Waits until what was written has been heard (a sound's last note must not be cut off).
    pub fn drain(&self) { let _ = self.0.drain(); }
}

pub struct Mic(libpulse_simple_binding::Simple);

impl Mic {
    pub fn read(&mut self, bytes: &mut [u8]) -> Result<(), String> {
        self.0.read(bytes).map_err(|e| format!("PulseAudio: {e}"))
    }
}

/// Opens a 48 kHz mono playback stream on `sink` (the default output when None or when it will not open).
pub fn open_playback(sink: Option<&str>) -> Option<Playback> {
    use libpulse_binding::def::BufferAttr;
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    let spec = Spec { format: Format::S16NE, channels: 1, rate: 48_000 };
    // About 60 ms of buffer: low delay, with room for the scheduler's hiccups.
    let attr = BufferAttr { maxlength: u32::MAX, tlength: 48_000 / 1000 * 60 * 2, prebuf: u32::MAX, minreq: u32::MAX, fragsize: u32::MAX };
    let open = |dev: Option<&str>| libpulse_simple_binding::Simple::new(None, "Fluxer", Direction::Playback, dev, "Voice", &spec, None, Some(&attr)).ok();
    open(sink).or_else(|| if sink.is_some() { open(None) } else { None }).map(Playback)
}

/// Opens the microphone `device`, reading a frame at a time (the default input when None or when it
/// will not open).
pub fn open_mic(device: Option<&str>) -> Option<Mic> {
    use libpulse_binding::def::BufferAttr;
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    let spec = Spec { format: Format::S16NE, channels: 1, rate: 48_000 };
    let attr = BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: (FRAME * 2) as u32 };
    let open = |dev: Option<&str>| libpulse_simple_binding::Simple::new(None, "Fluxer", Direction::Record, dev, "Voice", &spec, None, Some(&attr)).ok();
    open(device).or_else(|| if device.is_some() { open(None) } else { None }).map(Mic)
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

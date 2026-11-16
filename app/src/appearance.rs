//! The interface appearance: light, dark (the default) and Midnight (pure black), chosen in
//! Settings → Appearance. The choice is a process-wide atomic because the token helpers are called
//! deep in view code without the state at hand.

use std::sync::atomic::{AtomicU8, Ordering};

pub const LIGHT: u8 = 1;
pub const MIDNIGHT: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(0);

pub fn set(mode: u8) { MODE.store(mode.min(MIDNIGHT), Ordering::Relaxed) }
pub fn mode() -> u8 { MODE.load(Ordering::Relaxed) }
pub fn dark() -> bool { mode() != LIGHT }

/// The Rune kit theme for the current appearance; Midnight is the dark theme with black surfaces.
pub fn theme() -> rune::tok::Theme {
    match mode() {
        LIGHT => rune::tok::theme(false),
        MIDNIGHT => {
            let mut t = rune::tok::theme(true);
            t.bg = rune::oklch(0.0, 0.0, 0.0);
            t.card = rune::oklch(0.045, 0.0, 0.0);
            t.card_border = rune::oklcha(1.0, 0.0, 0.0, 0.07);
            t.control_track = rune::oklcha(1.0, 0.0, 0.0, 0.04);
            t.control_off = rune::oklcha(1.0, 0.0, 0.0, 0.07);
            t.indicator = rune::oklch(0.3, 0.0, 0.0);
            t.hover = rune::oklcha(1.0, 0.0, 0.0, 0.02);
            t.press = rune::oklcha(1.0, 0.0, 0.0, 0.045);
            t
        }
        _ => rune::tok::theme(true),
    }
}

/// One of the theme's accent colours, for the current appearance.
pub fn accent(i: usize) -> rune::Color { rune::tok::accent(i, dark()) }

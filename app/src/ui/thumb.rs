//! A translucent scroll thumb over the message list.
//!
//! It shows while the pointer moves over the messages, while they scroll, and while the pointer is
//! near it or drags it (then it is wide and brighter). Left alone it holds a moment, then narrows
//! and fades out together until it is invisible. Dragging it, or pressing its track, scrolls.
//!
//! Everything runs through live values, so neither scrolling nor moving the pointer rebuilds the view.

use rune::{Color, Drag, El, Live, Phase, PointerEvent, ScrollPos, Spring, col, el, oklcha};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// The strip along the right edge that takes presses; the thumb sits inside it.
const STRIP: f32 = 16.0;
/// The thumb's gap from the right edge, and the track's from the top.
const EDGE: f32 = 3.0;
const INSET: f32 = 8.0;
/// The track ends a little above the message box, whose top is the lowest the thumb may go: the
/// list runs on behind the box, but the thumb stays clear of it.
const BOTTOM: f32 = super::compose::BOX_H + super::compose::PAD_Y + 6.0;
const MIN_LEN: f32 = 32.0;
/// Thumb widths: faded out and about to vanish, while it shows, and with the pointer near or holding it.
const W_REST: f32 = 1.5;
const W_SHOWN: f32 = 7.0;
const W_NEAR: f32 = 10.0;
/// How near the right edge the pointer reveals and widens it.
const NEAR: f32 = 32.0;
/// How long it stays up after the last scroll or pointer move, before it narrows and fades.
const HOLD: Duration = Duration::from_millis(1100);

const SHOWN: fn() -> Color = || oklcha(1.0, 0.0, 0.0, 0.30);
const HELD: fn() -> Color = || oklcha(1.0, 0.0, 0.0, 0.55);

struct Inner {
    pos: Cell<ScrollPos>,
    /// The pointer is near the right edge.
    near: Cell<bool>,
    /// The thumb is being dragged.
    dragging: Cell<bool>,
    /// Shown at once and kept so, for `--scene scroll`.
    pinned: Cell<bool>,
    /// On the animation clock, when the current show ends, and when it was last renewed.
    until: Cell<f64>,
    renewed: Cell<f64>,
    /// Where the strip is in the area, the thumb's place and length in the strip (never eased, so
    /// a drag keeps up with the pointer), the pill in that place (eased in width), and how it looks.
    track: Live<[f32; 4]>,
    slot: Live<[f32; 4]>,
    pill: Live<[f32; 4]>,
    opacity: Live<f32>,
    fill: Live<Color>,
}

/// What the app keeps between views: one per message list.
#[derive(Clone)]
pub struct Thumb(Rc<Inner>);

impl Default for Thumb {
    fn default() -> Self {
        Thumb(Rc::new(Inner {
            pos: Cell::new(ScrollPos::default()),
            near: Cell::new(false),
            dragging: Cell::new(false),
            pinned: Cell::new(false),
            until: Cell::new(0.0),
            renewed: Cell::new(f64::NEG_INFINITY),
            track: Live::new([0.0; 4]),
            slot: Live::new([0.0; 4]),
            pill: Live::new([0.0; 4]),
            opacity: Live::new(0.0),
            fill: Live::new(SHOWN()),
        }))
    }
}

/// The track's length and whether a thumb shows at all, for a viewport at `at`.
fn track_len(at: ScrollPos) -> (f32, bool) {
    let len = at.viewport[1] - INSET - BOTTOM;
    (len, at.max[1] > 0.5 && len >= MIN_LEN)
}

/// The thumb's top (on the track) and length.
fn thumb_of(at: ScrollPos, track: f32) -> (f32, f32) {
    let (shown, max) = (at.viewport[1], at.max[1]);
    let len = (track * shown / (shown + max).max(1.0)).max(MIN_LEN).min(track);
    let top = if max > 0.0 { (track - len) * (at.offset[1] / max).clamp(0.0, 1.0) } else { 0.0 };
    (top, len)
}

/// The content offset that puts the thumb's top at `top` on the track.
fn offset_for(at: ScrollPos, track: f32, top: f32) -> f32 {
    let travel = track - thumb_of(at, track).1;
    if travel <= 0.0 { at.offset[1] } else { (top / travel * at.max[1]).clamp(0.0, at.max[1]) }
}

impl Inner {
    fn shows(&self) -> bool { track_len(self.pos.get()).1 }

    /// The thumb `w` wide, right-aligned in its slot, which is as long as the thumb.
    fn pill_rect(&self, w: f32) -> [f32; 4] {
        let at = self.pos.get();
        let (len, shows) = track_len(at);
        if !shows { return [0.0; 4] }
        [STRIP - EDGE - w, 0.0, w, thumb_of(at, len).1]
    }

    fn held(&self) -> bool { self.near.get() || self.dragging.get() }

    /// Up and wide while the pointer is near or holds it; it stays so until both let go.
    fn hold_wide(&self) {
        self.until.set(f64::INFINITY);
        if self.pinned.get() {
            self.pill.snap(self.pill_rect(W_NEAR));
            self.opacity.snap(1.0);
            self.fill.snap(HELD());
            return;
        }
        self.pill.set(self.pill_rect(W_NEAR));
        self.opacity.set(1.0);
        self.fill.set(HELD());
    }

    /// Shows the thumb now and lets it narrow and fade after [`HOLD`]; `now` is the animation clock.
    fn pulse(&self, now: f64, hold: Duration) {
        self.until.set(now + hold.as_secs_f64());
        self.renewed.set(now);
        self.fill.set(SHOWN());
        self.pill.pulse(self.pill_rect(W_SHOWN), self.pill_rect(W_REST), hold);
        self.opacity.pulse(1.0, 0.0, hold);
    }

    /// Activity at `now`, renewed at most every quarter second so a moving pointer is cheap.
    fn activity(&self, now: f64) {
        if self.shows() && !self.held() && now >= self.renewed.get() + 0.25 { self.pulse(now, HOLD) }
    }

    /// The viewport scrolled, resized or its content changed: follow it. `scrolled` says a hand did it.
    fn follow(&self, mut at: ScrollPos) {
        at.viewport = at.drawn;
        self.pos.set(at);
        let (scrolled, instant, now) = rune::scroll_watch_state();
        let (len, shows) = track_len(at);
        self.track.set(if shows { [at.viewport[0] - STRIP, INSET, STRIP, len] } else { [0.0; 4] });
        if !shows {
            self.slot.snap([0.0; 4]);
            self.opacity.snap(0.0);
            return;
        }
        let (top, h) = thumb_of(at, len);
        self.slot.snap([0.0, top, STRIP, h]);
        if self.held() {
            self.hold_wide();
        } else if scrolled && !instant {
            self.pulse(now, HOLD);
        } else if now < self.until.get() {
            // Shown and the content moved under it (a picture loaded, a message came): keep to it for what is left of the hold.
            self.pulse(now, Duration::from_secs_f64(self.until.get() - now));
        } else {
            self.pill.snap(self.pill_rect(W_REST));
        }
    }

    /// The pointer moved in the area: near the right edge it holds the thumb up and wide, elsewhere it just shows it.
    fn moved(&self, p: PointerEvent, now: f64) {
        let at = self.pos.get();
        let near = self.shows() && p.x >= at.viewport[0] - NEAR;
        if self.near.replace(near) != near {
            if near { self.hold_wide() } else if !self.dragging.get() { self.pulse(now, HOLD) }
        } else if !near {
            self.activity(now);
        }
    }

    /// The pointer left the area.
    fn left(&self, now: f64) {
        if self.near.replace(false) && !self.dragging.get() && self.shows() { self.pulse(now, HOLD) }
    }

    fn drag(&self, target: &str, d: &mut Drag, tx: &mut rune::Tx) {
        tx.live_only();
        let at = self.pos.get();
        let (len, shows) = track_len(at);
        if !shows { return }
        let (top, h) = thumb_of(at, len);
        match d.phase {
            Phase::Down => {
                // A press on the thumb holds it where it was caught; on the track it centres the thumb there.
                d.data = if (top..=top + h).contains(&d.y) { d.y - top } else { h / 2.0 };
                self.dragging.set(true);
                self.hold_wide();
            }
            Phase::Up => {
                self.dragging.set(false);
                if !self.near.get() { self.pulse(tx.clock(), HOLD) }
                return;
            }
            Phase::Move => {}
        }
        tx.scroll_to_instant(target, at.offset[0], offset_for(at, len, d.y - d.data));
    }
}

impl Thumb {
    /// Shows the thumb as if the pointer were near it, for looking at it without a mouse (`--scene scroll`).
    pub fn pin(&self) { self.0.near.set(true); self.0.pinned.set(true) }
}

/// `viewport` (a vertical scroll viewport keyed `viewport_key`, sized as it should be: it fills the
/// area) with the thumb over its right edge. `area_key` names the area the two share.
pub fn attach(thumb: &Thumb, area_key: &str, viewport_key: &str, viewport: El) -> El {
    let target = format!("{area_key}/{viewport_key}");
    let (watch, moved, left, drag) = (thumb.0.clone(), thumb.0.clone(), thumb.0.clone(), thumb.0.clone());
    let live = &thumb.0;
    col().key(area_key.to_string()).grow(1.0).min_h(0.0)
        .on_pointer_move_any(move |p, tx| { tx.live_only(); moved.moved(p, tx.clock()) })
        .on_hover_change_any(move |inside, tx| { tx.live_only(); if !inside { left.left(tx.clock()) } })
        .child(viewport.key(viewport_key.to_string()).chain_scroll_offset(move |at| watch.follow(at)))
        .child(el().key("msg-scroll-track").absolute(0.0, 0.0).live_rect(&live.track)
            .live_opacity(live.opacity.smooth(Spring::new(0.14, 0.0)).rest(Spring::new(0.35, 0.0)))
            .on_drag_any(move |d, tx| drag.drag(&target, d, tx))
            .a11y_hidden()
            .child(el().key("msg-scroll-slot").absolute(0.0, 0.0).live_rect(&live.slot)
                .child(el().key("msg-scroll-thumb").absolute(0.0, 0.0)
                    .live_rect(live.pill.smooth(Spring::new(0.16, 0.0)).rest(Spring::new(0.35, 0.0)))
                    .radius(W_NEAR / 2.0)
                    .live_fill(live.fill.smooth(Spring::new(0.15, 0.0))))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(offset: f32, max: f32) -> ScrollPos {
        let mut p = ScrollPos::default();
        (p.offset, p.max, p.viewport, p.drawn) = ([0.0, offset], [0.0, max], [400.0, 600.0], [400.0, 600.0]);
        p
    }

    #[test]
    fn the_thumb_is_as_long_as_the_view_is_of_the_content_and_travels_the_track() {
        let track = 584.0;
        let (top, len) = thumb_of(at(0.0, 600.0), track);
        assert_eq!(top, 0.0);
        assert!((len - track / 2.0).abs() < 1.0, "half the content is in view: {len}");
        let (top, len) = thumb_of(at(600.0, 600.0), track);
        assert!((top + len - track).abs() < 0.01, "scrolled to the end it rests on the track's end");
        assert_eq!(thumb_of(at(0.0, 600_000.0), track).1, MIN_LEN, "never shorter than the minimum");
    }

    #[test]
    fn dragging_to_a_place_gives_the_offset_that_puts_the_thumb_there_and_clamps() {
        let track = 584.0;
        let a = at(0.0, 6000.0);
        let (_, len) = thumb_of(a, track);
        assert_eq!(offset_for(a, track, 0.0), 0.0);
        assert!((offset_for(a, track, track - len) - 6000.0).abs() < 0.5, "the far end of its travel is the end of the content");
        assert_eq!(offset_for(a, track, -50.0), 0.0);
        assert_eq!(offset_for(a, track, 10_000.0), 6000.0);
        let mid = offset_for(a, track, (track - len) / 2.0);
        assert!((mid - 3000.0).abs() < 0.5);
    }

    #[test]
    fn nothing_shows_when_the_content_fits() {
        assert!(!track_len(at(0.0, 0.0)).1);
        assert!(track_len(at(0.0, 10.0)).1);
    }
}

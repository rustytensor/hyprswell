//! Auto-hide: the lip geometry, slide motion and the dwell/hide timeline. Pure: times are seconds on a
//! caller-supplied clock, distances are surface-local logical pixels, and positive offsets move the painted
//! dock down inside its fixed surface.

use crate::{motion::{self, InputRect, Layout, LayoutConfig, Rect}, rules::DockItem};
use std::f64::consts::PI;

/// Visible height of the hidden dock: the bar's rounded top and the running-app dots.
pub const LIP: f64 = 6.0;
/// Bottom screen-edge strip that also reveals; it lies inside the lip.
pub const TRIGGER: f64 = 2.0;
pub const DOT_RADIUS: f64 = 1.0;
pub const DWELL_SECONDS: f64 = 0.120;
pub const HIDE_DELAY_SECONDS: f64 = 0.700;
pub const HIDE_SECONDS: f64 = 0.200;
/// Same first peak and settling deadline as the icon spring, with 8% overshoot of the slide travel.
pub const REVEAL_PEAK_SECONDS: f64 = 0.140;
pub const REVEAL_SECONDS: f64 = 0.350;

pub fn reveal_damping() -> f64 { 12.5_f64.ln() / REVEAL_PEAK_SECONDS }

/// `AUTOHIDE=1|0`; missing or invalid is off, values are trimmed and the last occurrence wins.
pub fn enabled(config: &str) -> bool {
    let mut value = None;
    for line in config.lines() {
        if let Some((key, contents)) = line.split_once('=') {
            if key.trim() == "AUTOHIDE" { value = Some(contents.trim()); }
        }
    }
    value == Some("1")
}

/// The dock's layout for an icon size and slot count, and its fixed surface height (bar plus the 8 px gap).
pub fn dock_layout(size: u32, count: usize, apps: usize, reflections: bool) -> (LayoutConfig, i32) {
    let rest = crate::rules::drawn_size(size, apps).max(1) as f64 * 0.8;
    let bar_height = rest + 23.0;
    let base_height = (rest * 2.2 + 36.0).ceil() as i32;
    // Keep a fixed transparent surface; only the painted bar changes width.
    let width = (count as f64 * rest * 1.8 + count.saturating_sub(1) as f64 * 8.0 + 20.0).ceil();
    let config = motion::reflection_layout(LayoutConfig {
        surface_width: width, rest_width: rest, gap: 8.0, side_padding: 10.0,
        bar_y: base_height as f64 - bar_height, bar_height, rest_bottom: base_height as f64 - 17.0,
    }, reflections);
    (config, base_height + motion::reflection_padding(rest, reflections) as i32 + 8)
}

/// How far the painted dock slides so that exactly the lip stays on screen.
pub fn hidden_offset(surface_height: f64, bar_y: f64) -> f64 {
    surface_height - LIP - bar_y
}

/// The lip is reserved in every pose when auto-hide is on; otherwise the bar plus its 8 px gap.
pub fn exclusive_zone(autohide: bool, bar_height: f64) -> i32 {
    if autohide { LIP.ceil() as i32 } else { bar_height.ceil() as i32 + 8 }
}

/// The activation band under the rest bar's horizontal span, at the bottom of the surface.
pub fn lip_region(bar: Rect, surface_height: f64) -> InputRect {
    Rect { x: bar.x, y: surface_height - LIP, width: bar.width, height: LIP }.outward()
}

pub fn trigger_strip(bar: Rect, surface_height: f64) -> InputRect {
    Rect { x: bar.x, y: surface_height - TRIGGER, width: bar.width, height: TRIGGER }.outward()
}

/// Running apps get a lip dot: minimized windows count, several windows still give one, pinned-only and the
/// app grid give none.
pub fn running(items: &[DockItem]) -> Vec<bool> {
    items.iter().map(|item| item.id != "launcher" && !item.windows.is_empty()).collect()
}

/// One dot per running app, centred on its rest icon, inside the lip.
pub fn lip_dots(rest: &Layout, running: &[bool], surface_height: f64) -> Vec<(f64, f64)> {
    rest.icons.iter().zip(running).filter(|(_, on)| **on)
        .map(|(r, _)| (r.x + r.width / 2.0, surface_height - LIP / 2.0)).collect()
}

pub fn clip(rect: InputRect, width: i32, height: i32) -> Option<InputRect> {
    let (left, top) = (rect.x.max(0), rect.y.max(0));
    let (right, bottom) = ((rect.x + rect.width).min(width), (rect.y + rect.height).min(height));
    (right > left && bottom > top).then_some(InputRect { x: left, y: top, width: right - left, height: bottom - top })
}

pub fn translate(layout: &Layout, offset: f64) -> Layout {
    let shift = |r: &Rect| Rect { y: r.y + offset, ..*r };
    Layout { bar: shift(&layout.bar), icons: layout.icons.iter().map(shift).collect() }
}

/// Quadratic ease-in from `from` to the hidden distance over 200 ms.
pub fn hide_sample(from: f64, distance: f64, seconds: f64) -> (f64, f64) {
    let t = seconds.clamp(0.0, HIDE_SECONDS) / HIDE_SECONDS;
    let span = distance - from;
    (from + span * t * t, if seconds < HIDE_SECONDS { 2.0 * span * t / HIDE_SECONDS } else { 0.0 })
}

/// Unsnapped damped spring towards offset 0: value and velocity.
pub fn reveal_raw(from: f64, velocity: f64, seconds: f64) -> (f64, f64) {
    let t = seconds.max(0.0);
    let a = reveal_damping();
    let w = PI / REVEAL_PEAK_SECONDS;
    let d = (velocity + a * from) / w;
    let (sin, cos) = (w * t).sin_cos();
    let decay = (-a * t).exp();
    let offset = from * cos + d * sin;
    (decay * offset, decay * ((d * w * cos - from * w * sin) - a * offset))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone { Outside, Band, Dock }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pose {
    Hidden,
    Revealing { start: f64, from: f64, velocity: f64 },
    Shown,
    Hiding { start: f64, from: f64 },
}

/// One dock's auto-hide state. Waiting is done by the caller's one-shot timer at `next_deadline`, never by
/// frame polling; frames are needed only while `animating`.
#[derive(Clone, Debug)]
pub struct Controller {
    pub distance: f64,
    enabled: bool,
    pose: Pose,
    zone: Zone,
    blocked: bool,
    dwell: Option<f64>,
    hide_at: Option<f64>,
    band_held: bool,
    hide_after_reveal: bool,
}

impl Controller {
    /// Startup with auto-hide on is hidden.
    pub fn new(enabled: bool, distance: f64) -> Self {
        Self { distance, enabled, pose: if enabled { Pose::Hidden } else { Pose::Shown }, zone: Zone::Outside,
            blocked: false, dwell: None, hide_at: None, band_held: false, hide_after_reveal: false }
    }

    pub fn enabled(&self) -> bool { self.enabled }
    pub fn pose(&self) -> Pose { self.pose }
    pub fn settled_hidden(&self) -> bool { self.pose == Pose::Hidden }
    pub fn settled_shown(&self) -> bool { self.pose == Pose::Shown }
    pub fn animating(&self) -> bool { matches!(self.pose, Pose::Revealing { .. } | Pose::Hiding { .. }) }
    pub fn next_deadline(&self) -> Option<f64> {
        match (self.dwell, self.hide_at) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) }
    }

    /// The activation band takes input while hidden or hiding, and while the pointer stays in it after a reveal.
    pub fn band_input(&self) -> bool {
        matches!(self.pose, Pose::Hidden | Pose::Hiding { .. }) || self.band_held
    }

    fn motion(&self, now: f64) -> (f64, f64) {
        match self.pose {
            Pose::Hidden => (self.distance, 0.0),
            Pose::Shown => (0.0, 0.0),
            Pose::Hiding { start, from } => hide_sample(from, self.distance, now - start),
            Pose::Revealing { start, from, velocity } => {
                if now - start >= REVEAL_SECONDS { return (0.0, 0.0); }
                let (value, speed) = reveal_raw(from, velocity, now - start);
                // An interrupted hide still moving down must never carry the lip off screen.
                if value >= self.distance { (self.distance, 0.0) } else { (value, speed) }
            }
        }
    }

    pub fn offset(&self, now: f64) -> f64 { self.motion(now).0 }

    /// Apply every transition due by `now`, in time order.
    pub fn advance(&mut self, now: f64) {
        loop {
            let end = match self.pose {
                Pose::Revealing { start, .. } => Some(start + REVEAL_SECONDS),
                Pose::Hiding { start, .. } => Some(start + HIDE_SECONDS),
                _ => None,
            };
            let next = [end, self.dwell, self.hide_at].into_iter().flatten().filter(|&t| t <= now)
                .fold(None, |best: Option<f64>, t| Some(best.map_or(t, |b| b.min(t))));
            let Some(at) = next else { return; };
            if end == Some(at) {
                if matches!(self.pose, Pose::Revealing { .. }) {
                    self.pose = Pose::Shown;
                    if std::mem::take(&mut self.hide_after_reveal) { self.arm_hide(at); }
                } else {
                    self.pose = Pose::Hidden;
                    self.band_held = false;
                }
            } else if self.dwell == Some(at) {
                self.dwell = None;
                if self.pose == Pose::Hidden && self.zone != Zone::Outside { self.reveal(at); }
            } else {
                self.hide_at = None;
                if self.enabled && self.zone == Zone::Outside && !self.blocked
                    && matches!(self.pose, Pose::Shown | Pose::Revealing { .. }) {
                    self.pose = Pose::Hiding { start: at, from: self.offset(at) };
                    self.band_held = false;
                }
            }
        }
    }

    fn arm_hide(&mut self, at: f64) {
        if self.enabled && self.zone == Zone::Outside && !self.blocked && self.hide_at.is_none() {
            self.hide_at = Some(at + HIDE_DELAY_SECONDS);
        }
    }

    fn reveal(&mut self, at: f64) {
        let (from, velocity) = self.motion(at);
        self.pose = Pose::Revealing { start: at, from, velocity };
        self.band_held = self.zone == Zone::Band;
        self.dwell = None;
        self.hide_at = None;
    }

    pub fn pointer(&mut self, zone: Zone, now: f64) {
        self.advance(now);
        let previous = std::mem::replace(&mut self.zone, zone);
        if !self.enabled { return; }
        match self.pose {
            // Continuous presence: motion inside the band never restarts the dwell.
            Pose::Hidden => {
                if zone == Zone::Outside { self.dwell = None; }
                else if previous == Zone::Outside { self.dwell = Some(now + DWELL_SECONDS); }
            }
            Pose::Hiding { .. } => if zone != Zone::Outside { self.reveal(now); },
            Pose::Revealing { .. } | Pose::Shown => {
                if zone != Zone::Band { self.band_held = false; }
                if zone == Zone::Outside { self.arm_hide(now); } else { self.hide_at = None; }
            }
        }
    }

    /// An open menu or hover label blocks hiding; a fresh delay starts only when all are gone.
    pub fn set_blockers(&mut self, menu: bool, label: bool, now: f64) {
        self.advance(now);
        let blocked = menu || label;
        if blocked == self.blocked { return; }
        self.blocked = blocked;
        if blocked {
            self.hide_at = None;
            if matches!(self.pose, Pose::Hiding { .. }) { self.reveal(now); }
        } else if matches!(self.pose, Pose::Shown | Pose::Revealing { .. }) {
            self.arm_hide(now);
        }
    }

    /// Keyboard, control socket or attention: reveal at once, without dwell. Idempotent while revealing.
    pub fn show(&mut self, now: f64) {
        self.advance(now);
        if !self.enabled { return; }
        match self.pose {
            Pose::Hidden | Pose::Hiding { .. } => { self.reveal(now); self.hide_after_reveal = true; }
            Pose::Revealing { .. } => {}
            Pose::Shown => if self.hide_at.is_some() { self.hide_at = None; self.arm_hide(now); },
        }
    }

    /// Enabling keeps the current pose and uses the normal leave delay; disabling clears timers and reveals.
    pub fn set_enabled(&mut self, enabled: bool, now: f64) {
        self.advance(now);
        if enabled == self.enabled { return; }
        self.enabled = enabled;
        if enabled {
            if matches!(self.pose, Pose::Shown | Pose::Revealing { .. }) { self.arm_hide(now); }
        } else {
            let hidden = matches!(self.pose, Pose::Hidden | Pose::Hiding { .. });
            if hidden { self.reveal(now); }
            self.dwell = None;
            self.hide_at = None;
            self.band_held = false;
            self.hide_after_reveal = false;
        }
    }

    /// Surface input for this pose. `shown` is the untranslated shown region; everything is clipped.
    pub fn input_region(&self, now: f64, shown: &[InputRect], lip: InputRect, width: i32, height: i32) -> Vec<InputRect> {
        let mut region = Vec::new();
        if self.pose != Pose::Hidden {
            let offset = self.offset(now).round() as i32;
            region.extend(shown.iter().filter_map(|r| clip(InputRect { y: r.y + offset, ..*r }, width, height)));
        }
        if self.band_input() { region.extend(clip(lip, width, height)); }
        region
    }
}

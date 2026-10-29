//! The launcher is a fixed-size control, not an application icon.
use crate::{motion::{self, Layout, LayoutConfig, Rect, Spring}, rules::DockItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position { Left, Right, Center, Off }

pub fn position(config: &str) -> Position {
    let mut value = "";
    for line in config.lines() {
        if let Some((key, contents)) = line.split_once('=') {
            if key.trim() == "LAUNCHER" { value = contents.trim(); }
        }
    }
    match value { "right" => Position::Right, "center" => Position::Center,
        "off" => Position::Off, _ => Position::Left }
}

pub fn place_items(mut items: Vec<DockItem>, position: Position) -> Vec<DockItem> {
    let control = items.iter().position(|item| item.id == "launcher").map(|i| items.remove(i));
    if position != Position::Off {
        let index = match position { Position::Left => 0, Position::Right => items.len(),
            Position::Center => items.len().div_ceil(2), Position::Off => unreachable!() };
        items.insert(index, control.unwrap_or(DockItem { id: "launcher".into(), windows: vec![] }));
    }
    items
}

pub fn app_count(items: &[DockItem]) -> usize {
    items.iter().filter(|item| item.id != "launcher").count()
}

pub fn layout(config: LayoutConfig, scales: &[f64], grid: Option<usize>) -> Layout {
    let mut scales = scales.to_vec();
    if let Some(scale) = grid.and_then(|i| scales.get_mut(i)) { *scale = 1.0; }
    let mut layout = config.layout(&scales);
    if let Some(rect) = grid.and_then(|i| layout.icons.get_mut(i)) {
        rect.y = layout.bar.y + (layout.bar.height - rect.height) / 2.0;
    }
    layout
}

pub fn reflection(rect: Rect) -> motion::Reflection {
    motion::reflection(rect, rect.y + rect.height)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look { pub spacing: f64, pub accent: f64 }
impl Default for Look {
    fn default() -> Self { Self { spacing: 1.0, accent: 0.0 } }
}

pub fn dots(rect: Rect, look: Look) -> ([(f64, f64); 9], f64) {
    let size = rect.width.min(rect.height);
    let step = size / 4.0 * look.spacing;
    (std::array::from_fn(|i| (rect.x + rect.width / 2.0 + (i % 3) as f64 * step - step,
        rect.y + rect.height / 2.0 + (i / 3) as f64 * step - step)), size / 16.0)
}

#[derive(Clone, Copy, Debug)]
pub struct Sample { pub look: Look, pub active: bool, pub launch: bool }

#[derive(Default)]
pub struct Animation {
    hovered: bool,
    pressed: bool,
    from: Look,
    look: Look,
    began: f64,
    spring: Option<(f64, Spring)>,
    launch_at: Option<f64>,
}

impl Animation {
    fn target(&self) -> Look {
        if self.hovered { Look { spacing: 1.2, accent: 1.0 } } else { Look::default() }
    }

    fn evaluate(&mut self, now: f64) -> (Look, bool) {
        let target = self.target();
        let t = ((now - self.began) / 0.15).clamp(0.0, 1.0);
        let q = 1.0 - (1.0 - t).powi(3);
        let mut look = Look { spacing: self.from.spacing + (target.spacing - self.from.spacing) * q,
            accent: self.from.accent + (target.accent - self.from.accent) * q };
        let mut active = t < 1.0 && self.from != target;
        if self.pressed { look.spacing = 0.6; }
        else if let Some((start, spring)) = self.spring {
            let sample = spring.sample(now - start);
            look.spacing = sample.value;
            active |= sample.active;
            if !sample.active { self.spring = None; }
        }
        self.look = look;
        (look, active)
    }

    pub fn set_hover(&mut self, hovered: bool, now: f64) {
        if self.hovered == hovered { return; }
        self.evaluate(now);
        self.from = self.look;
        self.began = now;
        self.hovered = hovered;
        if let Some((start, spring)) = self.spring {
            let sample = spring.sample(now - start);
            self.spring = Some((now, Spring::new(sample.value, sample.velocity, self.target().spacing)));
        }
    }

    pub fn press(&mut self, now: f64) {
        self.evaluate(now);
        self.pressed = true;
        self.spring = None;
        self.launch_at = None;
    }

    pub fn is_pressed(&self) -> bool { self.pressed }

    pub fn release(&mut self, now: f64, activate: bool) {
        self.pressed = false;
        self.spring = Some((now, Spring::new(0.6, 0.0, self.target().spacing)));
        self.launch_at = activate.then_some(now + motion::SETTLE_SECONDS);
    }

    pub fn sample(&mut self, now: f64) -> Sample {
        let (look, active) = self.evaluate(now);
        let launch = self.launch_at.is_some_and(|deadline| now >= deadline);
        if launch { self.launch_at = None; }
        Sample { look, active: active || self.launch_at.is_some(), launch }
    }
}

//! Pure dock geometry and motion. Distances are logical pixels; times are seconds.

use std::f64::consts::PI;

pub const SETTLE_SECONDS: f64 = 0.350;
pub const SQUASH_SECONDS: f64 = 0.090;

/// Distance in rest icon widths, never in the changing packed layout.
pub fn swell_scale(distance: f64) -> f64 {
    let d = distance.abs();
    if d >= 2.5 {
        1.0
    } else {
        1.0 + 0.3 * (1.0 + (PI * d / 2.5).cos())
    }
}

/// Rest centres stay fixed even as the visible bar widens.
pub fn pointer_scales(rest_centres: &[f64], pointer_x: Option<f64>, rest_width: f64) -> Vec<f64> {
    assert!(rest_width.is_finite() && rest_width > 0.0);
    rest_centres.iter().map(|&x| {
        pointer_x.map_or(1.0, |p| swell_scale((p - x) / rest_width))
    }).collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    /// Outward rounding keeps fractional edge pixels clickable.
    pub fn outward(self) -> InputRect {
        let left = self.x.floor() as i32;
        let top = self.y.floor() as i32;
        InputRect {
            x: left,
            y: top,
            width: (self.x + self.width).ceil() as i32 - left,
            height: (self.y + self.height).ceil() as i32 - top,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutConfig {
    pub surface_width: f64,
    pub rest_width: f64,
    pub gap: f64,
    pub side_padding: f64,
    pub bar_y: f64,
    pub bar_height: f64,
    pub rest_bottom: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub bar: Rect,
    pub icons: Vec<Rect>,
}

impl LayoutConfig {
    /// Positive finite scales; nonnegative finite gaps/padding and positive sizes.
    pub fn layout(self, scales: &[f64]) -> Layout {
        self.pack(scales, None)
    }

    /// No swell: fixed sizes, with the caller's legacy hover rise in pixels.
    pub fn no_swell(self, count: usize, hovered: Option<usize>, hover_rise: f64) -> Layout {
        self.pack(&vec![1.0; count], hovered.map(|i| (i, hover_rise)))
    }

    fn pack(self, scales: &[f64], hover: Option<(usize, f64)>) -> Layout {
        let content_width = scales.iter().sum::<f64>() * self.rest_width
            + scales.len().saturating_sub(1) as f64 * self.gap;
        let width = content_width + 2.0 * self.side_padding;
        let bar = Rect {
            x: (self.surface_width - width) / 2.0,
            y: self.bar_y,
            width,
            height: self.bar_height,
        };
        let mut x = bar.x + self.side_padding;
        let icons = scales.iter().enumerate().map(|(i, &scale)| {
            let size = self.rest_width * scale;
            let rise = hover.filter(|&(index, _)| index == i)
                .map_or(0.5 * self.rest_width * (scale - 1.0), |(_, rise)| rise);
            let rect = Rect { x, y: self.rest_bottom - rise - size, width: size, height: size };
            x += size + self.gap;
            rect
        }).collect();
        Layout { bar, icons }
    }
}

impl Layout {
    /// Union the bar with icon portions outside it. Shadows never take input.
    pub fn input_region(&self) -> Vec<InputRect> {
        let mut region = vec![self.bar.outward()];
        let b = self.bar;
        for &r in &self.icons {
            let right = r.x + r.width;
            let bottom = r.y + r.height;
            let left = r.x.max(b.x);
            let top = r.y.max(b.y);
            let cut_right = right.min(b.x + b.width);
            let cut_bottom = bottom.min(b.y + b.height);
            if left >= cut_right || top >= cut_bottom {
                region.push(r.outward());
                continue;
            }
            for part in [
                Rect { x: r.x, y: r.y, width: r.width, height: top - r.y },
                Rect { x: r.x, y: cut_bottom, width: r.width, height: bottom - cut_bottom },
                Rect { x: r.x, y: top, width: left - r.x, height: cut_bottom - top },
                Rect { x: cut_right, y: top, width: right - cut_right, height: cut_bottom - top },
            ] {
                if part.width > 0.0 && part.height > 0.0 {
                    region.push(part.outward());
                }
            }
        }
        region
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    pub image: Rect,
    pub visible: Rect,
    pub flip_y: f64,
    pub stops: [(f64, f64); 2],
}

pub fn reflection(icon: Rect, baseline: f64) -> Reflection {
    let rise = baseline - (icon.y + icon.height);
    let top = baseline + rise;
    let image = Rect { x: icon.x, y: top, width: icon.width, height: icon.height };
    Reflection { image, visible: Rect { height: icon.height * 0.45, ..image },
        flip_y: top + icon.height, stops: [(0.0, 0.55), (1.0, 0.0)] }
}

pub fn reflection_padding(rest_width: f64, enabled: bool) -> f64 {
    if enabled { (rest_width * 0.45).ceil() } else { 0.0 }
}

pub fn reflection_layout(mut config: LayoutConfig, enabled: bool) -> LayoutConfig {
    config.bar_height += reflection_padding(config.rest_width, enabled);
    config
}

pub fn shelf(layout: &Layout, baseline: f64, side_padding: f64) -> Rect {
    Rect { x: layout.bar.x + side_padding, y: baseline,
        width: (layout.bar.width - 2.0 * side_padding).max(0.0), height: 1.0 }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub value: f64,
    /// Units per second, suitable for continuous retargeting.
    pub velocity: f64,
    /// A frame callback is needed only while this is true.
    pub active: bool,
}

/// Retarget by sampling the old spring and passing its value and velocity here.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub initial: f64,
    pub velocity: f64,
    pub target: f64,
}

impl Spring {
    pub fn new(initial: f64, velocity: f64, target: f64) -> Self {
        Self { initial, velocity, target }
    }

    /// Unsnapped analytic response, including the small residual after 350 ms.
    pub fn raw(self, seconds: f64) -> Sample {
        let t = seconds.max(0.0);
        let a = 7.5_f64.ln() / 0.14;
        let w = PI / 0.14;
        let c = self.initial - self.target;
        let d = (self.velocity + a * c) / w;
        let (sin, cos) = (w * t).sin_cos();
        let decay = (-a * t).exp();
        let offset = c * cos + d * sin;
        Sample {
            value: self.target + decay * offset,
            velocity: decay * ((d * w * cos - c * w * sin) - a * offset),
            active: c != 0.0 || self.velocity != 0.0,
        }
    }

    pub fn sample(self, seconds: f64) -> Sample {
        if seconds >= SETTLE_SECONDS {
            Sample { value: self.target, velocity: 0.0, active: false }
        } else {
            self.raw(seconds)
        }
    }
}

/// Multiply the current swell by this sample; do not replace it with 0.9.
pub fn click_squash(seconds: f64) -> Sample {
    let t = seconds.max(0.0);
    if t <= SQUASH_SECONDS {
        let phase = PI * t / SQUASH_SECONDS;
        Sample {
            value: 1.0 - 0.1 * (1.0 - phase.cos()) / 2.0,
            velocity: -0.05 * PI / SQUASH_SECONDS * phase.sin(),
            active: true,
        }
    } else {
        Spring::new(0.9, 0.0, 1.0).sample(t - SQUASH_SECONDS)
    }
}

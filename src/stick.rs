//! Analog stick processing: calibration, deadzone, response curve, N64 gate.

use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

pub const BINS: usize = 64;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gate {
    /// Octagonal output like a real N64 stick.
    Octagon,
    Circle,
    Square,
}

/// Recorded physical range of the host stick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Resting position.
    pub center: [f32; 2],
    /// Max reach per angle bin (relative to `center`), `BINS` entries starting at angle 0, CCW.
    pub radii: Vec<f32>,
}

impl Calibration {
    pub fn radius_at(&self, angle: f32) -> f32 {
        if self.radii.len() != BINS {
            return 1.0;
        }
        let f = angle.rem_euclid(TAU) / TAU * BINS as f32;
        let i = f.floor() as usize % BINS;
        let t = f.fract();
        let r = self.radii[i] * (1.0 - t) + self.radii[(i + 1) % BINS] * t;
        r.max(0.05)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StickSettings {
    /// Inner deadzone, fraction of full reach.
    pub deadzone: f32,
    /// Fraction of full reach at which output saturates.
    pub saturation: f32,
    /// Max output at cardinal directions (real controllers: ~80-85, hardware max 127).
    pub range: f32,
    /// Octagon diagonal per-axis value relative to `range` (N64 ≈ 0.82).
    pub diagonal: f32,
    pub gate: Gate,
    /// Response exponent; 1 = linear, >1 = finer control near center.
    pub curve: f32,
    /// Custom response curve: (input, output) points in 0..=1, sorted by input.
    /// When set, replaces `curve`. (0,0) and (1,1) are implied endpoints if missing.
    pub custom_curve: Option<Vec<[f32; 2]>>,
    pub calibration: Option<Calibration>,
}

pub fn default_custom_curve() -> Vec<[f32; 2]> {
    vec![
        [0.0, 0.0],
        [0.25, 0.25],
        [0.5, 0.5],
        [0.75, 0.75],
        [1.0, 1.0],
    ]
}

/// Piecewise-linear interpolation through `pts` (sorted by x).
pub fn eval_curve(pts: &[[f32; 2]], x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let mut prev = [0.0f32, 0.0];
    for p in pts.iter().chain(std::iter::once(&[1.0, 1.0])) {
        if x <= p[0] {
            let w = p[0] - prev[0];
            let t = if w <= 1e-6 { 1.0 } else { (x - prev[0]) / w };
            return (prev[1] + (p[1] - prev[1]) * t).clamp(0.0, 1.0);
        }
        prev = *p;
    }
    prev[1].clamp(0.0, 1.0)
}

impl Default for StickSettings {
    fn default() -> Self {
        StickSettings {
            deadzone: 0.08,
            saturation: 1.0,
            range: 85.0,
            diagonal: 0.82,
            gate: Gate::Octagon,
            curve: 1.0,
            custom_curve: None,
            calibration: None,
        }
    }
}

impl StickSettings {
    pub fn center(&self) -> [f32; 2] {
        self.calibration.as_ref().map_or([0.0, 0.0], |c| c.center)
    }

    /// Physical reach of the host stick at `angle` (raw units).
    pub fn reach(&self, angle: f32) -> f32 {
        self.calibration
            .as_ref()
            .map_or(1.0, |c| c.radius_at(angle))
    }

    /// Output gate radius at `angle`, in units of `range`.
    pub fn gate_radius(&self, angle: f32) -> f32 {
        match self.gate {
            Gate::Circle => 1.0,
            Gate::Square => 1.0 / angle.cos().abs().max(angle.sin().abs()).max(1e-6),
            Gate::Octagon => {
                let a = angle.rem_euclid(TAU);
                let sector = (a / (PI / 4.0)).floor();
                let v = |k: f32| {
                    let ang = k * PI / 4.0;
                    let r = if (k as i32) % 2 == 0 {
                        1.0
                    } else {
                        self.diagonal * std::f32::consts::SQRT_2
                    };
                    (r * ang.cos(), r * ang.sin())
                };
                let (p0, p1) = (v(sector), v(sector + 1.0));
                let e = (p1.0 - p0.0, p1.1 - p0.1);
                let d = (a.cos(), a.sin());
                let cross = |a: (f32, f32), b: (f32, f32)| a.0 * b.1 - a.1 * b.0;
                let den = cross(d, e);
                if den.abs() < 1e-6 {
                    1.0
                } else {
                    cross(p0, e) / den
                }
            }
        }
    }

    /// Normalized magnitude (0..=1) after calibration, deadzone and curve, plus angle.
    pub fn shape(&self, raw: [f32; 2]) -> (f32, f32) {
        let c = self.center();
        let (x, y) = (raw[0] - c[0], raw[1] - c[1]);
        let r = (x * x + y * y).sqrt();
        if r < 1e-6 {
            return (0.0, 0.0);
        }
        let angle = y.atan2(x);
        let n = (r / self.reach(angle)).min(1.0);
        let dz = self.deadzone.clamp(0.0, 0.9);
        let sat = self.saturation.clamp(dz + 0.01, 1.0);
        let m = ((n - dz) / (sat - dz)).clamp(0.0, 1.0);
        (self.respond(m), angle)
    }

    /// Response curve applied to the normalized magnitude.
    pub fn respond(&self, m: f32) -> f32 {
        match &self.custom_curve {
            Some(pts) if !pts.is_empty() => eval_curve(pts, m),
            _ => m.powf(self.curve.clamp(0.2, 5.0)),
        }
    }

    /// Final N64 stick values. `scale` comes from active modifiers.
    pub fn process(&self, raw: [f32; 2], scale: f32) -> (i8, i8) {
        let (m, angle) = self.shape(raw);
        let rad = m * scale * self.range * self.gate_radius(angle);
        let q = |v: f32| v.round().clamp(-128.0, 127.0) as i8;
        (q(rad * angle.cos()), q(rad * angle.sin()))
    }
}

/// Collects samples while the user holds the stick still.
#[derive(Default)]
pub struct CenterRecorder {
    samples: Vec<[f32; 2]>,
}

impl CenterRecorder {
    pub fn add(&mut self, raw: [f32; 2]) {
        self.samples.push(raw);
    }

    /// Returns (center, noise radius).
    pub fn finish(&self) -> Option<([f32; 2], f32)> {
        if self.samples.is_empty() {
            return None;
        }
        let n = self.samples.len() as f32;
        let cx = self.samples.iter().map(|s| s[0]).sum::<f32>() / n;
        let cy = self.samples.iter().map(|s| s[1]).sum::<f32>() / n;
        let noise = self
            .samples
            .iter()
            .map(|s| ((s[0] - cx).powi(2) + (s[1] - cy).powi(2)).sqrt())
            .fold(0.0, f32::max);
        Some(([cx, cy], noise))
    }
}

/// Records the outer reach while the user spins the stick around its gate.
pub struct SpinRecorder {
    pub center: [f32; 2],
    pub radii: Vec<f32>,
}

impl SpinRecorder {
    pub fn new(center: [f32; 2]) -> Self {
        SpinRecorder {
            center,
            radii: vec![0.0; BINS],
        }
    }

    pub fn add(&mut self, raw: [f32; 2]) {
        let (x, y) = (raw[0] - self.center[0], raw[1] - self.center[1]);
        let r = (x * x + y * y).sqrt();
        if r < 0.25 {
            return; // only rim samples matter
        }
        let bin = ((y.atan2(x).rem_euclid(TAU) / TAU * BINS as f32).round() as usize) % BINS;
        self.radii[bin] = self.radii[bin].max(r);
    }

    pub fn coverage(&self) -> f32 {
        self.radii.iter().filter(|r| **r > 0.0).count() as f32 / BINS as f32
    }

    /// Fill gaps by interpolating between recorded neighbours. Needs ≥50% coverage.
    pub fn finish(&self) -> Option<Calibration> {
        if self.coverage() < 0.5 {
            return None;
        }
        let mut out = self.radii.clone();
        for i in 0..BINS {
            if out[i] > 0.0 {
                continue;
            }
            let prev = (1..BINS).find(|k| self.radii[(i + BINS - k) % BINS] > 0.0)?;
            let next = (1..BINS).find(|k| self.radii[(i + k) % BINS] > 0.0)?;
            let (a, b) = (
                self.radii[(i + BINS - prev) % BINS],
                self.radii[(i + next) % BINS],
            );
            let t = prev as f32 / (prev + next) as f32;
            out[i] = a * (1.0 - t) + b * t;
        }
        Some(Calibration {
            center: self.center,
            radii: out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octagon_cardinal_and_diagonal() {
        let s = StickSettings::default();
        assert!((s.gate_radius(0.0) - 1.0).abs() < 1e-4);
        assert!((s.gate_radius(PI / 2.0) - 1.0).abs() < 1e-4);
        let d = s.gate_radius(PI / 4.0);
        assert!((d - s.diagonal * std::f32::consts::SQRT_2).abs() < 1e-4);
    }

    #[test]
    fn full_tilt_maps_to_range() {
        let s = StickSettings::default();
        assert_eq!(s.process([1.0, 0.0], 1.0), (85, 0));
        assert_eq!(s.process([0.0, -1.0], 1.0), (0, -85));
        let (x, y) = s.process([0.7071, 0.7071], 1.0);
        assert_eq!((x, y), (70, 70));
        // keyboard corner (1,1) clamps to the same diagonal
        assert_eq!(s.process([1.0, 1.0], 1.0), (70, 70));
    }

    #[test]
    fn deadzone_zeroes_small_input() {
        let s = StickSettings::default();
        assert_eq!(s.process([0.05, 0.0], 1.0), (0, 0));
        assert_eq!(s.process([0.0, 0.0], 1.0), (0, 0));
    }

    #[test]
    fn spin_calibration_normalizes_short_stick() {
        let mut rec = SpinRecorder::new([0.0, 0.0]);
        for i in 0..360 {
            let a = (i as f32).to_radians();
            rec.add([0.8 * a.cos(), 0.8 * a.sin()]);
        }
        let cal = rec.finish().unwrap();
        let s = StickSettings {
            calibration: Some(cal),
            ..Default::default()
        };
        assert_eq!(s.process([0.8, 0.0], 1.0), (85, 0));
    }

    #[test]
    fn custom_curve_interpolates() {
        let pts = vec![[0.0, 0.0], [0.5, 0.2], [1.0, 1.0]];
        assert!((eval_curve(&pts, 0.25) - 0.1).abs() < 1e-5);
        assert!((eval_curve(&pts, 0.75) - 0.6).abs() < 1e-5);
        assert_eq!(eval_curve(&pts, 1.0), 1.0);
        // missing endpoint (1,1) is implied
        assert!((eval_curve(&[[0.5, 0.5]], 0.75) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn spin_fills_gaps() {
        let mut rec = SpinRecorder::new([0.0, 0.0]);
        for i in 0..200 {
            let a = (i as f32).to_radians();
            rec.add([a.cos(), a.sin()]);
        }
        let cal = rec.finish().unwrap();
        assert!(cal.radii.iter().all(|r| *r > 0.9));
    }
}

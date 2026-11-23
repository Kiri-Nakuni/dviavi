//! Oscillators, envelopes and filters for instrument notes.

use std::f64::consts::TAU;

use super::mix::{Inst, Wave};

/// Band-limited step correction (PolyBLEP) at phase `t` with increment `dt`.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// One oscillator; phase in [0, 1).
pub struct Osc {
    wave: Wave,
    duty: f64,
    phase: f64,
    /// Leaky integrator state for the triangle (integrated square).
    tri: f64,
    noise: u32,
}

impl Osc {
    pub fn new(wave: Wave, duty: f64, seed: u32) -> Osc {
        Osc { wave, duty, phase: 0.0, tri: -1.0, noise: seed | 1 }
    }

    /// Next sample at `freq` for sample rate `rate`.
    pub fn next(&mut self, freq: f64, rate: f64) -> f64 {
        let dt = (freq / rate).min(0.5);
        let t = self.phase;
        let v = match self.wave {
            Wave::Sine => (TAU * t).sin(),
            Wave::Saw => 2.0 * t - 1.0 - blep(t, dt),
            Wave::Square => {
                let mut v = if t < self.duty { 1.0 } else { -1.0 };
                v += blep(t, dt);
                v -= blep((t - self.duty).rem_euclid(1.0), dt);
                v
            }
            Wave::Triangle => {
                // integrate a band-limited square: no aliasing corners
                let mut sq = if t < 0.5 { 1.0 } else { -1.0 };
                sq += blep(t, dt);
                sq -= blep((t - 0.5).rem_euclid(1.0), dt);
                self.tri = dt * sq * 4.0 + (1.0 - dt * 0.01) * self.tri;
                self.tri
            }
            Wave::Noise => {
                self.noise ^= self.noise << 13;
                self.noise ^= self.noise >> 17;
                self.noise ^= self.noise << 5;
                self.noise as f64 / u32::MAX as f64 * 2.0 - 1.0
            }
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        v
    }
}

/// RBJ biquad low-pass.
pub struct Lowpass {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Lowpass {
    pub fn new(cutoff: f64, rate: f64) -> Lowpass {
        let w = TAU * (cutoff / rate).min(0.49);
        let alpha = w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        Lowpass {
            b0: (1.0 - cos) / 2.0 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Renders one note of `inst` (gate `gate` seconds, then the release) as mono samples.
pub fn note(inst: &Inst, freq: f64, velocity: f64, gate: f64, rate: f64) -> Vec<f32> {
    let a = &inst.adsr;
    let gate_n = (gate * rate) as usize;
    let total = gate_n + (a.release * rate) as usize;
    let mut oscs: Vec<(Osc, f64)> = inst
        .detune
        .iter()
        .enumerate()
        .map(|(i, &c)| (Osc::new(inst.wave, inst.duty, 0x9E37_79B9 ^ (i as u32 * 7919)), 2f64.powf(c / 1200.0)))
        .collect();
    let mut lpf = inst.lpf.map(|f| Lowpass::new(f, rate));
    let norm = inst.gain * velocity / oscs.len().max(1) as f64;
    let mut out = Vec::with_capacity(total);
    let mut level_at_release = 0.0;
    for n in 0..total {
        let t = n as f64 / rate;
        let env = if n < gate_n {
            let e = if t < a.attack {
                t / a.attack.max(1e-6)
            } else if t < a.attack + a.decay {
                1.0 - (1.0 - a.sustain) * (t - a.attack) / a.decay.max(1e-6)
            } else {
                a.sustain
            };
            level_at_release = e;
            e
        } else {
            level_at_release * (1.0 - (n - gate_n) as f64 / (total - gate_n).max(1) as f64)
        };
        let f = match inst.vibrato {
            Some((vr, depth)) => freq * 2f64.powf(depth / 1200.0 * (TAU * vr * t).sin()),
            None => freq,
        };
        let mut v: f64 = oscs.iter_mut().map(|(o, ratio)| o.next(f * *ratio, rate)).sum();
        if let Some(l) = lpf.as_mut() {
            v = l.process(v);
        }
        out.push((v * env * norm) as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peak and mean of one oscillator over `secs` after a short settling time.
    fn measure(wave: Wave, freq: f64, secs: f64) -> (f64, f64, f64) {
        let rate = 48000.0;
        let mut o = Osc::new(wave, 0.5, 1);
        let n = (secs * rate) as usize;
        let v: Vec<f64> = (0..n).map(|_| o.next(freq, rate)).collect();
        let max = v.iter().cloned().fold(f64::MIN, f64::max);
        let min = v.iter().cloned().fold(f64::MAX, f64::min);
        (max, min, v.iter().sum::<f64>() / n as f64)
    }

    #[test]
    fn waves_stay_within_unit_amplitude() {
        for wave in [Wave::Sine, Wave::Square, Wave::Saw, Wave::Triangle] {
            for f in [65.0, 262.0, 1047.0, 3136.0] {
                let (max, min, mean) = measure(wave, f, 2.0);
                assert!(max < 1.15 && min > -1.15, "{wave:?} {f} Hz: {min:.3}..{max:.3}");
                assert!(mean.abs() < 0.05, "{wave:?} {f} Hz: DC {mean:.3}");
            }
        }
    }
}

#[cfg(test)]
mod note_tests {
    use super::*;
    use crate::audio::mix::Inst;

    #[test]
    fn note_level_follows_gain_sustain_and_velocity() {
        let mut inst = Inst::builtin("triangle").unwrap();
        inst.gain = 1.0;
        inst.adsr.sustain = 0.85;
        let s = note(&inst, 130.8, 100.0 / 127.0, 3.0, 48000.0);
        let sustain_peak = s[48000..120000].iter().fold(0f32, |m, v| m.max(v.abs()));
        let expect = 0.85 * 100.0 / 127.0;
        assert!((sustain_peak as f64 - expect).abs() < 0.05, "sustain peak {sustain_peak}, expected {expect:.3}");
    }
}

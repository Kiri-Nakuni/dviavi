//! Formant synthesis of speech and song: vowels and consonants (see
//! `phoneme`) for `\speak` and for voices singing MML.
//!
//! A band-limited glottal source runs through a cascade of resonators whose
//! frequencies glide towards the targets of the current phoneme; bursts and
//! frication are band-passed noise added after it.

use std::f64::consts::{PI, TAU};

use super::mix::{Tone, Voice};
use super::mml::morae;
use super::phoneme::{self, Kind, Spec, Table};
use super::synth::Osc;

/// One phoneme from `start` for `len` samples at pitch `f0` (Hz).
#[derive(Clone, Debug)]
pub struct Segment {
    pub start: usize,
    pub len: usize,
    pub f0: f64,
    pub spec: Spec,
    pub amp: f64,
    /// For an onset consonant: the formants of the vowel it leads into
    /// (aspiration glides to them, a velar burst follows them).
    pub next: Option<[f64; 3]>,
}

/// Klatt resonator.
#[derive(Default)]
struct Resonator {
    y1: f64,
    y2: f64,
}

impl Resonator {
    fn process(&mut self, x: f64, f: f64, bw: f64, rate: f64) -> f64 {
        let t = 1.0 / rate;
        let c = -(-TAU * bw * t).exp();
        let b = 2.0 * (-PI * bw * t).exp() * (TAU * f * t).cos();
        let a = 1.0 - b - c;
        let y = a * x + b * self.y1 + c * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// RBJ band-pass (constant 0 dB peak) for the noise of a consonant.
#[derive(Default)]
struct BandPass {
    b0: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BandPass {
    fn tune(&mut self, centre: f64, width: f64, rate: f64) {
        let w = TAU * (centre / rate).clamp(0.001, 0.49);
        let q = (centre / width.max(1.0)).max(0.1);
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b2 = -alpha / a0;
        self.a1 = -2.0 * w.cos() / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// How a voice makes its voiced sound (sound.mix `engine=`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Engine {
    /// A glottal source through a cascade of formant resonators.
    #[default]
    Formant,
    /// Square waves at the formant frequencies, restarted every pitch period.
    Square,
}

impl Engine {
    pub fn parse(s: &str) -> Result<Engine, String> {
        match s {
            "formant" => Ok(Engine::Formant),
            "square" => Ok(Engine::Square),
            _ => Err(format!("unknown engine `{s}' (formant or square)")),
        }
    }
}

/// Loudness of F1, F2, F3 in the square engine.
const SQUARE_WEIGHTS: [f64; 3] = [1.0, 0.5, 0.25];

/// What the current phoneme asks for at local time `tau` (seconds into it).
struct Targets {
    formants: [f64; 3],
    bw: [f64; 3],
    voice: f64,
    /// band-passed noise (bursts, frication)
    noise: f64,
    /// noise through the vocal tract (aspiration)
    asp: f64,
}

fn targets(s: &Spec, next: Option<[f64; 3]>, tau: f64, len: f64) -> Targets {
    let mut t = Targets { formants: s.f, bw: s.bw, voice: 0.0, noise: 0.0, asp: 0.0 };
    // short ramps so noise does not click
    let ramp = |x: f64, r: f64| (x / r).clamp(0.0, 1.0);
    match s.kind {
        Kind::Vowel => t.voice = s.amp,
        Kind::Silence => {}
        Kind::Nasal | Kind::Flap | Kind::Glide => t.voice = s.amp,
        Kind::Stop => {
            let burst_end = s.closure + s.dur;
            if tau < s.closure {
                t.voice = s.voiced * 0.15; // voice bar of b, d, g
            } else if tau < burst_end || s.aspiration <= 0.0 {
                let x = tau - s.closure;
                t.noise = s.noise * (-x / s.dur.max(0.002)).exp();
                t.voice = s.voiced * 0.3;
            } else {
                // aspiration: breathy noise shaped by the opening vocal tract,
                // with the constriction's band still sounding about 10 dB down
                let x = tau - burst_end;
                let fade = (1.0 - x / s.aspiration.max(0.001)).clamp(0.0, 1.0);
                t.asp = s.aspamp * fade;
                t.noise = s.noise * 0.3 * fade;
                t.voice = s.voiced * 0.5;
                if let Some(f) = next {
                    t.formants = f;
                }
            }
        }
        Kind::Fricative | Kind::Affricate => {
            if tau < s.closure {
                t.voice = s.voiced * 0.15;
            } else {
                let x = tau - s.closure;
                let rise = if s.kind == Kind::Affricate { 0.003 } else { 0.012 };
                t.noise = s.noise * ramp(x, rise) * ramp(len - tau, 0.012);
                t.voice = s.voiced * 0.4;
            }
        }
    }
    t
}

/// Centre of a segment's noise band: a velar burst (pinch) sits just above
/// the next vowel's F2.
fn noise_centre(seg: &Segment, formant: f64) -> f64 {
    let s = &seg.spec;
    match seg.next {
        Some(f) if s.pinch > 0.0 => s.noise_freq * (1.0 - s.pinch) + s.pinch * (f[1] * formant + 300.0),
        _ => s.noise_freq,
    }
}

/// Renders consecutive segments into a mono buffer starting at sample 0
/// (= the first segment's `start`), normalised to the voice's gain.
pub fn render(voice: &Voice, segments: &[Segment], rate: f64) -> Vec<f32> {
    let Some(first) = segments.first() else { return Vec::new() };
    let origin = first.start;
    let end = segments.iter().map(|s| s.start + s.len).max().unwrap_or(origin);
    let tail = (0.08 * rate) as usize;
    let total = end - origin + tail;
    let mut out = vec![0f64; total];

    let mut source = Osc::new(super::mix::Wave::Saw, 0.5, 1);
    let mut noise = Osc::new(super::mix::Wave::Noise, 0.5, 12345);
    let mut tilt = 0.0f64;
    let mut res: [Resonator; 4] = Default::default();
    // two band-pass sections: a 4th-order band for the noise
    let mut band = [BandPass::default(), BandPass::default()];
    let mut band2 = [BandPass::default(), BandPass::default()];
    let mut prev_x = 0.0f64;
    // square engine: glottal phase, formant phases, time since the pulse
    let (mut gphase, mut since) = (0.0f64, 0.0f64);
    let mut fphase = [0.0f64; 3];
    let mut formants = first.spec.f.map(|f| f * voice.formant);
    let mut bw = first.spec.bw;
    let mut f0 = first.f0;
    let (mut amp, mut namp, mut aamp) = (0.0f64, 0.0f64, 0.0f64);
    // smoothing per sample: formants ~18 ms, pitch ~25 ms, voicing ~6 ms, noise ~2 ms
    let k = |ms: f64| 1.0 - (-1.0 / (ms / 1000.0 * rate)).exp();
    let (kf, kp, ka, kn) = (k(18.0), k(25.0), k(6.0), k(2.0));
    let (vib_rate, vib_depth) = voice.vibrato;
    let mut seg_i = 0;
    let mut tuned = usize::MAX;
    for (n, sample) in out.iter_mut().enumerate() {
        let abs = origin + n;
        while seg_i + 1 < segments.len() && abs >= segments[seg_i + 1].start {
            seg_i += 1;
        }
        let seg = &segments[seg_i];
        let inside = abs >= seg.start && abs < seg.start + seg.len;
        let tau = (abs as f64 - seg.start as f64) / rate;
        let tg = targets(&seg.spec, seg.next, tau, seg.len as f64 / rate);
        if tuned != seg_i && seg.spec.noise > 0.0 {
            let centre = noise_centre(seg, voice.formant);
            for b in band.iter_mut() {
                b.tune(centre, seg.spec.noise_bw, rate);
            }
            if seg.spec.noise2 > 0.0 {
                for b in band2.iter_mut() {
                    b.tune(seg.spec.noise2_freq, seg.spec.noise2_bw, rate);
                }
            }
            tuned = seg_i;
        }
        if seg.spec.kind != Kind::Silence {
            for j in 0..3 {
                formants[j] += (tg.formants[j] * voice.formant - formants[j]) * kf;
                bw[j] += (tg.bw[j] - bw[j]) * kf;
            }
            f0 += (seg.f0 - f0) * kp;
        }
        let (va, na, aa) = if inside { (tg.voice * seg.amp, tg.noise * seg.amp, tg.asp * seg.amp) } else { (0.0, 0.0, 0.0) };
        amp += (va - amp) * ka;
        namp += (na - namp) * kn;
        aamp += (aa - aamp) * kn;
        let t = n as f64 / rate;
        let pitch = f0 * 2f64.powf(vib_depth / 1200.0 * (TAU * vib_rate * t).sin());
        let white = noise.next(0.0, rate);
        let radiated = match voice.engine {
            Engine::Formant => {
                // glottal source: saw tilted by a one-pole low-pass, plus breath noise;
                // scaled before the tract, so aspiration noise is shaped by it too
                let saw = source.next(pitch, rate);
                tilt += (saw - tilt) * (1.0 - (-TAU * 700.0 / rate).exp());
                let mut x = (tilt + white * voice.breath) * amp + white * aamp * 0.1;
                for j in 0..3 {
                    x = res[j].process(x, formants[j], bw[j], rate);
                }
                x = res[3].process(x, 3500.0 * voice.formant, 200.0, rate);
                // lip radiation: a differentiator (+6 dB/octave)
                let radiated = x - prev_x;
                prev_x = x;
                radiated
            }
            Engine::Square => {
                // each formant is a square wave restarted at every glottal pulse
                // and dying away with its bandwidth (a talking chip's voice)
                gphase += pitch / rate;
                since += 1.0 / rate;
                if gphase >= 1.0 {
                    gphase -= 1.0;
                    since = 0.0;
                    fphase = [0.0; 3];
                }
                let mut y = 0.0;
                for j in 0..3 {
                    let sq = if fphase[j] < 0.5 { 1.0 } else { -1.0 };
                    y += SQUARE_WEIGHTS[j] * (-PI * bw[j] * since).exp() * sq;
                    fphase[j] = (fphase[j] + formants[j] / rate).fract();
                }
                (y * amp + white * voice.breath * amp) * 0.06 + white * aamp * 0.02
            }
        };
        // the radiated voice is about 0.1 for a full vowel: noise is scaled to match
        let first = band[0].process(white);
        let mut hiss = band[1].process(first);
        if seg.spec.noise2 > 0.0 {
            let first2 = band2[0].process(white);
            hiss += band2[1].process(first2) * seg.spec.noise2;
        }
        *sample = radiated + hiss * namp * 0.35;
    }
    let peak = out.iter().fold(0f64, |m, v| m.max(v.abs())).max(1e-9);
    let g = voice.gain / peak;
    out.iter().map(|v| (v * g) as f32).collect()
}

/// A unit's phonemes looked up.
struct UnitSpecs {
    onset: Vec<Spec>,
    nucleus: Vec<Spec>,
    coda: Vec<Spec>,
}

impl UnitSpecs {
    fn new(u: &phoneme::Unit, table: &Table) -> UnitSpecs {
        let get = |v: &Vec<String>| v.iter().map(|p| table.get(p)).collect();
        UnitSpecs { onset: get(&u.onset), nucleus: get(&u.nucleus), coda: get(&u.coda) }
    }

    fn onset_length(&self) -> f64 {
        self.onset.iter().map(Spec::length).sum()
    }
}

/// Lays out one unit (samples): its onsets end at `nucleus_start` (each
/// `squeeze`d), the nucleus runs to the codas, which end at `end` (at most
/// 40% of the time). A diphthong gives its first vowel 60%.
#[allow(clippy::too_many_arguments)]
fn unit_segments(out: &mut Vec<Segment>, u: &UnitSpecs, nucleus_start: f64, end: f64, squeeze: f64, f0: f64, amp: f64, rate: f64) {
    let Some(first_vowel) = u.nucleus.first() else { return };
    let mut t = nucleus_start - u.onset_length() * squeeze * rate;
    for s in &u.onset {
        let len = s.length() * squeeze * rate;
        out.push(Segment { start: t.max(0.0) as usize, len: len as usize, f0, spec: *s, amp, next: Some(first_vowel.f) });
        t += len;
    }
    let span = (end - nucleus_start).max(0.0);
    let coda: f64 = u.coda.iter().map(Spec::length).sum::<f64>() * rate;
    let coda_squeeze = if coda > 0.4 * span && coda > 0.0 { 0.4 * span / coda } else { 1.0 };
    let coda_start = end - coda * coda_squeeze;
    let n = u.nucleus.len();
    let mut t = nucleus_start;
    for (i, s) in u.nucleus.iter().enumerate() {
        let share = match (n, i) {
            (1, _) => 1.0,
            (2, 0) => 0.6,
            (2, _) => 0.4,
            _ => 1.0 / n as f64,
        };
        let len = (coda_start - nucleus_start) * share;
        out.push(Segment { start: t.max(0.0) as usize, len: len.max(0.0) as usize, f0, spec: *s, amp, next: None });
        t += len;
    }
    let mut t = coda_start;
    for s in &u.coda {
        let len = s.length() * coda_squeeze * rate;
        out.push(Segment { start: t.max(0.0) as usize, len: len as usize, f0, spec: *s, amp, next: None });
        t += len;
    }
}

/// The vowel the last unit ends with (what `-` repeats).
fn last_nucleus(units: &[phoneme::Unit], prev: &str) -> String {
    units.last().and_then(|u| u.nucleus.last()).cloned().unwrap_or_else(|| prev.to_string())
}

/// Segments for `\speak`. Japanese: romaji with capitals starting morae;
/// `'` marks the accent nucleus, `/` a phrase break, `_` a pause, `-` a long
/// vowel. English: ARPAbet syllables (`D.EY`) separated by spaces or `|`,
/// `/` and `_` as in Japanese. Each mora or syllable takes `mora_len`;
/// consonants take its start (at most 60% of it).
#[allow(clippy::too_many_arguments)]
pub fn speak_segments(
    voice: &Voice,
    table: &Table,
    tone: &Tone,
    text: &str,
    lang: phoneme::Lang,
    speed: f64,
    mora_len: f64,
    start: usize,
    rate: f64,
) -> Result<Vec<Segment>, String> {
    let mut list: Vec<phoneme::Unit> = Vec::new();
    let mut phrases: Vec<usize> = Vec::new();
    let mut accents: Vec<bool> = Vec::new();
    let mut phrase = 0;
    let mut prev = "a".to_string();
    let tokens: Vec<String> = match lang {
        phoneme::Lang::Ja => morae(text),
        phoneme::Lang::En => text
            .split(|c: char| c.is_whitespace() || c == '|')
            .flat_map(|w| {
                // `/` and `_` may touch a syllable
                let mut parts = Vec::new();
                let mut cur = String::new();
                for c in w.chars() {
                    if c == '/' || c == '_' || c == '\'' {
                        if !cur.is_empty() {
                            parts.push(std::mem::take(&mut cur));
                        }
                        parts.push(c.to_string());
                    } else {
                        cur.push(c);
                    }
                }
                if !cur.is_empty() {
                    parts.push(cur);
                }
                parts
            })
            .collect(),
    };
    for m in tokens {
        match m.as_str() {
            "'" => {
                if let Some(last) = accents.last_mut() {
                    *last = true;
                }
            }
            "/" | "_" => {
                if m == "/" {
                    phrase += 1;
                }
                list.push(phoneme::Unit { nucleus: vec!["_".into()], ..Default::default() });
                phrases.push(phrase);
                accents.push(false);
            }
            _ => {
                if lang == phoneme::Lang::Ja && !(m == "-" || m.chars().all(|c| c.is_ascii_alphabetic())) {
                    continue;
                }
                for u in phoneme::units(&m, lang, &prev)? {
                    prev = last_nucleus(std::slice::from_ref(&u), &prev);
                    list.push(u);
                    phrases.push(phrase);
                    accents.push(false);
                }
            }
        }
    }
    phoneme::assimilate_units(&mut list);
    let base = voice.f0 * tone.f0x * speed;
    let total = list.len().max(1) as f64;
    let mut out = Vec::new();
    let mut pos_in_phrase = 0;
    let mut current_phrase = usize::MAX;
    let mut after_accent = false;
    let mut t = start as f64;
    for (i, u) in list.iter().enumerate() {
        if phrases[i] != current_phrase {
            current_phrase = phrases[i];
            pos_in_phrase = 0;
            after_accent = false;
        }
        // Tokyo-style contour: first mora low, high until the accent, then low
        let high = if after_accent { false } else { pos_in_phrase > 0 || accents[i] };
        let level = if high { 0.5 } else { -0.5 };
        let declination = -100.0 * i as f64 / total; // cents over the utterance
        let f0 = base * 2f64.powf((level * tone.range + declination) / 1200.0);
        let specs = UnitSpecs::new(u, table);
        let cons = specs.onset_length();
        let squeeze = if cons > 0.6 * mora_len { 0.6 * mora_len / cons } else { 1.0 };
        unit_segments(&mut out, &specs, t + cons * squeeze * rate, t + mora_len * rate, squeeze, f0, 1.0, rate);
        t += mora_len * rate;
        if accents[i] {
            after_accent = true;
        }
        if !matches!(u.nucleus.as_slice(), [p] if p == "_" || p == "Q") {
            pos_in_phrase += 1;
        }
    }
    Ok(out)
}

/// A note to sing: start and length (seconds), pitch, loudness, lyric token.
pub struct SungNote<'a> {
    pub start: f64,
    pub gate: f64,
    pub f0: f64,
    pub amp: f64,
    pub lyric: Option<&'a str>,
}

/// Segments for singing. A note's lyric is one or more units (Japanese
/// morae, an English syllable); several units share the note equally. The
/// first unit's nucleus starts on the note and its consonants just before
/// (like UTAU's preutterance), shortening the previous note; such consonants
/// never take more than half of the time since the previous note started.
/// Times are seconds from the clip start; `offset` (samples) is added so
/// early consonants stay positive.
pub fn sing_segments(table: &Table, notes: &[SungNote], lang: phoneme::Lang, offset: usize, rate: f64) -> Result<Vec<Segment>, String> {
    let mut per_note: Vec<Vec<phoneme::Unit>> = Vec::new();
    let mut prev = "a".to_string();
    for n in notes {
        let units = match n.lyric {
            Some(l) => phoneme::units(l, lang, &prev)?,
            None => Vec::new(),
        };
        // no lyric: the last vowel goes on
        let units = if units.is_empty() { vec![phoneme::Unit { nucleus: vec![prev.clone()], ..Default::default() }] } else { units };
        prev = last_nucleus(&units, &prev);
        per_note.push(units);
    }
    // ん assimilates across notes too
    let mut flat: Vec<phoneme::Unit> = per_note.iter().flatten().cloned().collect();
    phoneme::assimilate_units(&mut flat);
    let mut it = flat.into_iter();
    for units in per_note.iter_mut() {
        for u in units.iter_mut() {
            *u = it.next().expect("same count");
        }
    }
    let mut out: Vec<Segment> = Vec::new();
    let mut prev_start: Option<f64> = None;
    let at = |s: f64| offset as f64 + s * rate;
    for (n, units) in notes.iter().zip(&per_note) {
        let k = units.len() as f64;
        for (i, u) in units.iter().enumerate() {
            let specs = UnitSpecs::new(u, table);
            let slot = n.gate / k;
            let begin = n.start + slot * i as f64;
            let cons = specs.onset_length();
            let room = if i > 0 {
                slot * 0.5
            } else {
                match prev_start {
                    Some(p) => ((n.start - p) * 0.5).max(0.0),
                    None => cons,
                }
            };
            let squeeze = if cons > room && cons > 0.0 { room / cons } else { 1.0 };
            let first = at(begin - cons * squeeze).max(0.0) as usize;
            // what came before ends where this unit's first phoneme begins
            if let Some(last) = out.last_mut() {
                if last.start + last.len > first {
                    last.len = first.saturating_sub(last.start);
                }
            }
            unit_segments(&mut out, &specs, at(begin), at(begin + slot), squeeze, n.f0, n.amp, rate);
        }
        prev_start = Some(n.start);
    }
    Ok(out)
}

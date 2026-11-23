//! Phonemes of the formant voice: what each one sounds like (tunable from
//! sound.mix) and how romanised morae break into them.
//!
//! A consonant is drawn as `closure` (silence, or a voice bar if voiced)
//! followed by `dur` of its own sound: a burst (stop), frication (fricative,
//! affricate) or a murmur/glide with its own formants (nasal, flap, glide).
//! Its formants are also the locus the next vowel's formants glide from.

use std::collections::HashMap;

use super::units;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Vowel,
    Nasal,
    Stop,
    Fricative,
    Affricate,
    Flap,
    Glide,
    Silence,
}

impl Kind {
    fn parse(s: &str) -> Result<Kind, String> {
        Ok(match s {
            "vowel" => Kind::Vowel,
            "nasal" => Kind::Nasal,
            "stop" => Kind::Stop,
            "fricative" => Kind::Fricative,
            "affricate" => Kind::Affricate,
            "flap" => Kind::Flap,
            "glide" => Kind::Glide,
            "silence" => Kind::Silence,
            _ => return Err(format!("unknown phoneme kind `{s}'")),
        })
    }
}

/// How one phoneme sounds (frequencies for a voice at `formant=1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub kind: Kind,
    /// F1-F3 (Hz): the vowel's targets, or a consonant's own resonances/locus.
    pub f: [f64; 3],
    pub bw: [f64; 3],
    /// Amplitude of the voiced sound (vowels, nasals, flaps, glides).
    pub amp: f64,
    /// Voicing during a consonant's closure and noise (0 for k, s, t ...).
    pub voiced: f64,
    /// Noise (burst, frication): amplitude, band centre and width (Hz).
    pub noise: f64,
    pub noise_freq: f64,
    pub noise_bw: f64,
    /// A second noise band (level relative to `noise`, centre, width), for
    /// the higher hiss of a release (k: about 4.7 kHz, 20 dB down).
    pub noise2: f64,
    pub noise2_freq: f64,
    pub noise2_bw: f64,
    /// Seconds of silence/closure before the consonant's sound.
    pub closure: f64,
    /// Seconds of the consonant's sound (0 for vowels: they fill their time).
    pub dur: f64,
    /// Stops: seconds of aspiration after the burst (noise through the
    /// vocal tract, whose formants already move to the next vowel) and its level.
    pub aspiration: f64,
    pub aspamp: f64,
    /// 0..1: how far the burst's band follows the next vowel (velar pinch:
    /// k, g burst just above the vowel's F2).
    pub pinch: f64,
}

impl Spec {
    const fn vowel(f: [f64; 3], amp: f64) -> Spec {
        Spec {
            kind: Kind::Vowel,
            f,
            bw: [80.0, 100.0, 140.0],
            amp,
            voiced: 1.0,
            noise: 0.0,
            noise_freq: 0.0,
            noise_bw: 0.0,
            noise2: 0.0,
            noise2_freq: 0.0,
            noise2_bw: 0.0,
            closure: 0.0,
            dur: 0.0,
            aspiration: 0.0,
            aspamp: 0.0,
            pinch: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    const fn cons(kind: Kind, f: [f64; 3], amp: f64, voiced: f64, noise: f64, nf: f64, nbw: f64, closure: f64, dur: f64) -> Spec {
        Spec {
            kind,
            f,
            bw: [100.0, 150.0, 200.0],
            amp,
            voiced,
            noise,
            noise_freq: nf,
            noise_bw: nbw,
            noise2: 0.0,
            noise2_freq: 0.0,
            noise2_bw: 0.0,
            closure,
            dur,
            aspiration: 0.0,
            aspamp: 0.0,
            pinch: 0.0,
        }
    }

    /// A stop with aspiration and velar pinch.
    const fn aspirated(self, aspiration: f64, aspamp: f64, pinch: f64) -> Spec {
        Spec { aspiration, aspamp, pinch, ..self }
    }

    /// A moraic nasal variant: nasal murmur with damped upper formants.
    const fn moraic(f: [f64; 3]) -> Spec {
        Spec { kind: Kind::Nasal, bw: [110.0, 320.0, 450.0], ..Spec::vowel(f, 1.2) }
    }

    /// Seconds the phoneme takes as an onset before its mora's nucleus.
    pub fn length(&self) -> f64 {
        self.closure + self.dur + self.aspiration
    }

    /// Sets one parameter from sound.mix (`phoneme k { dur=40ms noise=-6dB }`).
    pub fn set(&mut self, key: &str, v: &str) -> Result<(), String> {
        let three = |v: &str| -> Result<[f64; 3], String> {
            let p: Vec<f64> = v.split(',').map(units::freq).collect::<Result<_, _>>()?;
            match p.as_slice() {
                [a, b, c] => Ok([*a, *b, *c]),
                _ => Err(format!("three values expected, got `{v}'")),
            }
        };
        let secs = |v: &str| -> Result<f64, String> {
            match v.strip_suffix("ms") {
                Some(ms) => units::number(ms).map(|x| x / 1000.0),
                None => units::number(v.trim_end_matches('s')),
            }
        };
        match key {
            "kind" => self.kind = Kind::parse(v)?,
            "f" => self.f = three(v)?,
            "bw" => self.bw = three(v)?,
            "amp" => self.amp = units::gain(v)?,
            "voiced" => self.voiced = units::number(v)?,
            "noise" => self.noise = units::gain(v)?,
            "noisef" => self.noise_freq = units::freq(v)?,
            "noisebw" => self.noise_bw = units::freq(v)?,
            "noise2" => self.noise2 = units::gain(v)?,
            "noise2f" => self.noise2_freq = units::freq(v)?,
            "noise2bw" => self.noise2_bw = units::freq(v)?,
            "closure" => self.closure = secs(v)?,
            "dur" => self.dur = secs(v)?,
            "aspiration" => self.aspiration = secs(v)?,
            "aspamp" => self.aspamp = units::gain(v)?,
            "pinch" => self.pinch = units::number(v)?.clamp(0.0, 1.0),
            _ => return Err(format!("unknown phoneme parameter `{key}'")),
        }
        Ok(())
    }
}

/// Built-in phonemes (Japanese).
pub fn builtin(name: &str) -> Option<Spec> {
    use Kind::*;
    Some(match name {
        "a" => Spec::vowel([800.0, 1200.0, 2500.0], 1.0),
        "i" => Spec::vowel([300.0, 2300.0, 3000.0], 0.8),
        "u" => Spec::vowel([350.0, 1400.0, 2400.0], 0.8),
        "e" => Spec::vowel([500.0, 1900.0, 2500.0], 0.9),
        "o" => Spec::vowel([500.0, 850.0, 2500.0], 0.95),
        // ん by what follows: [ɴ]/nasal vowel (before vowels, y, w, s, h, f,
        // at the end), [m] before p b m, [n] before t d n ts z r, [ɲ] before
        // ch j, [ŋ] before k g; N1 rises from m to ŋ
        "N" => Spec::moraic([320.0, 1250.0, 2500.0]),
        "N:m" => Spec::moraic([230.0, 1050.0, 2300.0]),
        "N:n" => Spec::moraic([250.0, 1650.0, 2600.0]),
        "N:ny" => Spec::moraic([270.0, 2100.0, 2800.0]),
        "N:ng" => Spec::moraic([300.0, 2000.0, 2700.0]),
        //                       f (locus)                 amp   voiced noise  noisef  noisebw closure dur
        // k: narrow burst just above the next vowel's F2 (about 1.5 kHz before
        // a, -6 dB width under 200 Hz), 20 ms strong, then aspiration; a
        // second hiss at about 4.7 kHz, 20 dB down
        "k" => Spec {
            noise2: 0.07,
            noise2_freq: 4700.0,
            noise2_bw: 500.0,
            ..Spec::cons(Stop, [300.0, 1800.0, 2500.0], 0.0, 0.0, 0.55, 2500.0, 250.0, 0.035, 0.020).aspirated(0.050, 0.2, 1.0)
        },
        "g" => Spec::cons(Stop, [300.0, 1800.0, 2500.0], 0.0, 0.6, 0.25, 2000.0, 2000.0, 0.030, 0.010).aspirated(0.0, 0.0, 1.0),
        "t" => Spec::cons(Stop, [400.0, 1800.0, 2600.0], 0.0, 0.0, 0.5, 4000.0, 3000.0, 0.035, 0.012).aspirated(0.020, 0.3, 0.0),
        "d" => Spec::cons(Stop, [400.0, 1800.0, 2600.0], 0.0, 0.01, 0.25, 3500.0, 3000.0, 0.030, 0.008),
        "p" => Spec::cons(Stop, [300.0, 900.0, 2300.0], 0.0, 0.0, 0.4, 1200.0, 2000.0, 0.040, 0.010).aspirated(0.020, 0.25, 0.0),
        "b" => Spec::cons(Stop, [300.0, 900.0, 2300.0], 0.0, 0.6, 0.2, 1200.0, 2000.0, 0.035, 0.006),
        "s" => Spec::cons(Fricative, [400.0, 1700.0, 2600.0], 0.0, 0.0, 0.66, 5150.0, 2050.0, 0.017, 0.105),
        "z" => Spec { bw: [40.0, 40.0, 40.0], ..Spec::cons(Affricate, [400.0, 1700.0, 2600.0], 0.0, 0.0, 0.6, 3900.0, 1350.0, 0.002, 0.013) },
        "sh" => Spec::cons(Fricative, [350.0, 2000.0, 2700.0], 0.0, 0.0, 0.6, 3500.0, 2000.0, 0.0, 0.100),
        "j" => Spec::cons(Affricate, [350.0, 2000.0, 2700.0], 0.0, 0.0, 0.11, 3200.0, 2000.0, 0.0, 0.023),
        "ch" => Spec::cons(Affricate, [350.0, 2000.0, 2700.0], 0.0, 0.0, 0.79, 5350.0, 3600.0, 0.072, 0.021),
        "ts" => Spec::cons(Affricate, [400.0, 1700.0, 2600.0], 0.0, 0.0, 0.41, 2600.0, 3000.0, 0.055, 0.026),
        "h" => Spec::cons(Fricative, [500.0, 1500.0, 2500.0], 0.0, 0.0, 0.6, 1750.0, 600.0, 0.0, 0.060),
        "f" => Spec::cons(Fricative, [350.0, 1000.0, 2300.0], 0.0, 0.0, 0.25, 1200.0, 2000.0, 0.0, 0.070),
        "m" => Spec::cons(Nasal, [250.0, 1100.0, 2300.0], 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.060),
        "n" => Spec::cons(Nasal, [250.0, 1700.0, 2600.0], 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.050),
        "r" => Spec::cons(Flap, [350.0, 1300.0, 1800.0], 0.3, 1.0, 0.0, 0.0, 0.0, 0.0, 0.025),
        "y" => Spec::cons(Glide, [280.0, 2300.0, 3000.0], 0.7, 1.0, 0.0, 0.0, 0.0, 0.0, 0.040),
        "w" => Spec::cons(Glide, [300.0, 700.0, 2300.0], 0.7, 1.0, 0.0, 0.0, 0.0, 0.0, 0.045),
        // っ (geminate) and pauses
        "Q" | "_" => Spec { kind: Silence, ..Spec::vowel([500.0, 1500.0, 2500.0], 0.0) },
        // English (ARPAbet; vowels after Peterson & Barney's adult averages)
        "IY" => Spec::vowel([270.0, 2290.0, 3010.0], 0.8),
        "IH" => Spec::vowel([390.0, 1990.0, 2550.0], 0.85),
        "EH" => Spec::vowel([530.0, 1840.0, 2480.0], 0.9),
        "AE" => Spec::vowel([660.0, 1720.0, 2410.0], 1.0),
        "AH" => Spec::vowel([520.0, 1190.0, 2390.0], 0.95),
        "AA" => Spec::vowel([730.0, 1090.0, 2440.0], 1.0),
        "AO" => Spec::vowel([570.0, 840.0, 2410.0], 0.95),
        "UH" => Spec::vowel([440.0, 1020.0, 2240.0], 0.85),
        "UW" => Spec::vowel([300.0, 870.0, 2240.0], 0.8),
        "ER" => Spec::vowel([490.0, 1350.0, 1690.0], 0.9),
        "V" => Spec::cons(Fricative, [300.0, 1100.0, 2300.0], 0.0, 0.5, 0.15, 1500.0, 2500.0, 0.0, 0.060),
        "TH" => Spec::cons(Fricative, [400.0, 1400.0, 2500.0], 0.0, 0.0, 0.2, 5000.0, 5000.0, 0.0, 0.080),
        "DH" => Spec::cons(Fricative, [400.0, 1400.0, 2500.0], 0.0, 0.5, 0.1, 4500.0, 5000.0, 0.0, 0.045),
        "Z" => Spec::cons(Fricative, [400.0, 1700.0, 2600.0], 0.0, 0.5, 0.4, 5000.0, 2500.0, 0.0, 0.080),
        "ZH" => Spec::cons(Fricative, [350.0, 2000.0, 2700.0], 0.0, 0.5, 0.35, 3200.0, 2000.0, 0.0, 0.070),
        "NG" => Spec::cons(Nasal, [300.0, 2000.0, 2700.0], 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.070),
        "L" => Spec::cons(Glide, [360.0, 1100.0, 2600.0], 0.8, 1.0, 0.0, 0.0, 0.0, 0.0, 0.060),
        "R" => Spec::cons(Glide, [420.0, 1300.0, 1600.0], 0.8, 1.0, 0.0, 0.0, 0.0, 0.0, 0.060),
        _ => return None,
    })
}

/// Phoneme parameters overridden in sound.mix, as `key=value` lists applied
/// in order (built-in, then top level, then the voice).
pub type Overrides = HashMap<String, Vec<(String, String)>>;

/// Applies `kv` to the built-in `name` (or a new vowel), checking the values.
pub fn spec_with(name: &str, layers: &[&Overrides]) -> Result<Spec, String> {
    let mut s = builtin(name).unwrap_or(Spec::vowel([500.0, 1500.0, 2500.0], 1.0));
    for layer in layers {
        if let Some(kv) = layer.get(name) {
            for (k, v) in kv {
                s.set(k, v)?;
            }
        }
    }
    Ok(s)
}

/// The language of lyrics and speech text.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Lang {
    /// Romanised Japanese: capitals start morae.
    #[default]
    Ja,
    /// ARPAbet syllables: phonemes joined by `.` (`D.EY`, `S.W.IY.T`).
    En,
}

impl Lang {
    pub fn parse(s: &str) -> Result<Lang, String> {
        match s {
            "ja" | "" => Ok(Lang::Ja),
            "en" => Ok(Lang::En),
            _ => Err(format!("unknown language `{s}' (ja or en)")),
        }
    }
}

/// One syllable or mora: consonants before the nucleus, the nucleus (a vowel,
/// a diphthong's two vowels, ん, a pause) and consonants after it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unit {
    pub onset: Vec<String>,
    pub nucleus: Vec<String>,
    pub coda: Vec<String>,
}

fn is_nucleus(name: &str) -> bool {
    matches!(name, "a" | "i" | "u" | "e" | "o" | "Q" | "_")
        || name.starts_with('N') && name != "NG"
        || builtin(name).is_some_and(|s| s.kind == Kind::Vowel)
}

/// ARPAbet to phoneme names: diphthongs become their two targets, the
/// consonants Japanese shares use its phonemes.
fn arpabet(p: &str) -> Vec<&'static str> {
    let p = p.trim_end_matches(|c: char| c.is_ascii_digit()); // stress marks
    let one = |n: &'static str| vec![n];
    match p.to_ascii_uppercase().as_str() {
        "EY" => vec!["EH", "IY"],
        "AY" => vec!["AA", "IH"],
        "OW" => vec!["AO", "UH"],
        "AW" => vec!["AA", "UH"],
        "OY" => vec!["AO", "IY"],
        "IY" => one("IY"),
        "IH" => one("IH"),
        "EH" => one("EH"),
        "AE" => one("AE"),
        "AH" | "AX" => one("AH"),
        "AA" => one("AA"),
        "AO" => one("AO"),
        "UH" => one("UH"),
        "UW" => one("UW"),
        "ER" | "AXR" => one("ER"),
        "P" => one("p"),
        "B" => one("b"),
        "T" => one("t"),
        "D" => one("d"),
        "K" => one("k"),
        "G" => one("g"),
        "CH" => one("ch"),
        "JH" => one("j"),
        "F" => one("f"),
        "V" => one("V"),
        "TH" => one("TH"),
        "DH" => one("DH"),
        "S" => one("s"),
        "Z" => one("Z"),
        "SH" => one("sh"),
        "ZH" => one("ZH"),
        "HH" | "H" => one("h"),
        "M" => one("m"),
        "N" => one("n"),
        "NG" => one("NG"),
        "L" => one("L"),
        "R" => one("R"),
        "W" => one("w"),
        "Y" => one("y"),
        _ => Vec::new(),
    }
}

/// The units of one lyric token (a note's lyric, a word of speech).
/// Japanese: a mora per capital (`DeI` → de, i); English: one syllable.
/// `prev` is the vowel `-` repeats.
pub fn units(token: &str, lang: Lang, prev: &str) -> Result<Vec<Unit>, String> {
    match lang {
        Lang::Ja => {
            let mut out = Vec::new();
            let mut prev = prev.to_string();
            for m in super::mml::morae(token) {
                let mut names = mora(&m, &prev);
                let Some(nucleus) = names.pop() else { continue };
                prev = nucleus.clone();
                out.push(Unit { onset: names, nucleus: vec![nucleus], coda: Vec::new() });
            }
            Ok(out)
        }
        Lang::En => {
            // `-` holds the vowel before, `_` rests (as in Japanese)
            match token {
                "-" => return Ok(vec![Unit { nucleus: vec![prev.to_string()], ..Default::default() }]),
                "_" => return Ok(vec![Unit { nucleus: vec!["_".into()], ..Default::default() }]),
                _ => {}
            }
            let mut names = Vec::new();
            for p in token.split('.').filter(|p| !p.is_empty()) {
                let n = arpabet(p);
                if n.is_empty() {
                    return Err(format!("unknown ARPAbet phoneme `{p}' in `{token}'"));
                }
                names.extend(n.into_iter().map(str::to_string));
            }
            if names.is_empty() {
                return Ok(Vec::new());
            }
            let first = names.iter().position(|n| is_nucleus(n));
            let last = names.iter().rposition(|n| is_nucleus(n));
            Ok(vec![match (first, last) {
                (Some(a), Some(b)) => Unit { onset: names[..a].to_vec(), nucleus: names[a..=b].to_vec(), coda: names[b + 1..].to_vec() },
                // a syllabic consonant (`L` of bicycle): the last one is the nucleus
                _ => {
                    let n = names.pop().unwrap();
                    Unit { onset: names, nucleus: vec![n], coda: Vec::new() }
                }
            }])
        }
    }
}

/// Gives each ん (a unit whose nucleus is `N`) the variant for what follows.
pub fn assimilate_units(units: &mut [Unit]) {
    for i in 0..units.len() {
        if units[i].nucleus == ["N"] && units[i].coda.is_empty() {
            let next = units.get(i + 1).and_then(|u| u.onset.first().or(u.nucleus.first())).map(String::as_str);
            units[i].nucleus[0] = moraic_nasal(next).to_string();
        }
    }
}

/// The variant of ん before `next` (the first phoneme of the next mora).
pub fn moraic_nasal(next: Option<&str>) -> &'static str {
    match next {
        Some("p" | "b" | "m" | "N:m") => "N:m",
        Some("t" | "d" | "n" | "ts" | "z" | "r") => "N:n",
        Some("ch" | "j") => "N:ny",
        Some("k" | "g") => "N:ng",
        _ => "N",
    }
}

/// The phonemes of one voice: built-ins overridden by `global` then `voice`.
pub struct Table<'a> {
    layers: [&'a Overrides; 2],
}

impl<'a> Table<'a> {
    pub fn new(global: &'a Overrides, voice: &'a Overrides) -> Table<'a> {
        Table { layers: [global, voice] }
    }

    pub fn get(&self, name: &str) -> Spec {
        spec_with(name, &self.layers).unwrap_or_else(|_| builtin("a").unwrap())
    }
}

/// The phoneme names of one romanised mora (`Ka` → k a, `Shi` → sh i,
/// `Kya` → k y a, `Tsu` → ts u, `Nn` → N, `Q` → Q, `-` → the vowel before).
/// Kunrei spellings are read as sounds: si → sh i, ti → ch i, tu → ts u,
/// hu → f u, zi/di → j i, du → z u.  Thi Dhi Twu Dwu (ティ ディ トゥ ドゥ)
/// keep t and d.
pub fn mora(m: &str, prev_vowel: &str) -> Vec<String> {
    let l = m.to_ascii_lowercase();
    match l.as_str() {
        "" => return Vec::new(),
        "n" | "nn" | "n'" => return vec!["N".into()],
        "q" | "xtu" | "ltu" => return vec!["Q".into()],
        "-" | "ー" => return vec![prev_vowel.to_string()],
        "_" => return vec!["_".into()],
        _ => {}
    }
    let Some(vpos) = l.rfind(|c: char| "aiueo".contains(c)) else {
        return vec!["Q".into()];
    };
    let vowel = l[vpos..vpos + 1].to_string();
    let onset = &l[..vpos];
    let mut out: Vec<String> = Vec::new();
    // palatalised: kya → k y a (sha, cha, ja carry it already)
    let (onset, glide) = match onset.strip_suffix('y') {
        // sya, zya, tya are sha, ja, cha (the consonant is palatal already)
        Some(o) if matches!(o, "s" | "z" | "j" | "t" | "c" | "d") => (onset, false),
        Some(o) if !o.is_empty() => (o, true),
        _ => (onset, false),
    };
    let c = match (onset, vowel.as_str()) {
        ("s", "i") | ("sh", _) | ("sy", _) => "sh",
        ("z", "i") | ("j", _) | ("zy", _) | ("jy", _) | ("dy", _) | ("d", "i") => "j",
        ("t", "i") | ("ch", _) | ("c", _) | ("ty", _) | ("cy", _) => "ch",
        ("t", "u") | ("ts", _) => "ts",
        ("h", "u") | ("f", _) => "f",
        ("d", "u") => "z",
        // ティ ディ トゥ ドゥ (and the like): t and d before any vowel
        ("th", _) | ("tw", _) => "t",
        ("dh", _) | ("dw", _) => "d",
        ("x", _) | ("l", _) => "",
        (o, _) => o,
    };
    if !c.is_empty() {
        out.push(c.to_string());
    }
    let palatal_onset = matches!(c, "sh" | "j" | "ch");
    if glide && !palatal_onset {
        out.push("y".into());
    }
    out.push(vowel);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morae_split() {
        let m = |s: &str| mora(s, "a").join(" ");
        assert_eq!(m("Ka"), "k a");
        assert_eq!(m("Si"), "sh i");
        assert_eq!(m("Shi"), "sh i");
        assert_eq!(m("Kya"), "k y a");
        assert_eq!(m("Sya"), "sh a");
        assert_eq!(m("Tu"), "ts u");
        assert_eq!(m("Ti"), "ch i");
        assert_eq!(m("Hu"), "f u");
        assert_eq!(m("Nn"), "N");
        assert_eq!(m("A"), "a");
        assert_eq!(m("-"), "a");
        assert_eq!(m("Q"), "Q");
        assert_eq!(m("Wa"), "w a");
        assert_eq!(m("Thi"), "t i");
        assert_eq!(m("Dhi"), "d i");
        assert_eq!(m("Twu"), "t u");
        assert_eq!(m("Dwu"), "d u");
    }

    #[test]
    fn english_syllables() {
        let u = |s: &str| units(s, Lang::En, "a").unwrap();
        assert_eq!(u("D.EY")[0], Unit { onset: vec!["d".into()], nucleus: vec!["EH".into(), "IY".into()], coda: vec![] });
        assert_eq!(u("S.W.IY.T")[0].coda, vec!["t".to_string()]);
        assert_eq!(u("S.W.IY.T")[0].onset, vec!["s".to_string(), "w".to_string()]);
        assert_eq!(u("K.AH.L")[0].coda, vec!["L".to_string()]);
        assert!(units("Q.X", Lang::En, "a").is_err());
        let j = units("DeI", Lang::Ja, "a").unwrap();
        assert_eq!(j.len(), 2);
        assert_eq!(j[1].nucleus, vec!["i".to_string()]);
    }

    #[test]
    fn moraic_nasal_assimilates() {
        let mut us = units("NnPaNnKaNnTaNnChiNnANn", Lang::Ja, "a").unwrap();
        assimilate_units(&mut us);
        let ns: Vec<&str> = us.iter().filter(|u| u.nucleus[0].starts_with('N')).map(|u| u.nucleus[0].as_str()).collect();
        assert_eq!(ns, ["N:m", "N:ng", "N:n", "N:ny", "N", "N"]);
    }

    #[test]
    fn overrides_apply_in_order() {
        let mut g = Overrides::new();
        g.insert("k".into(), vec![("dur".into(), "30ms".into()), ("noise".into(), "0.2".into())]);
        let mut v = Overrides::new();
        v.insert("k".into(), vec![("noise".into(), "-6dB".into())]);
        let s = Table::new(&g, &v).get("k");
        assert!((s.dur - 0.030).abs() < 1e-9);
        assert!((s.noise - 10f64.powf(-6.0 / 20.0)).abs() < 1e-9);
        assert_eq!(s.kind, Kind::Stop);
    }
}

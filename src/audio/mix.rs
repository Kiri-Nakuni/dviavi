//! The sound.mix language: instrument and voice definitions, and tracks of
//! clips placed on the video's time line.
//!
//! ```text
//! % comment
//! inst lead { wave=square duty=0.25 adsr=5ms,80ms,0.6,120ms gain=-6dB }
//! voice example { f0=220 mora=150ms tone normal { f0x=1 range=4st } }
//! track bgm gain=-6dB pan=0
//!   @0f lead t140 o4 l8 c d e f g4 r4
//!     <c e g>2          % continuation of the clip above
//!   @intro+12f file "boom.wav" gain=-3dB
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::phoneme;
use super::units::{self, TimeContext};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Sine,
    Square,
    Triangle,
    Saw,
    Noise,
}

#[derive(Clone, Debug)]
pub struct Adsr {
    pub attack: f64,
    pub decay: f64,
    pub sustain: f64,
    pub release: f64,
}

#[derive(Clone, Debug)]
pub struct Inst {
    pub wave: Wave,
    pub duty: f64,
    pub adsr: Adsr,
    pub gain: f64,
    /// One oscillator per entry, detuned by this many cents.
    pub detune: Vec<f64>,
    pub lpf: Option<f64>,
    /// (rate Hz, depth cents)
    pub vibrato: Option<(f64, f64)>,
}

impl Inst {
    /// The built-in instrument of a waveform name (`sine`, `square`, ...).
    pub fn builtin(name: &str) -> Option<Inst> {
        let wave = parse_wave(name).ok()?;
        Some(Inst {
            wave,
            duty: 0.5,
            adsr: Adsr { attack: 0.005, decay: 0.05, sustain: 0.8, release: 0.05 },
            gain: 0.5,
            detune: vec![0.0],
            lpf: None,
            vibrato: None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Tone {
    pub f0x: f64,
    /// Pitch range of the intonation, in cents.
    pub range: f64,
}

#[derive(Clone, Debug)]
pub struct Voice {
    pub f0: f64,
    /// Length of one mora at `pace=1`, in seconds.
    pub mora: f64,
    pub formant: f64,
    pub breath: f64,
    pub gain: f64,
    pub vibrato: (f64, f64),
    /// How voiced sound is made (`engine=formant|square`).
    pub engine: super::voice::Engine,
    /// What lyrics and text are written in when a clip or `\speak` does not say.
    pub lang: phoneme::Lang,
    pub tones: HashMap<String, Tone>,
    /// `phoneme NAME { ... }` inside the voice (over the top-level ones).
    pub phonemes: phoneme::Overrides,
}

impl Default for Voice {
    fn default() -> Voice {
        Voice {
            f0: 140.0,
            mora: 0.15,
            formant: 1.0,
            breath: 0.03,
            gain: 0.5,
            vibrato: (5.5, 12.0),
            engine: super::voice::Engine::Formant,
            lang: phoneme::Lang::Ja,
            tones: HashMap::new(),
            phonemes: phoneme::Overrides::new(),
        }
    }
}

impl Voice {
    pub fn tone(&self, name: &str) -> Tone {
        self.tones.get(name).cloned().unwrap_or(Tone { f0x: 1.0, range: 400.0 })
    }
}

#[derive(Debug)]
pub enum ClipBody {
    /// MML played by an instrument or sung by a voice.
    /// `lang` is the clip's `lang=` (else the voice's).
    Notes { source: String, mml: String, lang: Option<phoneme::Lang> },
    File { path: PathBuf },
}

#[derive(Debug)]
pub struct Clip {
    /// Start in seconds from the first frame.
    pub at: f64,
    pub gain: f64,
    pub body: ClipBody,
    /// Line of the clip in sound.mix, for messages.
    pub line: usize,
}

#[derive(Debug)]
pub struct Track {
    pub name: String,
    pub gain: f64,
    pub pan: f64,
    pub clips: Vec<Clip>,
}

#[derive(Default, Debug)]
pub struct Mix {
    pub insts: HashMap<String, Inst>,
    pub voices: HashMap<String, Voice>,
    /// Top-level `phoneme NAME { ... }`: for every voice.
    pub phonemes: phoneme::Overrides,
    pub tracks: Vec<Track>,
}

impl Mix {
    /// The phonemes `voice` speaks and sings with.
    pub fn phoneme_table<'a>(&'a self, voice: &'a Voice) -> phoneme::Table<'a> {
        phoneme::Table::new(&self.phonemes, &voice.phonemes)
    }

    pub fn load(path: &Path, ctx: &TimeContext) -> Result<Mix, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        Mix::parse(&text, base, ctx).map_err(|e| format!("{}:{e}", path.display()))
    }

    /// Parses sound.mix text; file clips are resolved against `base`.
    /// Errors are `line: message`.
    pub fn parse(text: &str, base: &Path, ctx: &TimeContext) -> Result<Mix, String> {
        let mut mix = Mix::default();
        let lines: Vec<&str> = text.lines().map(|l| l.split('%').next().unwrap_or("")).collect();
        let mut i = 0;
        // whether the last clip was MML, which continuation lines extend
        let mut in_mml = false;
        while i < lines.len() {
            let line_no = i + 1;
            let line = lines[i].trim();
            let err = |m: String| format!("{line_no}: {m}");
            let word = line.split_whitespace().next().unwrap_or("");
            match word {
                "" => {
                    i += 1;
                    continue;
                }
                "inst" | "voice" | "phoneme" => {
                    // gather the braced block, which may span lines
                    let mut block = String::new();
                    let mut depth = 0i32;
                    let mut seen_brace = false;
                    while i < lines.len() {
                        for c in lines[i].chars() {
                            match c {
                                '{' => {
                                    depth += 1;
                                    seen_brace = true;
                                }
                                '}' => depth -= 1,
                                _ => {}
                            }
                        }
                        block.push_str(lines[i]);
                        block.push(' ');
                        i += 1;
                        if seen_brace && depth <= 0 {
                            break;
                        }
                    }
                    let tokens = tokenize(&block);
                    let name = tokens.get(1).ok_or_else(|| err(format!("{word} needs a name")))?.clone();
                    let mut pos = 2;
                    let body = block_items(&tokens, &mut pos).map_err(err)?;
                    match word {
                        "inst" => {
                            mix.insts.insert(name, parse_inst(&body).map_err(err)?);
                        }
                        "voice" => {
                            mix.voices.insert(name, parse_voice(&body).map_err(err)?);
                        }
                        _ => add_phoneme(&mut mix.phonemes, &name, &body).map_err(err)?,
                    }
                    in_mml = false;
                    continue;
                }
                "track" => {
                    let rest = line["track".len()..].trim();
                    let (name, params) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    if name.is_empty() {
                        return Err(err("track needs a name".into()));
                    }
                    let mut track = Track { name: name.to_string(), gain: 1.0, pan: 0.0, clips: Vec::new() };
                    for (k, v) in units::key_values(params) {
                        match k.as_str() {
                            "gain" => track.gain = units::gain(&v).map_err(err)?,
                            "pan" => track.pan = units::number(&v).map_err(err)?.clamp(-1.0, 1.0),
                            _ => return Err(err(format!("unknown track parameter `{k}'"))),
                        }
                    }
                    mix.tracks.push(track);
                    in_mml = false;
                }
                _ if line.starts_with('@') => {
                    let clip = parse_clip(&line[1..], base, ctx, line_no).map_err(err)?;
                    in_mml = matches!(clip.body, ClipBody::Notes { .. });
                    if mix.tracks.is_empty() {
                        mix.tracks.push(Track { name: "main".into(), gain: 1.0, pan: 0.0, clips: Vec::new() });
                    }
                    mix.tracks.last_mut().unwrap().clips.push(clip);
                }
                _ if in_mml => {
                    let clip = mix.tracks.last_mut().and_then(|t| t.clips.last_mut());
                    if let Some(Clip { body: ClipBody::Notes { mml, .. }, .. }) = clip {
                        mml.push(' ');
                        mml.push_str(line);
                    }
                }
                _ => return Err(err(format!("unexpected `{word}'"))),
            }
            i += 1;
        }
        Ok(mix)
    }
}

/// `TIME NAME [k=v...] MML...` or `TIME file "path" [k=v...]`.
fn parse_clip(s: &str, base: &Path, ctx: &TimeContext, line: usize) -> Result<Clip, String> {
    let mut parts = s.trim().splitn(2, char::is_whitespace);
    let at = ctx.time(parts.next().unwrap_or(""))?;
    let rest = parts.next().unwrap_or("").trim_start();
    let mut parts = rest.splitn(2, char::is_whitespace);
    let source = parts.next().unwrap_or("").to_string();
    if source.is_empty() {
        return Err("clip needs an instrument, a voice or `file'".into());
    }
    let mut rest = parts.next().unwrap_or("").trim_start();
    let mut gain = 1.0;
    let mut lang = None;
    let mut file = None;
    if source == "file" {
        let q = rest.strip_prefix('"').ok_or("file needs a quoted path")?;
        let end = q.find('"').ok_or("unterminated file path")?;
        file = Some(base.join(&q[..end]));
        rest = q[end + 1..].trim_start();
    }
    // leading key=value words are clip parameters, the rest is MML
    loop {
        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..word_end];
        let Some((k, v)) = word.split_once('=') else { break };
        match k {
            "gain" => gain = units::gain(v)?,
            "lang" => lang = Some(phoneme::Lang::parse(v)?),
            _ => return Err(format!("unknown clip parameter `{k}'")),
        }
        rest = rest[word_end..].trim_start();
    }
    let body = match file {
        Some(path) => ClipBody::File { path },
        None => ClipBody::Notes { source, mml: rest.to_string(), lang },
    };
    Ok(Clip { at, gain, body, line })
}

#[derive(Debug, Clone)]
enum Token {
    Word(String),
    Open,
    Close,
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_whitespace() || c == '{' || c == '}' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if c == '{' || c == '}' {
                out.push(c.to_string());
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Items of a `{ ... }` block: `key=value` pairs and nested blocks
/// (`tone NAME { ... }`, `phoneme NAME { ... }`).
enum Item {
    Kv(String, String),
    Block(String, String, Vec<Item>),
}

fn block_items(tokens: &[String], pos: &mut usize) -> Result<Vec<Item>, String> {
    let t = |p: usize| -> Option<Token> {
        tokens.get(p).map(|w| match w.as_str() {
            "{" => Token::Open,
            "}" => Token::Close,
            _ => Token::Word(w.clone()),
        })
    };
    if !matches!(t(*pos), Some(Token::Open)) {
        return Err("`{' expected".into());
    }
    *pos += 1;
    let mut items = Vec::new();
    loop {
        match t(*pos) {
            Some(Token::Close) => {
                *pos += 1;
                return Ok(items);
            }
            Some(Token::Word(w)) if w == "tone" || w == "phoneme" => {
                let name = match t(*pos + 1) {
                    Some(Token::Word(n)) => n,
                    _ => return Err(format!("{w} needs a name")),
                };
                *pos += 2;
                let inner = block_items(tokens, pos)?;
                items.push(Item::Block(w, name, inner));
            }
            Some(Token::Word(w)) => {
                let (k, v) = w.split_once('=').ok_or_else(|| format!("`key=value' expected, got `{w}'"))?;
                items.push(Item::Kv(k.to_string(), v.to_string()));
                *pos += 1;
            }
            Some(Token::Open) => return Err("unexpected `{'".into()),
            None => return Err("unterminated block".into()),
        }
    }
}

fn parse_wave(s: &str) -> Result<Wave, String> {
    Ok(match s {
        "sine" => Wave::Sine,
        "square" => Wave::Square,
        "triangle" => Wave::Triangle,
        "saw" | "sawtooth" => Wave::Saw,
        "noise" => Wave::Noise,
        _ => return Err(format!("unknown wave `{s}'")),
    })
}

fn pair(v: &str) -> Result<(&str, &str), String> {
    v.split_once(',').ok_or_else(|| format!("`a,b' expected, got `{v}'"))
}

fn parse_inst(items: &[Item]) -> Result<Inst, String> {
    let mut inst = Inst::builtin("sine").expect("sine is built in");
    for item in items {
        let Item::Kv(k, v) = item else { return Err("tone and phoneme are only allowed in a voice".into()) };
        match k.as_str() {
            "wave" => inst.wave = parse_wave(v)?,
            "duty" => inst.duty = units::number(v)?.clamp(0.01, 0.99),
            "gain" => inst.gain = units::gain(v)?,
            "adsr" => {
                let p: Vec<&str> = v.split(',').collect();
                let [a, d, s, r] = p.as_slice() else { return Err("adsr=a,d,s,r expected".into()) };
                inst.adsr = Adsr { attack: seconds(a)?, decay: seconds(d)?, sustain: units::number(s)?, release: seconds(r)? };
            }
            "detune" => inst.detune = v.split(',').map(units::cents).collect::<Result<_, _>>()?,
            "lpf" => inst.lpf = Some(units::freq(v)?),
            "vib" => {
                let (r, d) = pair(v)?;
                inst.vibrato = Some((units::freq(r)?, units::cents(d)?));
            }
            _ => return Err(format!("unknown inst parameter `{k}'")),
        }
    }
    Ok(inst)
}

fn parse_voice(items: &[Item]) -> Result<Voice, String> {
    let mut voice = Voice::default();
    for item in items {
        match item {
            Item::Kv(k, v) => match k.as_str() {
                "f0" => voice.f0 = units::freq(v)?,
                "mora" => voice.mora = seconds(v)?,
                "formant" => voice.formant = units::number(v)?,
                "breath" => voice.breath = units::number(v)?,
                "gain" => voice.gain = units::gain(v)?,
                "engine" => voice.engine = super::voice::Engine::parse(v)?,
                "lang" => voice.lang = phoneme::Lang::parse(v)?,
                "vib" => {
                    let (r, d) = pair(v)?;
                    voice.vibrato = (units::freq(r)?, units::cents(d)?);
                }
                _ => return Err(format!("unknown voice parameter `{k}'")),
            },
            Item::Block(kind, name, inner) if kind == "phoneme" => add_phoneme(&mut voice.phonemes, name, inner)?,
            Item::Block(_, name, inner) => {
                let mut tone = Tone { f0x: 1.0, range: 400.0 };
                for it in inner {
                    let Item::Kv(k, v) = it else { return Err("a block inside tone".into()) };
                    match k.as_str() {
                        "f0x" => tone.f0x = units::number(v)?,
                        "range" => tone.range = units::cents(v)?,
                        _ => return Err(format!("unknown tone parameter `{k}'")),
                    }
                }
                voice.tones.insert(name.clone(), tone);
            }
        }
    }
    Ok(voice)
}

/// Records `phoneme NAME { k=v ... }`, checking the values on the phoneme.
fn add_phoneme(into: &mut phoneme::Overrides, name: &str, items: &[Item]) -> Result<(), String> {
    let mut kv = Vec::new();
    for it in items {
        let Item::Kv(k, v) = it else { return Err("a block inside phoneme".into()) };
        kv.push((k.clone(), v.clone()));
    }
    let mut check = phoneme::Overrides::new();
    check.insert(name.to_string(), kv.clone());
    phoneme::spec_with(name, &[&check]).map_err(|e| format!("phoneme {name}: {e}"))?;
    into.entry(name.to_string()).or_default().extend(kv);
    Ok(())
}

/// A duration that must not depend on the frame rate (`ms` or `s`).
fn seconds(v: &str) -> Result<f64, String> {
    if v.ends_with('s') {
        TimeContext { fps_num: 1, fps_den: 1, marks: HashMap::new() }.duration(v)
    } else {
        Err(format!("`{v}': give the length in s or ms"))
    }
}

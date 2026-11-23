//! The sound track: sound.mix tracks plus the sound events placed by specials,
//! mixed into one stereo buffer covering the whole video.

pub mod input;
pub mod mix;
pub mod mml;
pub mod phoneme;
pub mod synth;
pub mod units;
pub mod voice;

use mix::{ClipBody, Inst, Mix, Voice};
use units::TimeContext;

use crate::warn::warn_once;

/// Length of one mora for `\speak`.
#[derive(Debug, Clone, Copy)]
pub enum Pace {
    /// Multiple of the voice's `mora`.
    Factor(f64),
    Seconds(f64),
}

#[derive(Debug, Clone)]
pub struct Speak {
    pub ch: String,
    pub tone: String,
    /// Pitch multiplier.
    pub speed: f64,
    pub pace: Pace,
    pub text: String,
    /// `lang=`; else the voice's.
    pub lang: Option<phoneme::Lang>,
}

/// A `sing` special: MML with lyrics for a voice, from its page on.
#[derive(Debug, Clone)]
pub struct Sing {
    pub ch: String,
    pub mml: String,
    pub lang: Option<phoneme::Lang>,
    pub gain: f64,
}

/// A single note from a `tone` special.
#[derive(Debug, Clone)]
pub struct ToneNote {
    /// An instrument of sound.mix or a built-in wave name.
    pub inst: String,
    pub freq: f64,
    pub dur: f64,
    /// Replaces the instrument's gain when given.
    pub amp: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum EventKind {
    Speak(Speak),
    Sing(Sing),
    Tone(ToneNote),
}

/// A sound started by a special on page `frame`.
#[derive(Debug, Clone)]
pub struct Event {
    pub frame: u64,
    pub kind: EventKind,
}

/// Stereo accumulation buffer.
struct Master {
    samples: Vec<f32>,
}

impl Master {
    fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Adds mono `src` at sample `start` with equal-power `pan` (-1 left .. 1 right).
    fn add_mono(&mut self, start: usize, src: &[f32], gain: f64, pan: f64) {
        let a = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
        let (gl, gr) = ((a.cos() * gain * std::f64::consts::SQRT_2) as f32, (a.sin() * gain * std::f64::consts::SQRT_2) as f32);
        let n = src.len().min(self.frames().saturating_sub(start));
        for (i, &v) in src[..n].iter().enumerate() {
            self.samples[2 * (start + i)] += v * gl;
            self.samples[2 * (start + i) + 1] += v * gr;
        }
    }

    fn add_stereo(&mut self, start: usize, src: &[f32], gain: f64, pan: f64) {
        let (gl, gr) = ((gain * (1.0 - pan).min(1.0)) as f32, (gain * (1.0 + pan).min(1.0)) as f32);
        let n = (src.len() / 2).min(self.frames().saturating_sub(start));
        for i in 0..n {
            self.samples[2 * (start + i)] += src[2 * i] * gl;
            self.samples[2 * (start + i) + 1] += src[2 * i + 1] * gr;
        }
    }
}

fn sample_at(seconds: f64, rate: f64) -> Option<usize> {
    (seconds >= 0.0).then(|| (seconds * rate).round() as usize)
}

/// Sings `notes` with `voice` from sample `start`: where the sound begins
/// (its first consonant may come before `start`) and the samples.
fn sing(mix: &Mix, voice: &Voice, notes: &[mml::Note], lang: phoneme::Lang, start: usize, r: f64) -> Result<(usize, Vec<f32>), String> {
    let sung: Vec<voice::SungNote> = notes
        .iter()
        .map(|n| voice::SungNote { start: n.start, gate: n.gate, f0: n.freq(), amp: n.velocity, lyric: n.lyric.as_deref() })
        .collect();
    let segments = voice::sing_segments(&mix.phoneme_table(voice), &sung, lang, start, r)?;
    let first = segments.first().map(|s| s.start).unwrap_or(start);
    Ok((first, voice::render(voice, &segments, r)))
}

/// Ends stereo `s` (interleaved) after `frames`, fading out (a raised
/// cosine) over the last `fade` frames before that.
fn cut(s: &mut Vec<f32>, frames: usize, fade: f64) {
    s.truncate(frames * 2);
    let n = s.len() / 2;
    let fade = (fade.round() as usize).min(n);
    for i in 0..fade {
        let x = (i as f64 + 0.5) / fade as f64; // 0 at the fade's start, 1 at the end
        let g = (0.5 * (1.0 + (std::f64::consts::PI * x).cos())) as f32;
        let k = n - fade + i;
        s[2 * k] *= g;
        s[2 * k + 1] *= g;
    }
}

/// Mixes everything into `frames` stereo samples at `rate` (interleaved L R).
pub fn render(mix: &Mix, events: &[Event], ctx: &TimeContext, rate: u32, frames: usize) -> Vec<f32> {
    let r = rate as f64;
    let mut master = Master { samples: vec![0.0; frames * 2] };
    let default_voice = Voice::default();
    let voice_of = |name: &str| -> &Voice {
        mix.voices.get(name).unwrap_or_else(|| {
            if name != "default" {
                warn_once(format!("voice `{name}' is not defined in sound.mix; using the default voice"));
            }
            &default_voice
        })
    };
    let inst_of = |name: &str| -> Option<Inst> { mix.insts.get(name).cloned().or_else(|| Inst::builtin(name)) };

    for track in &mix.tracks {
        for clip in &track.clips {
            let Some(start) = sample_at(clip.at, r) else {
                warn_once(format!("sound.mix line {} (track {}): the clip starts before the video; skipped", clip.line, track.name));
                continue;
            };
            let gain = clip.gain * track.gain;
            match &clip.body {
                ClipBody::File { path } => match input::load(path, rate) {
                    Ok(mut s) => {
                        // where it ends: until=, else the video's end when it loops, else its own end
                        let end = match clip.until {
                            Some(u) => ((u - clip.at) * r).round() as usize,
                            None if clip.looped => frames.saturating_sub(start),
                            None => s.len() / 2,
                        };
                        if clip.looped && !s.is_empty() {
                            s = s.iter().copied().cycle().take(end * 2).collect();
                        }
                        cut(&mut s, end, clip.fade * r);
                        master.add_stereo(start, &s, gain, track.pan)
                    }
                    Err(e) => warn_once(e),
                },
                ClipBody::Notes { source, mml, lang } => {
                    let lang = lang.or(mix.voices.get(source).map(|v| v.lang)).unwrap_or_default();
                    let notes = match mml::parse(mml, lang) {
                        Ok(n) => n,
                        Err(e) => {
                            warn_once(format!("sound.mix line {}: {e}", clip.line));
                            continue;
                        }
                    };
                    if let Some(voice) = mix.voices.get(source) {
                        match sing(mix, voice, &notes, lang, start, r) {
                            Ok((first, s)) => master.add_mono(first, &s, gain, track.pan),
                            Err(e) => warn_once(format!("sound.mix line {}: {e}", clip.line)),
                        }
                    } else if let Some(inst) = inst_of(source) {
                        for n in &notes {
                            let s = synth::note(&inst, n.freq(), n.velocity, n.gate, r);
                            master.add_mono(start + (n.start * r) as usize, &s, gain, track.pan);
                        }
                    } else {
                        warn_once(format!("sound.mix line {}: `{source}' is neither an inst nor a voice", clip.line));
                    }
                }
            }
        }
    }

    for ev in events {
        let start = (ctx.frame_seconds(ev.frame) * r).round() as usize;
        match &ev.kind {
            EventKind::Speak(sp) => {
                let voice = voice_of(&sp.ch);
                let tone = voice.tone(&sp.tone);
                let mora = match sp.pace {
                    Pace::Factor(f) => voice.mora * f,
                    Pace::Seconds(s) => s,
                };
                let lang = sp.lang.unwrap_or(voice.lang);
                match voice::speak_segments(voice, &mix.phoneme_table(voice), &tone, &sp.text, lang, sp.speed, mora, start, r) {
                    Ok(segs) => master.add_mono(start, &voice::render(voice, &segs, r), 1.0, 0.0),
                    Err(e) => warn_once(format!("speak: {e}")),
                }
            }
            EventKind::Sing(sg) => {
                let voice = voice_of(&sg.ch);
                let lang = sg.lang.unwrap_or(voice.lang);
                let r2 = mml::parse(&sg.mml, lang).and_then(|notes| sing(mix, voice, &notes, lang, start, r));
                match r2 {
                    Ok((first, s)) => master.add_mono(first, &s, sg.gain, 0.0),
                    Err(e) => warn_once(format!("sing: {e}")),
                }
            }
            EventKind::Tone(t) => match inst_of(&t.inst) {
                Some(inst) => {
                    let g = t.amp.map(|a| a / inst.gain.max(1e-9)).unwrap_or(1.0);
                    master.add_mono(start, &synth::note(&inst, t.freq, 1.0, t.dur, r), g, 0.0)
                },
                None => warn_once(format!("tone: unknown instrument `{}'", t.inst)),
            },
        }
    }

    limit(&mut master.samples);
    master.samples
}

/// Soft-clips peaks above 0.8 so overlapping sounds never wrap around.
fn limit(s: &mut [f32]) {
    let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
    if peak > 1.0 {
        eprintln!("dviavi: audio peak {:.2} (> 1.0) soft-clipped; lower some gains", peak);
    }
    for v in s.iter_mut() {
        let a = v.abs();
        if a > 0.8 {
            *v = v.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn cut_fades_out() {
        let mut s = vec![1.0f32; 2 * 100];
        cut(&mut s, 50, 10.0);
        assert_eq!(s.len(), 100);
        assert_eq!(s[2 * 39], 1.0); // before the fade
        assert!(s[2 * 40] < 1.0 && s[2 * 40] > 0.9); // the fade's start
        assert!(s[2 * 49] < 0.05); // its end
        assert_eq!(s[2 * 45], s[2 * 45 + 1]); // both channels
    }

    #[test]
    fn file_clip_loop() {
        let ctx = TimeContext { fps_num: 10, fps_den: 1, marks: Default::default() };
        let mix = Mix::parse("track bgm\n  @0 file \"x.wav\" loop=on fade=2s\n", Path::new("."), &ctx).unwrap();
        assert!(mix.tracks[0].clips[0].looped);
        assert!(Mix::parse("track bgm\n  @0 file \"x.wav\" loop=maybe\n", Path::new("."), &ctx).is_err());
    }

    #[test]
    fn file_clip_until() {
        let ctx = TimeContext { fps_num: 10, fps_den: 1, marks: [("next".to_string(), 30)].into() };
        let mix = Mix::parse("track bgm\n  @0 file \"x.wav\" until=next fade=1s gain=-6dB\n", Path::new("."), &ctx).unwrap();
        let clip = &mix.tracks[0].clips[0];
        assert_eq!(clip.until, Some(3.0));
        assert_eq!(clip.fade, 1.0);
        assert!(Mix::parse("track t\n  @0 sine until=next c\n", Path::new("."), &ctx).is_err());
    }
}

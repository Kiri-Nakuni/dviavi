//! `dviavi say`: renders speech or song with a voice of a sound.mix to a WAV
//! file, for tuning voices and phonemes without making a video.
//!
//!     dviavi say [--mix FILE] [--voice NAME] [--tone T] [--speed X] [--pace P]
//!                [--rate HZ] [--lang ja|en] [-o OUT.wav] [--play] TEXT
//!     dviavi say [--mix FILE] [--voice NAME] ... --sing 'MML with "lyrics"'

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::audio::mix::{Mix, Voice};
use crate::audio::units::TimeContext;
use crate::audio::phoneme::Lang;
use crate::audio::{mml, voice};

pub const USAGE: &str = "usage: dviavi say [--mix FILE] [--voice NAME] [--tone T] [--speed X] [--pace P] [--rate HZ] [--lang ja|en] [-o OUT.wav] [--play] (TEXT | --sing MML)";

pub fn run(args: &[String]) -> Result<(), String> {
    let mut mix_path = None;
    let mut voice_name = String::from("default");
    let mut tone = String::from("normal");
    let (mut speed, mut pace, mut rate) = (1.0f64, 1.0f64, 48000u32);
    let mut out = String::from("say.wav");
    let mut play = false;
    let mut sing = None;
    let mut lang = None;
    let mut text = None;
    let mut it = args.iter();
    let val = |it: &mut std::slice::Iter<String>, k: &str| it.next().cloned().ok_or(format!("{k} needs a value"));
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mix" => mix_path = Some(val(&mut it, a)?),
            "--voice" => voice_name = val(&mut it, a)?,
            "--tone" => tone = val(&mut it, a)?,
            "--speed" => speed = val(&mut it, a)?.parse().map_err(|_| "bad --speed")?,
            "--pace" => pace = val(&mut it, a)?.parse().map_err(|_| "bad --pace")?,
            "--rate" => rate = val(&mut it, a)?.parse().map_err(|_| "bad --rate")?,
            "-o" => out = val(&mut it, a)?,
            "--play" => play = true,
            "--sing" => sing = Some(val(&mut it, a)?),
            "--lang" => lang = Some(Lang::parse(&val(&mut it, a)?)?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if text.is_none() && !a.starts_with('-') => text = Some(a.clone()),
            _ => return Err(USAGE.into()),
        }
    }
    let ctx = TimeContext { fps_num: 24000, fps_den: 1001, marks: HashMap::new() };
    let mix = match &mix_path {
        Some(p) => Mix::load(Path::new(p), &ctx)?,
        None => Mix::default(),
    };
    let default_voice = Voice::default();
    let v = match mix.voices.get(&voice_name) {
        Some(v) => v,
        None if voice_name == "default" => &default_voice,
        None => return Err(format!("voice `{voice_name}' is not in the mix file")),
    };
    let table = mix.phoneme_table(v);
    let lang = lang.unwrap_or(v.lang);
    let r = rate as f64;
    // a little room before the first note for its consonant
    let lead = (0.2 * r) as usize;
    let segments = match (&sing, &text) {
        (Some(m), _) => {
            let notes = mml::parse(m, lang)?;
            let sung: Vec<voice::SungNote> = notes
                .iter()
                .map(|n| voice::SungNote { start: n.start, gate: n.gate, f0: n.freq(), amp: n.velocity, lyric: n.lyric.as_deref() })
                .collect();
            voice::sing_segments(&table, &sung, lang, lead, r)?
        }
        (None, Some(t)) => voice::speak_segments(v, &table, &v.tone(&tone), t, lang, speed, v.mora * pace, lead, r)?,
        (None, None) => return Err(USAGE.into()),
    };
    let samples = voice::render(v, &segments, r);
    write_wav(&out, &samples, rate).map_err(|e| format!("{out}: {e}"))?;
    eprintln!("dviavi say: {out}: {:.2} s", samples.len() as f64 / r);
    if play {
        let ok = Command::new("ffplay").args(["-v", "error", "-nodisp", "-autoexit"]).arg(&out).status().map(|s| s.success());
        if !matches!(ok, Ok(true)) {
            Command::new("aplay").arg("-q").arg(&out).status().map_err(|e| format!("cannot play {out}: {e}"))?;
        }
    }
    Ok(())
}

/// 16-bit mono PCM WAV.
fn write_wav(path: &str, samples: &[f32], rate: u32) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data_len = samples.len() as u32 * 2;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        f.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    f.flush()
}

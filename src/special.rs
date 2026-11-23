//! The `dviavi:` specials and `papersize`, collected from all pages before
//! rendering: they fix the output format and place sounds on the time line.
//!
//! ```text
//! dviavi: config fps=24000/1001 size=1920x1080 rate=48000 mix=sound.mix keyint=240 paper=160mm,90mm
//! dviavi: mark name=intro
//! dviavi: speak ch=example tone=normal speed=.9 pace=1. text=KoNnNiTiWa
//! dviavi: tone inst=sine freq=440 dur=2s amp=0.5
//! ```

use std::collections::HashMap;

use crate::audio::units::{self, TimeContext};
use crate::audio::phoneme::Lang;
use crate::audio::{Event, EventKind, Pace, Sing, Speak, ToneNote};
use crate::tex::dvi::Dvi;
use crate::warn::warn_once;

/// Output format; fixed by the `config` special of the first page.
#[derive(Debug, Clone)]
pub struct Config {
    pub fps_num: u32,
    pub fps_den: u32,
    pub width: u32,
    pub height: u32,
    pub rate: u32,
    pub mix: Option<String>,
    /// Paper size in metres from `paper=`; preferred over `papersize`.
    pub paper: Option<(f64, f64)>,
    /// Frames between key frames.
    pub keyint: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config { fps_num: 24000, fps_den: 1001, width: 1920, height: 1080, rate: 48000, mix: None, paper: None, keyint: 240 }
    }
}

pub struct Timeline {
    pub config: Config,
    /// Paper size in metres (true dimensions), from the `papersize` special.
    pub papersize: Option<(f64, f64)>,
    pub marks: HashMap<String, u64>,
    pub events: Vec<Event>,
    /// sound.mix text given by `mixdef` specials, in page order.
    pub mixdefs: Vec<String>,
}

impl Timeline {
    pub fn time_context(&self) -> TimeContext {
        TimeContext { fps_num: self.config.fps_num, fps_den: self.config.fps_den, marks: self.marks.clone() }
    }
}

/// Reads the specials of every page.
pub fn scan(dvi: &Dvi) -> Result<Timeline, String> {
    let mut tl = Timeline { config: Config::default(), papersize: None, marks: HashMap::new(), events: Vec::new(), mixdefs: Vec::new() };
    // events need the frame rate to read their durations, so read config first
    let mut pending: Vec<(u64, String)> = Vec::new();
    for page in 0..dvi.page_count() {
        for raw in dvi.page_specials(page)? {
            let s = String::from_utf8_lossy(raw).trim().to_string();
            if let Some(p) = s.strip_prefix("papersize=") {
                if tl.papersize.is_none() {
                    tl.papersize = parse_papersize(p, dvi.mag);
                }
            } else if let Some(cmd) = s.strip_prefix("dviavi:") {
                let cmd = cmd.trim();
                let (word, rest) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
                if word == "config" {
                    if page == 0 {
                        apply_config(&mut tl.config, rest, dvi.mag)?;
                    } else {
                        warn_once(format!("dviavi config on page {} ignored (only the first page counts)", page + 1));
                    }
                } else {
                    pending.push((page as u64, cmd.to_string()));
                }
            }
        }
    }
    let ctx = TimeContext { fps_num: tl.config.fps_num, fps_den: tl.config.fps_den, marks: HashMap::new() };
    for (frame, cmd) in pending {
        let (word, rest) = cmd.split_once(char::is_whitespace).unwrap_or((&cmd, ""));
        let kv: HashMap<String, String> = units::key_values(rest).into_iter().collect();
        let get = |k: &str| kv.get(k).map(String::as_str);
        let r: Result<(), String> = (|| {
            match word {
                "mark" => {
                    let name = get("name").map(str::to_string).or_else(|| rest.split_whitespace().next().map(str::to_string));
                    let name = name.ok_or("mark needs a name")?;
                    if tl.marks.insert(name.clone(), frame).is_some() {
                        warn_once(format!("mark `{name}' defined twice; the later one counts"));
                    }
                }
                "speak" => {
                    let pace = match get("pace") {
                        None => Pace::Factor(1.0),
                        Some(p) if p.ends_with(['s', 'f']) => Pace::Seconds(ctx.duration(p)?),
                        Some(p) => Pace::Factor(units::number(p)?),
                    };
                    tl.events.push(Event {
                        frame,
                        kind: EventKind::Speak(Speak {
                            ch: get("ch").unwrap_or("default").to_string(),
                            tone: get("tone").unwrap_or("normal").to_string(),
                            speed: get("speed").map(units::number).transpose()?.unwrap_or(1.0),
                            pace,
                            text: get("text").ok_or("speak needs text=")?.to_string(),
                            lang: get("lang").map(Lang::parse).transpose()?,
                        }),
                    });
                }
                "sing" => tl.events.push(Event {
                    frame,
                    kind: EventKind::Sing(Sing {
                        ch: get("ch").unwrap_or("default").to_string(),
                        mml: get("mml").ok_or("sing needs mml=")?.to_string(),
                        lang: get("lang").map(Lang::parse).transpose()?,
                        gain: get("gain").map(units::gain).transpose()?.unwrap_or(1.0),
                    }),
                }),
                // sound.mix text from the document (voices, phonemes, insts)
                "mixdef" => tl.mixdefs.push(rest.to_string()),
                "tone" => tl.events.push(Event {
                    frame,
                    kind: EventKind::Tone(ToneNote {
                        inst: get("inst").or(get("wave")).unwrap_or("sine").to_string(),
                        freq: units::freq(get("freq").unwrap_or("440"))?,
                        dur: ctx.duration(get("dur").unwrap_or("1s"))?,
                        amp: get("amp").map(units::number).transpose()?,
                    }),
                }),
                _ => return Err(format!("unknown command `{word}'")),
            }
            Ok(())
        })();
        if let Err(e) = r {
            warn_once(format!("page {}: dviavi: {cmd}: {e}", frame + 1));
        }
    }
    Ok(tl)
}

fn apply_config(c: &mut Config, rest: &str, mag: f64) -> Result<(), String> {
    for (k, v) in units::key_values(rest) {
        let bad = || format!("dviavi config: bad {k}=`{v}'");
        match k.as_str() {
            "fps" => {
                let (n, d) = v.split_once('/').unwrap_or((&v, "1"));
                if let (Ok(n), Ok(d)) = (n.parse::<u32>(), d.parse::<u32>()) {
                    c.fps_num = n;
                    c.fps_den = d;
                } else {
                    // a decimal like 29.97 becomes n/1000
                    let f: f64 = v.parse().map_err(|_| bad())?;
                    c.fps_num = (f * 1000.0).round() as u32;
                    c.fps_den = 1000;
                }
                if c.fps_num == 0 || c.fps_den == 0 {
                    return Err(bad());
                }
            }
            "size" => {
                let (w, h) = v.split_once('x').ok_or_else(bad)?;
                c.width = w.parse().map_err(|_| bad())?;
                c.height = h.parse().map_err(|_| bad())?;
                if c.width == 0 || c.height == 0 {
                    return Err(bad());
                }
                c.width += c.width % 2;
                c.height += c.height % 2;
            }
            "rate" => c.rate = v.parse().map_err(|_| bad())?,
            "keyint" => c.keyint = v.parse::<u32>().map_err(|_| bad())?.max(1),
            "mix" => c.mix = Some(v),
            "paper" => c.paper = Some(parse_papersize(&v, mag).ok_or_else(bad)?),
            _ => warn_once(format!("dviavi config: unknown key `{k}'")),
        }
    }
    Ok(())
}

/// `W,H` with TeX units; dimensions without `true` are magnified.
fn parse_papersize(s: &str, mag: f64) -> Option<(f64, f64)> {
    let (w, h) = s.split_once(',')?;
    Some((dimen_m(w, mag)?, dimen_m(h, mag)?))
}

fn dimen_m(s: &str, mag: f64) -> Option<f64> {
    let s = s.trim();
    let num_end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(s.len());
    let v: f64 = s[..num_end].parse().ok()?;
    let mut unit = s[num_end..].trim();
    let mut m = mag;
    if let Some(u) = unit.strip_prefix("true") {
        unit = u.trim();
        m = 1.0;
    }
    let per = match unit {
        "pt" => 0.0254 / 72.27,
        "bp" => 0.0254 / 72.0,
        "mm" => 0.001,
        "cm" => 0.01,
        "in" => 0.0254,
        "pc" => 0.0254 / 72.27 * 12.0,
        "dd" => 0.0254 / 72.27 * 1238.0 / 1157.0,
        "cc" => 0.0254 / 72.27 * 1238.0 / 1157.0 * 12.0,
        "sp" => 0.0254 / 72.27 / 65536.0,
        "Q" => 0.00025,
        _ => return None,
    };
    Some(v * per * m)
}

//! MML (music macro language) as used in sound.mix clips.
//!
//! - notes `c d e f g a b`, sharp `+`/`#`, flat `-`, length `4`, dots `.`, tie `&`
//! - rest `r`, chord `<c e g>4`, lyric `"KoNnNi"` (one mora per following note;
//!   `"DeI|Ji-"`: a token per note; English `"D.EY Z.IY"`: a syllable per note)
//! - `t120` tempo (quarter notes per minute), `o4` octave (`o+`, `o-` relative),
//!   `l8` default length, `v100` velocity (0-127), `q7` gate in eighths, `|` ignored

use super::phoneme::Lang;

/// A note relative to the clip start; times in seconds.
#[derive(Debug, Clone)]
pub struct Note {
    pub start: f64,
    /// Sounding time (the gate); the release follows.
    pub gate: f64,
    pub midi: f64,
    pub velocity: f64,
    pub lyric: Option<String>,
}

impl Note {
    pub fn freq(&self) -> f64 {
        440.0 * 2f64.powf((self.midi - 69.0) / 12.0)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }
    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace() || c == b'|') {
            self.pos += 1;
        }
    }
    fn number(&mut self) -> Option<f64> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.s[start..self.pos]).ok()?.parse().ok()
    }
    /// `4.` etc. as a fraction of a whole note; `None` if no number follows.
    fn length(&mut self) -> Option<f64> {
        let n = self.number()?;
        Some(self.dots(1.0 / n.max(1.0)))
    }
    fn dots(&mut self, mut len: f64) -> f64 {
        let mut add = len / 2.0;
        while self.peek() == Some(b'.') {
            self.pos += 1;
            len += add;
            add /= 2.0;
        }
        len
    }
    /// Pitch letter with accidentals, as a semitone offset from c.
    fn pitch(&mut self) -> Option<i32> {
        let base = match self.peek()? {
            b'c' => 0,
            b'd' => 2,
            b'e' => 4,
            b'f' => 5,
            b'g' => 7,
            b'a' => 9,
            b'b' => 11,
            _ => return None,
        };
        self.pos += 1;
        let mut p = base;
        loop {
            match self.peek() {
                Some(b'+') | Some(b'#') => p += 1,
                Some(b'-') => p -= 1,
                _ => break,
            }
            self.pos += 1;
        }
        Some(p)
    }
}

/// Splits romanised text into morae: each capital letter starts one.
pub fn morae(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in text.chars() {
        if c.is_ascii_uppercase() || out.is_empty() || !c.is_ascii_alphabetic() {
            out.push(c.to_string());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    out.retain(|m| !m.trim().is_empty());
    out
}

/// The lyric tokens of a `"..."` string, one per note: separated by `|`
/// (Japanese: `"DeI|Ji-"`, several morae on one note), or for Japanese
/// without `|` one mora per note (`"KoNnNi"`), for English by spaces.
pub fn lyric_tokens(text: &str, lang: Lang) -> Vec<String> {
    match lang {
        Lang::Ja if !text.contains('|') => morae(text),
        _ => text.split(|c: char| c == '|' || (lang == Lang::En && c.is_whitespace())).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect(),
    }
}

pub fn parse(mml: &str, lang: Lang) -> Result<Vec<Note>, String> {
    let mut p = Parser { s: mml.as_bytes(), pos: 0 };
    let mut notes = Vec::new();
    let mut t = 0.0f64;
    let mut tempo = 120.0f64;
    let mut octave = 4i32;
    let mut default_len = 0.25f64;
    let mut velocity = 100.0f64;
    let mut gate_eighths = 7.0f64;
    let mut lyrics: std::collections::VecDeque<String> = Default::default();
    let whole = |tempo: f64| 240.0 / tempo;
    loop {
        p.skip_space();
        let Some(c) = p.peek() else { break };
        let at = p.pos;
        let err = |m: &str| format!("MML at column {}: {m}", at + 1);
        match c {
            b't' => {
                p.pos += 1;
                tempo = p.number().ok_or_else(|| err("tempo expected"))?.max(1.0);
            }
            b'o' => {
                p.pos += 1;
                match p.peek() {
                    Some(b'+') => {
                        p.pos += 1;
                        octave += 1;
                    }
                    Some(b'-') => {
                        p.pos += 1;
                        octave -= 1;
                    }
                    _ => octave = p.number().ok_or_else(|| err("octave expected"))? as i32,
                }
            }
            b'l' => {
                p.pos += 1;
                default_len = p.length().ok_or_else(|| err("length expected"))?;
            }
            b'v' => {
                p.pos += 1;
                velocity = p.number().ok_or_else(|| err("velocity expected"))?.clamp(0.0, 127.0);
            }
            b'q' => {
                p.pos += 1;
                gate_eighths = p.number().ok_or_else(|| err("gate expected"))?.clamp(1.0, 8.0);
            }
            b'"' => {
                p.pos += 1;
                let start = p.pos;
                while p.peek().is_some_and(|c| c != b'"') {
                    p.pos += 1;
                }
                let text = std::str::from_utf8(&p.s[start..p.pos]).unwrap_or("");
                p.pos += 1;
                lyrics.extend(lyric_tokens(text, lang));
            }
            b'r' => {
                p.pos += 1;
                let len = p.length().unwrap_or_else(|| p.dots(default_len));
                t += len * whole(tempo);
            }
            b'<' => {
                p.pos += 1;
                let mut pitches = Vec::new();
                loop {
                    p.skip_space();
                    match p.peek() {
                        Some(b'>') => {
                            p.pos += 1;
                            break;
                        }
                        Some(_) => {
                            let pitch = p.pitch().ok_or_else(|| err("note expected in chord"))?;
                            pitches.push(pitch);
                        }
                        None => return Err(err("unterminated chord")),
                    }
                }
                let len = p.length().unwrap_or_else(|| p.dots(default_len));
                let dur = len * whole(tempo);
                let lyric = lyrics.pop_front();
                for pitch in pitches {
                    notes.push(Note {
                        start: t,
                        gate: dur * gate_eighths / 8.0,
                        midi: (12 * (octave + 1) + pitch) as f64,
                        velocity: velocity / 127.0,
                        lyric: lyric.clone(),
                    });
                }
                t += dur;
            }
            _ => {
                let pitch = p.pitch().ok_or_else(|| err(&format!("unexpected `{}'", c as char)))?;
                let mut len = p.length().unwrap_or_else(|| p.dots(default_len));
                // ties: c4&c8 or c4&8
                loop {
                    p.skip_space();
                    if p.peek() != Some(b'&') {
                        break;
                    }
                    p.pos += 1;
                    p.skip_space();
                    let _ = p.pitch();
                    len += p.length().unwrap_or_else(|| p.dots(default_len));
                }
                let dur = len * whole(tempo);
                notes.push(Note {
                    start: t,
                    gate: dur * gate_eighths / 8.0,
                    midi: (12 * (octave + 1) + pitch) as f64,
                    velocity: velocity / 127.0,
                    lyric: lyrics.pop_front(),
                });
                t += dur;
            }
        }
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_pitch_chords_lyrics() {
        // t120: a quarter is 0.5 s
        let n = parse("t120 o4 l4 c d8. q8 e&e8 <c e g>2 r4 o+ c \"KoNnNi\" a", Lang::Ja).unwrap();
        let starts: Vec<f64> = n.iter().map(|n| n.start).collect();
        assert_eq!(starts, [0.0, 0.5, 0.875, 1.625, 1.625, 1.625, 3.125, 3.625]);
        assert_eq!(n[0].midi, 60.0);
        assert_eq!(n[6].midi, 72.0);
        assert_eq!(n[1].gate, 0.375 * 7.0 / 8.0);
        assert_eq!(n[2].gate, 0.75, "q8 and a tie: the full 4 + 8");
        assert_eq!(n[6].lyric, None, "the lyric comes after this note");
        assert_eq!(n[7].lyric.as_deref(), Some("Ko"));
        assert_eq!(morae("KoNnNiTiWa"), ["Ko", "Nn", "Ni", "Ti", "Wa"]);
    }
}

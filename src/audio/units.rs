//! Values with units used by sound.mix and the dviavi specials.

use std::collections::HashMap;

/// What a time expression needs to become seconds.
pub struct TimeContext {
    pub fps_num: u32,
    pub fps_den: u32,
    /// `\dviaviMark` names to frame numbers.
    pub marks: HashMap<String, u64>,
}

impl TimeContext {
    pub fn frame_seconds(&self, frame: u64) -> f64 {
        frame as f64 * self.fps_den as f64 / self.fps_num as f64
    }

    /// `12f`, `5s`, `150ms`, `1:23.5`, a bare number (frames), or
    /// `mark`, `mark+12f`, `mark-1s`.
    pub fn time(&self, s: &str) -> Result<f64, String> {
        let s = s.trim();
        if s.starts_with(|c: char| c.is_ascii_alphabetic()) {
            let split = s.find(['+', '-']).unwrap_or(s.len());
            let (name, off) = s.split_at(split);
            let frame = self.marks.get(name).ok_or_else(|| format!("unknown mark `{name}'"))?;
            let base = self.frame_seconds(*frame);
            return Ok(match off.chars().next() {
                Some('+') => base + self.duration(&off[1..])?,
                Some('-') => base - self.duration(&off[1..])?,
                _ => base,
            });
        }
        self.duration(s)
    }

    /// A time span: `12f`, `5s`, `150ms`, `1:23.5` or a bare number of frames.
    pub fn duration(&self, s: &str) -> Result<f64, String> {
        let s = s.trim();
        let bad = || format!("bad time `{s}'");
        if let Some((m, sec)) = s.split_once(':') {
            let m: f64 = m.parse().map_err(|_| bad())?;
            let sec: f64 = sec.parse().map_err(|_| bad())?;
            return Ok(m * 60.0 + sec);
        }
        if let Some(v) = s.strip_suffix("ms") {
            return v.parse::<f64>().map(|v| v / 1000.0).map_err(|_| bad());
        }
        if let Some(v) = s.strip_suffix('s') {
            return v.parse::<f64>().map_err(|_| bad());
        }
        let frames = s.strip_suffix('f').unwrap_or(s);
        frames.parse::<f64>().map(|f| f * self.fps_den as f64 / self.fps_num as f64).map_err(|_| bad())
    }
}

/// `440`, `440Hz`, `2k`, `2kHz`.
pub fn freq(s: &str) -> Result<f64, String> {
    let t = s.trim().trim_end_matches("Hz").trim_end_matches("hz");
    let (t, mul) = match t.strip_suffix('k') {
        Some(t) => (t, 1000.0),
        None => (t, 1.0),
    };
    t.parse::<f64>().map(|v| v * mul).map_err(|_| format!("bad frequency `{s}'"))
}

/// `-6dB` or a linear factor.
pub fn gain(s: &str) -> Result<f64, String> {
    let t = s.trim();
    match t.strip_suffix("dB").or_else(|| t.strip_suffix("db")) {
        Some(db) => db.parse::<f64>().map(|d| 10f64.powf(d / 20.0)),
        None => t.parse::<f64>(),
    }
    .map_err(|_| format!("bad gain `{s}'"))
}

/// `3c` (cents) or `4st` (semitones), returned in cents.
pub fn cents(s: &str) -> Result<f64, String> {
    let t = s.trim();
    if let Some(v) = t.strip_suffix("st") {
        return v.parse::<f64>().map(|v| v * 100.0).map_err(|_| format!("bad interval `{s}'"));
    }
    t.trim_end_matches('c').parse::<f64>().map_err(|_| format!("bad interval `{s}'"))
}

pub fn number(s: &str) -> Result<f64, String> {
    s.trim().parse::<f64>().map_err(|_| format!("bad number `{s}'"))
}

/// Splits `key=value key2="quoted value"` words.
pub fn key_values(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let end_key = rest.find(|c: char| c == '=' || c.is_whitespace()).unwrap_or(rest.len());
        let key = rest[..end_key].to_string();
        rest = &rest[end_key..];
        let value = if let Some(r) = rest.strip_prefix('=') {
            if let Some(b) = r.strip_prefix('{') {
                // a braced value may hold spaces, quotes and nested braces
                let mut depth = 1;
                let end = b
                    .char_indices()
                    .find(|&(_, c)| {
                        depth += match c {
                            '{' => 1,
                            '}' => -1,
                            _ => 0,
                        };
                        depth == 0
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(b.len());
                rest = b.get(end + 1..).unwrap_or("");
                b[..end].trim().to_string()
            } else if let Some(q) = r.strip_prefix('"') {
                let end = q.find('"').unwrap_or(q.len());
                rest = q.get(end + 1..).unwrap_or("");
                q[..end].to_string()
            } else {
                let end = r.find(char::is_whitespace).unwrap_or(r.len());
                rest = &r[end..];
                r[..end].to_string()
            }
        } else {
            String::new()
        };
        if !key.is_empty() {
            out.push((key, value));
        }
        rest = rest.trim_start();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braced_values() {
        let kv = key_values(r#"ch=daisy mml={t150 "D.EY Z.IY" d2. {x}} lang=en"#);
        assert_eq!(kv[1], ("mml".to_string(), r#"t150 "D.EY Z.IY" d2. {x}"#.to_string()));
        assert_eq!(kv[2], ("lang".to_string(), "en".to_string()));
    }

    #[test]
    fn times() {
        let ctx = TimeContext { fps_num: 24000, fps_den: 1001, marks: [("intro".to_string(), 48)].into() };
        let f = 1001.0 / 24000.0;
        assert!((ctx.time("24f").unwrap() - 24.0 * f).abs() < 1e-12);
        assert!((ctx.time("24").unwrap() - 24.0 * f).abs() < 1e-12);
        assert_eq!(ctx.time("1.5s").unwrap(), 1.5);
        assert_eq!(ctx.time("150ms").unwrap(), 0.15);
        assert_eq!(ctx.time("1:02.5").unwrap(), 62.5);
        assert!((ctx.time("intro+12f").unwrap() - 60.0 * f).abs() < 1e-12);
        assert!((ctx.time("intro-1s").unwrap() - (48.0 * f - 1.0)).abs() < 1e-12);
        assert!(ctx.time("nosuch").is_err());
    }
}

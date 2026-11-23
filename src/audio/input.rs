//! External sound files: WAV read directly, anything else (or a WAV at another
//! sample rate) decoded by an `ffmpeg` subprocess through a pipe.

use std::path::Path;
use std::process::Command;

/// Interleaved stereo samples at `rate`.
pub fn load(path: &Path, rate: u32) -> Result<Vec<f32>, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(wav) = parse_wav(&data) {
        if wav.rate == rate {
            return Ok(wav.stereo);
        }
    }
    ffmpeg(path, rate)
}

struct Wav {
    rate: u32,
    stereo: Vec<f32>,
}

fn parse_wav(d: &[u8]) -> Option<Wav> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let mut pos = 12;
    let mut fmt = None;
    while pos + 8 <= d.len() {
        let id = &d[pos..pos + 4];
        let size = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let body_end = (body + size).min(d.len());
        if id == b"fmt " && size >= 16 {
            let mut tag = u16_at(body);
            if tag == 0xFFFE && size >= 26 {
                tag = u16_at(body + 24); // WAVE_FORMAT_EXTENSIBLE subformat
            }
            fmt = Some((tag, u16_at(body + 2) as usize, u32_at(body + 4), u16_at(body + 14)));
        } else if id == b"data" {
            let (tag, ch, rate, bits) = fmt?;
            let bytes = (bits / 8) as usize;
            if ch == 0 || bytes == 0 {
                return None;
            }
            let sample = |o: usize| -> f32 {
                let s = &d[o..o + bytes];
                match (tag, bits) {
                    (1, 8) => (s[0] as f32 - 128.0) / 128.0,
                    (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                    (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
                    (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
                    (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                    _ => 0.0,
                }
            };
            if !matches!((tag, bits), (1, 8 | 16 | 24 | 32) | (3, 32)) {
                return None;
            }
            let frame = bytes * ch;
            let n = (body_end - body) / frame;
            let mut stereo = Vec::with_capacity(n * 2);
            for i in 0..n {
                let o = body + i * frame;
                let l = sample(o);
                let r = if ch > 1 { sample(o + bytes) } else { l };
                stereo.push(l);
                stereo.push(r);
            }
            return Some(Wav { rate, stereo });
        }
        pos = body + size + (size & 1);
    }
    None
}

fn ffmpeg(path: &Path, rate: u32) -> Result<Vec<f32>, String> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-ac", "2", "-ar", &rate.to_string(), "-"])
        .output()
        .map_err(|e| format!("{}: cannot run ffmpeg to decode it ({e})", path.display()))?;
    if !out.status.success() {
        return Err(format!("{}: ffmpeg: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

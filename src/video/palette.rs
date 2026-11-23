//! The 256-colour palette shared by every frame of the video.
//!
//! Built from a histogram of all rendered frames: exact when the video has at
//! most 256 colours (typical for text), median cut with ordered dithering
//! otherwise (gradients, images).

use std::collections::HashMap;

use crate::graphics::canvas::Rgb;

fn pack(c: Rgb) -> u32 {
    (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

fn unpack(v: u32) -> Rgb {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

#[derive(Default)]
pub struct Histogram {
    counts: HashMap<u32, u64>,
}

impl Histogram {
    /// Counts the pixels of one frame, `weight` times (for repeated frames).
    pub fn add(&mut self, pixels: &[Rgb], weight: u64) {
        let mut last = None;
        let mut run = 0u64;
        for &p in pixels {
            let v = pack(p);
            if Some(v) == last {
                run += 1;
            } else {
                if let Some(l) = last {
                    *self.counts.entry(l).or_default() += run * weight;
                }
                last = Some(v);
                run = 1;
            }
        }
        if let Some(l) = last {
            *self.counts.entry(l).or_default() += run * weight;
        }
    }
}

pub struct Palette {
    colors: Vec<Rgb>,
    /// Exact colour to index: every colour of the video when there are at
    /// most 256, otherwise just the palette's own colours.
    index: HashMap<u32, u8>,
    /// Nearest palette entry of each 15-bit colour, for ordered dithering
    /// (only when the video has more than 256 colours).
    dither: Option<Vec<u8>>,
}

/// 8x8 Bayer matrix (0..63).
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// How far (in 8-bit steps) dithering may move a colour.
const DITHER_SPREAD: f32 = 24.0;

impl Palette {
    pub fn build(hist: &Histogram) -> Palette {
        let entries: Vec<(u32, u64)> = hist.counts.iter().map(|(&c, &n)| (c, n)).collect();
        if entries.len() <= 256 {
            let colors: Vec<Rgb> = entries.iter().map(|&(c, _)| unpack(c)).collect();
            let index = colors.iter().enumerate().map(|(i, &c)| (pack(c), i as u8)).collect();
            return Palette { colors, index, dither: None };
        }
        eprintln!("dviavi: {} colours reduced to 256 (ordered dithering)", entries.len());
        let colors = median_cut(entries);
        let index = colors.iter().enumerate().map(|(i, &c)| (pack(c), i as u8)).collect();
        let lut = (0..32768u32)
            .map(|v| {
                let c = [((v >> 10) & 31) as u8, ((v >> 5) & 31) as u8, (v & 31) as u8].map(|x| x << 3 | x >> 2);
                nearest(&colors, c)
            })
            .collect();
        Palette { colors, index, dither: Some(lut) }
    }

    pub fn colors(&self) -> &[Rgb] {
        &self.colors
    }

    /// Palette index of every pixel (rows of `width`).
    pub fn map(&self, pixels: &[Rgb], width: usize, out: &mut Vec<u8>) {
        out.clear();
        let mut last: Option<(Rgb, u8)> = None;
        for (i, &p) in pixels.iter().enumerate() {
            let idx = match (self.index.get(&pack(p)), &self.dither) {
                (Some(&k), _) => k,
                (None, Some(lut)) => {
                    let (x, y) = (i % width, i / width);
                    let d = (BAYER[y & 7][x & 7] as f32 + 0.5) / 64.0 - 0.5;
                    let q = |c: u8| ((c as f32 + d * DITHER_SPREAD).clamp(0.0, 255.0) as u32) >> 3;
                    lut[(q(p[0]) << 10 | q(p[1]) << 5 | q(p[2])) as usize]
                }
                (None, None) => match last {
                    Some((c, k)) if c == p => k,
                    _ => {
                        let k = nearest(&self.colors, p);
                        last = Some((p, k));
                        k
                    }
                },
            };
            out.push(idx);
        }
    }
}

fn nearest(colors: &[Rgb], c: Rgb) -> u8 {
    let d = |p: &Rgb| (0..3).map(|k| (p[k] as i32 - c[k] as i32).pow(2)).sum::<i32>();
    colors.iter().enumerate().min_by_key(|(_, p)| d(p)).map(|(i, _)| i as u8).unwrap_or(0)
}

fn median_cut(entries: Vec<(u32, u64)>) -> Vec<Rgb> {
    let mut boxes = vec![entries];
    while boxes.len() < 256 {
        // split the box with the largest weighted extent
        let score = |b: &Vec<(u32, u64)>| -> (u64, usize) {
            if b.len() < 2 {
                return (0, 0);
            }
            let (ch, range) = widest(b);
            (range as u64 * b.iter().map(|e| e.1).sum::<u64>(), ch)
        };
        let (bi, (s, ch)) = boxes.iter().map(score).enumerate().max_by_key(|(_, s)| s.0).unwrap();
        if s == 0 {
            break;
        }
        let mut b = boxes.swap_remove(bi);
        b.sort_by_key(|e| unpack(e.0)[ch]);
        let total: u64 = b.iter().map(|e| e.1).sum();
        let mut acc = 0;
        let mut cut = 1;
        for (i, e) in b.iter().enumerate() {
            acc += e.1;
            if acc * 2 >= total {
                cut = (i + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let hi = b.split_off(cut);
        boxes.push(b);
        boxes.push(hi);
    }
    boxes
        .iter()
        .map(|b| {
            let n: u64 = b.iter().map(|e| e.1).sum::<u64>().max(1);
            let mut s = [0u64; 3];
            for e in b {
                let c = unpack(e.0);
                for k in 0..3 {
                    s[k] += c[k] as u64 * e.1;
                }
            }
            [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
        })
        .collect()
}

fn widest(b: &[(u32, u64)]) -> (usize, u8) {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for e in b {
        let c = unpack(e.0);
        for k in 0..3 {
            lo[k] = lo[k].min(c[k]);
            hi[k] = hi[k].max(c[k]);
        }
    }
    (0..3).map(|k| (k, hi[k] - lo[k])).max_by_key(|x| x.1).unwrap()
}

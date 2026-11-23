//! MS-RLE (BI_RLE8) encoding of palette-indexed frames.
//!
//! Frames are bottom-up bitmaps. A delta frame encodes only the pixels that
//! differ from the previous frame and skips the rest with the RLE8 delta
//! escape, so an unchanged frame is the two bytes "end of bitmap".

/// Encodes `cur` (top-down rows of `width`); with `prev`, only changed pixels.
pub fn encode(cur: &[u8], prev: Option<&[u8]>, width: usize, height: usize, out: &mut Vec<u8>) {
    out.clear();
    let mut cursor = Cursor { x: 0, line: 0 };
    for line in 0..height {
        let row_index = height - 1 - line;
        let row = &cur[row_index * width..(row_index + 1) * width];
        let spans: Vec<(usize, usize)> = match prev {
            None => vec![(0, width)],
            Some(p) => changed_spans(row, &p[row_index * width..(row_index + 1) * width]),
        };
        for (s, e) in spans {
            cursor.move_to(s, line, out);
            encode_span(&row[s..e], out);
            cursor.x = e;
        }
    }
    out.extend_from_slice(&[0, 1]);
}

struct Cursor {
    x: usize,
    line: usize,
}

impl Cursor {
    fn move_to(&mut self, x: usize, line: usize, out: &mut Vec<u8>) {
        if line > self.line && x < self.x {
            out.extend_from_slice(&[0, 0]); // end of line: to x = 0 of the next line
            self.x = 0;
            self.line += 1;
        }
        while line > self.line || x > self.x {
            let dx = (x - self.x).min(255);
            let dy = (line - self.line).min(255);
            out.extend_from_slice(&[0, 2, dx as u8, dy as u8]);
            self.x += dx;
            self.line += dy;
        }
    }
}

/// Differing ranges of a row; gaps shorter than a delta escape are merged.
fn changed_spans(row: &[u8], prev: &[u8]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut x = 0;
    while x < row.len() {
        if row[x] == prev[x] {
            x += 1;
            continue;
        }
        let s = x;
        while x < row.len() && row[x] != prev[x] {
            x += 1;
        }
        match spans.last_mut() {
            Some(last) if s - last.1 < 6 => last.1 = x,
            _ => spans.push((s, x)),
        }
    }
    spans
}

/// Runs (count, value) for repeats, absolute mode for literal stretches.
fn encode_span(px: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    let mut lit_start = 0;
    let flush_literal = |from: usize, to: usize, out: &mut Vec<u8>| {
        let mut a = from;
        while a < to {
            let n = (to - a).min(255);
            if n < 3 {
                for &p in &px[a..a + n] {
                    out.extend_from_slice(&[1, p]);
                }
            } else {
                out.extend_from_slice(&[0, n as u8]);
                out.extend_from_slice(&px[a..a + n]);
                if n % 2 == 1 {
                    out.push(0);
                }
            }
            a += n;
        }
    };
    while i < px.len() {
        let mut j = i + 1;
        while j < px.len() && px[j] == px[i] && j - i < 255 {
            j += 1;
        }
        if j - i >= 3 {
            flush_literal(lit_start, i, out);
            out.extend_from_slice(&[(j - i) as u8, px[i]]);
            lit_start = j;
        }
        i = j;
    }
    flush_literal(lit_start, px.len(), out);
}

#[cfg(test)]
mod tests {
    use super::encode;

    /// Reference RLE8 decoder (top-down output), updating `img` in place.
    fn decode(data: &[u8], img: &mut [u8], w: usize, h: usize) {
        let (mut x, mut line, mut i) = (0usize, 0usize, 0usize);
        let mut put = |x: usize, line: usize, v: u8, img: &mut [u8]| img[(h - 1 - line) * w + x] = v;
        while i < data.len() {
            let (a, b) = (data[i], data[i + 1]);
            i += 2;
            if a > 0 {
                for _ in 0..a {
                    put(x, line, b, img);
                    x += 1;
                }
            } else {
                match b {
                    0 => {
                        x = 0;
                        line += 1;
                    }
                    1 => return,
                    2 => {
                        x += data[i] as usize;
                        line += data[i + 1] as usize;
                        i += 2;
                    }
                    n => {
                        for k in 0..n as usize {
                            put(x, line, data[i + k], img);
                            x += 1;
                        }
                        i += n as usize + (n as usize & 1);
                    }
                }
            }
        }
        panic!("no end of bitmap");
    }

    fn image(w: usize, h: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..w * h)
            .map(|i| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
                if i % 7 < 3 { 5 } else { (s >> 16) as u8 % 4 }
            })
            .collect()
    }

    #[test]
    fn key_frame_round_trip() {
        let (w, h) = (300, 7);
        let img = image(w, h, 1);
        let mut out = Vec::new();
        encode(&img, None, w, h, &mut out);
        let mut got = vec![99; w * h];
        decode(&out, &mut got, w, h);
        assert_eq!(got, img);
    }

    #[test]
    fn delta_frame_round_trip() {
        let (w, h) = (600, 300);
        let prev = image(w, h, 1);
        let mut cur = prev.clone();
        // scattered changes, a wide change and a change right at a row end
        for (x, y) in [(3, 0), (599, 0), (0, 299), (10, 150), (11, 150), (300, 299)] {
            cur[y * w + x] ^= 0x40;
        }
        for x in 100..580 {
            cur[200 * w + x] = 9;
        }
        let mut out = Vec::new();
        encode(&cur, Some(&prev), w, h, &mut out);
        let mut got = prev.clone();
        decode(&out, &mut got, w, h);
        assert_eq!(got, cur);
        encode(&cur, Some(&cur), w, h, &mut out);
        assert_eq!(out, [0, 1], "an unchanged frame is just end-of-bitmap");
    }
}

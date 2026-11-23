//! The frame being drawn: an opaque tiny-skia pixmap (alpha is always 255,
//! so the premultiplied channels are plain RGB).

use tiny_skia::{Color, Mask, Pixmap};

use crate::fonts::Bitmap;

pub type Rgb = [u8; 3];

pub struct Canvas {
    pixmap: Pixmap,
    /// RGB copy for the palette and the encoder, refreshed by `rgb`.
    rgb: Vec<Rgb>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Canvas {
        let pixmap = Pixmap::new(width as u32, height as u32).expect("frame size");
        Canvas { pixmap, rgb: Vec::new() }
    }

    pub fn width(&self) -> usize {
        self.pixmap.width() as usize
    }

    pub fn height(&self) -> usize {
        self.pixmap.height() as usize
    }

    pub fn clear(&mut self, bg: Rgb) {
        self.pixmap.fill(Color::from_rgba8(bg[0], bg[1], bg[2], 255));
    }

    pub fn pixmap_mut(&mut self) -> &mut Pixmap {
        &mut self.pixmap
    }

    /// The pixels as RGB, top-down rows.
    pub fn rgb(&mut self) -> &[Rgb] {
        self.rgb.clear();
        self.rgb.extend(self.pixmap.data().chunks_exact(4).map(|p| [p[0], p[1], p[2]]));
        &self.rgb
    }

    /// Composites a glyph's coverage at pen position (x, y) in `color`,
    /// scaled by `alpha` and the clip `mask`.
    pub fn blit(&mut self, b: &Bitmap, x: i64, y: i64, color: Rgb, alpha: f32, mask: Option<&Mask>) {
        let (w, h) = (self.width() as i64, self.height() as i64);
        let data = self.pixmap.data_mut();
        let a_scale = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
        for r in 0..b.rows {
            let py = y - b.top as i64 + r as i64;
            if py < 0 || py >= h {
                continue;
            }
            for col in 0..b.width {
                let px = x + b.left as i64 + col as i64;
                if px < 0 || px >= w {
                    continue;
                }
                let mut a = b.coverage[r * b.width + col] as u32 * a_scale / 255;
                if let Some(m) = mask {
                    a = a * m.data()[(py * w + px) as usize] as u32 / 255;
                }
                if a == 0 {
                    continue;
                }
                let i = ((py * w + px) * 4) as usize;
                for k in 0..3 {
                    data[i + k] = ((data[i + k] as u32 * (255 - a) + color[k] as u32 * a + 127) / 255) as u8;
                }
            }
        }
    }
}

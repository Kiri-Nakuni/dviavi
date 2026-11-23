//! Font instances used by a DVI file and their glyph bitmaps.
//!
//! [`Fonts`] is built once before rendering (every DVI and VF font is resolved
//! up front) and is read-only afterwards; [`GlyphCache`] holds the mutable
//! rasterisation cache on top of it.

use std::collections::HashMap;
use std::rc::Rc;

use freetype::face::LoadFlag;
use freetype::{Face, Library};

use crate::tex::fontmap::{self, CodeMap, FontMap, MapEntry};
use crate::tex::tfm::{self, JfmDir, Metrics};
use crate::tex::vf::Vf;
use crate::tex::kpse;
use crate::warn::warn_once;

pub type FontId = usize;

/// One font at one size.
pub struct Font {
    /// At-size in DVI units.
    pub scale: i32,
    metrics: Rc<Metrics>,
    pub kind: FontKind,
}

pub enum FontKind {
    Real(Option<RealFont>),
    Virtual { vf: Rc<Vf>, local: HashMap<u32, FontId> },
}

pub struct RealFont {
    face: usize,
    glyphs: GlyphNames,
    entry: MapEntry,
}

enum GlyphNames {
    CharCodes,
    Names(Rc<Vec<String>>),
    Jis,
    Unicode,
}

impl Font {
    /// Advance width of `code` in DVI units.
    pub fn advance(&self, code: u32) -> i32 {
        tfm::scale(self.metrics.width_fix(code), self.scale)
    }

    pub fn jfm_dir(&self) -> Option<JfmDir> {
        self.metrics.jfm_dir()
    }
}

pub struct Fonts {
    library: Library,
    map: FontMap,
    list: Vec<Font>,
    by_key: HashMap<(String, i32), FontId>,
    metrics: HashMap<String, Option<Rc<Metrics>>>,
    vfs: HashMap<String, Option<Rc<Vf>>>,
    encs: HashMap<String, Rc<Vec<String>>>,
    faces: Vec<Face>,
    face_by_file: HashMap<(String, isize, bool), usize>,
}

impl Fonts {
    pub fn new() -> Result<Fonts, String> {
        Ok(Fonts {
            library: Library::init().map_err(|e| format!("FreeType: {e}"))?,
            map: FontMap::load(),
            list: Vec::new(),
            by_key: HashMap::new(),
            metrics: HashMap::new(),
            vfs: HashMap::new(),
            encs: HashMap::new(),
            faces: Vec::new(),
            face_by_file: HashMap::new(),
        })
    }

    pub fn get(&self, id: FontId) -> &Font {
        &self.list[id]
    }

    /// Resolves `name` at `scale` (DVI units), loading virtual fonts recursively.
    pub fn load(&mut self, name: &str, scale: i32) -> FontId {
        if let Some(&id) = self.by_key.get(&(name.to_string(), scale)) {
            return id;
        }
        let metrics = self.metrics_of(name).unwrap_or_else(|| {
            warn_once(format!("font metrics {name}.tfm not found"));
            Rc::new(Metrics::parse(&[0; 64]).expect("empty metrics"))
        });
        let kind = match self.vf_of(name) {
            Some(vf) => {
                let mut local = HashMap::new();
                for def in &vf.fonts {
                    let id = self.load(&def.name, tfm::scale(def.scale_fix, scale));
                    local.insert(def.number, id);
                }
                FontKind::Virtual { vf, local }
            }
            None => FontKind::Real(self.real_font(name)),
        };
        let id = self.list.len();
        self.list.push(Font { scale, metrics, kind });
        self.by_key.insert((name.to_string(), scale), id);
        id
    }

    fn metrics_of(&mut self, name: &str) -> Option<Rc<Metrics>> {
        if let Some(m) = self.metrics.get(name) {
            return m.clone();
        }
        let m = kpse::find(&format!("{name}.tfm"))
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|d| match Metrics::parse(&d) {
                Ok(m) => Some(Rc::new(m)),
                Err(e) => {
                    warn_once(format!("{name}.tfm: {e}"));
                    None
                }
            });
        self.metrics.insert(name.to_string(), m.clone());
        m
    }

    fn vf_of(&mut self, name: &str) -> Option<Rc<Vf>> {
        if let Some(v) = self.vfs.get(name) {
            return v.clone();
        }
        let v = kpse::find(&format!("{name}.vf"))
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|d| match Vf::parse(&d) {
                Ok(v) => Some(Rc::new(v)),
                Err(e) => {
                    warn_once(format!("{name}.vf: {e}"));
                    None
                }
            });
        self.vfs.insert(name.to_string(), v.clone());
        v
    }

    fn real_font(&mut self, name: &str) -> Option<RealFont> {
        let Some(entry) = self.map.get(name).cloned() else {
            warn_once(format!("font {name} is not in pdftex.map / kanjix.map; its characters are not drawn"));
            return None;
        };
        let glyphs = match &entry.code_map {
            CodeMap::BuiltIn => GlyphNames::CharCodes,
            CodeMap::EncFile(enc) => GlyphNames::Names(self.enc_of(enc)?),
            CodeMap::Jis => GlyphNames::Jis,
            CodeMap::Unicode => GlyphNames::Unicode,
        };
        let builtin = matches!(glyphs, GlyphNames::CharCodes);
        let key = (entry.file.clone(), entry.face_index, builtin);
        let face = match self.face_by_file.get(&key) {
            Some(&f) => f,
            None => {
                let Some(path) = kpse::find(&entry.file) else {
                    warn_once(format!("font file {} (for {name}) not found", entry.file));
                    return None;
                };
                let face = match self.library.new_face(&path, entry.face_index) {
                    Ok(f) => f,
                    Err(e) => {
                        warn_once(format!("{}: {e}", path.display()));
                        return None;
                    }
                };
                if builtin {
                    select_builtin_charmap(&face);
                }
                self.faces.push(face);
                self.face_by_file.insert(key, self.faces.len() - 1);
                self.faces.len() - 1
            }
        };
        Some(RealFont { face, glyphs, entry })
    }

    fn enc_of(&mut self, file: &str) -> Option<Rc<Vec<String>>> {
        if let Some(e) = self.encs.get(file) {
            return Some(e.clone());
        }
        let Some(text) = kpse::find(file).and_then(|p| std::fs::read(p).ok()) else {
            warn_once(format!("encoding file {file} not found"));
            return None;
        };
        let names = Rc::new(fontmap::parse_enc(&String::from_utf8_lossy(&text)));
        self.encs.insert(file.to_string(), names.clone());
        Some(names)
    }
}

/// Type 1 fonts used without an `.enc` file are addressed by their own encoding.
fn select_builtin_charmap(face: &Face) {
    for i in 0..face.num_charmaps() {
        let cm = face.get_charmap(i as isize);
        if cm.encoding() == freetype::ffi::FT_ENCODING_ADOBE_CUSTOM {
            let _ = face.set_charmap(&cm);
            return;
        }
    }
}

/// How a glyph is placed relative to the DVI reference point.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub enum Placement {
    /// Ordinary horizontal text.
    Upright,
    /// Latin text in a tate box: turned 90° clockwise.
    Rotated,
    /// A CJK glyph of a vertical font: upright, hung from the reference point.
    Vertical,
}

/// An 8-bit coverage bitmap and its offset from the pen position (pixels, y down).
pub struct Bitmap {
    pub width: usize,
    pub rows: usize,
    pub left: i32,
    pub top: i32,
    pub coverage: Vec<u8>,
}

/// A linear map of pixel space (y down) applied to a glyph, as
/// `[sx, ky, kx, sy]`: x' = sx x + kx y, y' = ky x + sy y.
pub type Linear = [f32; 4];

/// Rasterised glyphs for one output resolution.
pub struct GlyphCache {
    px_per_unit: f64,
    /// Keyed also by the linear map, in 1/4096 steps.
    cache: HashMap<(FontId, u32, Placement, [i32; 4]), Option<Rc<Bitmap>>>,
}

impl GlyphCache {
    pub fn new(px_per_unit: f64) -> GlyphCache {
        GlyphCache { px_per_unit, cache: HashMap::new() }
    }

    /// The bitmap of `code` in the real font `id` drawn through `linear`
    /// (rotation and scaling from graphics), or `None` if it has no glyph.
    pub fn glyph(&mut self, fonts: &Fonts, id: FontId, code: u32, placement: Placement, linear: Linear) -> Option<Rc<Bitmap>> {
        let q = linear.map(|v| (v * 4096.0).round() as i32);
        let key = (id, code, placement, q);
        if let Some(b) = self.cache.get(&key) {
            return b.clone();
        }
        let linear = q.map(|v| v as f32 / 4096.0);
        let b = rasterize(fonts, id, code, placement, linear, self.px_per_unit).map(Rc::new);
        self.cache.insert(key, b.clone());
        b
    }
}

fn rasterize(fonts: &Fonts, id: FontId, code: u32, placement: Placement, linear: Linear, px_per_unit: f64) -> Option<Bitmap> {
    let font = fonts.get(id);
    let FontKind::Real(Some(real)) = &font.kind else { return None };
    let face = &fonts.faces[real.face];
    let gid = match &real.glyphs {
        GlyphNames::CharCodes => face.get_char_index(code as usize)?,
        GlyphNames::Names(names) => face.get_name_index(names.get(code as usize)?)?,
        GlyphNames::Unicode => face.get_char_index(code as usize)?,
        GlyphNames::Jis => face.get_char_index(jis_to_unicode(code)? as usize)?,
    };
    let px = font.scale as f64 * px_per_unit;
    face.set_char_size(0, (px * 64.0).round().max(1.0) as isize, 72, 72).ok()?;
    let fx = |v: f64| (v * 65536.0).round() as freetype::ffi::FT_Fixed;
    let (e, s) = (real.entry.extend, real.entry.slant);
    // x' = e*x + s*y, y' = y; then optionally turned clockwise: (x, y) -> (y, -x)
    let p: [f64; 4] = match placement {
        Placement::Rotated => [0.0, 1.0, -e, -s],
        _ => [e, s, 0.0, 1.0],
    };
    // the graphics map in FreeType's y-up space, applied after `p`
    let [sx, ky, kx, sy] = linear.map(|v| v as f64);
    let g = [sx, -kx, -ky, sy];
    let c = [g[0] * p[0] + g[1] * p[2], g[0] * p[1] + g[1] * p[3], g[2] * p[0] + g[3] * p[2], g[2] * p[1] + g[3] * p[3]];
    let mut m = freetype::Matrix { xx: fx(c[0]), xy: fx(c[1]), yx: fx(c[2]), yy: fx(c[3]) };
    let mut delta = freetype::Vector { x: 0, y: 0 };
    face.set_transform(&mut m, &mut delta);
    face.load_glyph(gid, LoadFlag::RENDER | LoadFlag::NO_HINTING).ok()?;
    let slot = face.glyph();
    let bm = slot.bitmap();
    let (width, rows, pitch) = (bm.width() as usize, bm.rows() as usize, bm.pitch());
    let buf = bm.buffer();
    let mut coverage = vec![0u8; width * rows];
    for r in 0..rows {
        let src = if pitch >= 0 { r * pitch as usize } else { (rows - 1 - r) * (-pitch) as usize };
        coverage[r * width..(r + 1) * width].copy_from_slice(&buf[src..src + width]);
    }
    let (mut left, mut top) = (slot.bitmap_left(), slot.bitmap_top());
    if placement == Placement::Vertical {
        // the em box hangs below the reference point, centred on it
        let (ox, oy) = (-px / 2.0, px * 0.88);
        left += (sx * ox + kx * oy).round() as i32;
        top -= (ky * ox + sy * oy).round() as i32;
    }
    Some(Bitmap { width, rows, left, top, coverage })
}

/// JIS X 0208 row/cell code (as pTeX writes it) to a Unicode scalar.
fn jis_to_unicode(code: u32) -> Option<char> {
    let bytes = [((code >> 8) as u8) | 0x80, (code as u8) | 0x80];
    let (s, _, bad) = encoding_rs::EUC_JP.decode(&bytes);
    if bad { None } else { s.chars().next() }
}

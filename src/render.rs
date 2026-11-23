//! Drawing DVI pages: glyphs and rules from the DVI commands, colours and
//! graphics from the specials dvipdfmx understands (so that graphicx, xcolor
//! and PGF with the dvipdfmx driver draw as in the PDF).

use std::collections::HashMap;
use std::path::PathBuf;

use tiny_skia::{FilterQuality, PixmapPaint, Point, Transform};

use crate::fonts::{FontId, FontKind, Fonts, GlyphCache, Placement};
use crate::graphics::canvas::{Canvas, Rgb};
use crate::graphics::content::{Graphics, Resources};
use crate::graphics::image::Images;
use crate::graphics::pdfobj::{Lexer, Obj};
use crate::tex::dvi::{Decoder, Dvi, Op};
use crate::tex::tfm::{self, JfmDir};
use crate::warn::warn_once;

/// Colour state carried from page to page (dvips semantics: the colour stack
/// and the page background persist until changed).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ColorState {
    stack: Vec<Rgb>,
    background: Rgb,
}

impl ColorState {
    pub fn new() -> ColorState {
        ColorState { stack: vec![[0; 3]], background: [255; 3] }
    }

    fn current(&self) -> Rgb {
        *self.stack.last().unwrap_or(&[0; 3])
    }

    /// Applies a colour special; returns false if `s` is not one.
    fn apply(&mut self, s: &str) -> bool {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("color") {
            let rest = rest.trim();
            if rest == "pop" {
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
            } else if let Some(spec) = rest.strip_prefix("push") {
                let c = parse_color(spec).unwrap_or_else(|| self.current());
                self.stack.push(c);
            } else if let Some(c) = parse_color(rest) {
                self.stack.clear();
                self.stack.push(c);
            }
            true
        } else if let Some(spec) = s.strip_prefix("background") {
            if let Some(c) = parse_color(spec) {
                self.background = c;
            }
            true
        } else if let Some(rest) = s.strip_prefix("pdfcolorstack") {
            // dvipdfmx colour stacks: `N push (ops)`, `N pop`, `N set (ops)`
            let mut w = rest.split_whitespace();
            let _stack = w.next();
            match w.next() {
                Some("push") => {
                    let c = parse_pdf_ops(rest).unwrap_or_else(|| self.current());
                    self.stack.push(c);
                }
                Some("pop") => {
                    if self.stack.len() > 1 {
                        self.stack.pop();
                    }
                }
                Some("set") => {
                    if let Some(c) = parse_pdf_ops(rest) {
                        *self.stack.last_mut().unwrap() = c;
                    }
                }
                _ => {}
            }
            true
        } else if s.starts_with("pdfcolorstackinit") {
            true
        } else if let Some(arr) = pdf_command(s, "bcolor") {
            let c = parse_pdf_color(arr).unwrap_or_else(|| self.current());
            self.stack.push(c);
            true
        } else if pdf_command(s, "ecolor").is_some() {
            if self.stack.len() > 1 {
                self.stack.pop();
            }
            true
        } else if let Some(arr) = pdf_command(s, "bgcolor") {
            if let Some(c) = parse_pdf_color(arr) {
                self.background = c;
            }
            true
        } else {
            false
        }
    }
}

/// `pdf:NAME rest` (dvipdfmx allows a space after the colon): the rest.
fn pdf_command<'s>(s: &'s str, name: &str) -> Option<&'s str> {
    let rest = s.strip_prefix("pdf:")?.trim_start();
    let rest = rest.strip_prefix(name)?;
    (rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || c == '[' || c == '<' || c == '(' || c == '@'))
        .then_some(rest.trim_start())
}

fn to_byte(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn from_values(v: &[f64]) -> Option<Rgb> {
    match v {
        [g] => Some([to_byte(*g); 3]),
        [r, g, b] => Some([to_byte(*r), to_byte(*g), to_byte(*b)]),
        [c, m, y, k] => Some([
            to_byte((1.0 - c) * (1.0 - k)),
            to_byte((1.0 - m) * (1.0 - k)),
            to_byte((1.0 - y) * (1.0 - k)),
        ]),
        _ => None,
    }
}

/// `rgb r g b`, `cmyk c m y k`, `gray g`, `hsb h s b`.
fn parse_color(spec: &str) -> Option<Rgb> {
    let mut words = spec.split_whitespace();
    let model = words.next()?;
    let v: Vec<f64> = words.filter_map(|w| w.parse().ok()).collect();
    match (model, v.as_slice()) {
        ("rgb", [_, _, _]) | ("cmyk", [_, _, _, _]) | ("gray", [_]) => from_values(&v),
        ("hsb", [h, s, b]) => {
            let i = (h * 6.0).floor();
            let f = h * 6.0 - i;
            let (p, q, t) = (b * (1.0 - s), b * (1.0 - s * f), b * (1.0 - s * (1.0 - f)));
            let (r, g, bl) = match i as i64 % 6 {
                0 => (*b, t, p),
                1 => (q, *b, p),
                2 => (p, *b, t),
                3 => (p, q, *b),
                4 => (t, p, *b),
                _ => (*b, p, q),
            };
            from_values(&[r, g, bl])
        }
        _ => {
            warn_once(format!("colour `{}' is not supported", spec.trim()));
            None
        }
    }
}

/// dvipdfmx `[r g b]`, `[c m y k]`, `[g]` (optionally followed by a fill colour).
fn parse_pdf_color(s: &str) -> Option<Rgb> {
    let s = s.trim();
    let inner = s.strip_prefix('[')?.split(']').next()?;
    let v: Vec<f64> = inner.split_whitespace().filter_map(|w| w.parse().ok()).collect();
    from_values(&v)
}

/// The fill colour of PDF operators in parentheses: `(0 g 0 G)`,
/// `(1 0 0 rg 1 0 0 RG)`, `(0 0 0 1 k 0 0 0 1 K)`.
fn parse_pdf_ops(s: &str) -> Option<Rgb> {
    let inner = s.split_once('(')?.1.split(')').next()?;
    let mut nums = Vec::new();
    for w in inner.split_whitespace() {
        match w {
            "g" | "rg" | "k" => return from_values(&nums),
            _ => match w.parse::<f64>() {
                Ok(v) => nums.push(v),
                Err(_) => nums.clear(),
            },
        }
    }
    None
}

/// A TeX dimension (`12pt`, `3.5bp`, `1in`, ...) in bp.
fn dimen_bp(s: &str) -> Option<f32> {
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(s.len());
    let v: f32 = s[..end].parse().ok()?;
    let unit = s[end..].trim().trim_start_matches("true");
    let k = match unit {
        "" | "bp" => 1.0,
        "pt" => 72.0 / 72.27,
        "mm" => 72.0 / 25.4,
        "cm" => 72.0 / 2.54,
        "in" => 72.0,
        "pc" => 12.0 * 72.0 / 72.27,
        "dd" => 1238.0 / 1157.0 * 72.0 / 72.27,
        "cc" => 12.0 * 1238.0 / 1157.0 * 72.0 / 72.27,
        "sp" => 72.0 / 72.27 / 65536.0,
        _ => return None,
    };
    Some(v * k)
}

/// Something drawn inside a form XObject (`pdf:bxobj`), positioned in
/// pixels relative to where the form started.
#[derive(Clone)]
enum Event {
    Glyph { font: FontId, code: u32, placement: Placement, x: f64, y: f64 },
    Rule { x0: f64, y0: f64, x1: f64, y1: f64 },
    Special { text: String, x: f64, y: f64 },
}

enum XObject {
    Form { events: Vec<Event>, bbox: [f32; 4] },
    Image { file: String, page: u32, w: f32, h: f32, natural_w: f32 },
}

struct Capture {
    name: String,
    start: (f64, f64),
    bbox: [f32; 4],
    events: Vec<Event>,
}

/// Maps DVI coordinates to pixels and draws pages.
pub struct Renderer<'a> {
    dvi: &'a Dvi,
    fonts: &'a Fonts,
    glyphs: GlyphCache,
    /// Font number in the DVI file to loaded font.
    dvi_fonts: HashMap<u32, FontId>,
    px_per_unit: f64,
    origin_px: (f64, f64),
    /// User space (bp, y up, paper bottom-left) to pixels.
    device: Transform,
    // document-wide: pdf:obj objects, forms and named images
    objects: HashMap<String, Obj>,
    xobjects: HashMap<String, XObject>,
    images: Images,
    // per page
    gfx: Graphics,
    /// Resource dictionaries: the page's, then those of forms being drawn.
    scopes: Vec<Obj>,
    captures: Vec<Capture>,
}

#[derive(Clone, Copy, Default)]
struct Regs {
    /// Physical position in DVI units, x right and y down from the DVI origin.
    x: i32,
    y: i32,
    w: i32,
    xr: i32,
    yr: i32,
    z: i32,
    tate: bool,
}

impl<'a> Renderer<'a> {
    /// `px_per_m` is the output resolution and `paper_h_m` the paper height;
    /// the DVI origin is one true inch from the top-left corner.
    pub fn new(dvi: &'a Dvi, fonts: &'a Fonts, dvi_fonts: HashMap<u32, FontId>, px_per_m: f64, paper_h_m: f64, base: PathBuf) -> Renderer<'a> {
        let px_per_unit = dvi.unit_m * px_per_m;
        let inch = 0.0254 * px_per_m;
        let s = (px_per_m * 0.0254 / 72.0 * dvi.mag) as f32;
        let device = Transform::from_row(s, 0.0, 0.0, -s, 0.0, (paper_h_m * px_per_m) as f32);
        Renderer {
            dvi,
            fonts,
            glyphs: GlyphCache::new(px_per_unit),
            dvi_fonts,
            px_per_unit,
            origin_px: (inch, inch),
            device,
            objects: HashMap::new(),
            xobjects: HashMap::new(),
            images: Images::new(base),
            gfx: Graphics::new(device, [0; 3]),
            scopes: Vec::new(),
            captures: Vec::new(),
        }
    }

    /// Draws page `i` into `canvas`, updating `colors` with its colour specials.
    pub fn render(&mut self, i: usize, canvas: &mut Canvas, colors: &mut ColorState) -> Result<(), String> {
        // the background special may come anywhere in the page: find it first
        let mut probe = colors.clone();
        for s in self.dvi.page_specials(i)? {
            probe.apply(&String::from_utf8_lossy(s));
        }
        canvas.clear(probe.background);
        self.gfx = Graphics::new(self.device, colors.current());
        self.scopes = vec![Obj::Dict(HashMap::new())];
        self.captures.clear();
        let mut regs = Regs::default();
        let dvi_fonts = std::mem::take(&mut self.dvi_fonts);
        let r = self.run(self.dvi.page(i), &dvi_fonts, None, &mut regs, canvas, colors);
        self.dvi_fonts = dvi_fonts;
        if !self.captures.is_empty() {
            warn_once("pdf:bxobj without pdf:exobj at the end of a page".into());
        }
        r
    }

    fn to_px(&self, x: i32, y: i32) -> (f64, f64) {
        (x as f64 * self.px_per_unit + self.origin_px.0, y as f64 * self.px_per_unit + self.origin_px.1)
    }

    /// Executes DVI commands; `vf_scale` is `Some(z)` inside a VF packet, whose
    /// dimensions are fix_words of `z`.
    fn run(
        &mut self,
        mut dec: Decoder<'_>,
        font_table: &HashMap<u32, FontId>,
        vf_scale: Option<i32>,
        regs: &mut Regs,
        canvas: &mut Canvas,
        colors: &mut ColorState,
    ) -> Result<(), String> {
        let dim = |d: i32| match vf_scale {
            Some(z) => tfm::scale(d, z),
            None => d,
        };
        let mut stack: Vec<Regs> = Vec::new();
        let mut font: Option<FontId> = None;
        // a VF packet starts with its first local font selected
        if vf_scale.is_some() {
            font = font_table.get(&0).copied().or_else(|| font_table.values().min().copied());
        }
        while let Some(op) = dec.next()? {
            match op {
                Op::Char { code, advance } => {
                    let Some(fid) = font else { continue };
                    let w = self.fonts.get(fid).advance(code);
                    self.draw_char(fid, code, regs, canvas, colors)?;
                    if advance {
                        move_right(regs, w);
                    }
                }
                Op::Rule { height, width, advance } => {
                    let (h, w) = (dim(height), dim(width));
                    if h > 0 && w > 0 {
                        let (x0, y0, x1, y1) = if regs.tate {
                            (regs.x, regs.y, regs.x + h, regs.y + w)
                        } else {
                            (regs.x, regs.y - h, regs.x + w, regs.y)
                        };
                        let (a, b) = (self.to_px(x0, y0), self.to_px(x1, y1));
                        self.rule(canvas, a, b);
                    }
                    if advance {
                        move_right(regs, w);
                    }
                }
                Op::Push => stack.push(*regs),
                Op::Pop => *regs = stack.pop().ok_or("DVI pop without push")?,
                Op::Right(d) => move_right(regs, dim(d)),
                Op::W(d) => {
                    if let Some(d) = d {
                        regs.w = dim(d);
                    }
                    move_right(regs, regs.w);
                }
                Op::X(d) => {
                    if let Some(d) = d {
                        regs.xr = dim(d);
                    }
                    move_right(regs, regs.xr);
                }
                Op::Down(d) => move_down(regs, dim(d)),
                Op::Y(d) => {
                    if let Some(d) = d {
                        regs.yr = dim(d);
                    }
                    move_down(regs, regs.yr);
                }
                Op::Z(d) => {
                    if let Some(d) = d {
                        regs.z = dim(d);
                    }
                    move_down(regs, regs.z);
                }
                Op::Font(n) => {
                    font = font_table.get(&n).copied();
                    if font.is_none() {
                        warn_once(format!("undefined font number {n}"));
                    }
                }
                Op::Special(s) => {
                    let s = String::from_utf8_lossy(s).into_owned();
                    let at = self.to_px(regs.x, regs.y);
                    self.special(&s, at, canvas, colors);
                }
                Op::Dir(d) => {
                    if d == 3 {
                        warn_once("dtou direction is drawn as tate".into());
                    }
                    regs.tate = d != 0;
                }
                Op::Eop => break,
                Op::FontDef { .. } | Op::Nop | Op::Bop => {}
                Op::Post => break,
            }
        }
        Ok(())
    }

    fn draw_char(&mut self, fid: FontId, code: u32, regs: &mut Regs, canvas: &mut Canvas, colors: &mut ColorState) -> Result<(), String> {
        let fonts = self.fonts;
        let font = fonts.get(fid);
        match &font.kind {
            FontKind::Virtual { vf, local } => {
                let Some(packet) = vf.packet(code) else { return Ok(()) };
                // a packet starts with w = x = y = z = 0 (VF format)
                let mut inner = Regs { w: 0, xr: 0, yr: 0, z: 0, ..*regs };
                self.run(Decoder::new(packet, 0), local, Some(font.scale), &mut inner, canvas, colors)
            }
            FontKind::Real(None) => Ok(()),
            FontKind::Real(Some(_)) => {
                let placement = match (regs.tate, font.jfm_dir()) {
                    (false, _) => Placement::Upright,
                    (true, Some(JfmDir::Tate)) => Placement::Vertical,
                    (true, _) => Placement::Rotated,
                };
                let (x, y) = self.to_px(regs.x, regs.y);
                self.glyph(canvas, fid, code, placement, x, y);
                Ok(())
            }
        }
    }

    /// A glyph whose pen position is (x, y) in pixels before any transformation.
    fn glyph(&mut self, canvas: &mut Canvas, font: FontId, code: u32, placement: Placement, x: f64, y: f64) {
        if let Some(c) = self.captures.last_mut() {
            c.events.push(Event::Glyph { font, code, placement, x: x - c.start.0, y: y - c.start.1 });
            return;
        }
        let t = self.gfx.text_transform();
        let mut p = Point::from_xy(x as f32, y as f32);
        t.map_point(&mut p);
        let linear = [t.sx, t.ky, t.kx, t.sy];
        if let Some(b) = self.glyphs.glyph(self.fonts, font, code, placement, linear) {
            let st = self.gfx.state();
            canvas.blit(&b, p.x.round() as i64, p.y.round() as i64, st.fill, st.fill_alpha, st.clip.as_deref());
        }
    }

    fn rule(&mut self, canvas: &mut Canvas, a: (f64, f64), b: (f64, f64)) {
        if let Some(c) = self.captures.last_mut() {
            let (sx, sy) = c.start;
            c.events.push(Event::Rule { x0: a.0 - sx, y0: a.1 - sy, x1: b.0 - sx, y1: b.1 - sy });
            return;
        }
        self.gfx.fill_rule(canvas, a.0, a.1, b.0, b.1);
    }

    /// Runs PDF operators on the page.
    fn code(&mut self, ops: &str, canvas: &mut Canvas) {
        let scopes = std::mem::take(&mut self.scopes);
        let res = Resources { scopes: &scopes, objects: &self.objects };
        self.gfx.run(ops, canvas, &res);
        self.scopes = scopes;
    }

    /// The current point (pixels) in user space, relative to the current origin.
    fn here(&self, at: (f64, f64)) -> (Point, Point) {
        let abs = self.gfx.to_user(at.0, at.1);
        (abs, self.gfx.relative(abs))
    }

    fn special(&mut self, s: &str, at: (f64, f64), canvas: &mut Canvas, colors: &mut ColorState) {
        let t = s.trim();
        // inside pdf:bxobj everything but its end is recorded for pdf:uxobj
        if !self.captures.is_empty() && pdf_command(t, "exobj").is_none() && pdf_command(t, "bxobj").is_none() {
            let c = self.captures.last_mut().unwrap();
            c.events.push(Event::Special { text: t.to_string(), x: at.0 - c.start.0, y: at.1 - c.start.1 });
            return;
        }
        if colors.apply(t) {
            self.gfx.set_color(colors.current());
            return;
        }
        if let Some(rest) = t.strip_prefix("pdf:") {
            let rest = rest.trim_start();
            let (cmd, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let arg = arg.trim_start();
            match cmd {
                "code" => self.code(arg, canvas),
                "literal" => match arg.strip_prefix("direct") {
                    Some(ops) => self.code(ops, canvas),
                    None => {
                        let (_, rel) = self.here(at);
                        self.gfx.concat(Transform::from_translate(rel.x, rel.y));
                        self.code(arg, canvas);
                        self.gfx.concat(Transform::from_translate(-rel.x, -rel.y));
                    }
                },
                "content" => {
                    let (_, rel) = self.here(at);
                    self.gfx.save();
                    self.gfx.concat(Transform::from_translate(rel.x, rel.y));
                    self.code(arg, canvas);
                    self.gfx.restore();
                }
                "bcontent" => {
                    let (abs, rel) = self.here(at);
                    self.gfx.save();
                    self.gfx.concat(Transform::from_translate(rel.x, rel.y));
                    self.gfx.push_origin(abs);
                }
                "econtent" => {
                    self.gfx.pop_origin();
                    self.gfx.restore();
                    self.gfx.set_color(colors.current());
                }
                "btrans" => {
                    let (_, rel) = self.here(at);
                    let m = parse_transform(arg);
                    self.gfx.save();
                    self.gfx.concat(
                        Transform::from_translate(rel.x, rel.y).pre_concat(m).pre_concat(Transform::from_translate(-rel.x, -rel.y)),
                    );
                }
                "etrans" => {
                    self.gfx.restore();
                    self.gfx.set_color(colors.current());
                }
                "bxobj" => self.begin_form(arg, at),
                "exobj" => self.end_form(),
                "uxobj" => self.use_xobject(arg, at, canvas, colors),
                "put" => self.put(arg),
                "obj" => {
                    let mut lex = Lexer::new(arg);
                    if let (Some(Obj::Ref(name)), Some(obj)) = (lex.next_obj(), lex.next_obj()) {
                        self.objects.insert(name, obj);
                    }
                }
                "stream" => {
                    let mut lex = Lexer::new(arg);
                    if let (Some(Obj::Ref(name)), Some(obj)) = (lex.next_obj(), lex.next_obj()) {
                        self.objects.insert(name, obj);
                    }
                }
                "image" => self.image(arg, at, canvas),
                // document structure (links, outlines, metadata ...) has no picture
                _ => {}
            }
        } else if let Some(rest) = t.strip_prefix("x:") {
            let rest = rest.trim();
            let (cmd, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let (_, rel) = self.here(at);
            let around = |m: Transform| {
                Transform::from_translate(rel.x, rel.y).pre_concat(m).pre_concat(Transform::from_translate(-rel.x, -rel.y))
            };
            let v: Vec<f32> = arg.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            match cmd {
                "gsave" => self.gfx.save(),
                "grestore" => {
                    self.gfx.restore();
                    self.gfx.set_color(colors.current());
                }
                "scale" => {
                    let (sx, sy) = (*v.first().unwrap_or(&1.0), *v.get(1).or(v.first()).unwrap_or(&1.0));
                    self.gfx.concat(around(Transform::from_scale(sx, sy)));
                }
                "rotate" => {
                    let a = v.first().copied().unwrap_or(0.0).to_radians();
                    self.gfx.concat(around(Transform::from_row(a.cos(), a.sin(), -a.sin(), a.cos(), 0.0, 0.0)));
                }
                _ => warn_once(format!("special x:{cmd} is not supported")),
            }
        } else if let Some(rest) = t.strip_prefix("PSfile=") {
            self.psfile(rest, at, canvas);
        } else if t.starts_with("ps:") || t.starts_with('"') || t.starts_with('!') {
            warn_once("PostScript specials are not drawn".into());
        }
    }

    /// `pdf:put @resources << ... >>` or `pdf:put @name obj`.
    fn put(&mut self, arg: &str) {
        let mut lex = Lexer::new(arg);
        let (Some(Obj::Ref(name)), Some(obj)) = (lex.next_obj(), lex.next_obj()) else { return };
        match name.as_str() {
            "resources" => {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.merge(obj);
                }
            }
            "thispage" | "thisform" | "catalog" | "names" => {}
            _ => match self.objects.get_mut(&name) {
                Some(o) => o.merge(obj),
                None => {
                    self.objects.insert(name, obj);
                }
            },
        }
    }

    /// `pdf:bxobj @name width W height H depth D` or `... bbox llx lly urx ury`.
    fn begin_form(&mut self, arg: &str, at: (f64, f64)) {
        let mut name = String::new();
        let (mut w, mut h, mut d) = (0.0f32, 0.0f32, 0.0f32);
        let mut bbox = None;
        let mut words = arg.split_whitespace().peekable();
        while let Some(word) = words.next() {
            match word {
                "width" => w = words.next().and_then(dimen_bp).unwrap_or(0.0),
                "height" => h = words.next().and_then(dimen_bp).unwrap_or(0.0),
                "depth" => d = words.next().and_then(dimen_bp).unwrap_or(0.0),
                "bbox" => {
                    let v: Vec<f32> = (0..4).filter_map(|_| words.next().and_then(|x| x.parse().ok())).collect();
                    if v.len() == 4 {
                        bbox = Some([v[0], v[1], v[2], v[3]]);
                    }
                }
                _ if word.starts_with('@') => name = word[1..].to_string(),
                _ => {}
            }
        }
        let bbox = bbox.unwrap_or([0.0, -d, w, h]);
        self.captures.push(Capture { name, start: at, bbox, events: Vec::new() });
    }

    fn end_form(&mut self) {
        let Some(c) = self.captures.pop() else {
            warn_once("pdf:exobj without pdf:bxobj".into());
            return;
        };
        self.xobjects.insert(c.name, XObject::Form { events: c.events, bbox: c.bbox });
    }

    /// `pdf:uxobj @name`: draws a form or a named image at the current point.
    fn use_xobject(&mut self, arg: &str, at: (f64, f64), canvas: &mut Canvas, colors: &mut ColorState) {
        let Some(name) = arg.split_whitespace().next().and_then(|w| w.strip_prefix('@')) else { return };
        match self.xobjects.get(name) {
            Some(XObject::Form { events, bbox }) => {
                let (events, bbox) = (events.clone(), *bbox);
                let (abs, rel) = self.here(at);
                self.gfx.save();
                self.gfx.concat(Transform::from_translate(rel.x, rel.y));
                self.gfx.push_origin(abs);
                // the form is clipped to its bounding box
                self.code(&format!("{} {} {} {} re W n", bbox[0], bbox[1], bbox[2] - bbox[0], bbox[3] - bbox[1]), canvas);
                self.scopes.push(Obj::Dict(HashMap::new()));
                for e in events {
                    match e {
                        Event::Glyph { font, code, placement, x, y } => {
                            self.glyph(canvas, font, code, placement, at.0 + x, at.1 + y)
                        }
                        Event::Rule { x0, y0, x1, y1 } => self.rule(canvas, (at.0 + x0, at.1 + y0), (at.0 + x1, at.1 + y1)),
                        Event::Special { text, x, y } => self.special(&text, (at.0 + x, at.1 + y), canvas, colors),
                    }
                }
                self.scopes.pop();
                self.gfx.pop_origin();
                self.gfx.restore();
                self.gfx.set_color(colors.current());
            }
            Some(XObject::Image { file, page, w, h, natural_w }) => {
                let (file, page, w, h, nw) = (file.clone(), *page, *w, *h, *natural_w);
                self.draw_image(&file, page, w, h, nw, at, canvas);
            }
            None => warn_once(format!("pdf:uxobj @{name}: no such object")),
        }
    }

    /// `pdf:image [@name] [hide] [page n] [bbox a b c d] [width w] [height h] (file)`.
    fn image(&mut self, arg: &str, at: (f64, f64), canvas: &mut Canvas) {
        let (head, file) = match (arg.find('('), arg.rfind(')')) {
            (Some(a), Some(b)) if b > a => (&arg[..a], arg[a + 1..b].to_string()),
            _ => return,
        };
        let mut name = None;
        let mut hide = false;
        let mut page = 1;
        let (mut w, mut h, mut scale) = (None, None, 1.0f32);
        let mut bbox: Option<[f32; 4]> = None;
        let mut words = head.split_whitespace();
        while let Some(word) = words.next() {
            match word {
                "hide" => hide = true,
                "page" => page = words.next().and_then(|v| v.parse().ok()).unwrap_or(1),
                "width" => w = words.next().and_then(dimen_bp),
                "height" => h = words.next().and_then(dimen_bp),
                "scale" => scale = words.next().and_then(|v| v.parse().ok()).unwrap_or(1.0),
                "bbox" => {
                    let v: Vec<f32> = (0..4).filter_map(|_| words.next().and_then(|x| x.parse().ok())).collect();
                    if v.len() == 4 {
                        bbox = Some([v[0], v[1], v[2], v[3]]);
                    }
                }
                _ if word.starts_with('@') => name = Some(word[1..].to_string()),
                _ => {}
            }
        }
        let natural = bbox.map(|b| (b[2] - b[0], b[3] - b[1]));
        let (w, h) = match (w, h, natural) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some((nw, nh))) if nw > 0.0 => (w, w * nh / nw),
            (None, Some(h), Some((nw, nh))) if nh > 0.0 => (h * nw / nh, h),
            (None, None, Some((nw, nh))) => (nw * scale, nh * scale),
            (w, h, _) => (w.or(h).unwrap_or(72.0), h.or(w).unwrap_or(72.0)),
        };
        let natural_w = natural.map(|n| n.0).unwrap_or(0.0);
        if let Some(n) = name {
            self.xobjects.insert(n, XObject::Image { file: file.clone(), page, w, h, natural_w });
        }
        if !hide {
            self.draw_image(&file, page, w, h, natural_w, at, canvas);
        }
    }

    /// `PSfile="file" llx=.. lly=.. urx=.. ury=.. rwi=.. rhi=..` (graphicx for EPS).
    fn psfile(&mut self, rest: &str, at: (f64, f64), canvas: &mut Canvas) {
        let (file, params) = match rest.strip_prefix('"').and_then(|r| r.split_once('"')) {
            Some((f, p)) => (f.to_string(), p),
            None => {
                let (f, p) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                (f.to_string(), p)
            }
        };
        let mut v = HashMap::new();
        for kv in params.split_whitespace() {
            if let Some((k, val)) = kv.split_once('=') {
                if let Ok(x) = val.parse::<f32>() {
                    v.insert(k.to_string(), x);
                }
            }
        }
        let g = |k: &str| v.get(k).copied();
        let nw = g("urx").unwrap_or(72.0) - g("llx").unwrap_or(0.0);
        let nh = g("ury").unwrap_or(72.0) - g("lly").unwrap_or(0.0);
        let (w, h) = match (g("rwi").map(|x| x / 10.0), g("rhi").map(|x| x / 10.0)) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * nh / nw.max(1e-3)),
            (None, Some(h)) => (h * nw / nh.max(1e-3), h),
            (None, None) => (nw, nh),
        };
        self.draw_image(&file, 1, w, h, nw, at, canvas);
    }

    /// Draws an image `w` x `h` bp with its lower-left corner at the current point.
    #[allow(clippy::too_many_arguments)]
    fn draw_image(&mut self, file: &str, page: u32, w: f32, h: f32, natural_w: f32, at: (f64, f64), canvas: &mut Canvas) {
        if !self.captures.is_empty() {
            warn_once("images inside pdf:bxobj are not drawn".into());
            return;
        }
        let (_, rel) = self.here(at);
        let t = self.gfx.user_to_device();
        let width_px = w * (t.sx * t.sx + t.ky * t.ky).sqrt();
        let Some(img) = self.images.get(file, page, width_px, natural_w) else { return };
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let place = t.pre_concat(Transform::from_row(w / iw, 0.0, 0.0, -h / ih, rel.x, rel.y + h));
        let st = self.gfx.state();
        let paint = PixmapPaint { opacity: st.fill_alpha, quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
        canvas.pixmap_mut().draw_pixmap(0, 0, img.as_ref().as_ref(), &paint, place, st.clip.as_deref());
    }
}

/// `pdf:btrans` arguments: `matrix a b c d e f`, `rotate θ`, `scale s`,
/// `xscale s`, `yscale s` (none: the identity).
fn parse_transform(arg: &str) -> Transform {
    let mut m = Transform::identity();
    let mut words = arg.split_whitespace();
    let num = |w: Option<&str>| w.and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    while let Some(w) = words.next() {
        match w {
            "matrix" => {
                let v: Vec<f32> = (0..6).map(|_| num(words.next())).collect();
                m = m.pre_concat(Transform::from_row(v[0], v[1], v[2], v[3], v[4], v[5]));
            }
            "rotate" => {
                let a = num(words.next()).to_radians();
                m = m.pre_concat(Transform::from_row(a.cos(), a.sin(), -a.sin(), a.cos(), 0.0, 0.0));
            }
            "scale" => {
                let s = num(words.next());
                m = m.pre_concat(Transform::from_scale(s, s));
            }
            "xscale" => m = m.pre_concat(Transform::from_scale(num(words.next()), 1.0)),
            "yscale" => m = m.pre_concat(Transform::from_scale(1.0, num(words.next()))),
            _ => {}
        }
    }
    m
}

/// Moves along the writing direction (down the page in tate).
fn move_right(r: &mut Regs, d: i32) {
    if r.tate { r.y += d } else { r.x += d }
}

/// Moves across lines (to the left in tate).
fn move_down(r: &mut Regs, d: i32) {
    if r.tate { r.x -= d } else { r.y += d }
}

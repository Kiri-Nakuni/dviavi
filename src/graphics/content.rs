//! The PDF graphics state and content-stream operators (what `pdf:code` and
//! friends contain), drawn with tiny-skia.
//!
//! Coordinates follow dvipdfmx: user space is in bp, y up, origin at the
//! bottom-left of the paper; `device` maps it to pixels.  dvipdfmx keeps a
//! stack of "coordinate origins" (pushed by `pdf:bcontent`): text and new
//! blocks are placed relative to the top one.

use std::collections::HashMap;
use std::rc::Rc;

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Point, Rect, Shader, Stroke, StrokeDash,
    Transform,
};

use super::canvas::{Canvas, Rgb};
use super::pdfobj::{Lexer, Obj, Token};
use super::shading::{self, Resolve};
use crate::warn::warn_once;

#[derive(Clone)]
pub struct GState {
    /// User space to page space (bp, y up).
    pub ctm: Transform,
    pub fill: Rgb,
    pub stroke: Rgb,
    pub fill_alpha: f32,
    pub stroke_alpha: f32,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter: f32,
    dash: Option<(Vec<f32>, f32)>,
    /// Device-space clip (shared until changed).
    pub clip: Option<Rc<Mask>>,
}

impl GState {
    fn new(color: Rgb) -> GState {
        GState {
            ctm: Transform::identity(),
            fill: color,
            stroke: color,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: None,
            clip: None,
        }
    }
}

/// Named resources visible to a content stream (innermost scope last) and
/// the `pdf:obj` objects they may refer to.
pub struct Resources<'a> {
    pub scopes: &'a [Obj],
    pub objects: &'a HashMap<String, Obj>,
}

impl Resolve for Resources<'_> {
    fn resolve(&self, o: &Obj) -> Obj {
        let mut o = o.clone();
        for _ in 0..16 {
            match o {
                Obj::Ref(ref n) => match self.objects.get(n) {
                    Some(v) => o = v.clone(),
                    None => return Obj::Null,
                },
                _ => return o,
            }
        }
        Obj::Null
    }
}

impl Resources<'_> {
    /// `/Category /Name` (e.g. ExtGState, Shading), innermost scope first.
    fn lookup(&self, category: &str, name: &str) -> Option<Obj> {
        for scope in self.scopes.iter().rev() {
            let cat = scope.dict().and_then(|d| d.get(category)).map(|c| self.resolve(c));
            if let Some(v) = cat.as_ref().and_then(|c| c.dict()).and_then(|d| d.get(name)) {
                return Some(self.resolve(v));
            }
        }
        None
    }
}

/// The graphics state stack and the coordinate stack of one page.
pub struct Graphics {
    stack: Vec<GState>,
    /// dvipdfmx's coordinate origins (user space, absolute).
    coords: Vec<Point>,
    /// User space to pixels.
    pub device: Transform,
    path: PathBuilder,
    pending_clip: Option<FillRule>,
}

impl Graphics {
    pub fn new(device: Transform, color: Rgb) -> Graphics {
        Graphics { stack: vec![GState::new(color)], coords: Vec::new(), device, path: PathBuilder::new(), pending_clip: None }
    }

    pub fn state(&self) -> &GState {
        self.stack.last().expect("graphics state")
    }

    fn state_mut(&mut self) -> &mut GState {
        self.stack.last_mut().expect("graphics state")
    }

    /// Sets fill and stroke colour (a colour special).
    pub fn set_color(&mut self, c: Rgb) {
        let s = self.state_mut();
        s.fill = c;
        s.stroke = c;
    }

    pub fn save(&mut self) {
        let s = self.state().clone();
        self.stack.push(s);
    }

    pub fn restore(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        } else {
            warn_once("PDF graphics state: Q without q".into());
        }
    }

    pub fn concat(&mut self, m: Transform) {
        let s = self.state_mut();
        s.ctm = s.ctm.pre_concat(m);
    }

    /// The top coordinate origin (0,0 outside any `pdf:bcontent`).
    pub fn origin(&self) -> Point {
        self.coords.last().copied().unwrap_or(Point::zero())
    }

    pub fn push_origin(&mut self, p: Point) {
        self.coords.push(p);
    }

    pub fn pop_origin(&mut self) {
        self.coords.pop();
    }

    /// A user-space point (absolute) relative to the current origin.
    pub fn relative(&self, p: Point) -> Point {
        let o = self.origin();
        Point::from_xy(p.x - o.x, p.y - o.y)
    }

    /// Pixels to user space.
    pub fn to_user(&self, px: f64, py: f64) -> Point {
        let inv = self.device.invert().unwrap_or_default();
        let mut p = Point::from_xy(px as f32, py as f32);
        inv.map_point(&mut p);
        p
    }

    /// How DVI material (text, rules: placed in absolute page coordinates)
    /// is moved in pixel space by the current transformation.
    pub fn text_transform(&self) -> Transform {
        let o = self.origin();
        self.device
            .pre_concat(self.state().ctm)
            .pre_concat(Transform::from_translate(-o.x, -o.y))
            .pre_concat(self.device.invert().unwrap_or_default())
    }

    /// User space to pixels.
    pub fn user_to_device(&self) -> Transform {
        self.device.pre_concat(self.state().ctm)
    }

    /// Executes content-stream operators.
    pub fn run(&mut self, ops: &str, canvas: &mut Canvas, res: &Resources) {
        let mut lex = Lexer::new(ops);
        let mut args: Vec<Obj> = Vec::new();
        while let Some(tok) = lex.next_token() {
            match tok {
                Token::Obj(o) => args.push(o),
                Token::Op(op) => {
                    if op == "BI" {
                        // inline image: skip to EI
                        while let Some(t) = lex.next_token() {
                            if t == Token::Op("EI".into()) {
                                break;
                            }
                        }
                        warn_once("inline images (BI ... EI) are not drawn".into());
                    } else {
                        self.op(&op, &args, canvas, res);
                    }
                    args.clear();
                }
            }
        }
    }

    fn op(&mut self, op: &str, a: &[Obj], canvas: &mut Canvas, res: &Resources) {
        let n = |i: usize| a.get(i).and_then(Obj::num).unwrap_or(0.0) as f32;
        let nums: Vec<f32> = a.iter().filter_map(Obj::num).map(|v| v as f32).collect();
        match op {
            "q" => self.save(),
            "Q" => self.restore(),
            "cm" if a.len() >= 6 => self.concat(Transform::from_row(n(0), n(1), n(2), n(3), n(4), n(5))),
            "w" => self.state_mut().line_width = n(0),
            "J" => {
                self.state_mut().cap = match n(0) as i32 {
                    1 => LineCap::Round,
                    2 => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.state_mut().join = match n(0) as i32 {
                    1 => LineJoin::Round,
                    2 => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => self.state_mut().miter = n(0),
            "d" => {
                let arr: Vec<f32> = a.first().map(|o| o.nums().iter().map(|v| *v as f32).collect()).unwrap_or_default();
                self.state_mut().dash = if arr.is_empty() { None } else { Some((arr, n(1))) };
            }
            "i" | "ri" => {}
            "gs" => {
                if let Some(name) = a.first().and_then(Obj::name) {
                    match res.lookup("ExtGState", name) {
                        Some(gs) => self.ext_gstate(&gs, res),
                        None => warn_once(format!("ExtGState /{name} not found")),
                    }
                }
            }
            // colours
            "g" | "rg" | "k" | "sc" | "scn" => {
                if let Some(c) = color(&nums) {
                    self.state_mut().fill = c;
                }
            }
            "G" | "RG" | "K" | "SC" | "SCN" => {
                if let Some(c) = color(&nums) {
                    self.state_mut().stroke = c;
                }
            }
            "cs" | "CS" => {
                if a.first().and_then(Obj::name) == Some("Pattern") {
                    warn_once("pattern fills are not drawn".into());
                }
            }
            // path construction
            "m" => self.path.move_to(n(0), n(1)),
            "l" => self.path.line_to(n(0), n(1)),
            "c" => self.path.cubic_to(n(0), n(1), n(2), n(3), n(4), n(5)),
            "v" => {
                let p = self.path.last_point().unwrap_or(Point::zero());
                self.path.cubic_to(p.x, p.y, n(0), n(1), n(2), n(3))
            }
            "y" => self.path.cubic_to(n(0), n(1), n(2), n(3), n(2), n(3)),
            "h" => self.path.close(),
            "re" => {
                let (x, y, w, h) = (n(0), n(1), n(2), n(3));
                self.path.move_to(x, y);
                self.path.line_to(x + w, y);
                self.path.line_to(x + w, y + h);
                self.path.line_to(x, y + h);
                self.path.close();
            }
            "W" => self.pending_clip = Some(FillRule::Winding),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),
            // painting
            "S" => self.paint(canvas, false, None, true),
            "s" => {
                self.path.close();
                self.paint(canvas, false, None, true)
            }
            "f" | "F" => self.paint(canvas, false, Some(FillRule::Winding), false),
            "f*" => self.paint(canvas, false, Some(FillRule::EvenOdd), false),
            "B" => self.paint(canvas, false, Some(FillRule::Winding), true),
            "B*" => self.paint(canvas, false, Some(FillRule::EvenOdd), true),
            "b" => self.paint(canvas, true, Some(FillRule::Winding), true),
            "b*" => self.paint(canvas, true, Some(FillRule::EvenOdd), true),
            "n" => self.paint(canvas, false, None, false),
            "sh" => {
                if let Some(name) = a.first().and_then(Obj::name) {
                    match res.lookup("Shading", name) {
                        Some(sh) => self.shade(&sh, canvas, res),
                        None => warn_once(format!("shading /{name} not found")),
                    }
                }
            }
            "Do" => warn_once("the Do operator is not supported (use pdf:uxobj)".into()),
            "BT" | "ET" | "Tf" | "Tj" | "TJ" | "Td" | "TD" | "Tm" | "T*" | "'" | "\"" => {
                warn_once("text operators in PDF code are not drawn".into())
            }
            "BMC" | "BDC" | "EMC" | "MP" | "DP" | "BX" | "EX" | "d0" | "d1" => {}
            _ => warn_once(format!("PDF operator `{op}' is not supported")),
        }
    }

    fn ext_gstate(&mut self, gs: &Obj, res: &Resources) {
        let Some(d) = gs.dict() else { return };
        let s = self.state_mut();
        if let Some(v) = d.get("CA").and_then(Obj::num) {
            s.stroke_alpha = v as f32;
        }
        if let Some(v) = d.get("ca").and_then(Obj::num) {
            s.fill_alpha = v as f32;
        }
        if let Some(v) = d.get("LW").and_then(Obj::num) {
            s.line_width = v as f32;
        }
        if let Some(sm) = d.get("SMask") {
            if res.resolve(sm) != Obj::Name("None".into()) {
                warn_once("soft masks (fadings) are not supported".into());
            }
        }
    }

    /// Ends the current path: fills and/or strokes it, then applies a pending clip.
    fn paint(&mut self, canvas: &mut Canvas, close: bool, fill: Option<FillRule>, stroke: bool) {
        if close {
            self.path.close();
        }
        let path = std::mem::replace(&mut self.path, PathBuilder::new()).finish();
        let Some(path) = path else {
            self.pending_clip = None;
            return;
        };
        let t = self.user_to_device();
        let st = self.state().clone();
        let mask = st.clip.as_deref();
        if let Some(rule) = fill {
            let paint = solid(st.fill, st.fill_alpha);
            canvas.pixmap_mut().fill_path(&path, &paint, rule, t, mask);
        }
        if stroke {
            // width 0 means the thinnest visible line
            let scale = (t.sx * t.sy - t.kx * t.ky).abs().sqrt().max(1e-6);
            let width = if st.line_width <= 0.0 { 1.0 / scale } else { st.line_width };
            let mut s = Stroke { width, miter_limit: st.miter, line_cap: st.cap, line_join: st.join, dash: None };
            if let Some((arr, phase)) = &st.dash {
                let mut arr = arr.clone();
                if arr.len() % 2 == 1 {
                    arr.extend_from_slice(&arr.clone());
                }
                s.dash = StrokeDash::new(arr, *phase);
            }
            let paint = solid(st.stroke, st.stroke_alpha);
            canvas.pixmap_mut().stroke_path(&path, &paint, &s, t, mask);
        }
        if let Some(rule) = self.pending_clip.take() {
            self.clip(&path, rule, t, canvas);
        }
    }

    fn clip(&mut self, path: &Path, rule: FillRule, t: Transform, canvas: &Canvas) {
        let mut mask = match &self.state().clip {
            Some(m) => (**m).clone(),
            None => {
                let mut m = Mask::new(canvas.width() as u32, canvas.height() as u32).expect("mask");
                m.fill_path(path, rule, true, t);
                self.state_mut().clip = Some(Rc::new(m));
                return;
            }
        };
        mask.intersect_path(path, rule, true, t);
        self.state_mut().clip = Some(Rc::new(mask));
    }

    /// `sh`: paints the shading over the clip region.
    fn shade(&mut self, sh: &Obj, canvas: &mut Canvas, res: &Resources) {
        let Some(shader) = shading::shader(sh, self.user_to_device(), res) else { return };
        let mut paint = Paint { shader, anti_alias: true, ..Paint::default() };
        let alpha = self.state().fill_alpha;
        if alpha < 1.0 {
            paint.shader.apply_opacity(alpha);
        }
        let rect = Rect::from_xywh(0.0, 0.0, canvas.width() as f32, canvas.height() as f32).expect("rect");
        let clip = self.state().clip.clone();
        canvas.pixmap_mut().fill_rect(rect, &paint, Transform::identity(), clip.as_deref());
    }

    /// Fills a rectangle given in pixels (a TeX rule), through the current transformation.
    pub fn fill_rule(&self, canvas: &mut Canvas, x0: f64, y0: f64, x1: f64, y1: f64) {
        let st = self.state();
        let t = self.text_transform();
        let (mut xa, mut xb, mut ya, mut yb) = (x0 as f32, x1 as f32, y0 as f32, y1 as f32);
        if t.is_identity() || t.is_translate() {
            // a rule is at least one pixel thick, like in a previewer
            xa = xa.round();
            ya = ya.round();
            xb = xb.round().max(xa + 1.0);
            yb = yb.round().max(ya + 1.0);
        }
        let Some(rect) = Rect::from_ltrb(xa, ya, xb, yb) else { return };
        let paint = solid(st.fill, st.fill_alpha);
        canvas.pixmap_mut().fill_rect(rect, &paint, t, st.clip.as_deref());
    }
}

fn solid(c: Rgb, alpha: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(c[0], c[1], c[2], (alpha.clamp(0.0, 1.0) * 255.0).round() as u8));
    p.anti_alias = true;
    p.shader = Shader::SolidColor(Color::from_rgba8(c[0], c[1], c[2], (alpha.clamp(0.0, 1.0) * 255.0).round() as u8));
    p
}

/// Gray, RGB or CMYK components to RGB.
pub fn color(v: &[f32]) -> Option<Rgb> {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    match v {
        [g] => Some([b(*g); 3]),
        [r, g, bl] => Some([b(*r), b(*g), b(*bl)]),
        [c, m, y, k] => Some([b((1.0 - c) * (1.0 - k)), b((1.0 - m) * (1.0 - k)), b((1.0 - y) * (1.0 - k))]),
        _ => None,
    }
}

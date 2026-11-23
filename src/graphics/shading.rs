//! Axial and radial shadings (ShadingType 2 and 3) with sampled, exponential
//! and stitching functions, turned into tiny-skia gradients.

use tiny_skia::{Color, GradientStop, LinearGradient, Point, RadialGradient, Shader, SpreadMode, Transform};

use super::pdfobj::Obj;
use crate::warn::warn_once;

/// Resolves `@name` references (shadings may refer to `pdf:obj` functions).
pub trait Resolve {
    fn resolve(&self, o: &Obj) -> Obj;
}

/// Evaluates a PDF function at `t`; `None` for unsupported types.
fn eval(f: &Obj, t: f64, r: &dyn Resolve) -> Option<Vec<f64>> {
    let f = r.resolve(f);
    let d = f.dict()?;
    let domain = d.get("Domain").map(|o| r.resolve(o).nums()).unwrap_or_else(|| vec![0.0, 1.0]);
    let t = t.clamp(domain[0], *domain.get(1).unwrap_or(&1.0));
    match d.get("FunctionType").and_then(Obj::num)? as i64 {
        2 => {
            let c0 = d.get("C0").map(|o| r.resolve(o).nums()).unwrap_or_else(|| vec![0.0]);
            let c1 = d.get("C1").map(|o| r.resolve(o).nums()).unwrap_or_else(|| vec![1.0]);
            let n = d.get("N").and_then(Obj::num).unwrap_or(1.0);
            let x = t.powf(n);
            Some(c0.iter().zip(&c1).map(|(a, b)| a + x * (b - a)).collect())
        }
        3 => {
            let funcs = r.resolve(d.get("Functions")?);
            let funcs = funcs.array()?;
            let bounds = d.get("Bounds").map(|o| r.resolve(o).nums()).unwrap_or_default();
            let encode = d.get("Encode").map(|o| r.resolve(o).nums()).unwrap_or_default();
            let mut k = 0;
            while k < bounds.len() && t >= bounds[k] {
                k += 1;
            }
            let k = k.min(funcs.len().saturating_sub(1));
            let lo = if k == 0 { domain[0] } else { bounds[k - 1] };
            let hi = if k < bounds.len() { bounds[k] } else { domain[1] };
            let (e0, e1) = (*encode.get(2 * k).unwrap_or(&0.0), *encode.get(2 * k + 1).unwrap_or(&1.0));
            let u = if hi > lo { e0 + (t - lo) / (hi - lo) * (e1 - e0) } else { e0 };
            eval(&funcs[k], u, r)
        }
        other => {
            warn_once(format!("PDF function type {other} is not supported"));
            None
        }
    }
}

/// Device colour components to RGB.
fn to_color(c: &[f64], space: &str) -> Color {
    let cl = |v: f64| v.clamp(0.0, 1.0) as f32;
    match (space, c) {
        (_, [g]) => Color::from_rgba(cl(*g), cl(*g), cl(*g), 1.0).unwrap(),
        (_, [r, g, b]) => Color::from_rgba(cl(*r), cl(*g), cl(*b), 1.0).unwrap(),
        (_, [cy, m, y, k]) => Color::from_rgba(
            cl((1.0 - cy) * (1.0 - k)),
            cl((1.0 - m) * (1.0 - k)),
            cl((1.0 - y) * (1.0 - k)),
            1.0,
        )
        .unwrap(),
        _ => Color::BLACK,
    }
}

/// A shader for the shading dictionary `sh`, drawn through `transform`
/// (user space to device).
pub fn shader(sh: &Obj, transform: Transform, r: &dyn Resolve) -> Option<Shader<'static>> {
    let sh = r.resolve(sh);
    let d = sh.dict()?;
    let ty = d.get("ShadingType").and_then(Obj::num)? as i64;
    let space = d.get("ColorSpace").map(|o| r.resolve(o)).and_then(|o| o.name().map(str::to_string)).unwrap_or_default();
    let coords = d.get("Coords").map(|o| r.resolve(o).nums())?;
    let func = d.get("Function")?;
    let domain = d.get("Domain").map(|o| r.resolve(o).nums()).unwrap_or_else(|| vec![0.0, 1.0]);
    let (t0, t1) = (domain[0], *domain.get(1).unwrap_or(&1.0));
    const N: usize = 48;
    let mut stops = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let s = i as f64 / N as f64;
        let c = eval(func, t0 + s * (t1 - t0), r)?;
        stops.push(GradientStop::new(s as f32, to_color(&c, &space)));
    }
    let p = |x: f64, y: f64| Point::from_xy(x as f32, y as f32);
    match (ty, coords.as_slice()) {
        (2, [x0, y0, x1, y1]) => LinearGradient::new(p(*x0, *y0), p(*x1, *y1), stops, SpreadMode::Pad, transform),
        (3, [x0, y0, r0, x1, y1, r1]) => {
            RadialGradient::new(p(*x0, *y0), *r0 as f32, p(*x1, *y1), *r1 as f32, stops, SpreadMode::Pad, transform)
        }
        _ => {
            warn_once(format!("shading type {ty} is not supported"));
            None
        }
    }
}

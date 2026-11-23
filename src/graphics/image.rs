//! Images for `pdf:image` and `PSfile`, decoded by external programs:
//! ffmpeg (PNG, JPEG, BMP, GIF, TIFF, WebP ...), mutool or pdftocairo (PDF),
//! Ghostscript (EPS/PS).  Vector images are rendered at the size they are drawn.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use tiny_skia::{IntSize, Pixmap};

use crate::warn::warn_once;

#[derive(Default)]
pub struct Images {
    base: PathBuf,
    /// (file, page, pixel width for vector images; 0 for raster images)
    cache: HashMap<(PathBuf, u32, u32), Option<Rc<Pixmap>>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Raster,
    Pdf,
    Eps,
}

fn kind(path: &Path) -> Kind {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("pdf") | Some("ai") => Kind::Pdf,
        Some("eps") | Some("ps") | Some("mps") => Kind::Eps,
        _ => Kind::Raster,
    }
}

impl Images {
    /// `base` is the directory of the DVI file (where TeX looked first).
    pub fn new(base: PathBuf) -> Images {
        Images { base, cache: HashMap::new() }
    }

    fn find(&self, name: &str) -> Option<PathBuf> {
        let p = self.base.join(name);
        if p.exists() {
            return Some(p);
        }
        if Path::new(name).exists() {
            return Some(PathBuf::from(name));
        }
        crate::tex::kpse::find(name)
    }

    /// The image `name` (page `page` for PDF), for drawing `width_px` pixels
    /// wide; `natural_width_bp` is its size in bp (from the bounding box).
    pub fn get(&mut self, name: &str, page: u32, width_px: f32, natural_width_bp: f32) -> Option<Rc<Pixmap>> {
        let Some(path) = self.find(name) else {
            warn_once(format!("image {name} not found"));
            return None;
        };
        let k = kind(&path);
        let dpi = if natural_width_bp > 0.0 { (72.0 * width_px / natural_width_bp).clamp(18.0, 1200.0) } else { 150.0 };
        let key = (path.clone(), page, if k == Kind::Raster { 0 } else { width_px.round() as u32 });
        if let Some(c) = self.cache.get(&key) {
            return c.clone();
        }
        let result = match k {
            Kind::Raster => run(Command::new("ffmpeg").args(["-v", "error", "-nostdin", "-i"]).arg(&path).args(PAM_OUT), None),
            Kind::Pdf => run(
                Command::new("mutool")
                    .args(["draw", "-q", "-F", "pam", "-c", "rgba", "-r", &format!("{dpi:.2}"), "-o", "-"])
                    .arg(&path)
                    .arg(page.max(1).to_string()),
                None,
            )
            .or_else(|_| {
                let png = output(
                    Command::new("pdftocairo")
                        .args(["-png", "-singlefile", "-transp", "-r", &format!("{dpi:.2}")])
                        .args(["-f", &page.max(1).to_string(), "-l", &page.max(1).to_string()])
                        .arg(&path)
                        .arg("-"),
                    None,
                )?;
                run(Command::new("ffmpeg").args(["-v", "error", "-i", "pipe:0"]).args(PAM_OUT), Some(png))
            }),
            Kind::Eps => output(
                Command::new("gs")
                    .args(["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-dEPSCrop", "-sDEVICE=pngalpha"])
                    .arg(format!("-r{dpi:.2}"))
                    .args(["-sOutputFile=-"])
                    .arg(&path),
                None,
            )
            .and_then(|png| run(Command::new("ffmpeg").args(["-v", "error", "-i", "pipe:0"]).args(PAM_OUT), Some(png))),
        };
        let img = match result {
            Ok(p) => Some(Rc::new(p)),
            Err(e) => {
                warn_once(format!("image {}: {e}", path.display()));
                None
            }
        };
        self.cache.insert(key, img.clone());
        img
    }
}

const PAM_OUT: &[&str] = &["-frames:v", "1", "-f", "image2pipe", "-c:v", "pam", "-pix_fmt", "rgba", "-"];

fn output(cmd: &mut Command, input: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {name} ({e})"))?;
    if let Some(data) = input {
        let mut stdin = child.stdin.take().expect("stdin");
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let out = child.wait_with_output().map_err(|e| format!("{name}: {e}"))?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(format!("{name}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn run(cmd: &mut Command, input: Option<Vec<u8>>) -> Result<Pixmap, String> {
    parse_pam(&output(cmd, input)?)
}

/// A PAM (P7) image with RGB_ALPHA, RGB, GRAYSCALE or GRAYSCALE_ALPHA tuples.
fn parse_pam(d: &[u8]) -> Result<Pixmap, String> {
    let end = d.windows(7).position(|w| w == b"ENDHDR\n").ok_or("not a PAM image")? + 7;
    let header = String::from_utf8_lossy(&d[..end]);
    let field = |k: &str| -> Option<u32> {
        header.lines().find_map(|l| l.strip_prefix(k).and_then(|v| v.trim().parse().ok()))
    };
    let (w, h, depth) = (field("WIDTH").ok_or("PAM width")?, field("HEIGHT").ok_or("PAM height")?, field("DEPTH").unwrap_or(4));
    let maxval = field("MAXVAL").unwrap_or(255);
    if maxval != 255 {
        return Err(format!("PAM maxval {maxval}"));
    }
    let src = &d[end..];
    let n = (w * h) as usize;
    if src.len() < n * depth as usize {
        return Err("PAM data too short".into());
    }
    let mut rgba = Vec::with_capacity(n * 4);
    for px in src.chunks_exact(depth as usize).take(n) {
        let (r, g, b, a) = match depth {
            1 => (px[0], px[0], px[0], 255),
            2 => (px[0], px[0], px[0], px[1]),
            3 => (px[0], px[1], px[2], 255),
            _ => (px[0], px[1], px[2], px[3]),
        };
        // premultiplied
        let pm = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        rgba.extend_from_slice(&[pm(r), pm(g), pm(b), a]);
    }
    Pixmap::from_vec(rgba, IntSize::from_wh(w, h).ok_or("image size")?).ok_or_else(|| "bad image".to_string())
}

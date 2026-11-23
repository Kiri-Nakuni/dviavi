//! dviavi: turns a DVI file into an AVI video, one page per frame, with a
//! sound track synthesised from sound.mix and the `dviavi:` specials.

mod audio;
mod fonts;
mod graphics;
mod render;
mod say;
mod special;
mod tex;
mod video;
mod warn;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use graphics::canvas::Canvas;
use render::{ColorState, Renderer};
use video::avi::{AudioFormat, AviWriter, VideoFormat};
use video::palette::{Histogram, Palette};

const USAGE: &str = "usage: dviavi [-o OUTPUT.avi] INPUT[.dvi]\n       dviavi say --help";

fn main() {
    if let Err(e) = run() {
        eprintln!("dviavi: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.first().map(String::as_str) == Some("say") {
        return say::run(&all[1..]);
    }
    let mut input = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = Some(args.next().ok_or(USAGE)?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if input.is_none() => input = Some(a),
            _ => return Err(USAGE.into()),
        }
    }
    let input = input.ok_or(USAGE)?;
    let mut in_path = PathBuf::from(&input);
    if !in_path.exists() && in_path.extension().is_none() {
        in_path.set_extension("dvi");
    }
    let out_path = output.unwrap_or_else(|| in_path.with_extension("avi").to_string_lossy().into_owned());

    let data = std::fs::read(&in_path).map_err(|e| format!("{}: {e}", in_path.display()))?;
    let dvi = tex::dvi::Dvi::parse(data)?;
    if dvi.page_count() == 0 {
        return Err("the DVI file has no pages".into());
    }
    let timeline = special::scan(&dvi)?;
    let cfg = &timeline.config;
    let frames = dvi.page_count();

    // resolution: the paper fills the frame
    let (paper_w, paper_h) = cfg.paper.or(timeline.papersize).unwrap_or_else(|| {
        warn::warn_once("no papersize special; assuming a paper 160mm wide with the frame's aspect".into());
        (0.16, 0.16 * cfg.height as f64 / cfg.width as f64)
    });
    let px_per_m = cfg.width as f64 / paper_w;
    if (paper_h * px_per_m - cfg.height as f64).abs() > 1.0 {
        warn::warn_once(format!(
            "the paper ({:.1}mm x {:.1}mm) and the frame ({}x{}) differ in aspect ratio; the paper is scaled to the width",
            paper_w * 1000.0, paper_h * 1000.0, cfg.width, cfg.height
        ));
    }

    let mut fonts = fonts::Fonts::new()?;
    let dvi_fonts: HashMap<u32, fonts::FontId> = dvi.fonts.iter().map(|f| (f.number, fonts.load(&f.name, f.scale))).collect();

    // sound
    let ctx = timeline.time_context();
    let base = in_path.parent().unwrap_or(Path::new("."));
    let mut mix_text = String::new();
    let mut mix_name = String::from("mixdef");
    if let Some(m) = &cfg.mix {
        let p = base.join(m);
        mix_text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        mix_name = p.display().to_string();
    }
    // definitions from the document come after sound.mix, so they win
    for d in &timeline.mixdefs {
        mix_text.push('\n');
        mix_text.push_str(d);
    }
    let mix = audio::mix::Mix::parse(&mix_text, base, &ctx).map_err(|e| format!("{mix_name}:{e}"))?;
    let total_samples = (frames as u64 * cfg.rate as u64 * cfg.fps_den as u64 / cfg.fps_num as u64) as usize;
    let sound = audio::render(&mix, &timeline.events, &ctx, cfg.rate, total_samples);

    // pass 1: colours of every frame
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let base = in_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut renderer = Renderer::new(&dvi, &fonts, dvi_fonts, px_per_m, paper_h, base);
    let mut frame = Canvas::new(w, h);
    let mut hist = Histogram::default();
    let mut frames_iter = FrameSource::new(&dvi);
    while let Some(changed) = frames_iter.next(&mut renderer, &mut frame)? {
        if changed {
            hist.add(frame.rgb(), 1);
        }
        progress("scanning", frames_iter.done(), frames);
    }
    let palette = Palette::build(&hist);

    // pass 2: encode
    let mut avi = AviWriter::create(
        &out_path,
        VideoFormat { width: cfg.width, height: cfg.height, fps_num: cfg.fps_num, fps_den: cfg.fps_den },
        AudioFormat { rate: cfg.rate, channels: 2 },
        palette.colors(),
    )
    .map_err(|e| format!("{out_path}: {e}"))?;
    frames_iter = FrameSource::new(&dvi);
    let mut indices = Vec::new();
    let mut prev_indices: Vec<u8> = Vec::new();
    let mut rle = Vec::new();
    let mut sample_pos = 0usize;
    let mut i = 0usize;
    while let Some(changed) = frames_iter.next(&mut renderer, &mut frame)? {
        let key = i as u32 % cfg.keyint == 0;
        if changed || key {
            palette.map(frame.rgb(), w, &mut indices);
        }
        if key {
            video::rle::encode(&indices, None, w, h, &mut rle);
        } else if changed {
            video::rle::encode(&indices, Some(&prev_indices), w, h, &mut rle);
        } else {
            rle.clear();
            rle.extend_from_slice(&[0, 1]);
        }
        if changed || key {
            std::mem::swap(&mut indices, &mut prev_indices);
        }
        avi.video_frame(&rle, key).map_err(|e| format!("{out_path}: {e}"))?;
        let end = ((i as u64 + 1) * cfg.rate as u64 * cfg.fps_den as u64 / cfg.fps_num as u64) as usize;
        let pcm: Vec<i16> = sound[2 * sample_pos..2 * end].iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
        avi.audio(&pcm).map_err(|e| format!("{out_path}: {e}"))?;
        sample_pos = end;
        i += 1;
        progress("encoding", i, frames);
    }
    avi.finish().map_err(|e| format!("{out_path}: {e}"))?;
    eprintln!("\rdviavi: {out_path}: {frames} frames, {:.2} s{:20}", frames as f64 * cfg.fps_den as f64 / cfg.fps_num as f64, "");
    Ok(())
}

/// Walks the pages, rendering only when a page can differ from the last one
/// drawn: equal page bytes from an equal colour state give an equal picture.
struct FrameSource<'d> {
    dvi: &'d tex::dvi::Dvi,
    page: usize,
    colors: ColorState,
    /// (page bytes, colour state before it) of the last rendered page.
    last: Option<(&'d [u8], ColorState)>,
}

impl<'d> FrameSource<'d> {
    fn new(dvi: &'d tex::dvi::Dvi) -> FrameSource<'d> {
        FrameSource { dvi, page: 0, colors: ColorState::new(), last: None }
    }

    fn done(&self) -> usize {
        self.page
    }

    /// Advances one page; `Some(true)` if `frame` was redrawn, `Some(false)`
    /// if it is unchanged, `None` after the last page.
    fn next(&mut self, renderer: &mut Renderer<'_>, frame: &mut Canvas) -> Result<Option<bool>, String> {
        if self.page >= self.dvi.page_count() {
            return Ok(None);
        }
        let bytes = self.dvi.page_bytes(self.page);
        let same = matches!(&self.last, Some((b, c)) if *b == bytes && *c == self.colors);
        if !same {
            let before = self.colors.clone();
            renderer.render(self.page, frame, &mut self.colors)?;
            self.last = Some((bytes, before));
        }
        self.page += 1;
        Ok(Some(!same))
    }
}

fn progress(what: &str, done: usize, total: usize) {
    if done % 24 == 0 || done == total {
        eprint!("\rdviavi: {what} {done}/{total}");
    }
}

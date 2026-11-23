//! RIFF AVI 1.0 writer: one MS-RLE video stream and one 16-bit PCM stream.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};

use crate::graphics::canvas::Rgb;

pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

pub struct AudioFormat {
    pub rate: u32,
    pub channels: u16,
}

/// AVI 1.0 offsets are 32-bit; players commonly refuse files beyond 2 GiB.
const SIZE_LIMIT: u64 = 0x7FFF_0000;

pub struct AviWriter {
    out: BufWriter<File>,
    video: VideoFormat,
    audio: AudioFormat,
    /// File offsets to patch in `finish`.
    riff_size_at: u64,
    total_frames_at: u64,
    video_length_at: u64,
    audio_length_at: u64,
    movi_size_at: u64,
    movi_start: u64,
    index: Vec<IndexEntry>,
    frames: u32,
    audio_samples: u32,
    max_chunk: u32,
}

struct IndexEntry {
    id: [u8; 4],
    flags: u32,
    offset: u32,
    size: u32,
}

const KEYFRAME: u32 = 0x10;

impl AviWriter {
    pub fn create(path: &str, video: VideoFormat, audio: AudioFormat, palette: &[Rgb]) -> std::io::Result<AviWriter> {
        let mut w = AviWriter {
            out: BufWriter::new(File::create(path)?),
            video,
            audio,
            riff_size_at: 0,
            total_frames_at: 0,
            video_length_at: 0,
            audio_length_at: 0,
            movi_size_at: 0,
            movi_start: 0,
            index: Vec::new(),
            frames: 0,
            audio_samples: 0,
            max_chunk: 0,
        };
        w.write_headers(palette)?;
        Ok(w)
    }

    fn pos(&mut self) -> std::io::Result<u64> {
        self.out.stream_position()
    }

    fn u32(&mut self, v: u32) -> std::io::Result<()> {
        self.out.write_all(&v.to_le_bytes())
    }

    fn u16(&mut self, v: u16) -> std::io::Result<()> {
        self.out.write_all(&v.to_le_bytes())
    }

    fn write_headers(&mut self, palette: &[Rgb]) -> std::io::Result<()> {
        let (vw, vh) = (self.video.width, self.video.height);
        let block = self.audio.channels as u32 * 2;
        let pal_len = 256u32;
        let strf_v = 40 + 4 * pal_len;
        let strl_v = 4 + (8 + 56) + (8 + strf_v);
        let strl_a = 4 + (8 + 56) + (8 + 18);
        let hdrl = 4 + (8 + 56) + (8 + strl_v) + (8 + strl_a);

        self.out.write_all(b"RIFF")?;
        self.riff_size_at = self.pos()?;
        self.u32(0)?;
        self.out.write_all(b"AVI LIST")?;
        self.u32(hdrl)?;
        self.out.write_all(b"hdrlavih")?;
        self.u32(56)?;
        let usec = (1_000_000u64 * self.video.fps_den as u64 / self.video.fps_num as u64) as u32;
        self.u32(usec)?;
        self.u32(0)?; // max bytes per second
        self.u32(0)?; // padding granularity
        self.u32(0x10 | 0x100)?; // HASINDEX | ISINTERLEAVED
        self.total_frames_at = self.pos()?;
        self.u32(0)?;
        self.u32(0)?; // initial frames
        self.u32(2)?; // streams
        self.u32(0)?; // suggested buffer size
        self.u32(vw)?;
        self.u32(vh)?;
        for _ in 0..4 {
            self.u32(0)?;
        }

        // video stream
        self.out.write_all(b"LIST")?;
        self.u32(strl_v)?;
        self.out.write_all(b"strlstrh")?;
        self.u32(56)?;
        self.out.write_all(b"vidsmrle")?;
        self.u32(0)?; // flags
        self.u32(0)?; // priority, language
        self.u32(0)?; // initial frames
        self.u32(self.video.fps_den)?; // scale
        self.u32(self.video.fps_num)?; // rate
        self.u32(0)?; // start
        self.video_length_at = self.pos()?;
        self.u32(0)?;
        self.u32(0)?; // suggested buffer size
        self.u32(u32::MAX)?; // quality
        self.u32(0)?; // sample size
        self.u16(0)?;
        self.u16(0)?;
        self.u16(vw as u16)?;
        self.u16(vh as u16)?;
        self.out.write_all(b"strf")?;
        self.u32(strf_v)?;
        self.u32(40)?;
        self.u32(vw)?;
        self.u32(vh)?; // positive: bottom-up
        self.u16(1)?;
        self.u16(8)?;
        self.u32(1)?; // BI_RLE8
        self.u32(vw * vh)?;
        self.u32(2835)?;
        self.u32(2835)?;
        self.u32(pal_len)?;
        self.u32(0)?;
        for i in 0..pal_len as usize {
            let c = palette.get(i).copied().unwrap_or([0; 3]);
            self.out.write_all(&[c[2], c[1], c[0], 0])?;
        }

        // audio stream
        self.out.write_all(b"LIST")?;
        self.u32(strl_a)?;
        self.out.write_all(b"strlstrh")?;
        self.u32(56)?;
        self.out.write_all(b"auds")?;
        self.u32(0)?;
        self.u32(0)?;
        self.u32(0)?;
        self.u32(0)?;
        self.u32(1)?; // scale
        self.u32(self.audio.rate)?; // rate: samples per second
        self.u32(0)?;
        self.audio_length_at = self.pos()?;
        self.u32(0)?;
        self.u32(self.audio.rate * block)?;
        self.u32(u32::MAX)?;
        self.u32(block)?; // sample size
        for _ in 0..4 {
            self.u16(0)?;
        }
        self.out.write_all(b"strf")?;
        self.u32(18)?;
        self.u16(1)?; // PCM
        self.u16(self.audio.channels)?;
        self.u32(self.audio.rate)?;
        self.u32(self.audio.rate * block)?;
        self.u16(block as u16)?;
        self.u16(16)?;
        self.u16(0)?;

        self.out.write_all(b"LIST")?;
        self.movi_size_at = self.pos()?;
        self.u32(0)?;
        self.movi_start = self.pos()?;
        self.out.write_all(b"movi")?;
        Ok(())
    }

    fn chunk(&mut self, id: [u8; 4], data: &[u8], flags: u32) -> std::io::Result<()> {
        let at = self.pos()?;
        if at + data.len() as u64 > SIZE_LIMIT {
            return Err(std::io::Error::other("AVI exceeds 2 GiB (AVI 1.0 limit)"));
        }
        self.index.push(IndexEntry { id, flags, offset: (at - self.movi_start) as u32, size: data.len() as u32 });
        self.out.write_all(&id)?;
        self.u32(data.len() as u32)?;
        self.out.write_all(data)?;
        if data.len() % 2 == 1 {
            self.out.write_all(&[0])?;
        }
        self.max_chunk = self.max_chunk.max(data.len() as u32);
        Ok(())
    }

    pub fn video_frame(&mut self, rle: &[u8], key: bool) -> std::io::Result<()> {
        self.frames += 1;
        self.chunk(*b"00dc", rle, if key { KEYFRAME } else { 0 })
    }

    /// Interleaved 16-bit samples (`channels` values per sample).
    pub fn audio(&mut self, samples: &[i16]) -> std::io::Result<()> {
        self.audio_samples += (samples.len() / self.audio.channels as usize) as u32;
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        self.chunk(*b"01wb", &bytes, KEYFRAME)
    }

    pub fn finish(mut self) -> std::io::Result<()> {
        let movi_end = self.pos()?;
        self.out.write_all(b"idx1")?;
        self.u32(self.index.len() as u32 * 16)?;
        let index = std::mem::take(&mut self.index);
        for e in &index {
            self.out.write_all(&e.id)?;
            self.u32(e.flags)?;
            self.u32(e.offset)?;
            self.u32(e.size)?;
        }
        let end = self.pos()?;
        let patches = [
            (self.riff_size_at, (end - 8) as u32),
            (self.movi_size_at, (movi_end - self.movi_start) as u32),
            (self.total_frames_at, self.frames),
            (self.total_frames_at + 12, self.max_chunk + 8),
            (self.video_length_at, self.frames),
            (self.video_length_at + 4, self.max_chunk + 8),
            (self.audio_length_at, self.audio_samples),
        ];
        for (at, v) in patches {
            self.out.seek(SeekFrom::Start(at))?;
            self.u32(v)?;
        }
        self.out.flush()
    }
}

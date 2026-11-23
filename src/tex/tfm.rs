//! TFM and pTeX/upTeX JFM metric files.
//!
//! Only what DVI interpretation needs is kept: the character widths.

use std::collections::HashMap;

/// Writing direction a JFM was made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JfmDir {
    Yoko,
    Tate,
}

/// Character widths of one TFM or JFM, as fix_words relative to the at-size.
#[derive(Debug)]
pub struct Metrics {
    bc: u32,
    /// width index of each char_info entry, `bc..=ec`.
    width_index: Vec<u8>,
    widths: Vec<i32>,
    /// `Some` for a JFM: character code to char_type (missing codes are type 0).
    jfm: Option<Jfm>,
}

#[derive(Debug)]
struct Jfm {
    dir: JfmDir,
    types: HashMap<u32, u32>,
}

impl Metrics {
    pub fn parse(data: &[u8]) -> Result<Metrics, String> {
        let half = |i: usize| -> Result<u32, String> {
            data.get(2 * i..2 * i + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
                .ok_or_else(|| "truncated TFM".to_string())
        };
        let word = |i: usize| -> Result<[u8; 4], String> {
            data.get(4 * i..4 * i + 4)
                .map(|b| [b[0], b[1], b[2], b[3]])
                .ok_or_else(|| "truncated TFM".to_string())
        };
        let id = half(0)?;
        let (jfm_dir, base) = match id {
            11 => (Some(JfmDir::Yoko), 1),
            9 => (Some(JfmDir::Tate), 1),
            _ => (None, 0),
        };
        let nt = if jfm_dir.is_some() { half(1)? as usize } else { 0 };
        // the 12 halfword lengths start at halfword `2*base`
        let h0 = 2 * base;
        let lh = half(h0 + 1)? as usize;
        let bc = half(h0 + 2)?;
        let ec = half(h0 + 3)?;
        let nw = half(h0 + 4)? as usize;
        let header_words = (h0 + 12) / 2; // in words
        let mut pos = header_words + lh;
        let mut types = HashMap::new();
        if jfm_dir.is_some() {
            for _ in 0..nt {
                let b = word(pos)?;
                // pTeX: code(16) type(16); upTeX: code low 16, code high 8, type 8.
                let code = ((b[2] as u32) << 16) | ((b[0] as u32) << 8) | b[1] as u32;
                types.insert(code, b[3] as u32);
                pos += 1;
            }
        }
        let n_chars = if ec >= bc { (ec - bc + 1) as usize } else { 0 };
        let mut width_index = Vec::with_capacity(n_chars);
        for _ in 0..n_chars {
            width_index.push(word(pos)?[0]);
            pos += 1;
        }
        let mut widths = Vec::with_capacity(nw);
        for _ in 0..nw {
            widths.push(i32::from_be_bytes(word(pos)?));
            pos += 1;
        }
        Ok(Metrics {
            bc,
            width_index,
            widths,
            jfm: jfm_dir.map(|dir| Jfm { dir, types }),
        })
    }

    pub fn jfm_dir(&self) -> Option<JfmDir> {
        self.jfm.as_ref().map(|j| j.dir)
    }

    /// Width of `code` as a fix_word (2^20 = at-size); 0 for missing characters.
    pub fn width_fix(&self, code: u32) -> i32 {
        let slot = match &self.jfm {
            Some(j) => j.types.get(&code).copied().unwrap_or(0),
            None => code,
        };
        let Some(i) = slot.checked_sub(self.bc) else { return 0 };
        match self.width_index.get(i as usize) {
            Some(&wi) => self.widths.get(wi as usize).copied().unwrap_or(0),
            None => 0,
        }
    }
}

/// `fix * z / 2^20`, the TeX way of scaling a fix_word by a size in sp.
pub fn scale(fix: i32, z: i32) -> i32 {
    ((fix as i64 * z as i64) >> 20) as i32
}

//! Virtual font files.

use std::collections::HashMap;

/// A font referenced by a VF, with its size as a fix_word of the VF's at-size.
#[derive(Debug, Clone)]
pub struct VfFontDef {
    pub number: u32,
    pub name: String,
    pub scale_fix: i32,
}

#[derive(Debug)]
pub struct Vf {
    pub fonts: Vec<VfFontDef>,
    /// DVI fragment of each character; dimensions in it are fix_words.
    packets: HashMap<u32, Vec<u8>>,
}

impl Vf {
    pub fn parse(data: &[u8]) -> Result<Vf, String> {
        let err = || "truncated VF".to_string();
        let mut r = Reader { data, pos: 0 };
        if r.u8().ok_or_else(err)? != 247 || r.u8().ok_or_else(err)? != 202 {
            return Err("not a VF file".into());
        }
        let k = r.u8().ok_or_else(err)? as usize;
        r.skip(k + 8).ok_or_else(err)?;
        let mut fonts = Vec::new();
        let mut packets = HashMap::new();
        loop {
            let op = r.u8().ok_or_else(err)?;
            match op {
                0..=241 => {
                    let pl = op as usize;
                    let cc = r.u8().ok_or_else(err)? as u32;
                    r.skip(3).ok_or_else(err)?;
                    packets.insert(cc, r.bytes(pl).ok_or_else(err)?.to_vec());
                }
                242 => {
                    let pl = r.uint(4).ok_or_else(err)? as usize;
                    let cc = r.uint(4).ok_or_else(err)?;
                    r.skip(4).ok_or_else(err)?;
                    packets.insert(cc, r.bytes(pl).ok_or_else(err)?.to_vec());
                }
                243..=246 => {
                    let number = r.uint((op - 242) as usize).ok_or_else(err)?;
                    r.skip(4).ok_or_else(err)?; // checksum
                    let scale_fix = r.uint(4).ok_or_else(err)? as i32;
                    r.skip(4).ok_or_else(err)?; // design size
                    let a = r.u8().ok_or_else(err)? as usize;
                    let l = r.u8().ok_or_else(err)? as usize;
                    let name = r.bytes(a + l).ok_or_else(err)?[a..].to_vec();
                    fonts.push(VfFontDef {
                        number,
                        name: String::from_utf8_lossy(&name).into_owned(),
                        scale_fix,
                    });
                }
                248 => break,
                _ => return Err(format!("bad VF opcode {op}")),
            }
        }
        Ok(Vf { fonts, packets })
    }

    pub fn packet(&self, code: u32) -> Option<&[u8]> {
        self.packets.get(&code).map(Vec::as_slice)
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }
    fn uint(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 8) | self.u8()? as u32;
        }
        Some(v)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.bytes(n).map(|_| ())
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }
}

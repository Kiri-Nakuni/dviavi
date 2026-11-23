//! DVI files (TeX, pTeX and upTeX flavours): opcode decoding and the page list.

/// One decoded DVI command. Dimensions are raw (DVI units, or fix_words in a VF packet).
#[derive(Debug)]
pub enum Op<'a> {
    /// `set_char`/`set1..4` (`move` = true) or `put1..4`.
    Char { code: u32, advance: bool },
    Rule { height: i32, width: i32, advance: bool },
    Push,
    Pop,
    Right(i32),
    /// `w0`/`x0`/`y0`/`z0` carry `None` (reuse the register).
    W(Option<i32>),
    X(Option<i32>),
    Down(i32),
    Y(Option<i32>),
    Z(Option<i32>),
    Font(u32),
    Special(&'a [u8]),
    /// pTeX `dir`: 0 yoko, 1 tate, 3 dtou.
    Dir(u8),
    FontDef { number: u32, scale: i32, name: String },
    Bop,
    Eop,
    Nop,
    Post,
}

/// Sequential decoder over a byte slice.
pub struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Decoder<'a> {
        Decoder { data, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    fn u8(&mut self) -> Result<u8, String> {
        let b = *self.data.get(self.pos).ok_or("unexpected end of DVI")?;
        self.pos += 1;
        Ok(b)
    }

    fn uint(&mut self, n: usize) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 8) | self.u8()? as u32;
        }
        Ok(v)
    }

    fn int(&mut self, n: usize) -> Result<i32, String> {
        let v = self.uint(n)?;
        let shift = 32 - 8 * n as u32;
        Ok(((v << shift) as i32) >> shift)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.data.get(self.pos..self.pos + n).ok_or("unexpected end of DVI")?;
        self.pos += n;
        Ok(s)
    }

    /// Decodes the next command; `None` at the end of the data.
    pub fn next(&mut self) -> Result<Option<Op<'a>>, String> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let op = self.u8()?;
        let r = match op {
            0..=127 => Op::Char { code: op as u32, advance: true },
            128..=131 => Op::Char { code: self.uint((op - 127) as usize)?, advance: true },
            132 => Op::Rule { height: self.int(4)?, width: self.int(4)?, advance: true },
            133..=136 => Op::Char { code: self.uint((op - 132) as usize)?, advance: false },
            137 => Op::Rule { height: self.int(4)?, width: self.int(4)?, advance: false },
            138 => Op::Nop,
            139 => {
                self.bytes(44)?;
                Op::Bop
            }
            140 => Op::Eop,
            141 => Op::Push,
            142 => Op::Pop,
            143..=146 => Op::Right(self.int((op - 142) as usize)?),
            147 => Op::W(None),
            148..=151 => Op::W(Some(self.int((op - 147) as usize)?)),
            152 => Op::X(None),
            153..=156 => Op::X(Some(self.int((op - 152) as usize)?)),
            157..=160 => Op::Down(self.int((op - 156) as usize)?),
            161 => Op::Y(None),
            162..=165 => Op::Y(Some(self.int((op - 161) as usize)?)),
            166 => Op::Z(None),
            167..=170 => Op::Z(Some(self.int((op - 166) as usize)?)),
            171..=234 => Op::Font((op - 171) as u32),
            235..=238 => Op::Font(self.uint((op - 234) as usize)?),
            239..=242 => {
                let n = self.uint((op - 238) as usize)? as usize;
                Op::Special(self.bytes(n)?)
            }
            243..=246 => {
                let number = self.uint((op - 242) as usize)?;
                self.bytes(4)?;
                let scale = self.int(4)?;
                self.bytes(4)?;
                let a = self.u8()? as usize;
                let l = self.u8()? as usize;
                let name = String::from_utf8_lossy(&self.bytes(a + l)?[a..]).into_owned();
                Op::FontDef { number, scale, name }
            }
            248 | 249 => Op::Post,
            255 => Op::Dir(self.u8()?),
            _ => return Err(format!("unsupported DVI opcode {op} at byte {}", self.pos - 1)),
        };
        Ok(Some(r))
    }
}

#[derive(Debug, Clone)]
pub struct FontDef {
    pub number: u32,
    pub name: String,
    pub scale: i32,
}

/// A DVI file split into pages.
pub struct Dvi {
    data: Vec<u8>,
    /// Byte offset of the first command after each `bop`.
    pages: Vec<usize>,
    pub fonts: Vec<FontDef>,
    /// Metres per DVI unit, including the magnification.
    pub unit_m: f64,
    pub mag: f64,
}

impl Dvi {
    pub fn parse(data: Vec<u8>) -> Result<Dvi, String> {
        if data.len() < 15 || data[0] != 247 || !(data[1] == 2 || data[1] == 3) {
            return Err("not a DVI file".into());
        }
        let be = |i: usize| u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        let (num, den, mag) = (be(2) as f64, be(6) as f64, be(10) as f64);
        let k = data[14] as usize;
        let mut pages = Vec::new();
        let mut fonts: Vec<FontDef> = Vec::new();
        let mut dec = Decoder::new(&data, 15 + k);
        while let Some(op) = dec.next()? {
            match op {
                Op::Bop => pages.push(dec.pos()),
                Op::FontDef { number, scale, name } => {
                    if !fonts.iter().any(|f| f.number == number) {
                        fonts.push(FontDef { number, name, scale });
                    }
                }
                Op::Post => break,
                _ => {}
            }
        }
        // the postamble repeats every font definition; read them too
        let mut post = Decoder::new(&data, dec.pos() + 28);
        while let Some(op) = post.next()? {
            match op {
                Op::FontDef { number, scale, name } => {
                    if !fonts.iter().any(|f| f.number == number) {
                        fonts.push(FontDef { number, name, scale });
                    }
                }
                Op::Nop => {}
                _ => break,
            }
        }
        Ok(Dvi { pages, fonts, unit_m: num / den * 1e-7 * mag / 1000.0, mag: mag / 1000.0, data })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Decoder positioned at the start of page `i` (it stops at the `eop`).
    pub fn page(&self, i: usize) -> Decoder<'_> {
        Decoder::new(&self.data, self.pages[i])
    }

    /// The raw bytes of page `i`; equal bytes give equal pictures.
    pub fn page_bytes(&self, i: usize) -> &[u8] {
        let start = self.pages[i];
        let end = self.pages.get(i + 1).map(|&p| p - 45).unwrap_or(self.data.len());
        &self.data[start..end]
    }

    /// The specials of page `i` in order (not looking into virtual fonts).
    pub fn page_specials(&self, i: usize) -> Result<Vec<&[u8]>, String> {
        let mut dec = self.page(i);
        let mut out = Vec::new();
        while let Some(op) = dec.next()? {
            match op {
                Op::Special(s) => out.push(s),
                Op::Eop => break,
                _ => {}
            }
        }
        Ok(out)
    }
}

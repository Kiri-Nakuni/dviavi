//! Font map files: `pdftex.map` (TFM name to Type 1 file) and dvipdfmx's
//! `kanjix.map` (JFM name to OpenType file).

use std::collections::HashMap;
use std::path::Path;

/// How the character codes of a mapped font select glyphs.
#[derive(Debug, Clone, PartialEq)]
pub enum CodeMap {
    /// Type 1 with the font's built-in encoding.
    BuiltIn,
    /// Type 1 re-encoded by an `.enc` file.
    EncFile(String),
    /// JIS X 0208 codes (pTeX `H` / `2004-H` / `V` CMaps).
    Jis,
    /// Unicode code points (upTeX `UniJIS*-UTF16-*` CMaps).
    Unicode,
}

#[derive(Debug, Clone)]
pub struct MapEntry {
    pub file: String,
    pub face_index: isize,
    pub code_map: CodeMap,
    pub slant: f64,
    pub extend: f64,
}

/// All entries of the map files, keyed by TFM name.
#[derive(Default)]
pub struct FontMap {
    entries: HashMap<String, MapEntry>,
}

impl FontMap {
    /// Loads `pdftex.map` and `kanjix.map` as found by kpsewhich; missing files
    /// are skipped with a warning.
    pub fn load() -> FontMap {
        let mut map = FontMap::default();
        for (name, parse) in [
            ("pdftex.map", parse_pdftex_line as fn(&str) -> Option<(String, MapEntry)>),
            ("kanjix.map", parse_dvipdfmx_line),
        ] {
            match super::kpse::find(name).and_then(|p| read_lossy(&p)) {
                Some(text) => {
                    for line in text.lines() {
                        if let Some((k, e)) = parse(line) {
                            map.entries.entry(k).or_insert(e);
                        }
                    }
                }
                None => eprintln!("dviavi: warning: {name} not found"),
            }
        }
        map
    }

    pub fn get(&self, tfm: &str) -> Option<&MapEntry> {
        self.entries.get(tfm)
    }
}

fn read_lossy(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// `tfm [psname] ["effects"] [<enc] [<font]`
fn parse_pdftex_line(line: &str) -> Option<(String, MapEntry)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(['%', '#', ';', '*']) {
        return None;
    }
    let mut tokens = Vec::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if c.is_whitespace() {
            rest = &rest[1..];
        } else if c == '"' {
            let end = rest[1..].find('"').map(|i| i + 1).unwrap_or(rest.len() - 1);
            tokens.push(&rest[..=end]);
            rest = &rest[end + 1..];
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            tokens.push(&rest[..end]);
            rest = &rest[end..];
        }
    }
    let tfm = tokens.first()?.to_string();
    let mut entry = MapEntry {
        file: String::new(),
        face_index: 0,
        code_map: CodeMap::BuiltIn,
        slant: 0.0,
        extend: 1.0,
    };
    for tok in &tokens[1..] {
        if let Some(effects) = tok.strip_prefix('"') {
            let words: Vec<&str> = effects.trim_end_matches('"').split_whitespace().collect();
            for w in words.windows(2) {
                match w[1] {
                    "SlantFont" => entry.slant = w[0].parse().unwrap_or(0.0),
                    "ExtendFont" => entry.extend = w[0].parse().unwrap_or(1.0),
                    _ => {}
                }
            }
        } else if let Some(f) = tok.strip_prefix('<') {
            let f = f.trim_start_matches(['<', '[']);
            if f.ends_with(".enc") {
                entry.code_map = CodeMap::EncFile(f.to_string());
            } else {
                entry.file = f.to_string();
            }
        }
    }
    (!entry.file.is_empty()).then_some((tfm, entry))
}

/// `tfm cmap font [options]` with `:n:` / `!` prefixes on the font.
fn parse_dvipdfmx_line(line: &str) -> Option<(String, MapEntry)> {
    let line = line.split('%').next()?.trim();
    let mut tokens = line.split_whitespace();
    let tfm = tokens.next()?.to_string();
    let cmap = tokens.next()?;
    let mut file = tokens.next()?.trim_start_matches('!');
    let mut face_index = 0;
    if let Some(r) = file.strip_prefix(':') {
        let (n, f) = r.split_once(':')?;
        face_index = n.parse().ok()?;
        file = f;
    }
    let code_map = if cmap.starts_with("Uni") {
        CodeMap::Unicode
    } else if matches!(cmap, "H" | "V" | "2004-H" | "2004-V") {
        CodeMap::Jis
    } else {
        return None;
    };
    let mut entry = MapEntry {
        file: file.to_string(),
        face_index,
        code_map,
        slant: 0.0,
        extend: 1.0,
    };
    let opts: Vec<&str> = tokens.collect();
    for w in opts.windows(2) {
        match w[0] {
            "-s" => entry.slant = w[1].parse().unwrap_or(0.0),
            "-e" => entry.extend = w[1].parse().unwrap_or(1.0),
            _ => {}
        }
    }
    Some((tfm, entry))
}

/// Glyph names of an `.enc` file (`/Name [ /a /b ... ] def`).
pub fn parse_enc(text: &str) -> Vec<String> {
    let mut body = String::new();
    for line in text.lines() {
        body.push_str(line.split('%').next().unwrap_or(""));
        body.push(' ');
    }
    let Some(start) = body.find('[') else { return Vec::new() };
    let end = body[start..].find(']').map(|i| start + i).unwrap_or(body.len());
    body[start + 1..end]
        .split_whitespace()
        .map(|n| n.trim_start_matches('/').to_string())
        .collect()
}

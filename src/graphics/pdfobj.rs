//! PDF objects as they appear in dvipdfmx specials and content streams.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    Array(Vec<Obj>),
    Dict(HashMap<String, Obj>),
    /// `@name`: an object defined by `pdf:obj` (resolved by the caller).
    Ref(String),
}

impl Obj {
    pub fn num(&self) -> Option<f64> {
        match self {
            Obj::Num(v) => Some(*v),
            _ => None,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn dict(&self) -> Option<&HashMap<String, Obj>> {
        match self {
            Obj::Dict(d) => Some(d),
            _ => None,
        }
    }

    /// The numbers of an array (non-numbers are skipped).
    pub fn nums(&self) -> Vec<f64> {
        self.array().map(|a| a.iter().filter_map(Obj::num).collect()).unwrap_or_default()
    }

    /// Merges `other` into `self` (dictionaries key by key, arrays appended),
    /// as `pdf:put` does.
    pub fn merge(&mut self, other: Obj) {
        match (self, other) {
            (Obj::Dict(a), Obj::Dict(b)) => {
                for (k, v) in b {
                    match a.get_mut(&k) {
                        Some(old @ Obj::Dict(_)) if matches!(v, Obj::Dict(_)) => old.merge(v),
                        _ => {
                            a.insert(k, v);
                        }
                    }
                }
            }
            (Obj::Array(a), Obj::Array(b)) => a.extend(b),
            (s, o) => *s = o,
        }
    }
}

/// A content-stream token: an operand or an operator.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Obj(Obj),
    Op(String),
}

pub struct Lexer<'a> {
    s: &'a [u8],
    pos: usize,
}

fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

impl<'a> Lexer<'a> {
    pub fn new(s: &'a str) -> Lexer<'a> {
        Lexer { s: s.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if c == b'%' {
                while self.peek().is_some_and(|c| c != b'\n') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &'a str {
        let start = self.pos;
        while self.peek().is_some_and(|c| !c.is_ascii_whitespace() && !is_delim(c)) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.s[start..self.pos]).unwrap_or("")
    }

    /// The next token; `None` at the end. Errors stop the lexer.
    pub fn next_token(&mut self) -> Option<Token> {
        self.skip_space();
        let c = self.peek()?;
        let tok = match c {
            b'/' => {
                self.pos += 1;
                Token::Obj(Obj::Name(self.word().to_string()))
            }
            b'@' => {
                self.pos += 1;
                Token::Obj(Obj::Ref(self.word().to_string()))
            }
            b'(' => Token::Obj(Obj::Str(self.string())),
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    match self.peek() {
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        None => break,
                        _ => match self.next_token()? {
                            Token::Obj(o) => items.push(o),
                            Token::Op(w) => items.push(keyword(&w)),
                        },
                    }
                }
                Token::Obj(Obj::Array(items))
            }
            b'<' if self.s.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let mut d = HashMap::new();
                loop {
                    self.skip_space();
                    if self.s.get(self.pos..self.pos + 2) == Some(b">>") {
                        self.pos += 2;
                        break;
                    }
                    let Some(Token::Obj(Obj::Name(k))) = self.next_token() else { break };
                    let v = match self.next_token()? {
                        Token::Obj(o) => o,
                        Token::Op(w) => keyword(&w),
                    };
                    d.insert(k, v);
                }
                Token::Obj(Obj::Dict(d))
            }
            b'<' => {
                self.pos += 1;
                let start = self.pos;
                while self.peek().is_some_and(|c| c != b'>') {
                    self.pos += 1;
                }
                let hex: Vec<u8> = self.s[start..self.pos].iter().copied().filter(u8::is_ascii_hexdigit).collect();
                self.pos += 1;
                let bytes = hex
                    .chunks(2)
                    .map(|p| {
                        let h = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                        h(p[0]) * 16 + if p.len() > 1 { h(p[1]) } else { 0 }
                    })
                    .collect();
                Token::Obj(Obj::Str(bytes))
            }
            b']' | b'>' | b')' | b'{' | b'}' => {
                self.pos += 1;
                return self.next_token();
            }
            _ => {
                let w = self.word();
                if w.is_empty() {
                    self.pos += 1;
                    return self.next_token();
                }
                match w.parse::<f64>() {
                    Ok(v) => Token::Obj(Obj::Num(v)),
                    Err(_) => match w {
                        "true" | "false" | "null" => Token::Obj(keyword(w)),
                        _ => Token::Op(w.to_string()),
                    },
                }
            }
        };
        Some(tok)
    }

    fn string(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'\\' => {
                    if let Some(e) = self.peek() {
                        self.pos += 1;
                        out.push(match e {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            _ => e,
                        });
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// The next object (operators read as keywords).
    pub fn next_obj(&mut self) -> Option<Obj> {
        match self.next_token()? {
            Token::Obj(o) => Some(o),
            Token::Op(w) => Some(keyword(&w)),
        }
    }
}

fn keyword(w: &str) -> Obj {
    match w {
        "true" => Obj::Bool(true),
        "false" => Obj::Bool(false),
        "null" => Obj::Null,
        // `12 0 R` style references are not used by the drivers dviavi reads
        _ => Obj::Name(w.to_string()),
    }
}

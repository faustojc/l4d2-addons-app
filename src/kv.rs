//! Tiny, lenient Valve KeyValues reader (addoninfo.txt, addonlist.txt, libraryfolders.vdf).

#[derive(Debug, Clone)]
pub enum Value {
    Str(String),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(v) => v
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn pairs(&self) -> &[(String, Value)] {
        match self {
            Value::Obj(v) => v,
            _ => &[],
        }
    }
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

enum Tok {
    Str(String),
    Open,
    Close,
}

impl Lexer<'_> {
    fn next(&mut self) -> Option<Tok> {
        let s = self.s;
        loop {
            while self.i < s.len() && s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.i + 1 < s.len() && s[self.i] == b'/' && s[self.i + 1] == b'/' {
                while self.i < s.len() && s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            // Platform conditionals like [$WIN32] are ignored.
            if self.i < s.len() && s[self.i] == b'[' {
                while self.i < s.len() && s[self.i] != b']' {
                    self.i += 1;
                }
                self.i += 1;
                continue;
            }
            break;
        }
        let c = *s.get(self.i)?;
        self.i += 1;
        match c {
            b'{' => Some(Tok::Open),
            b'}' => Some(Tok::Close),
            b'"' => {
                let mut out = Vec::new();
                while self.i < s.len() && s[self.i] != b'"' {
                    if s[self.i] == b'\\' && s.get(self.i + 1) == Some(&b'"') {
                        out.push(b'"');
                        self.i += 2;
                        continue;
                    }
                    out.push(s[self.i]);
                    self.i += 1;
                }
                self.i += 1;
                Some(Tok::Str(String::from_utf8_lossy(&out).into_owned()))
            }
            _ => {
                let start = self.i - 1;
                while self.i < s.len()
                    && !s[self.i].is_ascii_whitespace()
                    && !b"{}\"".contains(&s[self.i])
                {
                    self.i += 1;
                }
                Some(Tok::Str(
                    String::from_utf8_lossy(&s[start..self.i]).into_owned(),
                ))
            }
        }
    }
}

fn parse_obj(lx: &mut Lexer) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    while let Some(t) = lx.next() {
        match t {
            Tok::Close => break,
            Tok::Open => {
                parse_obj(lx);
            }
            Tok::Str(k) => match lx.next() {
                Some(Tok::Str(v)) => out.push((k, Value::Str(v))),
                Some(Tok::Open) => out.push((k, Value::Obj(parse_obj(lx)))),
                Some(Tok::Close) | None => break,
            },
        }
    }
    out
}

/// Parses a document. Returns the root object (`"Name" { ... }` becomes one pair).
pub fn parse(bytes: &[u8]) -> Value {
    // Strip UTF-8 / UTF-16 BOM-ish garbage; UTF-16 files are decoded crudely.
    let owned;
    let bytes = if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        owned = String::from_utf16_lossy(&u).into_bytes();
        &owned[..]
    } else {
        bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
    };
    Value::Obj(parse_obj(&mut Lexer { s: bytes, i: 0 }))
}

/// Returns the first top-level object regardless of its name (e.g. "AddonInfo").
pub fn parse_root(bytes: &[u8]) -> Value {
    let doc = parse(bytes);
    doc.pairs()
        .iter()
        .find_map(|(_, v)| matches!(v, Value::Obj(_)).then(|| v.clone()))
        .unwrap_or(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_addoninfo() {
        let v = parse_root(b"\xEF\xBB\xBF\"AddonInfo\"\n{\n // c\n addontitle \"My \\\"Mod\\\"\"\n \"addonContent_Skin\" 1\n sub { a b }\n}\n");
        assert_eq!(v.str("AddonTitle"), Some("My \"Mod\""));
        assert_eq!(v.str("addoncontent_skin"), Some("1"));
        assert_eq!(v.get("sub").unwrap().str("a"), Some("b"));
    }
    #[test]
    fn keeps_backslashes() {
        let v = parse_root(b"\"AddonList\" { \"workshop\\123.vpk\" \"1\" }");
        assert_eq!(v.pairs()[0].0, "workshop\\123.vpk");
    }
}

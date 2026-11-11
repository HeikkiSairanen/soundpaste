//! A small JSON writer and parser for bench results (no dependencies).

use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn str(s: &str) -> Json {
        Json::Str(s.to_string())
    }
    pub fn num(v: impl Into<f64>) -> Json {
        Json::Num(v.into())
    }
    /// NaN and infinities are not JSON; they are stored as null.
    pub fn opt(v: Option<f64>) -> Json {
        match v {
            Some(x) if x.is_finite() => Json::Num(x),
            _ => Json::Null,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => &[],
        }
    }
    pub fn num_at(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.f64())
    }
    pub fn str_at(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }

    pub fn pretty(&self) -> String {
        let mut s = String::new();
        self.write(&mut s, 0);
        s.push('\n');
        s
    }

    fn is_flat(&self) -> bool {
        match self {
            Json::Arr(a) => a.iter().all(|v| !matches!(v, Json::Arr(_) | Json::Obj(_))),
            Json::Obj(o) => o.iter().all(|(_, v)| !matches!(v, Json::Arr(_) | Json::Obj(_))) && o.len() <= 8,
            _ => true,
        }
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{}", (n * 1e6).round() / 1e6);
                }
            }
            Json::Str(s) => {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c if (c as u32) < 0x20 => {
                            let _ = write!(out, "\\u{:04x}", c as u32);
                        }
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Json::Arr(a) => {
                let flat = self.is_flat();
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push_str(if flat { ", " } else { "," });
                    }
                    if !flat {
                        out.push('\n');
                        out.push_str(&"  ".repeat(indent + 1));
                    }
                    v.write(out, indent + 1);
                }
                if !flat && !a.is_empty() {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent));
                }
                out.push(']');
            }
            Json::Obj(o) => {
                let flat = self.is_flat();
                out.push('{');
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        out.push_str(if flat { ", " } else { "," });
                    }
                    if !flat {
                        out.push('\n');
                        out.push_str(&"  ".repeat(indent + 1));
                    }
                    Json::Str(k.clone()).write(out, 0);
                    out.push_str(": ");
                    v.write(out, indent + 1);
                }
                if !flat && !o.is_empty() {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent));
                }
                out.push('}');
            }
        }
    }

    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser { b: text.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("trailing content at {}", p.i));
        }
        Ok(v)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.b.get(self.i).copied() {
            Some(b'{') => {
                self.i += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(pairs));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    pairs.push((k, self.value()?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(pairs));
                        }
                        _ => return Err(format!("invalid object at {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err(format!("invalid array at {}", self.i)),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Json::Bool(true))
            }
            Some(b'f') if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Json::Bool(false))
            }
            Some(b'n') if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(Json::Null)
            }
            Some(_) => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i])
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(Json::Num)
                    .ok_or_else(|| format!("invalid value at {start}"))
            }
            None => Err("unexpected end".into()),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err(format!("expected a string at {}", self.i));
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match self.b.get(self.i).copied() {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(out).map_err(|e| e.to_string());
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.b.get(self.i).copied().ok_or("unterminated string")?;
                    self.i += 1;
                    match c {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4).ok_or("\\u katkeaa")?).map_err(|e| e.to_string())?;
                            let cp = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            self.i += 4;
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = Json::obj(vec![
            ("name", Json::str("café \"x\"\n")),
            ("number", Json::num(-3.25)),
            ("empty", Json::Null),
            ("list", Json::Arr(vec![Json::num(1), Json::Bool(true), Json::obj(vec![("a", Json::Arr(vec![]))])])),
        ]);
        assert_eq!(Json::parse(&v.pretty()).unwrap(), v);
    }
}

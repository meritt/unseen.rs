use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    #[must_use]
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Object(map) => map.get(key).unwrap_or(&Value::Null),
            _ => &Value::Null,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(n) => n.parse().ok(),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser { s: text.as_bytes(), at: 0, depth: 0 };
    let value = p.value()?;
    p.space();
    if p.at != p.s.len() {
        return Err(p.error("trailing data"));
    }
    Ok(value)
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    depth: usize,
}

const MAX_DEPTH: usize = 64;

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("json: {what} at byte {}", self.at)
    }

    fn space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.at) {
            self.at += 1;
        }
    }

    fn expect(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.s[self.at..].starts_with(literal) {
            self.at += literal.len();
            Ok(())
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.space();
        match self.s.get(self.at) {
            Some(b'n') => self.expect(b"null").map(|()| Value::Null),
            Some(b't') => self.expect(b"true").map(|()| Value::Bool(true)),
            Some(b'f') => self.expect(b"false").map(|()| Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => self.nested(Self::array),
            Some(b'{') => self.nested(Self::object),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("expected a value")),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> Result<Value, String>) -> Result<Value, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("nesting too deep"));
        }
        let value = f(self);
        self.depth -= 1;
        value
    }

    fn array(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.s.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.space();
            match self.s.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut map = BTreeMap::new();
        self.space();
        if self.s.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.space();
            if self.s.get(self.at) != Some(&b'"') {
                return Err(self.error("expected a member name"));
            }
            let key = self.string()?;
            self.space();
            self.expect(b":")?;
            let value = self.value()?;
            if map.insert(key, value).is_some() {
                return Err(self.error("duplicate member"));
            }
            self.space();
            match self.s.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        if self.s.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        let digits = |p: &mut Self| {
            let from = p.at;
            while p.s.get(p.at).is_some_and(u8::is_ascii_digit) {
                p.at += 1;
            }
            p.at - from
        };
        let int = self.at;
        if digits(self) == 0 || self.s[int] == b'0' && self.at - int > 1 {
            return Err(self.error("malformed number"));
        }
        if self.s.get(self.at) == Some(&b'.') {
            self.at += 1;
            if digits(self) == 0 {
                return Err(self.error("malformed fraction"));
            }
        }
        if let Some(b'e' | b'E') = self.s.get(self.at) {
            self.at += 1;
            if let Some(b'+' | b'-') = self.s.get(self.at) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return Err(self.error("malformed exponent"));
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.at]).expect("ASCII number");
        Ok(Value::Number(text.to_owned()))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.s.get(self.at..self.at + 4).ok_or_else(|| self.error("short escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.error("bad escape"))?;
        let n = u32::from_str_radix(text, 16).map_err(|_| self.error("bad escape"))?;
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return Err(self.error("bad escape"));
        }
        self.at += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(&b) = self.s.get(self.at) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.at]).expect("input is a str"));
            match self.s.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escape = *self.s.get(self.at).ok_or_else(|| self.error("short escape"))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let high = self.hex4()?;
                            let scalar = if (0xD800..0xDC00).contains(&high) {
                                self.expect(b"\\u")?;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.error("unpaired surrogate"));
                                }
                                0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                            } else {
                                high
                            };
                            out.push(char::from_u32(scalar).ok_or_else(|| self.error("unpaired surrogate"))?);
                        }
                        _ => return Err(self.error("unknown escape")),
                    }
                }
                Some(_) => return Err(self.error("control character in string")),
                None => return Err(self.error("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, parse};

    #[test]
    fn parses_corpus_shapes() {
        let v = parse(r#"{"a": [1, -2.5e3, true, null], "s": "xé😀\n", "n": 18446744073709551615}"#).expect("valid");
        assert_eq!(v.get("s").as_str(), Some("xé😀\n"));
        assert_eq!(v.get("n").as_u64(), Some(u64::MAX));
        assert_eq!(v.get("a").as_array().map(<[Value]>::len), Some(4));
        assert!(v.get("missing").is_null());
    }

    #[test]
    fn rejects_malformed() {
        for bad in [r#"{"a":1,"a":2}"#, "[1,]", "01", r#""\ud800""#, "{} x", r#""a"#, "\"\u{1}\""] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}

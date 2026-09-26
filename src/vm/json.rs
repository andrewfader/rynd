//! JSON data interchange shared by the VM and standalone generated programs.
use super::{runtime::error, value::Value};
use crate::error::{RyndError, RyndResult};
use std::collections::BTreeMap;
use std::fmt::Write;

const MAX_DEPTH: usize = 128;

pub fn parse(text: &str) -> RyndResult<Value> {
    let mut parser = Parser { text, pos: 0 };
    let value = parser.value(0)?;
    parser.space();
    if parser.pos != text.len() {
        return Err(parser.fail("Trailing input"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn fail(&self, message: &str) -> RyndError {
        error(format!("parse_json(): {message} at byte {}", self.pos + 1))
    }
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }
    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }
    fn value(&mut self, depth: usize) -> RyndResult<Value> {
        self.space();
        match self.peek() {
            Some(b'"') => self.string().map(Value::string),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b'[' | b'{') => {
                if depth >= MAX_DEPTH {
                    return Err(self.fail("Nesting exceeds 128 containers"));
                }
                let array = self.eat(b'[');
                if !array {
                    self.pos += 1;
                }
                let end = if array { b']' } else { b'}' };
                let mut items = Vec::new();
                let mut fields = BTreeMap::new();
                self.space();
                if !self.eat(end) {
                    loop {
                        self.space();
                        if array {
                            items.push(self.value(depth + 1)?);
                        } else {
                            let key = self.string()?;
                            self.space();
                            if !self.eat(b':') {
                                return Err(self.fail("Expected ':'"));
                            }
                            fields.insert(key, self.value(depth + 1)?);
                        }
                        self.space();
                        if self.eat(end) {
                            break;
                        }
                        if !self.eat(b',') {
                            return Err(self.fail("Expected ',' or closing delimiter"));
                        }
                    }
                }
                Ok(if array {
                    Value::list(items)
                } else {
                    Value::map(fields)
                })
            }
            _ => {
                for (word, value) in [
                    ("null", Value::Nil),
                    ("true", Value::Bool(true)),
                    ("false", Value::Bool(false)),
                ] {
                    if self.text[self.pos..].starts_with(word) {
                        self.pos += word.len();
                        return Ok(value);
                    }
                }
                Err(self.fail("Expected JSON value"))
            }
        }
    }
    fn hex(&mut self) -> RyndResult<u32> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = self
                .peek()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(|| self.fail("Expected four hexadecimal digits"))?;
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }
    fn string(&mut self) -> RyndResult<String> {
        if !self.eat(b'"') {
            return Err(self.fail("Expected string"));
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.fail("Unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let escaped = self
                        .peek()
                        .ok_or_else(|| self.fail("Unterminated escape"))?;
                    self.pos += 1;
                    out.push(match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut scalar = self.hex()?;
                            if (0xd800..=0xdbff).contains(&scalar) {
                                if !self.eat(b'\\') || !self.eat(b'u') {
                                    return Err(self.fail("Expected low surrogate"));
                                }
                                let low = self.hex()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err(self.fail("Invalid low surrogate"));
                                }
                                scalar = 0x10000 + ((scalar - 0xd800) << 10) + low - 0xdc00;
                            }
                            char::from_u32(scalar)
                                .ok_or_else(|| self.fail("Invalid Unicode scalar"))?
                        }
                        _ => return Err(self.fail("Invalid escape")),
                    });
                }
                Some(0..=31) => return Err(self.fail("Unescaped control character")),
                _ => {
                    let c = self.text[self.pos..].chars().next().unwrap();
                    self.pos += c.len_utf8();
                    out.push(c);
                }
            }
        }
    }
    fn digits(&mut self) -> RyndResult<()> {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if start == self.pos {
            Err(self.fail("Expected digit"))
        } else {
            Ok(())
        }
    }
    fn number(&mut self) -> RyndResult<Value> {
        let start = self.pos;
        self.eat(b'-');
        if !self.eat(b'0') {
            self.digits()?;
        }
        let mut float = false;
        if self.eat(b'.') {
            float = true;
            self.digits()?;
        }
        if self.eat(b'e') || self.eat(b'E') {
            float = true;
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            self.digits()?;
        }
        let text = &self.text[start..self.pos];
        if !float {
            return text
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| self.fail("Integer outside signed 64-bit range"));
        }
        let value = text
            .parse::<f64>()
            .map_err(|_| self.fail("Invalid number"))?;
        if !value.is_finite() {
            return Err(self.fail("Number outside finite f64 range"));
        }
        Ok(Value::Float(value))
    }
}

pub fn stringify(value: &Value) -> RyndResult<String> {
    let mut out = String::new();
    encode(value, &mut out, 0)?;
    Ok(out)
}

fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\u{0000}'..='\u{001f}' => {
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn encode(value: &Value, out: &mut String, depth: usize) -> RyndResult<()> {
    if matches!(value, Value::List(_) | Value::Map(_)) && depth >= MAX_DEPTH {
        return Err(error("to_json(): Nesting exceeds 128 containers"));
    }
    match value {
        Value::Nil => out.push_str("null"),
        Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
        Value::Int(v) => write!(out, "{v}").unwrap(),
        Value::Float(v) if v.is_finite() => {
            let text = format!("{v:?}");
            out.push_str(&text);
        }
        Value::String(v) => quote(v, out),
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode(item, out, depth + 1)?;
            }
            out.push(']');
        }
        Value::Map(fields) => {
            out.push('{');
            for (i, (key, value)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                quote(key, out);
                out.push(':');
                encode(value, out, depth + 1)?;
            }
            out.push('}');
        }
        _ => {
            return Err(error(format!(
                "to_json(): Unsupported {} value",
                value.type_name()
            )));
        }
    }
    Ok(())
}

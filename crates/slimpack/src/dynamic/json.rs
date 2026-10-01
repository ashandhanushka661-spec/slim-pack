//! JSON conversion for [`Value`] (`spec/SPEC.md` §4.2).
//!
//! Hand-written so that the core crate keeps zero dependencies, and so the
//! rules of §4.2 can be enforced exactly:
//!
//! | JSON | Value |
//! |------|-------|
//! | integer that fits `i64` | `Int` |
//! | integer in `i64::MAX+1 ..= u64::MAX` | `UInt` |
//! | integer outside `u64` | [`SlimError::NumberOutOfRange`] |
//! | number with a fraction or exponent | `Float` |
//! | `true`/`false`/`null`/string/array/object | `Bool`/`Null`/`Str`/`Array`/`Map` |
//!
//! Writing JSON back appends `.0` to a float that would otherwise re-parse as an
//! integer, and rejects non-finite floats with [`SlimError::NonFiniteFloat`].

use super::Value;
use crate::error::{Result, SlimError};
use crate::limits::Limits;

/// Parses a JSON document into a [`Value`].
pub fn parse_json(input: &str, limits: &Limits) -> Result<Value> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        pos: 0,
        limits,
        depth: 0,
    };
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(SlimError::TrailingBytes {
            extra: parser.bytes.len() - parser.pos,
        });
    }
    Ok(value)
}

/// Renders `value` as JSON.
///
/// Non-finite floats are rejected; every other value has an exact rendering.
pub fn write_json(value: &Value, out: &mut String) -> Result<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(v) => out.push_str(&v.to_string()),
        Value::UInt(v) => out.push_str(&v.to_string()),
        Value::Float(v) => {
            if !v.is_finite() {
                return Err(SlimError::NonFiniteFloat);
            }
            // Rust's Display for f64 is the shortest text that round-trips.
            let text = format!("{v}");
            out.push_str(&text);
            if !text.contains('.') && !text.contains('e') && !text.contains('E') {
                out.push_str(".0");
            }
        }
        Value::Str(s) => write_string(s, out),
        Value::Bytes(b) => {
            // Bytes have no JSON counterpart; they are shown as a hex string so
            // that the output stays valid JSON and round-trips readably.
            out.push('"');
            for byte in b {
                out.push_str(&format!("{byte:02x}"));
            }
            out.push('"');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, out)?;
            }
            out.push(']');
        }
        Value::Map(pairs) => {
            out.push('{');
            for (i, (key, item)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_json(item, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// Convenience wrapper around [`write_json`].
pub fn to_json(value: &Value) -> Result<String> {
    let mut out = String::new();
    write_json(value, &mut out)?;
    Ok(out)
}

fn write_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    limits: &'a Limits,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Result<u8> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or(SlimError::truncated(self.pos + 1, self.bytes.len()))
    }

    fn skip_whitespace(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.pos) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.peek()? == byte {
            self.pos += 1;
            Ok(())
        } else {
            Err(SlimError::UnsupportedLayout(match byte {
                b'[' => "expected '['",
                b']' => "expected ']'",
                b'{' => "expected '{'",
                b'}' => "expected '}'",
                b':' => "expected ':'",
                b',' => "expected ','",
                _ => "unexpected byte in JSON",
            }))
        }
    }

    fn literal(&mut self, word: &str) -> Result<()> {
        let end = self.pos + word.len();
        if self.bytes.get(self.pos..end) == Some(word.as_bytes()) {
            self.pos = end;
            Ok(())
        } else {
            Err(SlimError::UnsupportedLayout("invalid JSON literal"))
        }
    }

    fn value(&mut self) -> Result<Value> {
        self.skip_whitespace();
        match self.peek()? {
            b'n' => {
                self.literal("null")?;
                Ok(Value::Null)
            }
            b't' => {
                self.literal("true")?;
                Ok(Value::Bool(true))
            }
            b'f' => {
                self.literal("false")?;
                Ok(Value::Bool(false))
            }
            b'"' => Ok(Value::Str(self.string()?)),
            b'[' => self.array(),
            b'{' => self.object(),
            _ => self.number(),
        }
    }

    fn array(&mut self) -> Result<Value> {
        self.limits.check_depth(self.depth + 1)?;
        self.expect(b'[')?;
        self.depth += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(SlimError::UnsupportedLayout("expected ',' or ']'")),
            }
        }
        self.depth -= 1;
        Ok(Value::Array(items))
    }

    fn object(&mut self) -> Result<Value> {
        self.limits.check_depth(self.depth + 1)?;
        self.expect(b'{')?;
        self.depth += 1;
        let mut pairs: Vec<(String, Value)> = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Map(pairs));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.value()?;
            // Document order is preserved and duplicate keys are kept (spec 4.2).
            pairs.push((key, value));
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(SlimError::UnsupportedLayout("expected ',' or '}'")),
            }
        }
        self.depth -= 1;
        Ok(Value::Map(pairs))
    }

    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self.peek()?;
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self.peek()?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(SlimError::InvalidUtf8),
                    }
                }
                byte if byte < 0x20 => {
                    return Err(SlimError::UnsupportedLayout(
                        "raw control character in JSON string",
                    ))
                }
                byte => {
                    // Copy one whole UTF-8 sequence.
                    let len = utf8_len(byte);
                    let end = self
                        .pos
                        .checked_add(len - 1)
                        .ok_or(SlimError::SizeOverflow)?;
                    let slice = self
                        .bytes
                        .get(self.pos - 1..end)
                        .ok_or_else(|| SlimError::truncated(end, self.bytes.len()))?;
                    out.push_str(std::str::from_utf8(slice).map_err(|_| SlimError::InvalidUtf8)?);
                    self.pos = end;
                }
            }
        }
    }

    /// Reads `\uXXXX`, joining a surrogate pair into one `char`.
    fn unicode_escape(&mut self) -> Result<char> {
        let first = self.hex4()?;
        if (0xD800..0xDC00).contains(&first) {
            self.expect(b'\\')?;
            self.expect(b'u')?;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(SlimError::InvalidUtf8);
            }
            let combined = 0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00);
            return char::from_u32(combined).ok_or(SlimError::InvalidUtf8);
        }
        char::from_u32(first).ok_or(SlimError::InvalidUtf8)
    }

    fn hex4(&mut self) -> Result<u32> {
        let end = self.pos + 4;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| SlimError::truncated(end, self.bytes.len()))?;
        let text = std::str::from_utf8(slice).map_err(|_| SlimError::InvalidUtf8)?;
        let value = u32::from_str_radix(text, 16).map_err(|_| SlimError::InvalidUtf8)?;
        self.pos = end;
        Ok(value)
    }

    /// Parses a number, choosing `Int`, `UInt` or `Float` per §4.2.
    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        let mut float = false;
        while let Some(b) = self.bytes.get(self.pos) {
            match b {
                b'0'..=b'9' => self.pos += 1,
                b'.' | b'e' | b'E' | b'+' | b'-' => {
                    float = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| SlimError::InvalidUtf8)?;
        if text.is_empty() || text == "-" {
            return Err(SlimError::UnsupportedLayout("invalid JSON number"));
        }
        if float {
            return text
                .parse::<f64>()
                .map(Value::Float)
                .map_err(|_| SlimError::UnsupportedLayout("invalid JSON number"));
        }
        if let Ok(v) = text.parse::<i64>() {
            return Ok(Value::Int(v));
        }
        if let Ok(v) = text.parse::<u64>() {
            return Ok(Value::UInt(v));
        }
        // The token has no '.', 'e' or 'E', so it is an integer: outside u64 it
        // is out of range rather than a float (spec 4.2).
        Err(SlimError::NumberOutOfRange)
    }
}

/// Bytes needed for a UTF-8 sequence starting with `first`.
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        parse_json(text, &Limits::default()).unwrap()
    }

    #[test]
    fn integers_follow_the_signedness_rules() {
        assert_eq!(parse("0"), Value::Int(0));
        assert_eq!(parse("-1"), Value::Int(-1));
        assert_eq!(parse("9223372036854775807"), Value::Int(i64::MAX));
        assert_eq!(parse("9223372036854775808"), Value::UInt(1u64 << 63));
        assert_eq!(parse("18446744073709551615"), Value::UInt(u64::MAX));
    }

    #[test]
    fn integers_outside_u64_are_an_error() {
        assert!(matches!(
            parse_json("18446744073709551616", &Limits::default()),
            Err(SlimError::NumberOutOfRange)
        ));
        assert!(matches!(
            parse_json("-9223372036854775809", &Limits::default()),
            Err(SlimError::NumberOutOfRange)
        ));
    }

    #[test]
    fn fractions_and_exponents_become_floats() {
        assert_eq!(parse("1.5"), Value::Float(1.5));
        assert_eq!(parse("-0.25"), Value::Float(-0.25));
        assert_eq!(parse("1e3"), Value::Float(1000.0));
        assert_eq!(parse("1.5E-2"), Value::Float(0.015));
    }

    #[test]
    fn scalars_parse() {
        assert_eq!(parse("null"), Value::Null);
        assert_eq!(parse("true"), Value::Bool(true));
        assert_eq!(parse("false"), Value::Bool(false));
        assert_eq!(parse(r#""hi""#), Value::Str("hi".to_owned()));
    }

    #[test]
    fn strings_handle_escapes_and_surrogate_pairs() {
        assert_eq!(parse(r#""a\nb\tc""#), Value::Str("a\nb\tc".to_owned()));
        assert_eq!(
            parse(r#""\"\\\/\b\f""#),
            Value::Str("\"\\/\u{08}\u{0C}".to_owned())
        );
        assert_eq!(parse(r#""Aé""#), Value::Str("Aé".to_owned()));
        assert_eq!(parse(r#""😀""#), Value::Str("😀".to_owned()));
        assert_eq!(parse("\"héllo ✓\""), Value::Str("héllo ✓".to_owned()));
        assert!(matches!(
            parse_json(r#""\q""#, &Limits::default()),
            Err(SlimError::InvalidUtf8)
        ));
        assert!(matches!(
            parse_json("\"raw\ncontrol\"", &Limits::default()),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn arrays_and_objects_keep_document_order() {
        assert_eq!(
            parse("[1, \"two\", null, [3]]"),
            Value::Array(vec![
                Value::Int(1),
                Value::Str("two".to_owned()),
                Value::Null,
                Value::Array(vec![Value::Int(3)]),
            ])
        );
        assert_eq!(
            parse(r#"{"b": 1, "a": 2}"#),
            Value::Map(vec![
                ("b".to_owned(), Value::Int(1)),
                ("a".to_owned(), Value::Int(2)),
            ])
        );
        assert_eq!(parse("[]"), Value::Array(Vec::new()));
        assert_eq!(parse("{}"), Value::Map(Vec::new()));
    }

    #[test]
    fn duplicate_json_keys_are_preserved_not_merged() {
        let value = parse(r#"{"a": 1, "a": 2}"#);
        let Value::Map(pairs) = &value else {
            panic!("expected Map");
        };
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].1, Value::Int(1));
        assert_eq!(pairs[1].1, Value::Int(2));
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(
            parse("  { \"a\" : [ 1 , 2 ] }  "),
            Value::Map(vec![(
                "a".to_owned(),
                Value::Array(vec![Value::Int(1), Value::Int(2)])
            )])
        );
    }

    #[test]
    fn writing_json_never_adds_a_decimal_point_to_integers() {
        assert_eq!(to_json(&Value::Int(-5)).unwrap(), "-5");
        assert_eq!(
            to_json(&Value::UInt(u64::MAX)).unwrap(),
            "18446744073709551615"
        );
    }

    #[test]
    fn whole_floats_get_a_decimal_point() {
        assert_eq!(to_json(&Value::Float(27.0)).unwrap(), "27.0");
        assert_eq!(to_json(&Value::Float(-0.0)).unwrap(), "-0.0");
        // Rust's Display never uses exponent notation, so a huge float is
        // written in full and still gets its ".0".
        let huge = to_json(&Value::Float(1.0e300)).unwrap();
        assert!(huge.ends_with(".0"), "{huge}");
        assert!(huge.len() > 300);
        assert_eq!(parse(&huge), Value::Float(1.0e300), "and round-trips");
        assert_eq!(to_json(&Value::Float(27.5)).unwrap(), "27.5");
    }

    #[test]
    fn non_finite_floats_cannot_be_written() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(
                to_json(&Value::Float(v)),
                Err(SlimError::NonFiniteFloat)
            ));
        }
    }

    #[test]
    fn json_round_trips_through_value() {
        let text = r#"{"a": [1, -2, 3.5, true, null, "x"], "b": {"c": 18446744073709551615}}"#;
        let value = parse(text);
        let written = to_json(&value).unwrap();
        assert_eq!(parse(&written), value, "round trip must be stable");
        assert_eq!(
            written, r#"{"a":[1,-2,3.5,true,null,"x"],"b":{"c":18446744073709551615}}"#,
            "the writer emits compact JSON"
        );
        assert_eq!(
            to_json(&parse(&written)).unwrap(),
            written,
            "and is idempotent"
        );
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        for bad in [
            "",
            "{",
            "[",
            "\"",
            "{\"a\"}",
            "[1,]",
            "{\"a\": }",
            "tru",
            "1 2",
            "\"\\u00\"",
            "{\"a\": 1,}",
            "[1 2]",
            "--1",
            "\"unterminated",
        ] {
            assert!(
                parse_json(bad, &Limits::default()).is_err(),
                "{bad:?} must not parse"
            );
        }
    }

    #[test]
    fn nesting_is_bounded_by_max_depth() {
        let strict = Limits {
            max_depth: 3,
            ..Limits::default()
        };
        let deep = "[1,[2,[3,[4]]]]";
        assert!(parse_json(deep, &strict).is_err());
        assert!(parse_json("[1,[2,[3]]]", &strict).is_ok());
    }

    #[test]
    fn bytes_render_as_hex_and_do_not_break_the_json() {
        let value = Value::Bytes(vec![0x00, 0xFF, 0x10]);
        let text = to_json(&value).unwrap();
        assert_eq!(text, "\"00ff10\"");
        assert!(parse_json(&text, &Limits::default()).is_ok());
    }

    #[test]
    fn utf8_length_boundaries() {
        assert_eq!(utf8_len(0x00), 1);
        assert_eq!(utf8_len(0x7F), 1);
        assert_eq!(utf8_len(0xC3), 2);
        assert_eq!(utf8_len(0xE2), 3);
        assert_eq!(utf8_len(0xF0), 4);
    }
}

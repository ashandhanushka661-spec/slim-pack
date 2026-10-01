//! Dynamic mode: self-describing values (`spec/SPEC.md` §4).
//!
//! Every value starts with a tag byte `kind << 4 | form`. Forms are chosen
//! deterministically by the encoder as the smallest legal one, so the same
//! value always produces the same bytes; the decoder accepts every form this
//! version defines and rejects reserved ones.
//!
//! Encoder form choices (a Phase 1 clarification of §4.1, recorded as
//! D-0011): `Int`/`UInt` use `fixint` when they fit in one byte (`i8` for
//! `Int`, `u8` for `UInt`), otherwise `zigzag varint` when it is shorter than
//! 8 payload bytes, otherwise `raw`. The two's-complement varint form is
//! accepted on decode but never written, because `zigzag` is never larger for a
//! value that fits.

pub mod json;

use crate::error::{Limit, Result, SlimError};
use crate::limits::Limits;
use crate::varint::{read_varint, varint_len, write_varint, zigzag_decode, zigzag_encode};

/// Value kinds, i.e. the high nibble of a tag byte.
pub mod kind {
    /// `Null`.
    pub const NULL: u8 = 0;
    /// `Bool`.
    pub const BOOL: u8 = 1;
    /// `Int(i64)`.
    pub const INT: u8 = 2;
    /// `UInt(u64)`.
    pub const UINT: u8 = 3;
    /// `Float(f64)`.
    pub const FLOAT: u8 = 4;
    /// `Str`.
    pub const STR: u8 = 5;
    /// `Bytes`.
    pub const BYTES: u8 = 6;
    /// `Array`.
    pub const ARRAY: u8 = 7;
    /// `Map`.
    pub const MAP: u8 = 8;
}

/// Builds a tag byte.
pub fn tag(kind: u8, form: u8) -> u8 {
    (kind << 4) | (form & 0x0F)
}

/// A self-describing value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer above `i64::MAX`.
    UInt(u64),
    /// A double, stored bit-exactly.
    Float(f64),
    /// A UTF-8 string.
    Str(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// An ordered list.
    Array(Vec<Value>),
    /// An ordered list of pairs; insertion order is preserved and duplicate
    /// keys are preserved on purpose (they are only an error on decode).
    Map(Vec<(String, Value)>),
}

impl Value {
    /// The kind constant for this value.
    pub fn kind(&self) -> u8 {
        match self {
            Value::Null => kind::NULL,
            Value::Bool(_) => kind::BOOL,
            Value::Int(_) => kind::INT,
            Value::UInt(_) => kind::UINT,
            Value::Float(_) => kind::FLOAT,
            Value::Str(_) => kind::STR,
            Value::Bytes(_) => kind::BYTES,
            Value::Array(_) => kind::ARRAY,
            Value::Map(_) => kind::MAP,
        }
    }

    /// Nesting depth of this value, counting the value itself as 1.
    pub fn depth(&self) -> usize {
        match self {
            Value::Array(items) => 1 + items.iter().map(Value::depth).max().unwrap_or(0),
            Value::Map(pairs) => 1 + pairs.iter().map(|(_, v)| v.depth()).max().unwrap_or(0),
            _ => 1,
        }
    }
}

/// Writes `values` as a Dynamic body: `varint value_count` then each value.
pub fn encode_body(values: &[Value], keys: &[String], out: &mut Vec<u8>) -> Result<()> {
    write_varint(out, values.len() as u64);
    for value in values {
        encode(value, keys, out)?;
    }
    Ok(())
}

/// Reads a Dynamic body written by [`encode_body`].
pub fn decode_body(body: &[u8], keys: &[String], limits: &Limits) -> Result<Vec<Value>> {
    let mut pos = 0;
    let count = read_varint(body, &mut pos)?;
    let count = limits.check_count(count, Limit::Records, limits.max_records)?;
    let mut out = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        out.push(decode(body, &mut pos, keys, limits, 1)?);
    }
    if pos != body.len() {
        return Err(SlimError::TrailingBytes {
            extra: body.len() - pos,
        });
    }
    Ok(out)
}

/// Appends one value to `out`.
pub fn encode(value: &Value, keys: &[String], out: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => {
            out.push(tag(kind::NULL, 0));
            Ok(())
        }
        Value::Bool(b) => {
            out.push(tag(kind::BOOL, u8::from(*b)));
            Ok(())
        }
        Value::Int(v) => encode_int(*v, out),
        Value::UInt(v) => encode_uint(*v, out),
        Value::Float(v) => {
            out.push(tag(kind::FLOAT, 0));
            out.extend_from_slice(&v.to_bits().to_le_bytes());
            Ok(())
        }
        Value::Str(s) => {
            encode_str(s, None, out);
            Ok(())
        }
        Value::Bytes(b) => {
            if b.len() <= u8::MAX as usize {
                out.push(tag(kind::BYTES, 0));
                out.push(b.len() as u8);
            } else {
                out.push(tag(kind::BYTES, 1));
                write_varint(out, b.len() as u64);
            }
            out.extend_from_slice(b);
            Ok(())
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push(tag(kind::ARRAY, 1));
                return Ok(());
            }
            out.push(tag(kind::ARRAY, 0));
            write_varint(out, items.len() as u64);
            for item in items {
                encode(item, keys, out)?;
            }
            Ok(())
        }
        Value::Map(pairs) => {
            if pairs.is_empty() {
                out.push(tag(kind::MAP, 1));
                return Ok(());
            }
            out.push(tag(kind::MAP, 0));
            write_varint(out, pairs.len() as u64);
            for (k, v) in pairs {
                encode_str(k, Some(keys), out);
                encode(v, keys, out)?;
            }
            Ok(())
        }
    }
}

/// The smallest form for an `i64`: fixint, zigzag varint, or raw 8 bytes.
fn encode_int(value: i64, out: &mut Vec<u8>) -> Result<()> {
    if (-128..=127).contains(&value) {
        out.push(tag(kind::INT, 0));
        out.push(value as u8);
        return Ok(());
    }
    let zig = zigzag_encode(value);
    if varint_len(zig) < 8 {
        out.push(tag(kind::INT, 1));
        write_varint(out, zig);
    } else {
        out.push(tag(kind::INT, 3));
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

/// The smallest form for a `u64`: fixint (`u8`), zigzag varint, or raw.
fn encode_uint(value: u64, out: &mut Vec<u8>) -> Result<()> {
    if value <= u8::MAX as u64 {
        out.push(tag(kind::UINT, 0));
        out.push(value as u8);
        return Ok(());
    }
    if varint_len(value) < 8 {
        out.push(tag(kind::UINT, 1));
        write_varint(out, value);
    } else {
        out.push(tag(kind::UINT, 3));
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

/// Writes a string; map keys use the interned form when the key is in `keys`.
fn encode_str(value: &str, keys: Option<&[String]>, out: &mut Vec<u8>) {
    if let Some(keys) = keys {
        if let Some(index) = keys.iter().position(|k| k == value) {
            out.push(tag(kind::STR, 2));
            write_varint(out, index as u64);
            return;
        }
    }
    if value.len() <= u8::MAX as usize {
        out.push(tag(kind::STR, 0));
        out.push(value.len() as u8);
    } else {
        out.push(tag(kind::STR, 1));
        write_varint(out, value.len() as u64);
    }
    out.extend_from_slice(value.as_bytes());
}

/// Reads one value at `*pos`.
pub fn decode(
    input: &[u8],
    pos: &mut usize,
    keys: &[String],
    limits: &Limits,
    depth: usize,
) -> Result<Value> {
    limits.check_depth(depth)?;
    let byte = *input
        .get(*pos)
        .ok_or(SlimError::truncated(*pos + 1, input.len()))?;
    *pos += 1;
    let kind = byte >> 4;
    let form = byte & 0x0F;
    Ok(match kind {
        kind::NULL => {
            expect_form(form, 0)?;
            Value::Null
        }
        kind::BOOL => match form {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            other => return Err(reserved_form(byte, other)),
        },
        kind::INT => match form {
            0 => {
                let byte = take_byte(input, pos)?;
                Value::Int(byte as i8 as i64)
            }
            1 => Value::Int(zigzag_decode(read_varint(input, pos)?)),
            2 => Value::Int(read_twos_complement(input, pos)?),
            3 => Value::Int(read_raw_i64(input, pos)?),
            other => return Err(reserved_form(byte, other)),
        },
        kind::UINT => match form {
            0 => {
                let byte = take_byte(input, pos)?;
                Value::UInt(u64::from(byte))
            }
            1 => Value::UInt(read_varint(input, pos)?),
            2 => Value::UInt(read_twos_complement(input, pos)? as u64),
            3 => Value::UInt(read_raw_i64(input, pos)? as u64),
            other => return Err(reserved_form(byte, other)),
        },
        kind::FLOAT => {
            expect_form(form, 0)?;
            Value::Float(f64::from_bits(read_raw_i64(input, pos)? as u64))
        }
        kind::STR => match form {
            0 => {
                let len = usize::from(take_byte(input, pos)?);
                Value::Str(read_string(input, pos, len, limits)?)
            }
            1 => {
                let len = read_len(input, pos, limits)?;
                Value::Str(read_string(input, pos, len, limits)?)
            }
            2 => {
                let index = read_varint(input, pos)?;
                let key = keys
                    .get(usize::try_from(index).map_err(|_| SlimError::SizeOverflow)?)
                    .ok_or(SlimError::LimitExceeded {
                        limit: Limit::Dictionary,
                        value: index,
                        max: keys.len() as u64,
                    })?;
                Value::Str(key.clone())
            }
            other => return Err(reserved_form(byte, other)),
        },
        kind::BYTES => {
            let len = match form {
                0 => usize::from(take_byte(input, pos)?),
                1 => read_len(input, pos, limits)?,
                other => return Err(reserved_form(byte, other)),
            };
            // take() already advanced the cursor.
            let slice = take(input, pos, len)?;
            Value::Bytes(slice.to_vec())
        }
        kind::ARRAY => {
            let count = match form {
                0 => {
                    let count = read_varint(input, pos)?;
                    limits.check_count(count, Limit::Records, limits.max_records)?
                }
                1 => 0,
                other => return Err(reserved_form(byte, other)),
            };
            let mut items = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                items.push(decode(input, pos, keys, limits, depth + 1)?);
            }
            Value::Array(items)
        }
        kind::MAP => {
            let count = match form {
                0 => {
                    let count = read_varint(input, pos)?;
                    limits.check_count(count, Limit::Records, limits.max_records)?
                }
                1 => 0,
                other => return Err(reserved_form(byte, other)),
            };
            let mut pairs = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let key = match decode(input, pos, keys, limits, depth + 1)? {
                    Value::Str(s) => s,
                    _ => {
                        return Err(SlimError::UnsupportedLayout(
                            "map keys must be Str (spec 4.1)",
                        ))
                    }
                };
                if pairs.iter().any(|(existing, _)| *existing == key) {
                    return Err(SlimError::DuplicateKey(key));
                }
                let value = decode(input, pos, keys, limits, depth + 1)?;
                pairs.push((key, value));
            }
            Value::Map(pairs)
        }
        _ => return Err(SlimError::UnknownEncoding(byte)),
    })
}

fn expect_form(form: u8, want: u8) -> Result<()> {
    if form == want {
        Ok(())
    } else {
        Err(SlimError::UnsupportedLayout(
            "reserved value form (spec 4.1)",
        ))
    }
}

/// A tag byte whose form this version reserves or does not define.
fn reserved_form(_tag: u8, _form: u8) -> SlimError {
    SlimError::UnsupportedLayout("reserved or unknown value form (spec 4.1)")
}

fn take_byte(input: &[u8], pos: &mut usize) -> Result<u8> {
    let byte = *input
        .get(*pos)
        .ok_or(SlimError::truncated(*pos + 1, input.len()))?;
    *pos += 1;
    Ok(byte)
}

fn take<'a>(input: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = pos.checked_add(len).ok_or(SlimError::SizeOverflow)?;
    let slice = input
        .get(*pos..end)
        .ok_or_else(|| SlimError::truncated(end, input.len()))?;
    *pos = end;
    Ok(slice)
}

fn read_len(input: &[u8], pos: &mut usize, limits: &Limits) -> Result<usize> {
    let declared = read_varint(input, pos)?;
    limits.check_len(declared, input.len().saturating_sub(*pos))
}

fn read_string(input: &[u8], pos: &mut usize, len: usize, limits: &Limits) -> Result<String> {
    limits.check_len(len as u64, input.len().saturating_sub(*pos))?;
    let bytes = take(input, pos, len)?;
    Ok(std::str::from_utf8(bytes)
        .map_err(|_| SlimError::InvalidUtf8)?
        .to_owned())
}

fn read_raw_i64(input: &[u8], pos: &mut usize) -> Result<i64> {
    let bytes = take(input, pos, 8)?;
    let mut raw = [0u8; 8];
    raw.copy_from_slice(bytes);
    Ok(i64::from_le_bytes(raw))
}

/// Two's-complement varint form: a varint whose value is reinterpreted as `i64`.
fn read_twos_complement(input: &[u8], pos: &mut usize) -> Result<i64> {
    Ok(read_varint(input, pos)? as i64)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits::default()
    }

    fn round_trip(values: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        encode_body(values, &[], &mut out).unwrap();
        out
    }

    fn decode_one(bytes: &[u8]) -> Result<Value> {
        let mut pos = 0;
        decode(bytes, &mut pos, &[], &limits(), 1)
    }

    #[test]
    fn every_kind_round_trips() {
        let cases = [
            Value::Null,
            Value::Bool(false),
            Value::Bool(true),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(127),
            Value::Int(-128),
            Value::Int(i64::MIN),
            Value::Int(i64::MAX),
            Value::UInt(0),
            Value::UInt(u8::MAX as u64),
            Value::UInt(i64::MAX as u64 + 1),
            Value::UInt(u64::MAX),
            Value::Float(0.0),
            Value::Float(-0.0),
            Value::Float(f64::NAN.copysign(1.0)),
            Value::Float(27.5),
            Value::Str(String::new()),
            Value::Str("héllo ✓".to_owned()),
            Value::Bytes(Vec::new()),
            Value::Bytes(vec![0, 1, 2, 255]),
            Value::Array(Vec::new()),
            Value::Array(vec![Value::Int(1), Value::Array(vec![Value::Null])]),
            Value::Map(Vec::new()),
            Value::Map(vec![
                ("a".to_owned(), Value::Int(1)),
                ("b".to_owned(), Value::Array(vec![Value::Bool(true)])),
            ]),
        ];
        for value in &cases {
            let bytes = round_trip(std::slice::from_ref(value));
            let got = decode_body(&bytes, &[], &limits())
                .unwrap_or_else(|e| panic!("{value:?} -> {bytes:02x?} failed: {e}"));
            assert_eq!(got.len(), 1, "{value:?}");
            // Float is compared by bits: NaN != NaN in PartialEq.
            match (value, &got[0]) {
                (Value::Float(want), Value::Float(have)) => assert_eq!(
                    have.to_bits(),
                    want.to_bits(),
                    "{value:?} must keep its bit pattern"
                ),
                _ => assert_eq!(&got[0], value, "{value:?} must round-trip"),
            }
        }
    }

    #[test]
    fn a_body_round_trips_several_values() {
        let values = vec![
            Value::Int(1),
            Value::Str("two".to_owned()),
            Value::Map(vec![("three".to_owned(), Value::Float(3.5))]),
        ];
        let bytes = round_trip(&values);
        assert_eq!(bytes[0], 3, "varint value_count");
        assert_eq!(decode_body(&bytes, &[], &limits()).unwrap(), values);
    }

    #[test]
    fn the_smallest_form_is_chosen_for_integers() {
        let mut out = Vec::new();
        encode(&Value::Int(7), &[], &mut out).unwrap();
        assert_eq!(out, vec![0x20, 0x07], "fixint: 1 payload byte");
        out.clear();
        encode(&Value::Int(-70_000), &[], &mut out).unwrap();
        assert_eq!(out[0], 0x21, "zigzag varint form");
        out.clear();
        encode(&Value::Int(i64::MAX), &[], &mut out).unwrap();
        assert_eq!(out[0], 0x23, "raw i64 form for the extreme");
        assert_eq!(out.len(), 9, "SPEC 4.1: i64 extremes fit in 9 bytes");
        out.clear();
        encode(&Value::UInt(200), &[], &mut out).unwrap();
        assert_eq!(out, vec![0x30, 0xC8]);
        out.clear();
        encode(&Value::UInt(u64::MAX), &[], &mut out).unwrap();
        assert_eq!(&out[..1], &[0x33]);
    }

    #[test]
    fn all_four_integer_forms_decode() {
        // fixint, zigzag, two's complement, raw
        assert_eq!(decode_one(&[0x20, 0xFF]).unwrap(), Value::Int(-1));
        assert_eq!(decode_one(&[0x21, 0x02]).unwrap(), Value::Int(1));
        assert_eq!(decode_one(&[0x22, 0x80, 0x01]).unwrap(), Value::Int(128));
        let mut raw = vec![0x23];
        raw.extend_from_slice(&i64::MIN.to_le_bytes());
        assert_eq!(decode_one(&raw).unwrap(), Value::Int(i64::MIN));
        assert_eq!(decode_one(&[0x30, 0xFF]).unwrap(), Value::UInt(255));
        assert_eq!(decode_one(&[0x31, 0xFF, 0x01]).unwrap(), Value::UInt(255));
    }

    #[test]
    fn float_forms_round_trip_bit_exactly() {
        for bits in [
            0x0000_0000_0000_0000u64,
            0x8000_0000_0000_0000,
            0x7FF0_0000_0000_0000,
            0xFFF0_0000_0000_0000,
            0x7FF8_0000_DEAD_BEEF,
        ] {
            let mut out = Vec::new();
            encode(&Value::Float(f64::from_bits(bits)), &[], &mut out).unwrap();
            assert_eq!(out[0], 0x40, "Float raw form");
            assert_eq!(out.len(), 9);
            let Value::Float(got) = decode_one(&out).unwrap() else {
                panic!("expected Float");
            };
            assert_eq!(got.to_bits(), bits);
        }
    }

    #[test]
    fn the_reserved_f32_form_is_rejected() {
        assert!(matches!(
            decode_one(&[0x41, 0, 0, 0, 0]),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn unknown_kinds_are_unknown_encodings() {
        for tag in [0x90u8, 0xA0, 0xF0, 0xFF] {
            assert!(matches!(
                decode_one(&[tag]),
                Err(SlimError::UnknownEncoding(t)) if t == tag
            ));
        }
    }

    #[test]
    fn interned_map_keys_use_the_index_form() {
        let keys = vec!["a".to_owned(), "b".to_owned()];
        let value = Value::Map(vec![
            ("a".to_owned(), Value::Int(1)),
            ("b".to_owned(), Value::Int(2)),
        ]);
        let mut out = Vec::new();
        encode(&value, &keys, &mut out).unwrap();
        // MAP form 0, count 2, then key form 2 + index 0, Int(1), key + index 1, Int(2)
        assert_eq!(
            out,
            vec![0x80, 0x02, 0x52, 0x00, 0x20, 0x01, 0x52, 0x01, 0x20, 0x02]
        );
        assert_eq!(decode_one_with(&out, &keys).unwrap(), value);
    }

    #[test]
    fn an_interned_index_outside_the_table_is_rejected() {
        assert!(matches!(
            decode_one(&[0x52, 0x05]),
            Err(SlimError::LimitExceeded {
                limit: Limit::Dictionary,
                ..
            })
        ));
    }

    #[test]
    fn non_interned_map_keys_use_the_length_form() {
        let value = Value::Map(vec![("zzz".to_owned(), Value::Null)]);
        let mut out = Vec::new();
        encode(&value, &[], &mut out).unwrap();
        assert_eq!(out, vec![0x80, 0x01, 0x50, 0x03, b'z', b'z', b'z', 0x00]);
        assert_eq!(decode_one(&out).unwrap(), value);
    }

    #[test]
    fn duplicate_map_keys_are_an_error_on_decode() {
        let mut out = Vec::new();
        encode(
            &Value::Map(vec![
                ("a".to_owned(), Value::Int(1)),
                ("a".to_owned(), Value::Int(2)),
            ]),
            &[],
            &mut out,
        )
        .unwrap();
        assert!(matches!(
            decode_one(&out),
            Err(SlimError::DuplicateKey(key)) if key == "a"
        ));
        // The same map is preserved by Value itself, as SPEC 4.2 requires.
        let value = Value::Map(vec![
            ("a".to_owned(), Value::Int(1)),
            ("a".to_owned(), Value::Int(2)),
        ]);
        assert_eq!(value.kind(), kind::MAP);
        assert!(matches!(
            decode_body(&round_trip(std::slice::from_ref(&value)), &[], &limits()),
            Err(SlimError::DuplicateKey(_))
        ));
    }

    #[test]
    fn a_non_string_map_key_is_rejected() {
        assert!(matches!(
            decode_one(&[0x80, 0x01, 0x20, 0x01, 0x00]),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn empty_containers_use_their_one_byte_forms() {
        assert_eq!(round_trip(&[Value::Array(Vec::new())]), vec![0x01, 0x71]);
        assert_eq!(round_trip(&[Value::Map(Vec::new())]), vec![0x01, 0x81]);
    }

    #[test]
    fn a_declared_count_is_checked_against_the_limits() {
        // ARRAY form 0 with a huge count.
        let huge = [0x70u8, 0xFF, 0xFF, 0xFF, 0x7F];
        assert!(matches!(
            decode_one(&huge),
            Err(SlimError::LimitExceeded { .. })
        ));
    }

    #[test]
    fn a_declared_length_is_checked_before_allocation() {
        let huge = [0x51u8, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x41];
        assert!(matches!(
            decode_one(&huge),
            Err(SlimError::LimitExceeded {
                limit: Limit::Len,
                ..
            })
        ));
        // A length inside the limit but longer than the input is truncation.
        assert!(matches!(
            decode_one(&[0x51, 0x20, b'x']),
            Err(SlimError::Truncated { .. })
        ));
    }

    #[test]
    fn invalid_utf8_in_a_string_is_rejected() {
        assert!(matches!(
            decode_one(&[0x50, 0x02, 0xFF, 0xFE]),
            Err(SlimError::InvalidUtf8)
        ));
    }

    #[test]
    fn nesting_is_bounded_by_max_depth() {
        let mut bytes = Vec::new();
        for _ in 0..10 {
            bytes.push(tag(kind::ARRAY, 0));
            bytes.push(0x01); // one element each
        }
        bytes.push(tag(kind::NULL, 0));
        let strict = Limits {
            max_depth: 4,
            ..Limits::default()
        };
        let mut pos = 0;
        assert!(matches!(
            decode(&bytes, &mut pos, &[], &strict, 1),
            Err(SlimError::LimitExceeded {
                limit: Limit::Depth,
                ..
            })
        ));
    }

    #[test]
    fn trailing_bytes_in_a_body_are_rejected() {
        let mut bytes = round_trip(&[Value::Int(1)]);
        bytes.push(0x00);
        assert!(matches!(
            decode_body(&bytes, &[], &limits()),
            Err(SlimError::TrailingBytes { extra: 1 })
        ));
    }

    #[test]
    fn a_short_input_is_truncation_not_a_panic() {
        for tag in [0x40u8, 0x60, 0x70, 0x80, 0x20, 0x23, 0x33] {
            let _ = decode_one(&[tag]);
        }
        assert!(matches!(decode_one(&[]), Err(SlimError::Truncated { .. })));
    }

    #[test]
    fn depth_is_reported_for_nested_values() {
        assert_eq!(Value::Null.depth(), 1);
        assert_eq!(Value::Array(vec![Value::Null]).depth(), 2);
        assert_eq!(
            Value::Map(vec![(
                "a".to_owned(),
                Value::Array(vec![Value::Bool(true)])
            )])
            .depth(),
            3
        );
    }

    #[test]
    fn long_strings_use_the_varint_length_form() {
        let long = "x".repeat(300);
        let mut out = Vec::new();
        encode(&Value::Str(long.clone()), &[], &mut out).unwrap();
        assert_eq!(out[0], tag(kind::STR, 1));
        assert_eq!(out.len(), 1 + 2 + 300);
        assert_eq!(decode_one(&out).unwrap(), Value::Str(long));
        let bytes = vec![7u8; 300];
        let mut out = Vec::new();
        encode(&Value::Bytes(bytes.clone()), &[], &mut out).unwrap();
        assert_eq!(out[0], tag(kind::BYTES, 1));
        assert_eq!(decode_one(&out).unwrap(), Value::Bytes(bytes));
    }

    fn decode_one_with(bytes: &[u8], keys: &[String]) -> Result<Value> {
        let mut pos = 0;
        decode(bytes, &mut pos, keys, &limits(), 1)
    }
}

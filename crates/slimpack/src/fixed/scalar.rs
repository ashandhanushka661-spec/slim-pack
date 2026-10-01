//! Scalar values and their byte-level codecs.
//!
//! Every integer write is **range-checked against the field's declared width**
//! before it is narrowed, so a narrow field can never silently truncate, and
//! every float write goes through `to_bits()`/`from_bits()`, so NaN payloads,
//! signalling NaNs, `-0.0` and both infinities survive a round trip unchanged.

use crate::error::{Result, SlimError};

/// One value as it appears in a Fixed record.
///
/// Integers are widened to `i64`/`u64` and floats to `f64`; the field's
/// declared [`Type`] decides how many bytes are actually stored.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar<'a> {
    /// `bool`.
    Bool(bool),
    /// Any `i8`..`i64`.
    Int(i64),
    /// Any `u8`..`u64`, and `bool` as 0/1.
    UInt(u64),
    /// `f64`, by value.
    Float(f64),
    /// `f32`, by IEEE-754 bit pattern.
    ///
    /// `f32` values are carried as bits rather than as `f64`, because a
    /// signalling NaN does not survive `f32 -> f64 -> f32` (the conversion
    /// quiets it) and `spec/SPEC.md` promises bit-exact round trips.
    F32Bits(u32),
    /// `str`, validated as UTF-8 on write.
    Str(&'a str),
    /// `bytes`.
    Bytes(&'a [u8]),
    /// `Vec<T>`; Phase 1 supports scalar `T`.
    Seq(Vec<Scalar<'a>>),
}

impl Scalar<'_> {
    /// The variant name, for error messages and test failures.
    pub fn kind(&self) -> &'static str {
        match self {
            Scalar::Bool(_) => "Bool",
            Scalar::Int(_) => "Int",
            Scalar::UInt(_) => "UInt",
            Scalar::Float(_) | Scalar::F32Bits(_) => "Float",
            Scalar::Str(_) => "Str",
            Scalar::Bytes(_) => "Bytes",
            Scalar::Seq(_) => "Seq",
        }
    }
}

/// Smallest and largest value storable in `bytes` bytes, two's complement.
pub fn signed_domain(bytes: usize) -> Option<(i64, i64)> {
    if bytes == 8 {
        return Some((i64::MIN, i64::MAX));
    }
    if !(1..=8).contains(&bytes) {
        return None;
    }
    let bits = 8 * bytes as u32;
    let min = -(1i64 << (bits - 1));
    let max = (1i64 << (bits - 1)) - 1;
    Some((min, max))
}

/// Largest unsigned value storable in `bytes` bytes.
pub fn unsigned_domain(bytes: usize) -> Option<u64> {
    if !(1..=8).contains(&bytes) {
        return None;
    }
    if bytes == 8 {
        return Some(u64::MAX);
    }
    Some((1u64 << (8 * bytes as u32)) - 1)
}

/// Appends `value` as `bytes` little-endian two's complement bytes.
pub fn write_int(out: &mut Vec<u8>, value: i64, bytes: usize) -> Result<()> {
    let (min, max) = signed_domain(bytes).ok_or(SlimError::UnsupportedLayout("bad width"))?;
    if value < min || value > max {
        return Err(SlimError::ValueOutOfRange {
            value: value as i128,
            min: min as i128,
            max: max as i128,
        });
    }
    out.extend_from_slice(&value.to_le_bytes()[..bytes]);
    Ok(())
}

/// Appends `value` as `bytes` little-endian unsigned bytes.
pub fn write_uint(out: &mut Vec<u8>, value: u64, bytes: usize) -> Result<()> {
    let max = unsigned_domain(bytes).ok_or(SlimError::UnsupportedLayout("bad width"))?;
    if value > max {
        return Err(SlimError::ValueOutOfRange {
            value: value as i128,
            min: 0,
            max: max as i128,
        });
    }
    out.extend_from_slice(&value.to_le_bytes()[..bytes]);
    Ok(())
}

/// Appends a `bool` as one byte; only `0x00`/`0x01` are ever written.
pub fn write_bool(out: &mut Vec<u8>, value: bool) {
    out.push(u8::from(value));
}

/// Reads `bytes` little-endian two's complement bytes as `i64`.
///
/// The bytes are assembled into the low end of a `u64`, then shifted up and back
/// down arithmetically, so the sign extends without any signed shift overflow.
pub fn read_int(input: &[u8], offset: usize, bytes: usize) -> Result<i64> {
    let slice = take(input, offset, bytes)?;
    let mut raw = [0u8; 8];
    raw[..bytes].copy_from_slice(slice);
    let unsigned = u64::from_le_bytes(raw);
    let shift = 64 - (8 * bytes as u32);
    Ok(((unsigned << shift) as i64) >> shift)
}

/// Reads `bytes` little-endian unsigned bytes as `u64`.
pub fn read_uint(input: &[u8], offset: usize, bytes: usize) -> Result<u64> {
    let slice = take(input, offset, bytes)?;
    let mut raw = [0u8; 8];
    raw[..bytes].copy_from_slice(slice);
    Ok(u64::from_le_bytes(raw))
}

/// Reads one byte as a `bool`, rejecting anything but `0x00`/`0x01`.
pub fn read_bool(input: &[u8], offset: usize) -> Result<bool> {
    let byte = take(input, offset, 1)?[0];
    match byte {
        0x00 => Ok(false),
        0x01 => Ok(true),
        other => Err(SlimError::InvalidBool(other)),
    }
}

/// Narrows an `f64` to `f32`, returning `None` if the value is not exactly
/// representable (so a narrowing is never silent).
pub fn to_f32_exact(value: f64) -> Option<f32> {
    let narrowed = value as f32;
    if narrowed as f64 == value || (narrowed.is_nan() && value.is_nan()) {
        Some(narrowed)
    } else {
        None
    }
}

/// Bounds-checked slice of `bytes` bytes at `offset`.
fn take(input: &[u8], offset: usize, bytes: usize) -> Result<&[u8]> {
    let end = offset.checked_add(bytes).ok_or(SlimError::SizeOverflow)?;
    input
        .get(offset..end)
        .ok_or_else(|| SlimError::truncated(end, input.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_domains_match_two_complement_widths() {
        assert_eq!(signed_domain(1), Some((-128, 127)));
        assert_eq!(signed_domain(2), Some((-32_768, 32_767)));
        assert_eq!(signed_domain(4), Some((-2_147_483_648, 2_147_483_647)));
        assert_eq!(signed_domain(8), Some((i64::MIN, i64::MAX)));
        assert_eq!(signed_domain(0), None);
        assert_eq!(signed_domain(9), None);
        assert_eq!(unsigned_domain(1), Some(255));
        assert_eq!(unsigned_domain(8), Some(u64::MAX));
        assert_eq!(unsigned_domain(0), None);
    }

    #[test]
    fn integers_are_little_endian_and_range_checked() {
        let mut out = Vec::new();
        write_int(&mut out, -1, 2).unwrap();
        write_int(&mut out, i16::MIN as i64, 2).unwrap();
        write_uint(&mut out, 258, 2).unwrap();
        assert_eq!(out, vec![0xFF, 0xFF, 0x00, 0x80, 0x02, 0x01]);
        assert_eq!(read_int(&out, 0, 2).unwrap(), -1);
        assert_eq!(read_int(&out, 2, 2).unwrap(), i16::MIN as i64);
        assert_eq!(read_uint(&out, 4, 2).unwrap(), 258);
    }

    #[test]
    fn narrowing_never_truncates_silently() {
        let mut out = Vec::new();
        for (value, bytes) in [(128i64, 1usize), (-129, 1), (32_768, 2), (-32_769, 2)] {
            assert!(matches!(
                write_int(&mut out, value, bytes),
                Err(SlimError::ValueOutOfRange { .. })
            ));
        }
        assert!(matches!(
            write_uint(&mut out, 256, 1),
            Err(SlimError::ValueOutOfRange {
                value: 256,
                min: 0,
                max: 255
            })
        ));
        assert!(out.is_empty(), "nothing may be written on rejection");
    }

    #[test]
    fn extremes_round_trip_at_every_width() {
        for bytes in 1..=8usize {
            let (min, max) = signed_domain(bytes).unwrap();
            let mut out = Vec::new();
            write_int(&mut out, min, bytes).unwrap();
            write_int(&mut out, max, bytes).unwrap();
            write_int(&mut out, -1, bytes).unwrap();
            write_int(&mut out, 0, bytes).unwrap();
            let step = bytes;
            assert_eq!(read_int(&out, 0, bytes).unwrap(), min);
            assert_eq!(read_int(&out, step, bytes).unwrap(), max);
            assert_eq!(read_int(&out, step * 2, bytes).unwrap(), -1);
            assert_eq!(read_int(&out, step * 3, bytes).unwrap(), 0);
        }
        let mut out = Vec::new();
        write_uint(&mut out, u64::MAX, 8).unwrap();
        assert_eq!(read_uint(&out, 0, 8).unwrap(), u64::MAX);
    }

    #[test]
    fn bools_reject_every_other_byte() {
        let mut out = Vec::new();
        write_bool(&mut out, false);
        write_bool(&mut out, true);
        assert_eq!(out, vec![0x00, 0x01]);
        assert!(!read_bool(&out, 0).unwrap());
        assert!(read_bool(&out, 1).unwrap());
        for bad in [0x02u8, 0x7F, 0xFF] {
            assert!(matches!(read_bool(&[bad], 0), Err(SlimError::InvalidBool(b)) if b == bad));
        }
    }

    #[test]
    fn reads_past_the_end_are_truncated_not_panics() {
        assert!(matches!(
            read_int(&[0x00], 0, 2),
            Err(SlimError::Truncated { .. })
        ));
        assert!(matches!(
            read_uint(&[], 0, 8),
            Err(SlimError::Truncated { .. })
        ));
        assert!(matches!(
            read_bool(&[], 0),
            Err(SlimError::Truncated { .. })
        ));
        assert!(matches!(
            read_int(&[0; 4], usize::MAX, 1),
            Err(SlimError::SizeOverflow | SlimError::Truncated { .. })
        ));
    }

    #[test]
    fn f32_narrowing_is_exact_or_refused() {
        assert_eq!(to_f32_exact(27.5), Some(27.5f32));
        assert_eq!(to_f32_exact(-0.0).map(f32::to_bits), Some(0x8000_0000));
        assert_eq!(to_f32_exact(f64::from(f32::MAX)), Some(f32::MAX));
        assert_eq!(to_f32_exact(0.1), None, "0.1 is not exact in f32");
        assert!(to_f32_exact(1e300).is_none());
        assert!(to_f32_exact(f64::NAN).is_some());
    }

    #[test]
    fn scalar_kinds_are_reported_for_diagnostics() {
        assert_eq!(Scalar::Bool(true).kind(), "Bool");
        assert_eq!(Scalar::Int(-1).kind(), "Int");
        assert_eq!(Scalar::UInt(1).kind(), "UInt");
        assert_eq!(Scalar::Float(1.5).kind(), "Float");
        assert_eq!(Scalar::F32Bits(0).kind(), "Float");
        assert_eq!(Scalar::Str("x").kind(), "Str");
        assert_eq!(Scalar::Bytes(b"x").kind(), "Bytes");
        assert_eq!(Scalar::Seq(Vec::new()).kind(), "Seq");
    }
}

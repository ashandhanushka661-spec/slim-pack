//! `packed` layout: LSB-first bit string (`spec/SPEC.md` §2.4).
//!
//! Fields are concatenated with no gaps; logical bit *i* is bit `i % 8` of
//! byte `i / 8`. `stride = ceil(total_bits / 8)`; the unused high bits of the
//! last byte are written as zero and **ignored** on read, so a writer may leave
//! anything there.
//!
//! * `range = A..=B` stores `value - A` in `ceil(log2(B - A + 1))` bits, and a
//!   stored value `> B - A` is rejected on decode.
//! * `bits = N` stores an unsigned value in exactly `N` bits and sign-extends a
//!   signed one, so a value that does not fit returns `ValueOutOfRange` on
//!   encode instead of being truncated.

use super::scalar::Scalar;
use crate::error::{Result, SlimError};
use crate::schema::{signed_bits_domain, FieldLayout, Layout, Type};

/// Writes `width` low bits of `value` at `bit_offset`, LSB-first.
///
/// Fails with [`SlimError::ValueOutOfRange`] if the value does not fit in
/// `width` bits.
pub fn write_bits(buf: &mut [u8], bit_offset: u64, width: u32, value: u64) -> Result<()> {
    if width == 0 || width > 64 {
        return Err(SlimError::UnsupportedLayout("bit width out of range"));
    }
    if width < 64 && (value >> width) != 0 {
        return Err(SlimError::ValueOutOfRange {
            value: value as i128,
            min: 0,
            max: (1u128 << width).saturating_sub(1) as i128,
        });
    }
    let end = bit_offset
        .checked_add(u64::from(width))
        .ok_or(SlimError::SizeOverflow)?;
    let needed = end.div_ceil(8);
    if needed > buf.len() as u64 {
        return Err(SlimError::truncated(needed as usize, buf.len()));
    }
    for bit in 0..u64::from(width) {
        let source = (value >> bit) & 1;
        let target = bit_offset + bit;
        let mask = 1u8 << (target % 8);
        let byte = &mut buf[target as usize / 8];
        if source == 1 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
    Ok(())
}

/// Reads `width` bits at `bit_offset`, LSB-first, zero-extended.
pub fn read_bits(record: &[u8], bit_offset: u64, width: u32) -> Result<u64> {
    if width == 0 || width > 64 {
        return Err(SlimError::UnsupportedLayout("bit width out of range"));
    }
    let end = bit_offset
        .checked_add(u64::from(width))
        .ok_or(SlimError::SizeOverflow)?;
    let needed = end.div_ceil(8);
    if needed > record.len() as u64 {
        return Err(SlimError::truncated(needed as usize, record.len()));
    }
    let mut value = 0u64;
    for bit in 0..u64::from(width) {
        let target = bit_offset + bit;
        let set = record[target as usize / 8] & (1u8 << (target % 8)) != 0;
        if set {
            value |= 1u64 << bit;
        }
    }
    Ok(value)
}

/// Keeps only the low `width` bits of `value`.
pub fn mask_to_width(value: u64, width: u32) -> u64 {
    if width >= 64 {
        value
    } else {
        value & ((1u64 << width) - 1)
    }
}

/// Sign-extends the low `width` bits of `value`.
pub fn sign_extend(value: u64, width: u32) -> i64 {
    if width >= 64 {
        return value as i64;
    }
    let shift = 64 - width;
    ((value << shift) as i64) >> shift
}

/// Encodes one packed record, appending exactly `stride` zeroed bytes.
pub fn encode_record(layout: &Layout, fields: &[Scalar<'_>], out: &mut Vec<u8>) -> Result<usize> {
    if fields.len() != layout.fields.len() {
        return Err(SlimError::MissingField);
    }
    let start = out.len();
    let mut record = vec![0u8; layout.stride()];
    for (field, value) in layout.fields.iter().zip(fields) {
        let offset = (field.byte_offset.unwrap_or(0) as u64) * 8 + u64::from(field.bit_offset);
        let raw = encode_field(field, value)?;
        write_bits(&mut record[..], offset, field.bit_width, raw)?;
    }
    out.extend_from_slice(&record);
    Ok(out.len() - start)
}

/// Maps a value to the unsigned integer stored in its bit field.
///
/// `range` stores the **unsigned offset** `value - A`, so a signed field with a
/// range never uses two's complement; only `bits = N` on a signed type does.
fn encode_field(field: &FieldLayout, value: &Scalar<'_>) -> Result<u64> {
    let width = field.bit_width;
    if let Some((min, max)) = field.range {
        let (raw, is_int) = match value {
            Scalar::Int(v) => ((*v as i128) - min, true),
            Scalar::UInt(v) => ((*v as i128) - min, true),
            Scalar::Bool(b) => (i128::from(*b) - min, true),
            _ => (0, false),
        };
        if !is_int || raw < 0 || raw > max - min {
            return Err(SlimError::ValueOutOfRange {
                value: raw + min,
                min,
                max,
            });
        }
        return Ok(raw as u64);
    }
    if field.ty.is_signed() {
        let Scalar::Int(v) = value else {
            return Err(SlimError::UnsupportedLayout(
                "signed integer field needs an Int value",
            ));
        };
        let (min, max) =
            signed_bits_domain(width as u8).unwrap_or((i64::MIN as i128, i64::MAX as i128));
        if (*v as i128) < min || (*v as i128) > max {
            return Err(SlimError::ValueOutOfRange {
                value: *v as i128,
                min,
                max,
            });
        }
        // Two's complement in `width` bits: the raw value is masked here so
        // that write_bits sees exactly the bits that will be stored.
        return Ok(mask_to_width(*v as u64, width));
    }
    match (&field.ty, value) {
        // Only 0 and 1 are legal bool values, whatever the bit width is.
        (Type::Bool, Scalar::Bool(b)) => Ok(u64::from(*b)),
        (Type::Bool, Scalar::UInt(b)) if *b <= 1 => Ok(*b),
        (Type::Bool, Scalar::UInt(b)) => Err(SlimError::ValueOutOfRange {
            value: *b as i128,
            min: 0,
            max: 1,
        }),
        (Type::U8 | Type::U16 | Type::U32 | Type::U64, Scalar::UInt(v)) => {
            Ok(mask_to_width(*v, width))
        }
        (Type::U8 | Type::U16 | Type::U32 | Type::U64, Scalar::Int(v)) if *v >= 0 => {
            Ok(mask_to_width(*v as u64, width))
        }
        (Type::F32, Scalar::F32Bits(bits)) => Ok(u64::from(*bits)),
        (Type::F64, Scalar::Float(v)) => Ok(v.to_bits()),
        _ => Err(SlimError::UnsupportedLayout(
            "value does not match the field type",
        )),
    }
}

/// Decodes one packed record; the unused high bits of the last byte are ignored.
///
/// Packed records hold only scalars, so the decoded values borrow nothing.
pub fn decode_record(layout: &Layout, record: &[u8]) -> Result<Vec<Scalar<'static>>> {
    if record.len() < layout.stride() {
        return Err(SlimError::truncated(layout.stride(), record.len()));
    }
    let mut out = Vec::with_capacity(layout.fields.len());
    for field in &layout.fields {
        let offset = (field.byte_offset.unwrap_or(0) as u64) * 8 + u64::from(field.bit_offset);
        let raw = read_bits(record, offset, field.bit_width)?;
        out.push(decode_field(field, raw)?);
    }
    Ok(out)
}

fn decode_field(field: &FieldLayout, raw: u64) -> Result<Scalar<'static>> {
    if let Some((min, max)) = field.range {
        let value = raw as i128 + min;
        if raw as i128 > max - min {
            return Err(SlimError::ValueOutOfRange { value, min, max });
        }
        return Ok(if field.ty.is_signed() {
            if value > i64::MAX as i128 || value < i64::MIN as i128 {
                return Err(SlimError::ValueOutOfRange {
                    value,
                    min: i64::MIN as i128,
                    max: i64::MAX as i128,
                });
            }
            Scalar::Int(value as i64)
        } else {
            Scalar::UInt(raw.wrapping_add(min as u64))
        });
    }
    Ok(match &field.ty {
        Type::Bool => match raw {
            0 => Scalar::Bool(false),
            1 => Scalar::Bool(true),
            other => {
                return Err(SlimError::ValueOutOfRange {
                    value: other as i128,
                    min: 0,
                    max: 1,
                })
            }
        },
        t if t.is_signed() => Scalar::Int(sign_extend(raw, field.bit_width)),
        Type::F32 => Scalar::F32Bits(raw as u32),
        Type::F64 => Scalar::Float(f64::from_bits(raw)),
        _ => Scalar::UInt(raw),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Attr, FieldSpec, LayoutKind, StructSpec, Type};

    fn telemetry() -> StructSpec {
        StructSpec::new(
            "Telemetry",
            vec![
                FieldSpec::new("rsrp", Type::I16).with(Attr::Range {
                    min: -140,
                    max: -44,
                }),
                FieldSpec::new("band", Type::U8),
                FieldSpec::new("speed", Type::F32),
            ],
        )
    }

    fn packed() -> Layout {
        Layout::compute(&telemetry(), LayoutKind::Packed).unwrap()
    }

    #[test]
    fn spec_2_5_example_encodes_to_the_documented_bytes() {
        let layout = packed();
        let mut out = Vec::new();
        let n = encode_record(
            &layout,
            &[
                Scalar::Int(-103),
                Scalar::UInt(3),
                Scalar::F32Bits(0x41DC_0000),
            ],
            &mut out,
        )
        .unwrap();
        assert_eq!(n, 6, "SPEC 2.5: ceil(47 / 8) = 6 bytes");
        assert_eq!(out, vec![0xA5, 0x01, 0x00, 0x00, 0xEE, 0x20]);
    }

    #[test]
    fn spec_2_5_example_decodes_back_to_the_value() {
        let layout = packed();
        let record = [0xA5u8, 0x01, 0x00, 0x00, 0xEE, 0x20];
        let got = decode_record(&layout, &record).unwrap();
        assert_eq!(got[0], Scalar::Int(-103));
        assert_eq!(got[1], Scalar::UInt(3));
        assert_eq!(got[2], Scalar::F32Bits(0x41DC_0000));
    }

    #[test]
    fn spec_2_5_bits_3_variant_encodes_to_the_documented_bytes() {
        let spec = StructSpec::new(
            "Telemetry",
            vec![
                FieldSpec::new("rsrp", Type::I16).with(Attr::Range {
                    min: -140,
                    max: -44,
                }),
                FieldSpec::new("band", Type::U8).with(Attr::Bits(3)),
                FieldSpec::new("speed", Type::F32),
            ],
        );
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        let mut out = Vec::new();
        encode_record(
            &layout,
            &[
                Scalar::Int(-103),
                Scalar::UInt(3),
                Scalar::F32Bits(0x41DC_0000),
            ],
            &mut out,
        )
        .unwrap();
        assert_eq!(out, vec![0xA5, 0x01, 0x00, 0x70, 0x07, 0x01]);
    }

    #[test]
    fn bit_io_is_lsb_first() {
        let mut buf = [0u8; 2];
        write_bits(&mut buf, 0, 1, 1).unwrap();
        assert_eq!(buf[0], 0b0000_0001);
        write_bits(&mut buf, 3, 4, 0b1111).unwrap();
        assert_eq!(buf[0], 0b0111_1001, "bits 3..6 set, bit 0 set, bit 7 clear");
        write_bits(&mut buf, 8, 8, 0xAA).unwrap();
        assert_eq!(buf[1], 0xAA);
        assert_eq!(read_bits(&buf, 0, 1).unwrap(), 1);
        assert_eq!(read_bits(&buf, 3, 4).unwrap(), 0b1111);
        assert_eq!(
            read_bits(&buf, 4, 4).unwrap(),
            0b0111,
            "bits 4..7 of 0b0111_1001"
        );
        assert_eq!(read_bits(&buf, 8, 8).unwrap(), 0xAA);
    }

    #[test]
    fn a_value_that_does_not_fit_is_refused() {
        let mut buf = [0u8; 8];
        assert!(matches!(
            write_bits(&mut buf, 0, 7, 128),
            Err(SlimError::ValueOutOfRange {
                value: 128,
                min: 0,
                max: 127
            })
        ));
        assert_eq!(buf, [0u8; 8], "nothing written on rejection");
        assert!(matches!(
            write_bits(&mut buf, 0, 0, 0),
            Err(SlimError::UnsupportedLayout(_))
        ));
        assert!(matches!(
            write_bits(&mut buf, 0, 65, 0),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn reads_past_the_end_are_truncation() {
        assert!(matches!(
            read_bits(&[0xFF, 0xFF], 8, 16),
            Err(SlimError::Truncated {
                needed: 3,
                available: 2
            })
        ));
    }

    #[test]
    fn padding_bits_are_ignored_on_read() {
        let layout = packed();
        let clean = [0xA5u8, 0x01, 0x00, 0x00, 0xEE, 0x20];
        let mut dirty = clean;
        dirty[5] |= 0b1000_0000; // the single padding bit
        assert_eq!(
            decode_record(&layout, &clean).unwrap(),
            decode_record(&layout, &dirty).unwrap()
        );
    }

    #[test]
    fn a_short_record_is_truncation() {
        let layout = packed();
        assert!(matches!(
            decode_record(&layout, &[0xA5, 0x01]),
            Err(SlimError::Truncated {
                needed: 6,
                available: 2
            })
        ));
    }

    #[test]
    fn range_is_stored_as_an_offset_from_the_minimum() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("x", Type::I16).with(Attr::Range {
                min: -140,
                max: -44,
            })],
        );
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        assert_eq!(layout.total_bits, 7);
        for value in [-140i64, -103, -44] {
            let mut out = Vec::new();
            encode_record(&layout, &[Scalar::Int(value)], &mut out).unwrap();
            let stored = read_bits(&out, 0, 7).unwrap();
            assert_eq!(stored as i64, value + 140);
            assert_eq!(decode_record(&layout, &out).unwrap()[0], Scalar::Int(value));
        }
    }

    #[test]
    fn a_stored_offset_above_the_range_is_rejected() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("x", Type::I16).with(Attr::Range {
                min: -140,
                max: -44,
            })],
        );
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        let mut record = vec![0u8; 1];
        write_bits(&mut record, 0, 7, 97).unwrap(); // 97 > 96 = max - min
        assert!(matches!(
            decode_record(&layout, &record),
            Err(SlimError::ValueOutOfRange {
                value: -43,
                max: -44,
                ..
            })
        ));
    }

    #[test]
    fn a_value_outside_the_declared_range_is_refused_on_encode() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("x", Type::I16).with(Attr::Range {
                min: -140,
                max: -44,
            })],
        );
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        let mut out = Vec::new();
        for bad in [-141i64, -43] {
            assert!(matches!(
                encode_record(&layout, &[Scalar::Int(bad)], &mut out),
                Err(SlimError::ValueOutOfRange { value, .. }) if value == bad as i128
            ));
        }
    }

    #[test]
    fn signed_bits_round_trip_and_sign_extend() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("x", Type::I16).with(Attr::Bits(8))],
        );
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        assert_eq!(layout.total_bits, 8);
        for value in [-128i64, -1, 0, 127] {
            let mut out = Vec::new();
            encode_record(&layout, &[Scalar::Int(value)], &mut out).unwrap();
            assert_eq!(decode_record(&layout, &out).unwrap()[0], Scalar::Int(value));
        }
        assert!(matches!(
            encode_record(&layout, &[Scalar::Int(128)], &mut Vec::new()),
            Err(SlimError::ValueOutOfRange { max: 127, .. })
        ));
        assert_eq!(sign_extend(0b1111_1111, 8), -1);
        assert_eq!(sign_extend(0b1000_0000, 8), -128);
        assert_eq!(sign_extend(1u64 << 63, 64), i64::MIN);
        assert_eq!(sign_extend(0b1000_0000, 64), 128, "bit 63 is not set");
        assert_eq!(sign_extend(1, 1), -1);
    }

    #[test]
    fn wrapping_extremes_round_trip_through_a_64_bit_field() {
        let spec = StructSpec::new("S", vec![FieldSpec::new("x", Type::I64)]);
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        for value in [i64::MIN, i64::MAX, -1, 0, 1] {
            let mut out = Vec::new();
            encode_record(&layout, &[Scalar::Int(value)], &mut out).unwrap();
            assert_eq!(out.len(), 8);
            assert_eq!(decode_record(&layout, &out).unwrap()[0], Scalar::Int(value));
        }
    }

    #[test]
    fn bool_is_one_bit_and_ignores_its_padding() {
        let spec = StructSpec::new("S", vec![FieldSpec::new("b", Type::Bool)]);
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        assert_eq!(layout.total_bits, 1);
        assert_eq!(layout.stride(), 1);
        let mut out = vec![0u8; 1];
        write_bits(&mut out, 0, 1, 1).unwrap();
        assert_eq!(decode_record(&layout, &out).unwrap()[0], Scalar::Bool(true));
        // The other 7 bits of the byte are padding and are ignored on read.
        out[0] = 0xFF;
        assert_eq!(
            decode_record(&layout, &out).unwrap()[0],
            Scalar::Bool(true),
            "padding bits must not be interpreted as the value"
        );
        let mut out = Vec::new();
        encode_record(&layout, &[Scalar::Bool(false)], &mut out).unwrap();
        assert_eq!(out, vec![0x00]);
        assert!(matches!(
            encode_record(&layout, &[Scalar::UInt(5)], &mut Vec::new()),
            Err(SlimError::ValueOutOfRange { .. })
        ));
    }

    #[test]
    fn bool_with_bits_rejects_values_other_than_zero_or_one() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("b", Type::Bool).with(Attr::Bits(4))],
        );
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        assert_eq!(layout.total_bits, 4);
        for (raw, want) in [(0u8, false), (1, true)] {
            let mut out = vec![0u8; 1];
            write_bits(&mut out, 0, 4, u64::from(raw)).unwrap();
            assert_eq!(decode_record(&layout, &out).unwrap()[0], Scalar::Bool(want));
        }
        let mut out = vec![0u8; 1];
        write_bits(&mut out, 0, 4, 5).unwrap();
        assert!(matches!(
            decode_record(&layout, &out),
            Err(SlimError::ValueOutOfRange {
                value: 5,
                min: 0,
                max: 1
            })
        ));
        assert!(matches!(
            encode_record(&layout, &[Scalar::UInt(5)], &mut Vec::new()),
            Err(SlimError::ValueOutOfRange { .. })
        ));
    }

    #[test]
    fn float_bits_are_preserved() {
        let spec = StructSpec::new(
            "S",
            vec![
                FieldSpec::new("f", Type::F32),
                FieldSpec::new("g", Type::F64),
            ],
        );
        let layout = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        for (f, g) in [
            (0x7F80_0001u32, 0xFFF0_0000_0000_0000u64), // sNaN, -inf
            (0x8000_0000, 0x0000_0000_0000_0000),       // -0.0, +0.0
            (0x7FC0_0042, 0x7FF8_0000_DEAD_BEEF),       // NaN payloads
        ] {
            let mut out = Vec::new();
            encode_record(
                &layout,
                &[Scalar::F32Bits(f), Scalar::Float(f64::from_bits(g))],
                &mut out,
            )
            .unwrap();
            let got = decode_record(&layout, &out).unwrap();
            assert_eq!(got[0], Scalar::F32Bits(f));
            let Scalar::Float(value) = got[1] else {
                panic!("expected Float");
            };
            assert_eq!(value.to_bits(), g, "{g:#018x} must round-trip");
        }
    }

    #[test]
    fn field_count_must_match() {
        let layout = packed();
        let mut out = Vec::new();
        assert!(matches!(
            encode_record(&layout, &[Scalar::Int(-1)], &mut out),
            Err(SlimError::MissingField)
        ));
    }
}

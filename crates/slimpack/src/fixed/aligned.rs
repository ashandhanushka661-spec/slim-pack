//! `aligned` layout: dense records with no padding (`spec/SPEC.md` §2.2).
//!
//! Fields are written in declaration order at their natural width. Scalars
//! have a fixed byte offset; after a variable-length field (`str`, `bytes`,
//! `Vec<T>`, nested struct) the offsets are only known while walking, so the
//! codec walks in order and never trusts a precomputed offset after that point.

use super::scalar::{
    read_bool, read_int, read_uint, to_f32_exact, write_bool, write_int, write_uint, Scalar,
};
use crate::error::{Limit, Result, SlimError};
use crate::limits::Limits;
use crate::schema::{Layout, Type};
use crate::varint::{read_varint, write_varint};

/// Encodes one record in `aligned` layout, appending to `out`.
///
/// Returns the number of bytes written. `fields` must have exactly one value
/// per field in the layout.
pub fn encode_record(
    layout: &Layout,
    fields: &[Scalar<'_>],
    out: &mut Vec<u8>,
    limits: &Limits,
) -> Result<usize> {
    if fields.len() != layout.fields.len() {
        return Err(SlimError::MissingField);
    }
    let start = out.len();
    for (field_layout, value) in layout.fields.iter().zip(fields) {
        encode_field(&field_layout.ty, value, out, limits, 1)?;
    }
    Ok(out.len() - start)
}

fn encode_field(
    ty: &Type,
    value: &Scalar<'_>,
    out: &mut Vec<u8>,
    limits: &Limits,
    depth: usize,
) -> Result<()> {
    limits.check_depth(depth)?;
    let bytes = ty.scalar_bytes().unwrap_or(0);
    match ty {
        Type::Bool => {
            match value {
                Scalar::Bool(b) => write_bool(out, *b),
                Scalar::UInt(0) => write_bool(out, false),
                Scalar::UInt(1) => write_bool(out, true),
                Scalar::UInt(v) => {
                    return Err(SlimError::InvalidBool(u8::try_from(*v).unwrap_or(u8::MAX)))
                }
                _ => return Err(type_error(ty)),
            }
            Ok(())
        }
        Type::I8 | Type::I16 | Type::I32 | Type::I64 => {
            match value {
                Scalar::Int(v) => write_int(out, *v, bytes)?,
                _ => return Err(type_error(ty)),
            }
            Ok(())
        }
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => {
            match value {
                Scalar::UInt(v) => write_uint(out, *v, bytes)?,
                Scalar::Int(v) if *v >= 0 => write_uint(out, *v as u64, bytes)?,
                _ => return Err(type_error(ty)),
            }
            Ok(())
        }
        Type::F32 => {
            match value {
                // The bit path is the lossless one; an f64 value is accepted
                // only when it is exactly representable as f32.
                Scalar::F32Bits(bits) => out.extend_from_slice(&bits.to_le_bytes()),
                Scalar::Float(v) => {
                    let narrowed = to_f32_exact(*v).ok_or(SlimError::UnsupportedLayout(
                        "f64 value is not exactly representable as f32",
                    ))?;
                    out.extend_from_slice(&narrowed.to_bits().to_le_bytes());
                }
                _ => return Err(type_error(ty)),
            }
            Ok(())
        }
        Type::F64 => {
            match value {
                Scalar::Float(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
                _ => return Err(type_error(ty)),
            }
            Ok(())
        }
        Type::Str => {
            let Scalar::Str(text) = value else {
                return Err(type_error(ty));
            };
            let len = limits.check_len(text.len() as u64, text.len())?;
            write_varint(out, len as u64);
            out.extend_from_slice(text.as_bytes());
            Ok(())
        }
        Type::Bytes => {
            let Scalar::Bytes(body) = value else {
                return Err(type_error(ty));
            };
            let len = limits.check_len(body.len() as u64, body.len())?;
            write_varint(out, len as u64);
            out.extend_from_slice(body);
            Ok(())
        }
        Type::Vec(inner) => {
            let Scalar::Seq(items) = value else {
                return Err(type_error(ty));
            };
            let len = limits.check_len(items.len() as u64, items.len())?;
            write_varint(out, len as u64);
            for item in items {
                encode_field(inner, item, out, limits, depth + 1)?;
            }
            Ok(())
        }
        Type::Struct(_) => Err(SlimError::UnsupportedLayout(
            "nested structs need the derive macro (Phase 5)",
        )),
    }
}

/// Decodes one record in `aligned` layout from `record`, which is exactly
/// `stride` bytes long.
///
/// Decoded `Str`/`Bytes`/`Seq` values borrow from `record`.
pub fn decode_record<'a>(
    layout: &Layout,
    record: &'a [u8],
    limits: &Limits,
) -> Result<Vec<Scalar<'a>>> {
    let mut pos = 0usize;
    let out = decode_fields(layout, record, &mut pos, limits)?;
    if pos != record.len() {
        return Err(SlimError::TrailingBytes {
            extra: record.len() - pos,
        });
    }
    Ok(out)
}

/// Decodes one record starting at `pos`, leaving `pos` just past it.
///
/// Records with no variable-length field are exactly `stride` bytes, so the
/// next record starts at `pos + stride`. When a field *is* variable length the
/// record has no fixed size and the next one starts wherever this one ended.
fn decode_fields<'a>(
    layout: &Layout,
    record: &'a [u8],
    pos: &mut usize,
    limits: &Limits,
) -> Result<Vec<Scalar<'a>>> {
    let mut out = Vec::with_capacity(layout.fields.len());
    for field in &layout.fields {
        out.push(decode_field(&field.ty, record, pos, limits, 1)?);
    }
    Ok(out)
}

/// Decodes a whole Fixed body: the body header followed by `record_count`
/// records.
///
/// Applies SPEC rule R4: a stored `stride` below the reader's computed stride
/// is a [`SlimError::MissingField`], never a silently truncated record.
pub fn decode_records<'a>(
    layout: &Layout,
    body: &'a [u8],
    limits: &Limits,
) -> Result<Vec<Vec<Scalar<'a>>>> {
    let (count, stride, start) = read_body_header(body, limits)?;
    if count > 0 && stride < layout.stride() {
        return Err(SlimError::MissingField);
    }
    let records = &body[start..];
    let variable = layout.fields.iter().any(|field| field.variable);
    let mut out = Vec::with_capacity(count.min(1024));
    let mut pos = 0usize;
    for index in 0..count {
        if !variable {
            let end = pos.checked_add(stride).ok_or(SlimError::SizeOverflow)?;
            let record = records
                .get(pos..end)
                .ok_or_else(|| SlimError::truncated(end, records.len()))?;
            out.push(decode_record(layout, record, limits)?);
            pos = end;
        } else {
            out.push(decode_fields(layout, records, &mut pos, limits)?);
        }
        debug_assert!(pos <= records.len(), "record {index} overran the body");
    }
    Ok(out)
}

fn decode_field<'a>(
    ty: &Type,
    record: &'a [u8],
    pos: &mut usize,
    limits: &Limits,
    depth: usize,
) -> Result<Scalar<'a>> {
    limits.check_depth(depth)?;
    let bytes = ty.scalar_bytes().unwrap_or(0);
    match ty {
        Type::Bool => {
            let value = read_bool(record, *pos)?;
            *pos += 1;
            Ok(Scalar::Bool(value))
        }
        Type::I8 | Type::I16 | Type::I32 | Type::I64 => {
            let value = read_int(record, *pos, bytes)?;
            *pos += bytes;
            Ok(Scalar::Int(value))
        }
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => {
            let value = read_uint(record, *pos, bytes)?;
            *pos += bytes;
            Ok(Scalar::UInt(value))
        }
        Type::F32 => {
            let bits = read_uint(record, *pos, bytes)?;
            *pos += bytes;
            Ok(Scalar::F32Bits(bits as u32))
        }
        Type::F64 => {
            let bits = read_uint(record, *pos, bytes)?;
            *pos += bytes;
            Ok(Scalar::Float(f64::from_bits(bits)))
        }
        Type::Str | Type::Bytes => {
            let mut cursor = *pos;
            let declared = read_varint(record, &mut cursor)?;
            let len = limits.check_len(declared, record.len().saturating_sub(cursor))?;
            let body = record
                .get(cursor..cursor + len)
                .ok_or_else(|| SlimError::truncated(cursor + len, record.len()))?;
            if matches!(ty, Type::Str) && std::str::from_utf8(body).is_err() {
                return Err(SlimError::InvalidUtf8);
            }
            *pos = cursor + len;
            Ok(if matches!(ty, Type::Str) {
                Scalar::Str(std::str::from_utf8(body).map_err(|_| SlimError::InvalidUtf8)?)
            } else {
                Scalar::Bytes(body)
            })
        }
        Type::Vec(inner) => {
            let mut cursor = *pos;
            let declared = read_varint(record, &mut cursor)?;
            let len = limits.check_len(declared, record.len().saturating_sub(cursor))?;
            *pos = cursor;
            let mut items: Vec<Scalar<'a>> = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                items.push(decode_field(inner, record, pos, limits, depth + 1)?);
            }
            Ok(Scalar::Seq(items))
        }
        // Nested structs are inlined by the derive macro (Phase 5); a hand
        // written StructSpec cannot be inlined without a schema resolver.
        Type::Struct(_) => Err(SlimError::UnsupportedLayout(
            "nested structs need the derive macro (Phase 5)",
        )),
    }
}

/// A value of the wrong kind for a field is a schema error, not a corrupt
/// payload, so it is reported as an unsupported layout naming the field type.
fn type_error(ty: &Type) -> SlimError {
    SlimError::UnsupportedLayout(if matches!(ty, Type::Bool) {
        "bool field needs a Bool value"
    } else if ty.is_signed() {
        "signed integer field needs an Int value"
    } else if ty.is_float() {
        "float field needs a Float value"
    } else {
        "unsigned integer field needs a UInt value"
    })
}

/// Appends the body header (`record_count`, `stride`) for a Fixed payload.
pub fn write_body_header(out: &mut Vec<u8>, record_count: usize, stride: usize) {
    write_varint(out, record_count as u64);
    write_varint(out, stride as u64);
}

/// Encodes a whole Fixed `aligned` body: the body header, then every record.
///
/// `stride` is the size of the first record (SPEC 2), which is exact for a
/// fixed-width layout and a hint for a variable-length one. An empty body still
/// stores a stride of at least 1, as the format requires.
pub fn encode_records(
    layout: &Layout,
    records: &[Vec<Scalar<'_>>],
    out: &mut Vec<u8>,
    limits: &Limits,
) -> Result<usize> {
    limits.check_records(records.len() as u64)?;
    let start = out.len();
    let mut stride = 0usize;
    for (index, record) in records.iter().enumerate() {
        let record_start = out.len();
        encode_record(layout, record, out, limits)?;
        if index == 0 {
            stride = out.len() - record_start;
        }
    }
    let stride = stride.max(layout.stride()).max(1);
    let mut header = Vec::new();
    write_body_header(&mut header, records.len(), stride);
    out.splice(start..start, header);
    Ok(out.len() - start)
}

/// Reads `record_count` and `stride` from a Fixed body.
pub fn read_body_header(body: &[u8], limits: &Limits) -> Result<(usize, usize, usize)> {
    let mut pos = 0;
    let records = read_varint(body, &mut pos)?;
    let stride = read_varint(body, &mut pos)?;
    let records = limits.check_records(records)?;
    if stride == 0 {
        // SPEC 2: stride is always at least 1, so 0 can only be corruption.
        return Err(SlimError::UnsupportedLayout("stride must be >= 1"));
    }
    // A body with no records still stores a stride (SPEC 2 requires >= 1), so
    // the stride is only bounded by max_len until at least one record follows.
    let stride = if records == 0 {
        limits.check_count(stride, Limit::Len, limits.max_len)?
    } else {
        limits.check_len(stride, body.len().saturating_sub(pos))?
    };
    Ok((records, stride, pos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Attr, FieldSpec, LayoutKind, StructSpec};
    fn rows() -> StructSpec {
        StructSpec::new(
            "Row",
            vec![
                FieldSpec::new("id", Type::U32),
                FieldSpec::new("label", Type::Str),
            ],
        )
    }

    #[test]
    fn a_body_round_trips_every_record() {
        let limits = Limits::default();
        let layout = Layout::compute(&rows(), LayoutKind::Aligned).unwrap();
        let records = vec![
            vec![Scalar::UInt(1), Scalar::Str("a")],
            vec![Scalar::UInt(2), Scalar::Str("a longer label")],
            vec![Scalar::UInt(3), Scalar::Str("")],
        ];
        let mut body = Vec::new();
        let written = encode_records(&layout, &records, &mut body, &limits).unwrap();
        assert_eq!(written, body.len());
        let (count, stride, _) = read_body_header(&body, &limits).unwrap();
        assert_eq!(count, 3);
        // The first record's size, because the second field is variable.
        assert_eq!(stride, 4 + 1 + 1);
        assert_eq!(decode_records(&layout, &body, &limits).unwrap(), records);
    }

    #[test]
    fn variable_records_are_walked_in_order_not_by_stride() {
        let limits = Limits::default();
        let layout = Layout::compute(&rows(), LayoutKind::Aligned).unwrap();
        let records = vec![
            vec![Scalar::UInt(1), Scalar::Str("xx")],
            vec![Scalar::UInt(2), Scalar::Str("yyyy")],
        ];
        let mut body = Vec::new();
        encode_records(&layout, &records, &mut body, &limits).unwrap();
        // Records are not a multiple of stride apart, so jumping would fail.
        let decoded = decode_records(&layout, &body, &limits).unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn an_empty_body_stores_a_stride_of_at_least_one() {
        let limits = Limits::default();
        let layout = Layout::compute(&rows(), LayoutKind::Aligned).unwrap();
        let mut body = Vec::new();
        encode_records(&layout, &[], &mut body, &limits).unwrap();
        let (count, stride, _) = read_body_header(&body, &limits).unwrap();
        // Nothing to measure, so the schema's fixed prefix is stored.
        assert_eq!((count, stride), (0, 4));
        assert!(decode_records(&layout, &body, &limits).unwrap().is_empty());
    }

    #[test]
    fn a_zero_stride_is_corruption() {
        let limits = Limits::default();
        let mut body = Vec::new();
        write_varint(&mut body, 1);
        write_varint(&mut body, 0);
        assert_eq!(
            read_body_header(&body, &limits),
            Err(SlimError::UnsupportedLayout("stride must be >= 1"))
        );
    }

    #[test]
    fn a_short_stride_is_a_missing_field() {
        let limits = Limits::default();
        let layout = Layout::compute(&rows(), LayoutKind::Aligned).unwrap();
        let mut body = Vec::new();
        write_varint(&mut body, 1);
        write_varint(&mut body, 2);
        body.extend_from_slice(&[1, 0, 0, 0]);
        assert_eq!(
            decode_records(&layout, &body, &limits),
            Err(SlimError::MissingField)
        );
    }

    #[test]
    fn a_bool_field_rejects_a_byte_that_is_not_zero_or_one() {
        let limits = Limits::default();
        let layout = Layout::compute(
            &StructSpec::new("B", vec![FieldSpec::new("flag", Type::Bool)]),
            LayoutKind::Aligned,
        )
        .unwrap();
        let mut out = Vec::new();
        assert_eq!(
            encode_record(&layout, &[Scalar::UInt(7)], &mut out, &limits),
            Err(SlimError::InvalidBool(7))
        );
        let mut out = Vec::new();
        encode_record(&layout, &[Scalar::UInt(1)], &mut out, &limits).unwrap();
        assert_eq!(out, vec![1]);
        assert_eq!(
            decode_record(&layout, &[2], &limits),
            Err(SlimError::InvalidBool(2))
        );
    }

    #[test]
    fn a_truncated_record_is_reported_not_silently_short() {
        let limits = Limits::default();
        let layout = Layout::compute(
            &StructSpec::new("B", vec![FieldSpec::new("id", Type::U32)]),
            LayoutKind::Aligned,
        )
        .unwrap();
        let mut body = Vec::new();
        write_varint(&mut body, 2);
        write_varint(&mut body, 4);
        body.extend_from_slice(&[1, 0, 0, 0, 2]);
        assert!(matches!(
            decode_records(&layout, &body, &limits),
            Err(SlimError::Truncated { .. })
        ));
    }

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

    fn aligned() -> Layout {
        Layout::compute(&telemetry(), LayoutKind::Aligned).unwrap()
    }

    #[test]
    fn spec_2_5_example_encodes_to_the_documented_bytes() {
        let layout = aligned();
        let mut out = Vec::new();
        let n = encode_record(
            &layout,
            &[Scalar::Int(-103), Scalar::UInt(3), Scalar::Float(27.5)],
            &mut out,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(n, 7);
        assert_eq!(out, vec![0x99, 0xFF, 0x03, 0x00, 0x00, 0xDC, 0x41]);
    }

    #[test]
    fn spec_2_5_example_decodes_back_to_the_value() {
        let layout = aligned();
        let record = [0x99u8, 0xFF, 0x03, 0x00, 0x00, 0xDC, 0x41];
        let got = decode_record(&layout, &record, &Limits::default()).unwrap();
        assert_eq!(got[0], Scalar::Int(-103));
        assert_eq!(got[1], Scalar::UInt(3));
        assert_eq!(got[2], Scalar::F32Bits(0x41DC_0000));
    }

    #[test]
    fn float_bits_are_preserved_exactly() {
        let spec = StructSpec::new("F", vec![FieldSpec::new("f", Type::F32)]);
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        let cases = [
            0x0000_0000u32, // +0.0
            0x8000_0000,    // -0.0
            0x7F80_0000,    // +inf
            0xFF80_0000,    // -inf
            0x7FC0_0042,    // quiet NaN with payload
            0x7F80_0001,    // signalling NaN
            0x41DC_0000,    // 27.5
            0x7F7F_FFFF,    // f32::MAX
            0x0080_0000,    // f32::MIN_POSITIVE
        ];
        let mut out = Vec::new();
        for bits in cases {
            encode_record(
                &layout,
                &[Scalar::F32Bits(bits)],
                &mut out,
                &Limits::default(),
            )
            .unwrap();
        }
        assert_eq!(out.len(), cases.len() * 4);
        for (i, bits) in cases.iter().enumerate() {
            let record = &out[i * 4..(i + 1) * 4];
            assert_eq!(record, &bits.to_le_bytes(), "byte order for {bits:#010x}");
            let got = decode_record(&layout, record, &Limits::default()).unwrap();
            assert_eq!(
                got[0],
                Scalar::F32Bits(*bits),
                "{bits:#010x} must round-trip"
            );
        }
    }

    #[test]
    fn an_f64_that_f32_cannot_hold_is_refused() {
        let spec = StructSpec::new("F", vec![FieldSpec::new("f", Type::F32)]);
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        let mut out = Vec::new();
        assert!(matches!(
            encode_record(&layout, &[Scalar::Float(0.1)], &mut out, &Limits::default()),
            Err(SlimError::UnsupportedLayout(_))
        ));
        encode_record(
            &layout,
            &[Scalar::Float(27.5)],
            &mut out,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(out, 0x41DC_0000u32.to_le_bytes());
    }

    #[test]
    fn f64_bits_are_preserved_exactly() {
        let spec = StructSpec::new("F", vec![FieldSpec::new("f", Type::F64)]);
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        for bits in [
            0x0000_0000_0000_0000u64, // +0.0
            0x8000_0000_0000_0000,    // -0.0
            0x7FF0_0000_0000_0000,    // +inf
            0xFFF0_0000_0000_0000,    // -inf
            0x7FF8_0000_DEAD_BEEF,    // NaN with payload
            0x400D_8000_0000_0000,    // 27.5
        ] {
            let mut out = Vec::new();
            encode_record(
                &layout,
                &[Scalar::Float(f64::from_bits(bits))],
                &mut out,
                &Limits::default(),
            )
            .unwrap();
            assert_eq!(out, bits.to_le_bytes());
            let got = decode_record(&layout, &out, &Limits::default()).unwrap();
            let Scalar::Float(value) = got[0] else {
                panic!("expected Float");
            };
            assert_eq!(value.to_bits(), bits, "{bits:#018x} must round-trip");
        }
    }

    #[test]
    fn strings_and_bytes_are_length_prefixed() {
        let spec = StructSpec::new(
            "S",
            vec![
                FieldSpec::new("name", Type::Str),
                FieldSpec::new("raw", Type::Bytes),
            ],
        );
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        let mut out = Vec::new();
        encode_record(
            &layout,
            &[Scalar::Str("héllo"), Scalar::Bytes(&[0, 1, 2])],
            &mut out,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(out[0], 6, "str length is a varint");
        assert_eq!(&out[1..7], "héllo".as_bytes());
        assert_eq!(out[7], 3);
        assert_eq!(&out[8..], &[0, 1, 2]);
        let got = decode_record(&layout, &out, &Limits::default()).unwrap();
        assert_eq!(got[0], Scalar::Str("héllo"));
        assert_eq!(got[1], Scalar::Bytes(&[0, 1, 2]));
    }

    #[test]
    fn vectors_are_length_prefixed() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("xs", Type::Vec(Box::new(Type::U8)))],
        );
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        let items = vec![Scalar::UInt(1), Scalar::UInt(2), Scalar::UInt(255)];
        let mut out = Vec::new();
        encode_record(
            &layout,
            &[Scalar::Seq(items.clone())],
            &mut out,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(out, vec![3, 1, 2, 255]);
        let got = decode_record(&layout, &out, &Limits::default()).unwrap();
        assert_eq!(got[0], Scalar::Seq(items));
    }

    #[test]
    fn invalid_utf8_in_a_str_field_is_rejected() {
        let spec = StructSpec::new("S", vec![FieldSpec::new("s", Type::Str)]);
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        let bad = [2u8, 0xFF, 0xFE];
        assert!(matches!(
            decode_record(&layout, &bad, &Limits::default()),
            Err(SlimError::InvalidUtf8)
        ));
    }

    #[test]
    fn a_declared_string_length_is_checked_before_use() {
        let spec = StructSpec::new("S", vec![FieldSpec::new("s", Type::Str)]);
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        let huge = [0xFFu8, 0xFF, 0xFF, 0x7F, 0x01];
        assert!(matches!(
            decode_record(&layout, &huge, &Limits::default()),
            Err(SlimError::LimitExceeded { .. })
        ));
        assert!(matches!(
            decode_record(&layout, &[200, 1], &Limits::default()),
            Err(SlimError::Truncated { .. })
        ));
    }

    #[test]
    fn trailing_bytes_in_a_record_are_rejected() {
        let spec = StructSpec::new("S", vec![FieldSpec::new("a", Type::U8)]);
        let layout = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        assert!(matches!(
            decode_record(&layout, &[1, 2], &Limits::default()),
            Err(SlimError::TrailingBytes { extra: 1 })
        ));
    }

    #[test]
    fn wrong_value_kind_is_reported_not_written() {
        let layout = aligned();
        let mut out = Vec::new();
        assert!(matches!(
            encode_record(
                &layout,
                &[Scalar::Str("x"), Scalar::UInt(3), Scalar::Float(1.0)],
                &mut out,
                &Limits::default(),
            ),
            Err(SlimError::UnsupportedLayout(_))
        ));
        assert!(out.is_empty());
    }

    #[test]
    fn field_count_must_match_the_layout() {
        let layout = aligned();
        let mut out = Vec::new();
        assert!(matches!(
            encode_record(&layout, &[Scalar::Int(-1)], &mut out, &Limits::default()),
            Err(SlimError::MissingField)
        ));
        assert!(matches!(
            encode_record(&layout, &[], &mut out, &Limits::default()),
            Err(SlimError::MissingField)
        ));
    }

    #[test]
    fn body_header_round_trips() {
        let mut out = Vec::new();
        write_body_header(&mut out, 10, 3);
        out.extend_from_slice(&[0u8; 30]);
        let (records, stride, pos) = read_body_header(&out, &Limits::default()).unwrap();
        assert_eq!((records, stride, pos), (10, 3, 2));
        assert_eq!(records * stride, out.len() - pos, "records fill the rest");
    }

    #[test]
    fn body_header_enforces_limits() {
        let mut out = Vec::new();
        write_body_header(&mut out, 10_000_001, 1);
        assert!(matches!(
            read_body_header(&out, &Limits::default()),
            Err(SlimError::LimitExceeded {
                limit: crate::Limit::Records,
                ..
            })
        ));
        let mut out = Vec::new();
        write_body_header(&mut out, 1, 1000);
        assert!(matches!(
            read_body_header(&out, &Limits::default()),
            Err(SlimError::Truncated { .. })
        ));
    }
}

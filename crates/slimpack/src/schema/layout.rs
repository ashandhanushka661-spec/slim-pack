//! The layout engine: offsets, bit widths and strides for a [`StructSpec`].
//!
//! One implementation, used by the library, the derive macro and
//! `slim schema layout`, so a printed layout is always the encoded layout.
//!
//! * `aligned` (§2.2): fields in declaration order, natural widths, no padding.
//!   Scalars have a fixed byte offset; once a variable-length field (`str`,
//!   `bytes`, `Vec`, nested struct) has been seen, the offsets that follow are
//!   only known while walking, so [`FieldLayout::byte_offset`] is `None` for
//!   them and [`Layout::fixed_prefix_bytes`] says how many bytes are known up
//!   front.
//! * `packed` (§2.4): every field gets a bit width, the bit string is stored
//!   LSB-first, and `stride = ceil(total_bits / 8)`.

use super::model::{Attr, StructSpec, Type};
use crate::error::{Result, SlimError};

/// Which of the two Fixed layouts is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    /// `layout = aligned` (the default, §2.6).
    Aligned,
    /// `layout = packed`.
    Packed,
}

/// The computed layout of one field.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldLayout {
    /// Field name.
    pub name: String,
    /// Field type.
    pub ty: Type,
    /// Byte offset of the field, or `None` in `aligned` mode when an earlier
    /// field was variable-length.
    pub byte_offset: Option<usize>,
    /// Bit offset inside `byte_offset`; always 0 in `aligned` mode.
    pub bit_offset: u32,
    /// Width in bits.
    pub bit_width: u32,
    /// Fixed byte width in `aligned` mode; 0 for variable-length types.
    pub byte_width: usize,
    /// True for `str`, `bytes`, `Vec<T>` and nested structs.
    pub variable: bool,
    /// The `range` attribute in force, if any.
    pub range: Option<(i128, i128)>,
    /// The `bits` attribute in force, if any.
    pub bits: Option<u8>,
    /// Width in bits an unconstrained field would use.
    pub natural_bits: u32,
}

/// The computed layout of a whole struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Which layout was computed.
    pub kind: LayoutKind,
    /// Per-field layout, in declaration order.
    pub fields: Vec<FieldLayout>,
    /// Bytes of the record before the first variable-length field.
    pub fixed_prefix_bytes: usize,
    /// Total bit width of the record (`aligned`: `fixed_prefix_bytes * 8`).
    pub total_bits: u64,
}

impl Layout {
    /// `stride` in bytes, as stored in the body (§2).
    pub fn stride(&self) -> usize {
        match self.kind {
            LayoutKind::Aligned => self.fixed_prefix_bytes,
            LayoutKind::Packed => (self.total_bits as usize).div_ceil(8),
        }
    }

    /// Computes the layout of `spec` in `kind`.
    pub fn compute(spec: &StructSpec, kind: LayoutKind) -> Result<Self> {
        match kind {
            LayoutKind::Aligned => Self::compute_aligned(spec),
            LayoutKind::Packed => Self::compute_packed(spec),
        }
    }

    fn compute_aligned(spec: &StructSpec) -> Result<Self> {
        let mut fields = Vec::with_capacity(spec.fields.len());
        let mut byte_cursor: usize = 0;
        let mut seen_variable = false;
        let mut fixed_prefix_bytes = 0;
        for field in &spec.fields {
            // Attributes are validated in both layouts: an attribute that cannot
            // be encoded losslessly is a schema error, not a layout detail.
            validate_attrs_for(&field.ty, field.range(), field.bits())?;
            let variable = field.ty.scalar_bytes().is_none();
            // This field's own offset is known iff everything before it is
            // fixed; the fields after a variable-length one are not.
            let byte_offset = (!seen_variable).then_some(byte_cursor);
            let byte_width = field.ty.scalar_bytes().unwrap_or(0);
            if !variable && byte_offset.is_some() {
                fixed_prefix_bytes = byte_cursor + byte_width;
                byte_cursor += byte_width;
            }
            seen_variable |= variable;
            fields.push(FieldLayout {
                name: field.name.clone(),
                ty: field.ty.clone(),
                byte_offset,
                bit_offset: 0,
                bit_width: (byte_width * 8) as u32,
                byte_width,
                variable,
                range: field.range(),
                bits: field.bits(),
                natural_bits: (byte_width * 8) as u32,
            });
        }
        Ok(Layout {
            kind: LayoutKind::Aligned,
            fields,
            fixed_prefix_bytes,
            total_bits: (fixed_prefix_bytes * 8) as u64,
        })
    }

    fn compute_packed(spec: &StructSpec) -> Result<Self> {
        let mut fields = Vec::with_capacity(spec.fields.len());
        let mut bit_cursor: u64 = 0;
        for field in &spec.fields {
            if !field.ty.is_scalar() {
                return Err(SlimError::UnsupportedLayout(
                    "packed mode supports scalars only (spec 2.4)",
                ));
            }
            // bool is 1 bit in packed mode (spec 2.4), not its 1-byte
            // aligned width.
            let natural_bits = if matches!(field.ty, Type::Bool) {
                1
            } else {
                (field.ty.scalar_bytes().expect("scalar") * 8) as u32
            };
            validate_attrs_for(&field.ty, field.range(), field.bits())?;
            let bit_width = match (field.range(), field.bits()) {
                (Some((min, max)), _) => {
                    let width = bits_for_values(range_width(min, max)?);
                    if width > natural_bits && field.ty.is_integral() {
                        return Err(SlimError::UnsupportedLayout(
                            "range needs more bits than the type provides (spec 2.4)",
                        ));
                    }
                    width
                }
                (None, Some(n)) => {
                    // bool may use 1..=8 bits (only 0 and 1 are legal values);
                    // every other type may not exceed its natural width.
                    let max_bits = if matches!(field.ty, Type::Bool) {
                        8
                    } else {
                        natural_bits
                    };
                    if n == 0 || n as u32 > max_bits {
                        return Err(SlimError::UnsupportedLayout("bits = N out of range"));
                    }
                    n as u32
                }
                (None, None) => natural_bits,
            };
            fields.push(FieldLayout {
                name: field.name.clone(),
                ty: field.ty.clone(),
                byte_offset: Some((bit_cursor / 8) as usize),
                bit_offset: (bit_cursor % 8) as u32,
                bit_width,
                byte_width: 0,
                variable: false,
                range: field.range(),
                bits: field.bits(),
                natural_bits,
            });
            bit_cursor += u64::from(bit_width);
        }
        Ok(Layout {
            kind: LayoutKind::Packed,
            fields,
            fixed_prefix_bytes: (bit_cursor as usize).div_ceil(8),
            total_bits: bit_cursor,
        })
    }
}

/// Number of values in the inclusive range `min..=max`, rejecting inverted and
/// unrepresentable ranges.
pub fn range_width(min: i128, max: i128) -> Result<u128> {
    if min > max {
        return Err(SlimError::UnsupportedLayout("range min > max"));
    }
    Ok((max - min) as u128 + 1)
}

/// Bits needed to store `count` distinct values as `0..=count-1`, minimum 1.
pub fn bits_for_values(count: u128) -> u32 {
    if count <= 1 {
        return 1;
    }
    128 - (count - 1).leading_zeros()
}

/// Legal value span for `bits = N` on a signed type, per §2.4.
pub fn signed_bits_domain(n: u8) -> Option<(i128, i128)> {
    if n == 0 || n > 64 {
        return None;
    }
    if n == 64 {
        return Some((i64::MIN as i128, i64::MAX as i128));
    }
    let half = 1i128 << (n - 1);
    Some((-half, half - 1))
}

/// Rejects attribute combinations that cannot be encoded losslessly.
fn validate_attrs_for(ty: &Type, range: Option<(i128, i128)>, bits: Option<u8>) -> Result<()> {
    if let Some((min, max)) = range {
        if !ty.is_integral() {
            return Err(SlimError::UnsupportedLayout(
                "range applies to integers and bool only; floats need quantize() (spec 2.4)",
            ));
        }
        if let (Some(dmin), Some(dmax)) = (ty.domain_min(), ty.domain_max()) {
            if min < dmin || max > dmax {
                return Err(SlimError::UnsupportedLayout(
                    "range exceeds the domain of the field type",
                ));
            }
        }
        let _ = range_width(min, max)?;
        if bits_for_values(range_width(min, max)?) > 64 {
            return Err(SlimError::UnsupportedLayout("range too wide to store"));
        }
    }
    if let Some(n) = bits {
        if ty.is_float() {
            return Err(SlimError::UnsupportedLayout(
                "bits = N does not apply to floats (spec 2.4)",
            ));
        }
        if ty.is_signed() && signed_bits_domain(n).is_none() {
            return Err(SlimError::UnsupportedLayout("bits = N out of range"));
        }
    }
    Ok(())
}

/// True if any field carries the LOSSY `quantize` attribute.
pub fn is_lossy(spec: &StructSpec) -> bool {
    spec.fields
        .iter()
        .any(|f| f.attrs.iter().any(|a| matches!(a, Attr::Quantize(_))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::model::FieldSpec;

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

    #[test]
    fn spec_2_5_aligned_layout_is_7_bytes() {
        let l = Layout::compute(&telemetry(), LayoutKind::Aligned).unwrap();
        assert_eq!(l.stride(), 7, "SPEC 2.5: 2 + 1 + 4 = 7 bytes");
        assert_eq!(l.total_bits, 56);
        assert_eq!(
            l.fields.iter().map(|f| f.byte_offset).collect::<Vec<_>>(),
            vec![Some(0), Some(2), Some(3)]
        );
        assert_eq!(
            l.fields.iter().map(|f| f.byte_width).collect::<Vec<_>>(),
            vec![2, 1, 4]
        );
        // range has no effect in aligned mode (spec 2.2)
        assert_eq!(l.fields[0].bit_width, 16);
    }

    #[test]
    fn spec_2_5_packed_layout_is_47_bits_and_6_bytes() {
        let l = Layout::compute(&telemetry(), LayoutKind::Packed).unwrap();
        assert_eq!(l.total_bits, 47, "7 (range) + 8 (u8) + 32 (f32)");
        assert_eq!(l.stride(), 6, "SPEC 2.5: ceil(47 / 8) = 6 bytes");
        assert_eq!(l.fields[0].bit_width, 7, "97 values need 7 bits");
        assert_eq!(
            l.fields
                .iter()
                .map(|f| (f.byte_offset, f.bit_offset))
                .collect::<Vec<_>>(),
            vec![(Some(0), 0), (Some(0), 7), (Some(1), 7)]
        );
    }

    #[test]
    fn spec_2_5_packed_bits_3_variant_is_42_bits() {
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
        let l = Layout::compute(&spec, LayoutKind::Packed).unwrap();
        assert_eq!(l.total_bits, 42, "SPEC 2.5: 7 + 3 + 32");
        assert_eq!(l.stride(), 6);
    }

    #[test]
    fn bits_for_values_matches_the_spec_formula() {
        assert_eq!(bits_for_values(1), 1);
        assert_eq!(bits_for_values(2), 1);
        assert_eq!(bits_for_values(3), 2);
        assert_eq!(bits_for_values(97), 7, "ceil(log2(97)) = 7");
        assert_eq!(bits_for_values(256), 8);
        assert_eq!(bits_for_values(257), 9);
        assert_eq!(bits_for_values(u64::MAX as u128), 64);
        assert_eq!(bits_for_values(1u128 << 63), 63);
    }

    #[test]
    fn range_width_rejects_inverted_ranges() {
        assert_eq!(range_width(-140, -44).unwrap(), 97);
        assert_eq!(range_width(5, 5).unwrap(), 1);
        assert!(matches!(
            range_width(10, 0),
            Err(SlimError::UnsupportedLayout(_))
        ));
        assert_eq!(
            range_width(i64::MIN as i128, i64::MAX as i128).unwrap(),
            (i64::MAX as i128 - i64::MIN as i128) as u128 + 1
        );
    }

    #[test]
    fn signed_bits_domain_follows_the_spec() {
        assert_eq!(signed_bits_domain(8), Some((-128, 127)));
        // -2^(n-1) ..= 2^(n-1)-1, so n = 1 is the degenerate two's complement
        // case -1..=0 (spec 2.4).
        assert_eq!(signed_bits_domain(1), Some((-1, 0)));
        assert_eq!(signed_bits_domain(16), Some((-32768, 32767)));
        assert_eq!(
            signed_bits_domain(64),
            Some((i64::MIN as i128, i64::MAX as i128))
        );
        assert_eq!(signed_bits_domain(0), None);
        assert_eq!(signed_bits_domain(65), None);
    }

    #[test]
    fn packed_rejects_non_scalars() {
        for ty in [
            Type::Str,
            Type::Bytes,
            Type::Vec(Box::new(Type::U8)),
            Type::Struct("Inner".into()),
        ] {
            let spec = StructSpec::new("S", vec![FieldSpec::new("f", ty.clone())]);
            assert!(
                matches!(
                    Layout::compute(&spec, LayoutKind::Packed),
                    Err(SlimError::UnsupportedLayout(_))
                ),
                "{ty:?} must not be packable"
            );
        }
    }

    #[test]
    fn aligned_marks_offsets_after_a_variable_field_as_unknown() {
        let spec = StructSpec::new(
            "S",
            vec![
                FieldSpec::new("a", Type::U8),
                FieldSpec::new("name", Type::Str),
                FieldSpec::new("b", Type::U16),
            ],
        );
        let l = Layout::compute(&spec, LayoutKind::Aligned).unwrap();
        assert_eq!(l.fields[0].byte_offset, Some(0));
        assert_eq!(
            l.fields[1].byte_offset,
            Some(1),
            "known up to the variable field"
        );
        assert_eq!(l.fields[2].byte_offset, None, "offset depends on len(name)");
        assert!(l.fields[1].variable, "str is variable-length");
        assert!(
            !l.fields[2].variable,
            "u16 is fixed-width but not at a fixed offset"
        );
        assert_eq!(l.fields[2].byte_width, 2, "its own width is still known");
        assert_eq!(l.fixed_prefix_bytes, 1, "only `a` is at a fixed offset");
    }

    #[test]
    fn aligned_rejects_range_on_a_float_and_bits_on_a_float() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("f", Type::F32).with(Attr::Range { min: 0, max: 1 })],
        );
        assert!(matches!(
            Layout::compute(&spec, LayoutKind::Aligned),
            Err(SlimError::UnsupportedLayout(_))
        ));
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("f", Type::F64).with(Attr::Bits(32))],
        );
        assert!(matches!(
            Layout::compute(&spec, LayoutKind::Packed),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn range_wider_than_the_type_is_rejected() {
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("b", Type::U8).with(Attr::Range { min: 0, max: 300 })],
        );
        assert!(matches!(
            Layout::compute(&spec, LayoutKind::Aligned),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn bool_is_one_bit_in_packed_and_may_use_bits_attr() {
        let plain = StructSpec::new("S", vec![FieldSpec::new("b", Type::Bool)]);
        let l = Layout::compute(&plain, LayoutKind::Packed).unwrap();
        assert_eq!(l.total_bits, 1, "spec 2.4: bool is 1 bit");
        let ranged = StructSpec::new(
            "S",
            vec![FieldSpec::new("b", Type::Bool).with(Attr::Range { min: 0, max: 1 })],
        );
        assert_eq!(
            Layout::compute(&ranged, LayoutKind::Packed)
                .unwrap()
                .total_bits,
            1
        );
        for n in [1u8, 4, 8] {
            let spec = StructSpec::new(
                "S",
                vec![FieldSpec::new("b", Type::Bool).with(Attr::Bits(n))],
            );
            assert_eq!(
                Layout::compute(&spec, LayoutKind::Packed)
                    .unwrap()
                    .total_bits,
                u64::from(n),
                "bool with bits = {n}"
            );
        }
        let spec = StructSpec::new(
            "S",
            vec![FieldSpec::new("b", Type::Bool).with(Attr::Bits(9))],
        );
        assert!(matches!(
            Layout::compute(&spec, LayoutKind::Packed),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn bits_wider_than_the_type_is_rejected() {
        let spec = StructSpec::new("S", vec![FieldSpec::new("b", Type::U8).with(Attr::Bits(9))]);
        assert!(matches!(
            Layout::compute(&spec, LayoutKind::Packed),
            Err(SlimError::UnsupportedLayout(_))
        ));
        let spec = StructSpec::new("S", vec![FieldSpec::new("b", Type::U8).with(Attr::Bits(0))]);
        assert!(matches!(
            Layout::compute(&spec, LayoutKind::Packed),
            Err(SlimError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn lossiness_is_detected_from_the_quantize_attribute() {
        assert!(!is_lossy(&telemetry()));
        let lossy = StructSpec::new(
            "S",
            vec![FieldSpec::new("x", Type::I16).with(Attr::Quantize(0.5))],
        );
        assert!(is_lossy(&lossy));
    }

    #[test]
    fn empty_struct_has_stride_zero() {
        let spec = StructSpec::new("Empty", vec![]);
        for kind in [LayoutKind::Aligned, LayoutKind::Packed] {
            let l = Layout::compute(&spec, kind).unwrap();
            assert_eq!(l.stride(), 0);
            assert_eq!(l.total_bits, 0);
            assert!(l.fields.is_empty());
        }
    }
}

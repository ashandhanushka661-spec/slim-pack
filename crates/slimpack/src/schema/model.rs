//! The field/type model that layouts are computed from.
//!
//! The library, the derive macro and `slim schema layout` all share this model
//! (`spec/SPEC.md` §2.1, §8), so they cannot disagree about a layout.

use crate::error::{Result, SlimError};

/// A field type, with the code used by the §1.2 layout descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// `bool`, 1 byte / 1 bit.
    Bool,
    /// `i8`, 1 byte / 8 bits.
    I8,
    /// `u8`, 1 byte / 8 bits.
    U8,
    /// `i16`, 2 bytes / 16 bits.
    I16,
    /// `u16`, 2 bytes / 16 bits.
    U16,
    /// `i32`, 4 bytes / 32 bits.
    I32,
    /// `u32`, 4 bytes / 32 bits.
    U32,
    /// `i64`, 8 bytes / 64 bits.
    I64,
    /// `u64`, 8 bytes / 64 bits.
    U64,
    /// `f32`, 4 bytes / 32 bits, bit-exact.
    F32,
    /// `f64`, 8 bytes / 64 bits, bit-exact.
    F64,
    /// `str`: varint length + validated UTF-8 bytes.
    Str,
    /// `bytes`: varint length + opaque bytes.
    Bytes,
    /// `Vec<T>`: varint length + `len` × `T`.
    Vec(Box<Type>),
    /// A nested struct, inlined in `aligned` mode.
    Struct(String),
}

impl Type {
    /// The `type_code` from `spec/SPEC.md` §2.1.
    pub fn code(&self) -> u8 {
        match self {
            Type::Bool => 1,
            Type::I8 => 2,
            Type::U8 => 3,
            Type::I16 => 4,
            Type::U16 => 5,
            Type::I32 => 6,
            Type::U32 => 7,
            Type::I64 => 8,
            Type::U64 => 9,
            Type::F32 => 10,
            Type::F64 => 11,
            Type::Str => 12,
            Type::Bytes => 13,
            Type::Vec(_) => 14,
            Type::Struct(_) => 15,
        }
    }

    /// Inverse of [`Type::code`].
    pub fn from_code(code: u8) -> Result<Type> {
        Ok(match code {
            1 => Type::Bool,
            2 => Type::I8,
            3 => Type::U8,
            4 => Type::I16,
            5 => Type::U16,
            6 => Type::I32,
            7 => Type::U32,
            8 => Type::I64,
            9 => Type::U64,
            10 => Type::F32,
            11 => Type::F64,
            12 => Type::Str,
            13 => Type::Bytes,
            14 => Type::Vec(Box::new(Type::U8)),
            _ => return Err(SlimError::UnknownEncoding(code)),
        })
    }

    /// True for the scalars that `packed` mode supports.
    pub fn is_scalar(&self) -> bool {
        !matches!(
            self,
            Type::Str | Type::Bytes | Type::Vec(_) | Type::Struct(_)
        )
    }

    /// True for the signed integer types.
    pub fn is_signed(&self) -> bool {
        matches!(self, Type::I8 | Type::I16 | Type::I32 | Type::I64)
    }

    /// True for `i8`..`i64`, `u8`..`u64` and `bool`.
    pub fn is_integral(&self) -> bool {
        self.is_signed()
            || matches!(
                self,
                Type::U8 | Type::U16 | Type::U32 | Type::U64 | Type::Bool
            )
    }

    /// True for `f32`/`f64`.
    pub fn is_float(&self) -> bool {
        matches!(self, Type::F32 | Type::F64)
    }

    /// Natural width in bytes of a scalar, or `None` for a variable-length type.
    pub fn scalar_bytes(&self) -> Option<usize> {
        Some(match self {
            Type::Bool | Type::I8 | Type::U8 => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            _ => return None,
        })
    }

    /// Smallest legal value of the type, as `i128`.
    pub fn domain_min(&self) -> Option<i128> {
        Some(match self {
            Type::Bool => 0,
            Type::I8 => i8::MIN as i128,
            Type::U8 => 0,
            Type::I16 => i16::MIN as i128,
            Type::U16 => 0,
            Type::I32 => i32::MIN as i128,
            Type::U32 => 0,
            Type::I64 => i64::MIN as i128,
            Type::U64 => 0,
            _ => return None,
        })
    }

    /// Largest legal value of the type, as `i128`.
    pub fn domain_max(&self) -> Option<i128> {
        Some(match self {
            Type::Bool => 1,
            Type::I8 => i8::MAX as i128,
            Type::U8 => u8::MAX as i128,
            Type::I16 => i16::MAX as i128,
            Type::U16 => u16::MAX as i128,
            Type::I32 => i32::MAX as i128,
            Type::U32 => u32::MAX as i128,
            Type::I64 => i64::MAX as i128,
            Type::U64 => u64::MAX as i128,
            _ => return None,
        })
    }
}

/// A field attribute from `spec/SPEC.md` §2.4 / §8.
#[derive(Debug, Clone, PartialEq)]
pub enum Attr {
    /// `range(A..=B)`: lossless, stored as `value - A`.
    Range {
        /// Smallest legal value.
        min: i128,
        /// Largest legal value.
        max: i128,
    },
    /// `bits(N)`: lossless for bool and unsigned integers that fit in N bits.
    Bits(u8),
    /// `quantize(scale)`: LOSSY, opt-in.
    Quantize(f64),
    /// `optional`: Batch mode only.
    Optional,
}

/// One field of a struct.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSpec {
    /// Field name; never present in a Bare payload.
    pub name: String,
    /// Field type.
    pub ty: Type,
    /// Attributes, in declaration order.
    pub attrs: Vec<Attr>,
}

impl FieldSpec {
    /// Builds a field with no attributes.
    pub fn new(name: &str, ty: Type) -> Self {
        Self {
            name: name.to_owned(),
            ty,
            attrs: Vec::new(),
        }
    }

    /// Adds an attribute.
    pub fn with(mut self, attr: Attr) -> Self {
        self.attrs.push(attr);
        self
    }

    /// The `range` attribute, if present.
    pub fn range(&self) -> Option<(i128, i128)> {
        self.attrs.iter().find_map(|a| match a {
            Attr::Range { min, max } => Some((*min, *max)),
            _ => None,
        })
    }

    /// The `bits` attribute, if present.
    pub fn bits(&self) -> Option<u8> {
        self.attrs.iter().find_map(|a| match a {
            Attr::Bits(n) => Some(*n),
            _ => None,
        })
    }
}

/// A struct: the schema both sides of a Fixed payload must agree on.
#[derive(Debug, Clone, PartialEq)]
pub struct StructSpec {
    /// Struct name, used only by self-describing payloads.
    pub name: String,
    /// Fields, in declaration order.
    pub fields: Vec<FieldSpec>,
}

impl StructSpec {
    /// Builds a struct spec.
    pub fn new(name: &str, fields: Vec<FieldSpec>) -> Self {
        Self {
            name: name.to_owned(),
            fields,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_codes_match_the_spec_table() {
        let all = [
            Type::Bool,
            Type::I8,
            Type::U8,
            Type::I16,
            Type::U16,
            Type::I32,
            Type::U32,
            Type::I64,
            Type::U64,
            Type::F32,
            Type::F64,
            Type::Str,
            Type::Bytes,
            Type::Vec(Box::new(Type::U8)),
        ];
        for (want, ty) in all.iter().enumerate() {
            let code = (want + 1) as u8;
            assert_eq!(ty.code(), code, "code of {ty:?}");
            assert_eq!(Type::from_code(code).unwrap(), *ty, "round trip of {ty:?}");
        }
        assert_eq!(Type::Struct("T".into()).code(), 15);
        assert!(matches!(
            Type::from_code(16),
            Err(SlimError::UnknownEncoding(16))
        ));
        assert!(matches!(
            Type::from_code(0),
            Err(SlimError::UnknownEncoding(0))
        ));
    }

    #[test]
    fn widths_match_the_spec_table() {
        assert_eq!(Type::Bool.scalar_bytes(), Some(1));
        assert_eq!(Type::I16.scalar_bytes(), Some(2));
        assert_eq!(Type::F32.scalar_bytes(), Some(4));
        assert_eq!(Type::F64.scalar_bytes(), Some(8));
        assert_eq!(Type::Str.scalar_bytes(), None);
        assert_eq!(Type::Vec(Box::new(Type::U8)).scalar_bytes(), None);
        assert_eq!(Type::Struct("T".into()).scalar_bytes(), None);
    }

    #[test]
    fn domains_match_the_rust_types() {
        assert_eq!(Type::I8.domain_min(), Some(-128));
        assert_eq!(Type::I8.domain_max(), Some(127));
        assert_eq!(Type::U8.domain_max(), Some(255));
        assert_eq!(Type::I16.domain_min(), Some(-32768));
        assert_eq!(Type::U64.domain_max(), Some(u64::MAX as i128));
        assert_eq!(Type::I64.domain_min(), Some(i64::MIN as i128));
        assert_eq!(Type::Bool.domain_max(), Some(1));
        assert_eq!(Type::F32.domain_max(), None);
        assert_eq!(Type::Str.domain_min(), None);
    }

    #[test]
    fn classification_is_consistent() {
        assert!(Type::I32.is_scalar() && Type::I32.is_signed() && Type::I32.is_integral());
        assert!(!Type::I32.is_float());
        assert!(Type::U64.is_integral() && !Type::U64.is_signed());
        assert!(Type::Bool.is_integral());
        assert!(Type::F64.is_float() && Type::F64.is_scalar() && !Type::F64.is_integral());
        assert!(!Type::Str.is_scalar());
        assert!(!Type::Struct("T".into()).is_scalar());
    }

    #[test]
    fn attribute_accessors_see_only_their_own_kind() {
        let f = FieldSpec::new("rsrp", Type::I16)
            .with(Attr::Range {
                min: -140,
                max: -44,
            })
            .with(Attr::Bits(16));
        assert_eq!(f.range(), Some((-140, -44)));
        assert_eq!(f.bits(), Some(16));
        let plain = FieldSpec::new("band", Type::U8);
        assert_eq!(plain.range(), None);
        assert_eq!(plain.bits(), None);
        assert_eq!(plain.attrs.len(), 0);
    }
}

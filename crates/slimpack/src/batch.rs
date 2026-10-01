//! Phase 2: Batch mode scaffolding.
//!
//! This module starts the batch/columnar implementation described in
//! `spec/SPEC.md` §3. The framing here is intentionally small and explicit:
//! it handles the record/column counts plus the column header, so later phases
//! can add per-column encodings without changing the outer wire format.

use crate::error::{Result, SlimError};
use crate::varint::{read_varint, write_varint};

/// Encoding ids for batch-mode columns described in `spec/SPEC.md` §3.2.
pub mod encoding {
    /// Raw `u8` values.
    pub const RAW_U8: u8 = 0x10;
    /// Raw `u16` values.
    pub const RAW_U16: u8 = 0x11;
    /// Raw `u32` values.
    pub const RAW_U32: u8 = 0x12;
    /// Raw `u64` values.
    pub const RAW_U64: u8 = 0x13;
    /// Raw signed `i8` values.
    pub const RAW_I8: u8 = 0x14;
    /// Raw signed `i16` values.
    pub const RAW_I16: u8 = 0x15;
    /// Raw signed `i32` values.
    pub const RAW_I32: u8 = 0x16;
    /// Raw signed `i64` values.
    pub const RAW_I64: u8 = 0x17;
    /// Bit-packed range encoding.
    pub const FOR: u8 = 0x20;
    /// Delta varint encoding.
    pub const DELTA_VARINT: u8 = 0x21;
    /// Second-difference varint encoding.
    pub const DELTA2_VARINT: u8 = 0x22;
    /// Run-length encoding.
    pub const RLE: u8 = 0x23;
    /// Constant encoding.
    pub const CONSTANT: u8 = 0x24;
    /// Dictionary-packed encoding.
    pub const DICT_PACK: u8 = 0x25;
    /// Raw `f32` values.
    pub const RAW_F32: u8 = 0x30;
    /// Raw `f64` values.
    pub const RAW_F64: u8 = 0x31;
    /// XOR-previous float encoding.
    pub const XOR_PREV: u8 = 0x32;
    /// Quantized float encoding.
    pub const QUANT: u8 = 0x33;
    /// Raw byte-string column.
    pub const BYTES: u8 = 0x40;
    /// Dictionary string column.
    pub const DICT: u8 = 0x41;
    /// Fixed-width string column.
    pub const FIXED: u8 = 0x42;
    /// Bit-packed bool column.
    pub const BITPACK: u8 = 0x50;
}

/// Batch-mode column flags.
pub mod flags {
    /// Presence bitmap follows the column header.
    pub const HAS_PRESENCE: u8 = 0x01;
    /// The column uses a lossy transform.
    pub const LOSSY: u8 = 0x02;
    /// All higher bits are reserved and must be zero.
    pub const RESERVED: u8 = 0xFC;
}

/// Top-level batch framing: `varint record_count`, then `varint column_count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchFrame {
    /// Number of records in the batch.
    pub record_count: u64,
    /// Number of columns in the batch.
    pub column_count: u64,
}

impl BatchFrame {
    /// Builds a batch frame for a payload with `record_count` rows and
    /// `column_count` columns.
    pub fn new(record_count: u64, column_count: u64) -> Self {
        Self {
            record_count,
            column_count,
        }
    }

    /// Encodes the batch framing into its canonical layout.
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::new();
        write_varint(&mut out, self.record_count);
        write_varint(&mut out, self.column_count);
        out
    }

    /// Decodes a batch framing from `input`.
    pub fn decode(input: &[u8]) -> Result<Self> {
        let mut pos = 0;
        let record_count = read_varint(input, &mut pos)?;
        let column_count = read_varint(input, &mut pos)?;
        if pos != input.len() {
            return Err(SlimError::TrailingBytes {
                extra: input.len() - pos,
            });
        }
        Ok(Self {
            record_count,
            column_count,
        })
    }
}

/// Column header: `encoding_id`, `flags`, `varint byte_len`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnHeader {
    /// Column encoding id.
    pub encoding_id: u8,
    /// Column flags.
    pub flags: u8,
    /// Total bytes in the column payload, including the optional presence bitmap.
    pub byte_len: u64,
}

impl ColumnHeader {
    /// Builds a column header while validating the reserved bits.
    pub fn new(encoding_id: u8, flags: u8, byte_len: u64) -> Result<Self> {
        let reserved = flags & flags::RESERVED;
        if reserved != 0 {
            return Err(SlimError::ReservedBitsSet { mask: reserved });
        }
        Ok(Self {
            encoding_id,
            flags,
            byte_len,
        })
    }

    /// Encodes the column header into its wire format.
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.encoding_id);
        out.push(self.flags);
        write_varint(&mut out, self.byte_len);
        out
    }

    /// Decodes one column header from the start of `input`.
    pub fn decode(input: &[u8]) -> Result<(Self, &[u8])> {
        if input.len() < 2 {
            return Err(SlimError::truncated(2, input.len()));
        }

        let encoding_id = input[0];
        let flags = input[1];
        let reserved = flags & flags::RESERVED;
        if reserved != 0 {
            return Err(SlimError::ReservedBitsSet { mask: reserved });
        }

        let mut pos = 2;
        let byte_len = read_varint(input, &mut pos)?;
        let header = Self {
            encoding_id,
            flags,
            byte_len,
        };
        Ok((header, &input[pos..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_frame_has_record_and_column_counts() {
        let bytes = BatchFrame::new(13, 2).encode();
        let frame = BatchFrame::decode(&bytes).unwrap();
        assert_eq!(frame.record_count, 13);
        assert_eq!(frame.column_count, 2);
    }

    #[test]
    fn column_header_validates_reserved_bits() {
        let err = ColumnHeader::new(0x10, 0xF8, 9).unwrap_err();
        assert!(matches!(err, crate::SlimError::ReservedBitsSet { .. }));
    }

    #[test]
    fn column_header_round_trips() {
        let header = ColumnHeader::new(encoding::RAW_I32, flags::HAS_PRESENCE, 19).unwrap();
        let bytes = header.encode();
        let (decoded, rest) = ColumnHeader::decode(&bytes).unwrap();
        assert_eq!(decoded, header);
        assert!(rest.is_empty());
    }
}

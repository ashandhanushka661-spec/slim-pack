//! LEB128 varints and zig-zag mapping.
//!
//! Varints are unsigned LEB128 (7 payload bits per byte, little-endian
//! groups, high bit = continuation). Encoding is canonical: a value is written
//! with the minimum number of bytes, and decoding rejects non-canonical
//! (overlong) encodings, so one value has exactly one byte sequence. That
//! keeps the golden vectors unambiguous.
//!
//! Every read takes a `&mut usize` cursor and returns
//! [`SlimError::Truncated`] rather than panicking on a short input.

use crate::error::{Result, SlimError};

/// Appends `value` to `out` as a canonical LEB128 varint.
pub fn write_varint(out: &mut Vec<u8>, value: u64) {
    let mut rest = value;
    loop {
        let byte = (rest & 0x7F) as u8;
        rest >>= 7;
        if rest == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Returns how many bytes `value` needs when varint encoded.
pub fn varint_len(value: u64) -> usize {
    let bits = 64 - value.leading_zeros() as usize;
    bits.div_ceil(7).max(1)
}

/// Reads a canonical LEB128 varint at `*pos`, advancing the cursor.
///
/// Errors:
/// * [`SlimError::Truncated`] if the input ends mid-varint, reported as
///   `needed` = cursor position after the bytes consumed (i.e. the length the
///   varint must reach) and `available` = `input.len()`,
/// * [`SlimError::SizeOverflow`] if a 10th byte carries bits beyond `u64`,
/// * [`SlimError::PaddingBitsSet`] if the encoding is overlong (a redundant
///   trailing zero group), which would give one value two byte sequences.
pub fn read_varint(input: &[u8], pos: &mut usize) -> Result<u64> {
    let start = *pos;
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *input
            .get(*pos)
            .ok_or(SlimError::truncated(*pos + 1, input.len()))?;
        *pos += 1;
        let payload = (byte & 0x7F) as u64;
        if shift == 63 && payload > 1 {
            // The 10th byte may only contribute bit 63.
            return Err(SlimError::SizeOverflow);
        }
        value |= payload << shift;
        if byte & 0x80 == 0 {
            if byte == 0 && *pos - start > 1 {
                // Overlong: the last byte is a redundant zero group.
                return Err(SlimError::PaddingBitsSet { mask: 0 });
            }
            return Ok(value);
        }
        shift += 7;
        if shift > 63 {
            return Err(SlimError::SizeOverflow);
        }
    }
}

/// Maps a signed integer to an unsigned one, small magnitudes first.
///
/// Uses wrapping shifts so that `i64::MIN` and `i64::MAX` are handled without
/// overflow.
pub fn zigzag_encode(value: i64) -> u64 {
    ((value as u64) << 1) ^ ((value >> 63) as u64)
}

/// Inverse of [`zigzag_encode`], with wrapping arithmetic.
pub fn zigzag_decode(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: u64) {
        let mut buf = Vec::new();
        write_varint(&mut buf, value);
        assert_eq!(buf.len(), varint_len(value), "length for {value}");
        let mut pos = 0;
        assert_eq!(read_varint(&buf, &mut pos).unwrap(), value);
        assert_eq!(pos, buf.len(), "cursor must land after the varint");
        // Re-encoding the decoded value reproduces the same bytes.
        let mut again = Vec::new();
        write_varint(&mut again, value);
        assert_eq!(again, buf);
    }

    #[test]
    fn canonical_round_trips() {
        for v in [
            0,
            1,
            2,
            127,
            128,
            129,
            255,
            256,
            16_383,
            16_384,
            u32::MAX as u64,
        ] {
            round_trip(v);
        }
        round_trip(u64::MAX);
        round_trip(i64::MAX as u64);
    }

    #[test]
    fn known_encodings() {
        let max_bytes: Vec<u8> = vec![0xFF; 9].into_iter().chain([0x01]).collect();
        let cases: [(u64, &[u8]); 6] = [
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7F]),
            (128, &[0x80, 0x01]),
            (300, &[0xAC, 0x02]),
            (u64::MAX, &max_bytes),
        ];
        for (value, expect) in cases {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            assert_eq!(buf, expect, "encoding of {value}");
        }
    }

    #[test]
    fn truncated_input_is_an_error_not_a_panic() {
        let buf = [0x80u8, 0x80];
        let mut pos = 0;
        match read_varint(&buf, &mut pos) {
            Err(SlimError::Truncated { needed, available }) => {
                assert_eq!(
                    needed, 3,
                    "two continuation bytes consumed, a third is needed"
                );
                assert_eq!(available, 2);
            }
            other => panic!("expected Truncated, got {other:?}"),
        }
        let mut pos = 0;
        assert!(matches!(
            read_varint(&[], &mut pos),
            Err(SlimError::Truncated { .. })
        ));
    }

    #[test]
    fn overlong_encoding_is_rejected() {
        // 0x80 0x00 encodes 0 in two bytes: not canonical.
        let buf = [0x80u8, 0x00];
        let mut pos = 0;
        assert!(matches!(
            read_varint(&buf, &mut pos),
            Err(SlimError::PaddingBitsSet { .. })
        ));
    }

    #[test]
    fn overflowing_encoding_is_rejected() {
        // 11 continuation bytes: more than u64 can hold.
        let buf = [0x80u8; 11];
        let mut pos = 0;
        assert!(matches!(
            read_varint(&buf, &mut pos),
            Err(SlimError::SizeOverflow)
        ));
        // The 10th byte may only carry bit 63, i.e. payload 0 or 1.
        let mut buf = [0xFFu8; 9].to_vec();
        buf.push(0x02);
        let mut pos = 0;
        assert!(matches!(
            read_varint(&buf, &mut pos),
            Err(SlimError::SizeOverflow)
        ));
        // 9 groups of 0x7F plus payload 1 in the 10th byte is u64::MAX.
        buf[9] = 0x01;
        let mut pos = 0;
        assert_eq!(read_varint(&buf, &mut pos).unwrap(), u64::MAX);
    }

    #[test]
    fn zigzag_round_trips_at_the_extremes() {
        for v in [
            0i64,
            -1,
            1,
            -2,
            2,
            i64::MAX,
            i64::MIN,
            i64::MIN + 1,
            i64::MAX - 1,
        ] {
            assert_eq!(zigzag_decode(zigzag_encode(v)), v, "zigzag of {v}");
        }
        // Small magnitudes get small codes, so deltas stay 1 byte.
        assert_eq!(zigzag_encode(-1), 1);
        assert_eq!(zigzag_encode(1), 2);
        assert_eq!(zigzag_encode(-64), 127);
        assert_eq!(zigzag_encode(64), 128);
    }

    #[test]
    fn zigzag_codes_are_exactly_ten_bytes_at_the_extremes() {
        let mut buf = Vec::new();
        write_varint(&mut buf, zigzag_encode(i64::MIN));
        assert_eq!(buf.len(), 10);
        let mut pos = 0;
        assert_eq!(
            zigzag_decode(read_varint(&buf, &mut pos).unwrap()),
            i64::MIN
        );
    }

    #[test]
    fn cursor_is_not_moved_on_error() {
        let buf = [0xFFu8, 0x02];
        let mut pos = 5;
        assert!(read_varint(&buf, &mut pos).is_err());
        // Position 5 is past the end, so the error is Truncated, not SizeOverflow.
        assert!(matches!(
            read_varint(&buf, &mut pos),
            Err(SlimError::Truncated { .. })
        ));
    }
}

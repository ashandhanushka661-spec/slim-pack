//! The 8-byte file header and the extension-section framing.
//!
//! ```text
//! 0        4        5        6        7        8
//! +--------+--------+--------+--------+--------+
//! | "SLMP" | ver    | mode   | flags  | reserved|  reserved = 0x00
//! +--------+--------+--------+--------+--------+
//! ```
//!
//! * `ver` is `1` for this revision; a reader of a different version must fail
//!   rather than guess.
//! * `mode` is [`mode::FIXED`] (1), [`mode::BATCH`] (2) or
//!   [`mode::DYNAMIC`] (3).
//! * `flags` bit 0 is LOSSY, bit 1 is EXT, bits 2..7 are reserved and must be
//!   zero. A reader that finds a reserved bit set must reject the payload,
//!   because it cannot know what the writer meant.
//! * `reserved` must be zero, so that a future minor revision can define it.

use crate::error::{Result, SlimError};

/// File magic: `SLMP`.
pub const MAGIC: [u8; 4] = *b"SLMP";
/// Format version implemented by this crate.
pub const VERSION: u8 = 1;
/// Fixed size of the header in bytes.
pub const HEADER_LEN: usize = 8;

/// Payload modes.
pub mod mode {
    /// Fixed-layout records.
    pub const FIXED: u8 = 1;
    /// Column-oriented batch.
    pub const BATCH: u8 = 2;
    /// Self-describing dynamic values.
    pub const DYNAMIC: u8 = 3;

    /// Human-readable name of a mode byte, or `None` if unknown.
    pub fn name(value: u8) -> Option<&'static str> {
        match value {
            FIXED => Some("fixed"),
            BATCH => Some("batch"),
            DYNAMIC => Some("dynamic"),
            _ => None,
        }
    }
}

/// Header flag bits.
pub mod flags {
    /// The payload uses a declared lossy transform.
    pub const LOSSY: u8 = 0x01;
    /// At least one extension section follows the body.
    pub const EXT: u8 = 0x02;
    /// All flag bits that are reserved and must be zero.
    pub const RESERVED: u8 = 0xFC;
}

/// Kind bytes for extension sections.
pub mod section {
    /// `keys` section: string dictionary.
    pub const KEYS: u8 = 1;
    /// `layout` section: field layout, for `Dynamic` mode.
    pub const LAYOUT: u8 = 2;
    /// `fields` section: field names, for `Batch` mode.
    pub const FIELDS: u8 = 3;
}

/// The parsed 8-byte header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Format version; always [`VERSION`] for values this crate builds.
    pub version: u8,
    /// Payload mode, one of [`mode`].
    pub mode: u8,
    /// Flag bits, from [`flags`].
    pub flags: u8,
}

impl Header {
    /// Builds a header for `mode` with `flags`, validating both.
    pub fn new(mode: u8, flags: u8) -> Result<Self> {
        Self::validate_mode(mode)?;
        Self::validate_flags(flags)?;
        Ok(Self {
            version: VERSION,
            mode,
            flags,
        })
    }

    /// Rejects modes this version does not define.
    pub fn validate_mode(mode: u8) -> Result<()> {
        if mode::name(mode).is_none() {
            return Err(SlimError::UnsupportedMode(mode));
        }
        Ok(())
    }

    /// Rejects reserved flag bits and duplicates.
    pub fn validate_flags(flags: u8) -> Result<()> {
        let reserved = flags & flags::RESERVED;
        if reserved != 0 {
            return Err(SlimError::ReservedBitsSet { mask: reserved });
        }
        Ok(())
    }

    /// True if the LOSSY flag is set.
    pub fn is_lossy(self) -> bool {
        self.flags & flags::LOSSY != 0
    }

    /// True if the EXT flag is set.
    pub fn has_ext(self) -> bool {
        self.flags & flags::EXT != 0
    }

    /// Writes the header bytes to `out`.
    pub fn write(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        out.push(self.version);
        out.push(self.mode);
        out.push(self.flags);
        out.push(0x00);
    }

    /// Returns the 8 header bytes.
    pub fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut buf = Vec::with_capacity(HEADER_LEN);
        self.write(&mut buf);
        let bytes: [u8; HEADER_LEN] = buf[..HEADER_LEN]
            .try_into()
            .expect("write emits exactly HEADER_LEN bytes");
        bytes
    }

    /// Reads a header from the start of `input`, returning it and the rest.
    ///
    /// Errors: [`SlimError::InvalidMagic`], [`SlimError::Truncated`],
    /// [`SlimError::UnsupportedVersion`], [`SlimError::UnsupportedMode`],
    /// [`SlimError::ReservedBitsSet`].
    pub fn read(input: &[u8]) -> Result<(Self, &[u8])> {
        if input.len() < HEADER_LEN {
            return Err(SlimError::truncated(HEADER_LEN, input.len()));
        }
        let magic: [u8; 4] = input[..4].try_into().expect("checked length");
        if magic != MAGIC {
            return Err(SlimError::InvalidMagic(magic));
        }
        let version = input[4];
        let mode = input[5];
        let flags = input[6];
        let reserved = input[7];
        if version != VERSION {
            return Err(SlimError::UnsupportedVersion(version));
        }
        Self::validate_mode(mode)?;
        Self::validate_flags(flags)?;
        if reserved != 0 {
            return Err(SlimError::ReservedBitsSet { mask: reserved });
        }
        Ok((
            Self {
                version,
                mode,
                flags,
            },
            &input[HEADER_LEN..],
        ))
    }
}

/// Writes an extension section: `kind`, a varint payload length, the payload,
/// and the padding needed to reach a 4-byte boundary.
///
/// Sections are ordered so that `keys` precedes `layout`, which precedes
/// `fields`, and each kind appears at most once.
pub fn write_section(out: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    out.push(kind);
    crate::varint::write_varint(out, payload.len() as u64);
    out.extend_from_slice(payload);
    while (out.len() - HEADER_LEN) % 4 != 0 {
        out.push(0x00);
    }
}

/// A decoded extension section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section<'a> {
    /// Section kind, from [`section`].
    pub kind: u8,
    /// Section payload.
    pub payload: &'a [u8],
}

/// Reads one extension section from the start of `input`.
///
/// `input` must be the bytes that follow the header. Returns the section and
/// the remaining bytes, skipping the alignment padding.
pub fn read_section(input: &[u8]) -> Result<(Section<'_>, &[u8])> {
    let (&kind, rest) = input.split_first().ok_or(SlimError::truncated(1, 0))?;
    let mut pos = 0;
    let declared = crate::varint::read_varint(rest, &mut pos)?;
    let declared = usize::try_from(declared).map_err(|_| SlimError::SizeOverflow)?;
    let end = pos.checked_add(declared).ok_or(SlimError::SizeOverflow)?;
    let body = rest
        .get(pos..end)
        .ok_or_else(|| SlimError::truncated(end, rest.len()))?;
    // Padding aligns each section relative to the start of the body, i.e. the
    // end of the header, which is where write_section measured it from. The
    // 1 is the kind byte, which `pos` does not count.
    let mut consumed = 1 + end;
    while consumed % 4 != 0 {
        consumed += 1;
    }
    if consumed > input.len() {
        return Err(SlimError::truncated(consumed, input.len()));
    }
    Ok((
        Section {
            kind,
            payload: body,
        },
        &input[consumed..],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_header_bytes_round_trip() {
        let h = Header::new(mode::FIXED, 0x00).unwrap();
        assert_eq!(h.to_bytes(), [b'S', b'L', b'M', b'P', 1, 1, 0, 0]);
        let bytes = h.to_bytes();
        let (back, rest) = Header::read(&bytes).unwrap();
        assert_eq!(back, h);
        assert!(rest.is_empty());
    }

    #[test]
    fn all_three_modes_are_accepted() {
        for (byte, name) in [
            (mode::FIXED, "fixed"),
            (mode::BATCH, "batch"),
            (mode::DYNAMIC, "dynamic"),
        ] {
            let h = Header::new(byte, 0x00).unwrap();
            assert_eq!(mode::name(h.mode), Some(name));
            let bytes = h.to_bytes();
            assert_eq!(Header::read(&bytes).unwrap().0.mode, byte);
        }
        assert_eq!(mode::name(0), None);
        assert_eq!(mode::name(4), None);
    }

    #[test]
    fn flags_are_read_and_validated() {
        let h = Header::new(mode::BATCH, flags::LOSSY | flags::EXT).unwrap();
        assert!(h.is_lossy());
        assert!(h.has_ext());
        assert_eq!(h.to_bytes()[6], 0x03);
    }

    #[test]
    fn reserved_flag_bits_are_rejected_on_write_and_read() {
        assert!(matches!(
            Header::new(mode::FIXED, 0x04),
            Err(SlimError::ReservedBitsSet { mask: 0x04 })
        ));
        let mut bytes = Header::new(mode::FIXED, 0x00).unwrap().to_bytes();
        bytes[6] = 0b1010_0000;
        assert!(matches!(
            Header::read(&bytes),
            Err(SlimError::ReservedBitsSet { mask: 0xA0 })
        ));
    }

    #[test]
    fn nonzero_reserved_byte_is_rejected() {
        let mut bytes = Header::new(mode::DYNAMIC, 0x00).unwrap().to_bytes();
        bytes[7] = 0x01;
        assert!(matches!(
            Header::read(&bytes),
            Err(SlimError::ReservedBitsSet { mask: 0x01 })
        ));
    }

    #[test]
    fn wrong_magic_is_reported() {
        let mut bytes = Header::new(mode::FIXED, 0x00).unwrap().to_bytes();
        bytes[0] = b'X';
        assert!(matches!(
            Header::read(&bytes),
            Err(SlimError::InvalidMagic(m)) if m[0] == b'X'
        ));
    }

    #[test]
    fn unknown_version_is_rejected_without_reading_further() {
        let mut bytes = Header::new(mode::FIXED, 0x00).unwrap().to_bytes();
        bytes[4] = 2;
        assert!(matches!(
            Header::read(&bytes),
            Err(SlimError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let mut bytes = Header::new(mode::FIXED, 0x00).unwrap().to_bytes();
        bytes[5] = 0x09;
        assert!(matches!(
            Header::read(&bytes),
            Err(SlimError::UnsupportedMode(9))
        ));
        assert!(matches!(
            Header::new(0x09, 0),
            Err(SlimError::UnsupportedMode(9))
        ));
    }

    #[test]
    fn short_input_is_truncation_not_a_panic() {
        for len in 0..HEADER_LEN {
            assert!(matches!(
                Header::read(&[0u8; 8][..len]),
                Err(SlimError::Truncated { needed: 8, available }) if available == len
            ));
        }
    }

    #[test]
    fn read_leaves_the_rest_of_the_payload_untouched() {
        let mut buf = Vec::new();
        Header::new(mode::DYNAMIC, flags::EXT)
            .unwrap()
            .write(&mut buf);
        buf.extend_from_slice(&[0xAA, 0xBB]);
        let (h, rest) = Header::read(&buf).unwrap();
        assert!(h.has_ext());
        assert_eq!(rest, &[0xAA, 0xBB]);
    }

    #[test]
    fn sections_round_trip_with_alignment_padding() {
        for len in 0..12usize {
            let payload: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_add(1)).collect();
            let mut buf = Vec::new();
            Header::new(mode::DYNAMIC, flags::EXT)
                .unwrap()
                .write(&mut buf);
            write_section(&mut buf, section::KEYS, &payload);
            let (h, rest) = Header::read(&buf).unwrap();
            assert!(h.has_ext());
            let (s, rest2) = read_section(rest).unwrap();
            assert_eq!(s.kind, section::KEYS);
            assert_eq!(s.payload, payload.as_slice());
            assert!(
                rest2.is_empty(),
                "payload of {len} bytes left {} bytes",
                rest2.len()
            );
            assert_eq!(buf.len() % 4, 0, "buffer must stay 4-byte aligned");
        }
    }

    #[test]
    fn several_sections_are_read_in_order() {
        let mut buf = Vec::new();
        Header::new(mode::DYNAMIC, flags::EXT)
            .unwrap()
            .write(&mut buf);
        write_section(&mut buf, section::KEYS, b"rsrp");
        write_section(&mut buf, section::LAYOUT, &[1, 2, 3]);
        let mut rest = &buf[HEADER_LEN..];
        let mut kinds = Vec::new();
        while !rest.is_empty() {
            let (s, next) = read_section(rest).unwrap();
            kinds.push(s.kind);
            rest = next;
        }
        assert_eq!(kinds, vec![section::KEYS, section::LAYOUT]);
    }

    #[test]
    fn truncated_section_is_rejected() {
        let mut buf = Vec::new();
        Header::new(mode::DYNAMIC, flags::EXT)
            .unwrap()
            .write(&mut buf);
        write_section(&mut buf, section::KEYS, b"abcdef");
        let full = &buf[HEADER_LEN..];
        for cut in 1..full.len() {
            assert!(
                read_section(&full[..cut]).is_err(),
                "cut at {cut} must not decode"
            );
        }
        assert!(read_section(&[]).is_err());
    }

    #[test]
    fn huge_declared_section_length_is_rejected() {
        let mut buf = Vec::new();
        buf.push(section::KEYS);
        crate::varint::write_varint(&mut buf, u64::MAX);
        buf.extend_from_slice(b"tiny");
        assert!(matches!(
            read_section(&buf),
            Err(SlimError::SizeOverflow | SlimError::Truncated { .. })
        ));
    }
}

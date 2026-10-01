//! Error type for every SlimPack operation.
//!
//! Rules (see `spec/SPEC.md` §9):
//!
//! * Unknown encoding ids, reserved flag bits set and truncated data return a
//!   [`SlimError`]. They never panic and never hang.
//! * No `SlimError` is constructed after an allocation has been made on the
//!   strength of an untrusted length; limits are checked first.
//! * Error variants are stable: adding one is a minor change, changing the
//!   meaning of one is a format version bump.

use std::fmt;

/// Which configured limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Limit {
    /// Total payload size (`Limits::max_bytes`).
    Bytes,
    /// Record or value count (`Limits::max_records`).
    Records,
    /// Nesting depth (`Limits::max_depth`).
    Depth,
    /// String/bytes length or run length (`Limits::max_len`).
    Len,
    /// Dictionary entries (`Limits::max_dictionary`).
    Dictionary,
    /// Batch column count (`Limits::max_columns`).
    Columns,
}

impl Limit {
    /// Name used in error messages.
    pub fn name(self) -> &'static str {
        match self {
            Limit::Bytes => "max_bytes",
            Limit::Records => "max_records",
            Limit::Depth => "max_depth",
            Limit::Len => "max_len",
            Limit::Dictionary => "max_dictionary",
            Limit::Columns => "max_columns",
        }
    }
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Everything that can go wrong while encoding or decoding SlimPack data.
#[derive(Debug, Clone, PartialEq)]
pub enum SlimError {
    /// The input ended before a declared length was satisfied.
    Truncated {
        /// Bytes the decoder needed.
        needed: usize,
        /// Bytes actually available.
        available: usize,
    },
    /// A declared count or length exceeds the configured limit.
    LimitExceeded {
        /// Which limit was hit.
        limit: Limit,
        /// The offending value from the payload.
        value: u64,
        /// The configured maximum.
        max: u64,
    },
    /// Reserved flag bits were set; a reader must never guess.
    ReservedBitsSet {
        /// The reserved bits that were set.
        mask: u8,
    },
    /// An `encoding_id` or `section_kind` this version does not define.
    UnknownEncoding(u8),
    /// A value violates a `range = A..=B` or `bits = N` attribute.
    ValueOutOfRange {
        /// The offending value.
        value: i128,
        /// Smallest legal value.
        min: i128,
        /// Largest legal value.
        max: i128,
    },
    /// A map contained the same key twice.
    DuplicateKey(String),
    /// A string payload was not valid UTF-8.
    InvalidUtf8,
    /// The stored record stride is smaller than the reader's known stride.
    MissingField,
    /// A non-finite float was requested for JSON output.
    NonFiniteFloat,
    /// A JSON integer did not fit in `u64`.
    NumberOutOfRange,
    /// The payload's version byte is not this reader's version.
    UnsupportedVersion(u8),
    /// The mode byte is not one of Fixed/Batch/Dynamic.
    UnsupportedMode(u8),
    /// The payload's magic is not `SLMP`.
    InvalidMagic([u8; 4]),
    /// The requested combination of type and layout does not exist.
    UnsupportedLayout(&'static str),
    /// A length or offset computation overflowed `usize`.
    SizeOverflow,
    /// `decode` was given trailing bytes after a complete payload.
    TrailingBytes {
        /// Number of unexpected trailing bytes.
        extra: usize,
    },
    /// A `bool` byte was neither `0x00` nor `0x01`.
    InvalidBool(u8),
    /// A bit that must be zero was set (e.g. unused high bits of the last byte
    /// of a packed record).
    PaddingBitsSet {
        /// The offending bits.
        mask: u8,
    },
}

impl SlimError {
    /// Convenience constructor for [`SlimError::Truncated`].
    pub fn truncated(needed: usize, available: usize) -> Self {
        SlimError::Truncated { needed, available }
    }

    /// Convenience constructor for [`SlimError::LimitExceeded`].
    pub fn limit(limit: Limit, value: u64, max: u64) -> Self {
        SlimError::LimitExceeded { limit, value, max }
    }
}

impl fmt::Display for SlimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlimError::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated input: needed {needed} bytes, {available} available"
                )
            }
            SlimError::LimitExceeded { limit, value, max } => {
                write!(f, "limit {limit} exceeded: {value} > {max}")
            }
            SlimError::ReservedBitsSet { mask } => {
                write!(f, "reserved bits must be zero, got mask 0x{mask:02x}")
            }
            SlimError::UnknownEncoding(id) => write!(f, "unknown encoding id 0x{id:02x}"),
            SlimError::ValueOutOfRange { value, min, max } => {
                write!(f, "value {value} outside allowed range {min}..={max}")
            }
            SlimError::DuplicateKey(key) => write!(f, "duplicate map key {key:?}"),
            SlimError::InvalidUtf8 => f.write_str("string payload is not valid UTF-8"),
            SlimError::MissingField => f.write_str(
                "payload is missing fields the reader requires (stored stride too small)",
            ),
            SlimError::NonFiniteFloat => {
                f.write_str("NaN and infinity cannot be represented in JSON")
            }
            SlimError::NumberOutOfRange => f.write_str("JSON number outside the u64 range"),
            SlimError::UnsupportedVersion(v) => write!(f, "unsupported format version {v}"),
            SlimError::UnsupportedMode(m) => write!(f, "unsupported mode 0x{m:02x}"),
            SlimError::InvalidMagic(m) => write!(f, "invalid magic {m:?}, expected \"SLMP\""),
            SlimError::UnsupportedLayout(what) => write!(f, "unsupported layout: {what}"),
            SlimError::SizeOverflow => f.write_str("size computation overflowed"),
            SlimError::TrailingBytes { extra } => {
                write!(f, "{extra} trailing byte(s) after the payload")
            }
            SlimError::InvalidBool(byte) => {
                write!(f, "invalid bool byte 0x{byte:02x}, expected 0x00 or 0x01")
            }
            SlimError::PaddingBitsSet { mask } => {
                write!(f, "padding bits must be zero, got mask 0x{mask:02x}")
            }
        }
    }
}

impl std::error::Error for SlimError {}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, SlimError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_displays_a_non_empty_message() {
        let all = [
            SlimError::truncated(10, 4),
            SlimError::limit(Limit::Len, 99, 8),
            SlimError::ReservedBitsSet { mask: 0b1100_0000 },
            SlimError::UnknownEncoding(0xEE),
            SlimError::ValueOutOfRange {
                value: -200,
                min: -140,
                max: -44,
            },
            SlimError::DuplicateKey("rsrp".to_owned()),
            SlimError::InvalidUtf8,
            SlimError::MissingField,
            SlimError::NonFiniteFloat,
            SlimError::NumberOutOfRange,
            SlimError::UnsupportedVersion(9),
            SlimError::UnsupportedMode(0x7F),
            SlimError::InvalidMagic(*b"NOPE"),
            SlimError::UnsupportedLayout("packed str"),
            SlimError::SizeOverflow,
            SlimError::TrailingBytes { extra: 3 },
            SlimError::InvalidBool(0x7F),
            SlimError::PaddingBitsSet { mask: 0x80 },
        ];
        for err in &all {
            let text = err.to_string();
            assert!(text.len() > 4, "{err:?} has an unhelpful message");
            assert!(!text.contains('{'), "{err:?} message has a format hole");
        }
        assert_eq!(all.len(), 18, "a new variant must be added to this test");
    }

    #[test]
    fn limit_names_match_the_spec_table() {
        assert_eq!(Limit::Bytes.name(), "max_bytes");
        assert_eq!(Limit::Records.name(), "max_records");
        assert_eq!(Limit::Depth.name(), "max_depth");
        assert_eq!(Limit::Len.name(), "max_len");
        assert_eq!(Limit::Dictionary.name(), "max_dictionary");
        assert_eq!(Limit::Columns.name(), "max_columns");
    }

    #[test]
    fn errors_are_comparable_and_clonable() {
        let err = SlimError::limit(Limit::Records, 11, 10);
        assert_eq!(err, err.clone());
        assert_ne!(err, SlimError::limit(Limit::Records, 11, 9));
    }

    #[test]
    fn error_can_be_used_as_a_std_error() {
        let boxed: Box<dyn std::error::Error> = Box::new(SlimError::InvalidUtf8);
        assert_eq!(boxed.to_string(), "string payload is not valid UTF-8");
    }
}

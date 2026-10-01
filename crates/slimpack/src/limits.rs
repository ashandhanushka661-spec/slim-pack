//! Configurable limits and the guards that every length/count must pass.
//!
//! `spec/SPEC.md` §9: no length, count or size from a payload is ever used to
//! allocate before it has been checked against **both** the remaining input
//! bytes and the configured limits. [`Limits::check_len`] is the single place
//! where that happens; the check order (limits first, then remaining bytes) is
//! part of the contract so that a declared length of `u64::MAX` is rejected
//! without any 64-bit-to-`usize` narrowing and without allocating.

use crate::error::{Limit, Result, SlimError};

/// Maximum accepted size of any single operation's output.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum accepted record or value count.
pub const DEFAULT_MAX_RECORDS: u64 = 10_000_000;
/// Maximum nesting depth for arrays, maps and nested structs.
pub const DEFAULT_MAX_DEPTH: usize = 64;
/// Maximum length of a string or bytes payload, and of a single run.
pub const DEFAULT_MAX_LEN: u64 = 16 * 1024 * 1024;
/// Maximum number of entries in a string dictionary.
pub const DEFAULT_MAX_DICTIONARY: u64 = 1_000_000;
/// Maximum number of columns in a Batch payload.
pub const DEFAULT_MAX_COLUMNS: u64 = 4096;

/// Limits for one decode operation.
///
/// Every field is public so a caller can tighten or relax any of them; the
/// defaults are the values in `spec/SPEC.md` §9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Total payload size accepted.
    pub max_bytes: u64,
    /// Record or value count accepted.
    pub max_records: u64,
    /// Nesting depth accepted.
    pub max_depth: usize,
    /// String/bytes length and run length accepted.
    pub max_len: u64,
    /// Dictionary entries accepted.
    pub max_dictionary: u64,
    /// Batch column count accepted.
    pub max_columns: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_records: DEFAULT_MAX_RECORDS,
            max_depth: DEFAULT_MAX_DEPTH,
            max_len: DEFAULT_MAX_LEN,
            max_dictionary: DEFAULT_MAX_DICTIONARY,
            max_columns: DEFAULT_MAX_COLUMNS,
        }
    }
}

impl Limits {
    /// Tight limits, handy in tests and for untrusted input.
    pub fn strict() -> Self {
        Self {
            max_bytes: 1024,
            max_records: 16,
            max_depth: 4,
            max_len: 32,
            max_dictionary: 8,
            max_columns: 4,
        }
    }

    /// Validates a declared count of items (`record_count`, `value_count`,
    /// `run count`, `element count`).
    pub fn check_count(&self, value: u64, limit: Limit, max: u64) -> Result<usize> {
        if value > max {
            return Err(SlimError::limit(limit, value, max));
        }
        usize::try_from(value).map_err(|_| SlimError::limit(limit, value, max))
    }

    /// Validates a declared byte length against [`Limit::Len`] and the bytes
    /// that are actually left in the input.
    ///
    /// This is the guard that must run *before* any allocation sized by
    /// `declared`. Order: limit first (cheap, no narrowing), then remaining
    /// bytes.
    pub fn check_len(&self, declared: u64, remaining: usize) -> Result<usize> {
        if declared > self.max_len {
            return Err(SlimError::limit(Limit::Len, declared, self.max_len));
        }
        if declared > remaining as u64 {
            return Err(SlimError::truncated(remaining.saturating_add(1), remaining));
        }
        Ok(declared as usize)
    }

    /// Validates a total payload size against [`Limit::Bytes`].
    pub fn check_total(&self, size: u64) -> Result<usize> {
        if size > self.max_bytes {
            return Err(SlimError::limit(Limit::Bytes, size, self.max_bytes));
        }
        usize::try_from(size).map_err(|_| SlimError::SizeOverflow)
    }

    /// Validates one level of nesting against [`Limit::Depth`].
    pub fn check_depth(&self, depth: usize) -> Result<()> {
        if depth > self.max_depth {
            return Err(SlimError::limit(
                Limit::Depth,
                depth as u64,
                self.max_depth as u64,
            ));
        }
        Ok(())
    }

    /// Validates a dictionary entry count against [`Limit::Dictionary`].
    pub fn check_dictionary(&self, entries: u64) -> Result<usize> {
        self.check_count(entries, Limit::Dictionary, self.max_dictionary)
    }

    /// Validates a column count against [`Limit::Columns`].
    pub fn check_columns(&self, columns: u64) -> Result<usize> {
        self.check_count(columns, Limit::Columns, self.max_columns)
    }

    /// Validates a record count against [`Limit::Records`].
    pub fn check_records(&self, records: u64) -> Result<usize> {
        self.check_count(records, Limit::Records, self.max_records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec_table() {
        let l = Limits::default();
        assert_eq!(l.max_bytes, 64 * 1024 * 1024);
        assert_eq!(l.max_records, 10_000_000);
        assert_eq!(l.max_depth, 64);
        assert_eq!(l.max_len, 16 * 1024 * 1024);
        assert_eq!(l.max_dictionary, 1_000_000);
        assert_eq!(l.max_columns, 4096);
    }

    #[test]
    fn check_len_accepts_a_length_that_fits_both_rules() {
        let l = Limits::default();
        assert_eq!(l.check_len(10, 100).unwrap(), 10);
        assert_eq!(l.check_len(0, 0).unwrap(), 0);
    }

    #[test]
    fn huge_declared_lengths_are_rejected_without_allocating() {
        let l = Limits::default();
        // u64::MAX, i64::MAX-ish and other absurd declarations, all against a
        // tiny input. None of these may allocate or panic.
        for declared in [
            u64::MAX,
            u64::MAX - 1,
            1 << 62,
            0xFFFF_FFFF,
            usize::MAX as u64,
            i64::MAX as u64,
        ] {
            match l.check_len(declared, 8) {
                Err(SlimError::LimitExceeded {
                    limit: Limit::Len,
                    value,
                    ..
                }) => {
                    assert_eq!(value, declared);
                }
                other => panic!("declared {declared} should be a limit error, got {other:?}"),
            }
        }
    }

    #[test]
    fn limit_is_checked_before_remaining_bytes() {
        let strict = Limits::strict();
        // Both rules would fail; the limit error must win so that no narrowing
        // or allocation on the declared value happens first.
        assert!(matches!(
            strict.check_len(1_000_000, 0),
            Err(SlimError::LimitExceeded { .. })
        ));
        // Under the limit but more than remains -> truncation.
        assert!(matches!(
            strict.check_len(20, 10),
            Err(SlimError::Truncated { .. })
        ));
        assert_eq!(strict.check_len(10, 10).unwrap(), 10);
    }

    #[test]
    fn counts_use_their_own_limits() {
        let strict = Limits::strict();
        assert_eq!(strict.check_records(16).unwrap(), 16);
        assert!(matches!(
            strict.check_records(17),
            Err(SlimError::LimitExceeded {
                limit: Limit::Records,
                ..
            })
        ));
        assert_eq!(strict.check_columns(4).unwrap(), 4);
        assert!(matches!(
            strict.check_columns(5),
            Err(SlimError::LimitExceeded {
                limit: Limit::Columns,
                ..
            })
        ));
        assert_eq!(strict.check_dictionary(8).unwrap(), 8);
        assert!(matches!(
            strict.check_dictionary(9),
            Err(SlimError::LimitExceeded {
                limit: Limit::Dictionary,
                ..
            })
        ));
        assert!(matches!(
            strict.check_records(u64::MAX),
            Err(SlimError::LimitExceeded { .. })
        ));
    }

    #[test]
    fn depth_and_total_bytes_are_checked() {
        let strict = Limits::strict();
        assert!(strict.check_depth(4).is_ok());
        assert!(matches!(
            strict.check_depth(5),
            Err(SlimError::LimitExceeded {
                limit: Limit::Depth,
                ..
            })
        ));
        assert_eq!(strict.check_total(1024).unwrap(), 1024);
        assert!(matches!(
            strict.check_total(1025),
            Err(SlimError::LimitExceeded {
                limit: Limit::Bytes,
                ..
            })
        ));
    }

    #[test]
    fn a_limit_of_zero_permits_only_empty_values() {
        let none = Limits {
            max_bytes: 0,
            max_records: 0,
            max_depth: 0,
            max_len: 0,
            max_dictionary: 0,
            max_columns: 0,
        };
        // A limit of zero means "only the empty value is allowed".
        assert_eq!(none.check_len(0, 10).unwrap(), 0);
        assert!(matches!(
            none.check_len(1, 10),
            Err(SlimError::LimitExceeded {
                limit: Limit::Len,
                ..
            })
        ));
        assert_eq!(none.check_records(0).unwrap(), 0);
        assert!(matches!(
            none.check_records(1),
            Err(SlimError::LimitExceeded {
                limit: Limit::Records,
                ..
            })
        ));
        assert!(matches!(
            none.check_columns(1),
            Err(SlimError::LimitExceeded {
                limit: Limit::Columns,
                ..
            })
        ));
        assert_eq!(none.check_total(0).unwrap(), 0);
        assert!(none.check_depth(1).is_err());
        assert_eq!(none.check_dictionary(0).unwrap(), 0);
    }
}

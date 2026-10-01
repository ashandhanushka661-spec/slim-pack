//! rkyv 0.8 competitor, pinned at 0.8.18.
//!
//! rkyv is given its best realistic configuration: the high-level API
//! (`api::high`), which uses an arena allocator for shared pointers and
//! `bytecheck` validation.
//!
//! The measurement protocol requires verified-vs-verified and
//! unverified-vs-unverified reads, and rkyv offers both:
//!
//! * [`access_verified`] runs `bytecheck` validation over the whole archive,
//! * [`access_unverified`] is rkyv's own `access_unchecked`, which is `unsafe`
//!   in rkyv's API and therefore in ours.

/// rkyv error type used by every call below.
pub type Error = rkyv::rancor::Error;

/// Serializes `value` into a fresh buffer (cold encode).
pub fn to_vec<T>(value: &T) -> Vec<u8>
where
    T: for<'a> rkyv::Serialize<
        rkyv::api::high::HighSerializer<
            rkyv::util::AlignedVec,
            rkyv::ser::allocator::ArenaHandle<'a>,
            Error,
        >,
    >,
{
    rkyv::api::high::to_bytes::<Error>(value)
        .expect("rkyv encode")
        .into_vec()
}

/// Deserializes to an owned value with validation (checked decode).
pub fn from_bytes_checked<T>(bytes: &[u8]) -> Result<T, Error>
where
    T: rkyv::Archive,
    T::Archived: for<'a> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, Error>>
        + rkyv::Deserialize<T, rkyv::rancor::Strategy<rkyv::de::Pool, Error>>,
{
    rkyv::api::high::from_bytes::<T, Error>(bytes)
}

/// Verified zero-copy read: validates the archive, then hands out a reference.
pub fn access_verified<'a, T>(bytes: &'a [u8]) -> Result<&'a T, Error>
where
    T: rkyv::Portable
        + for<'b> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'b, Error>>,
{
    rkyv::api::high::access::<T, Error>(bytes)
}

/// Unverified zero-copy read.
///
/// # Safety
/// `bytes` must be a buffer produced by rkyv for the same `T` and must not be
/// mutated while the returned reference is alive. rkyv's own
/// `access_unchecked` is `unsafe` for the same reason: it skips `bytecheck`.
pub unsafe fn access_unverified<T>(bytes: &[u8]) -> &T
where
    T: rkyv::Portable,
{
    rkyv::access_unchecked::<T>(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::TelemetrySmall;

    fn sample() -> TelemetrySmall {
        TelemetrySmall {
            rsrp: -97,
            band: 3,
            speed: 42.5,
        }
    }

    #[test]
    fn checked_round_trip() {
        let bytes = to_vec(&sample());
        assert_eq!(
            from_bytes_checked::<TelemetrySmall>(&bytes).unwrap(),
            sample()
        );
    }

    #[test]
    fn verified_and_unverified_access_read_the_same_fields() {
        let bytes = to_vec(&sample());
        let verified = access_verified::<rkyv::Archived<TelemetrySmall>>(&bytes).unwrap();
        // SAFETY: `bytes` is a valid rkyv buffer for `Archived<TelemetrySmall>`.
        let unverified = unsafe { access_unverified::<rkyv::Archived<TelemetrySmall>>(&bytes) };
        assert_eq!(verified.rsrp, unverified.rsrp);
        assert_eq!(verified.band, unverified.band);
        assert_eq!(
            verified.speed.to_native().to_bits(),
            unverified.speed.to_native().to_bits()
        );
    }

    #[test]
    fn checked_access_rejects_truncated_bytes() {
        let bytes = to_vec(&sample());
        assert!(
            access_verified::<rkyv::Archived<TelemetrySmall>>(&bytes[..bytes.len() - 1]).is_err()
        );
    }
}

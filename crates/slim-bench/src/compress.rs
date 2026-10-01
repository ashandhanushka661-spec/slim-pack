//! Size-only compression helpers (gzip and zstd).
//!
//! SlimPack does not compress its own output (out of scope for this project),
//! so these exist purely to report "raw, gzip and zstd bytes" per the
//! measurement protocol. Both use each library's default level
//! (gzip: 6 via `Compression::default()`, zstd: 3 via `zstd::DEFAULT_COMPRESSION_LEVEL`),
//! applied identically to every codec's bytes.

use std::io::Write;

/// gzip bytes of `input` at the default level.
pub fn gzip(input: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(input).expect("gzip write");
    encoder.finish().expect("gzip finish")
}

/// zstd bytes of `input` at the default level.
pub fn zstd(input: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(input, zstd::DEFAULT_COMPRESSION_LEVEL).expect("zstd encode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_shrinks_repetitive_input_and_round_trips_sizes() {
        let input = vec![b'a'; 4096];
        assert!(gzip(&input).len() < input.len());
        assert!(zstd(&input).len() < input.len());
        assert!(!gzip(&input).is_empty());
        assert!(!zstd(&input).is_empty());
    }
}

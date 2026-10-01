#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! SlimPack: a spec-first binary format for high-throughput telemetry and
//! structured records.
//!
//! Status: Phase 0 skeleton. The format specification lives in
//! `/spec/SPEC.md`; the implementation is built in later phases against it.
//!
//! Hard rules this crate must never break:
//!
//! * zero runtime dependencies (std only),
//! * `#![forbid(unsafe_code)]` unless a single `unsafe` block is justified by
//!   a comment, a safety test and a measured speedup (see PROGRESS.md),
//! * lossless means bit-exact (float NaN payloads and `-0.0` round-trip),
//! * lossy features (quantization) are explicit per-field opt-ins and are
//!   never reported in lossless results.

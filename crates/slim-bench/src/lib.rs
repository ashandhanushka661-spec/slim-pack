//! Benchmark harness for SlimPack.
//!
//! Status: Phase 0 skeleton. This crate owns every competitor dependency
//! (serde_json, rmp-serde, postcard, bincode, bitcode, rkyv, flatbuffers,
//! flate2, zstd) so that the core crate keeps zero runtime dependencies and
//! stays fast to build.
//!
//! It also owns the committed, seeded, deterministic datasets in `/datasets`
//! and their generator (`src/bin/gen_datasets.rs`).

pub mod codecs;
pub mod compress;
pub mod datasets;
pub mod flat;
pub mod gen;
pub mod rkyv_codec;

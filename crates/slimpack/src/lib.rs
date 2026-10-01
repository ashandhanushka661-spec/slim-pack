#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! SlimPack: a spec-first binary format for high-throughput telemetry and
//! structured records.
//!
//! Status: Phase 1 (header, Fixed `aligned`/`packed`, Dynamic, JSON). Batch
//! mode (§3) arrives in Phase 2. The format specification lives in
//! `/spec/SPEC.md`; the implementation is built against it phase by phase, and
//! `/spec/vectors` pins the bytes.
//!
//! Hard rules this crate must never break:
//!
//! * zero runtime dependencies (std only),
//! * `#![forbid(unsafe_code)]` unless a single `unsafe` block is justified by
//!   a comment, a safety test and a measured speedup (see PROGRESS.md),
//! * lossless means bit-exact (float NaN payloads and `-0.0` round-trip),
//! * lossy features (quantization) are explicit per-field opt-ins and are
//!   never reported in lossless results.

pub mod batch;
pub mod dynamic;
mod error;
pub mod fixed;
pub mod header;
pub mod limits;
pub mod schema;
pub mod varint;

pub use dynamic::Value;
pub use error::{Limit, Result, SlimError};
pub use header::{Header, Section};
pub use limits::Limits;
pub use schema::{Attr, FieldSpec, Layout, LayoutKind, StructSpec, Type};

/// Marker trait for types that participate in the SlimPack schema system.
pub trait SlimPack: Sized {
    /// Returns the schema for this Rust type, as used by the shared layout
    /// engine and the derive macro.
    fn slim_schema() -> StructSpec;

    /// Computes the fixed layout for this type in the requested mode.
    fn slim_layout(kind: LayoutKind) -> Result<Layout> {
        Layout::compute(&Self::slim_schema(), kind)
    }
}

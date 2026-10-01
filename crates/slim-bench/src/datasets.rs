//! Typed dataset models shared by the generator, the competitors and the
//! SlimPack benchmarks.
//!
//! The JSON files in `/datasets` are the single source of truth for benchmark
//! inputs. Every competitor is measured on a *typed struct* (never on
//! `serde_json::Value`) so that serde_json / MessagePack / postcard / bincode /
//! bitcode / rkyv all see the same shape. Only dataset `e` (mixed arbitrary
//! JSON) has no typed model, and that exception is stated in results.json.
//!
//! All models derive every competitor's native trait set as well as serde's,
//! so every competitor gets its best realistic configuration:
//! serde (`Serialize`/`Deserialize`), bincode's own `Encode`/`Decode`
//! (bincode 2 has no serde path), bitcode's native `Encode`/`Decode`
//! (bitcode's compact mode) and rkyv's `Archive`/`Serialize`/`Deserialize`.

use serde::{Deserialize, Serialize};

/// Single telemetry record, dataset `a`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct TelemetrySmall {
    /// Signal strength, dBm, always in `-140..=-44`.
    pub rsrp: i8,
    /// Band index from a small set.
    pub band: u8,
    /// Speed in km/h.
    pub speed: f32,
}

/// Telemetry record, datasets `b-clean`, `b-noisy`, `b-adversarial`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct Telemetry {
    /// Signal strength, dBm, always in `-140..=-44`.
    pub rsrp: i8,
    /// Band index from a small set.
    pub band: u8,
    /// Speed in km/h.
    pub speed: f32,
    /// Unix milliseconds.
    #[serde(rename = "timestamp")]
    pub timestamp: i64,
    /// Cell identifier.
    #[serde(rename = "cellId")]
    pub cell_id: u32,
}

/// User record with strings, bools and optional fields, dataset `c`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct User {
    pub id: u32,
    pub name: String,
    pub email: String,
    pub active: bool,
    pub age: u16,
    /// Optional, missing in ~30% of records (absent key, not `null`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// Optional, missing in ~30% of records (absent key, not `null`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Optional, missing in ~30% of records (absent key, not `null`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

/// Nested configuration object, dataset `d`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct Config {
    pub name: String,
    pub version: u32,
    pub debug: bool,
    pub limits: Limits,
    pub backends: Vec<Backend>,
}

/// `Config.limits`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct Limits {
    pub max_records: u64,
    pub max_bytes: u64,
    pub timeout_ms: u32,
}

/// `Config.backends[]` element.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct Backend {
    pub kind: String,
    pub weight: f32,
    pub hosts: Vec<String>,
    pub tls: bool,
}

/// High-entropy record, dataset `f`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct RandomRow {
    pub f1: f32,
    pub f2: f32,
    pub f3: f32,
    pub f4: f32,
    pub i1: i64,
    pub i2: i32,
    pub i3: u64,
}

/// Sparse record with optional fields, dataset `g` (~70% of the optional
/// fields are absent; absent means "key not present in the object").
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    bincode::Encode,
    bincode::Decode,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct SparseRow {
    pub id: u32,
    pub ts: i64,
    pub kind: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<u32>,
}

/// Path of the committed datasets directory (it may not exist yet on a fresh
/// clone before the generator has run).
pub fn datasets_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../datasets")
        .to_path_buf()
}

/// Canonicalized [`datasets_dir`], for read paths that require it to exist.
pub fn datasets_dir_existing() -> std::path::PathBuf {
    datasets_dir()
        .canonicalize()
        .expect("datasets directory (run gen-datasets)")
}

/// File name of every committed dataset.
pub const DATASET_FILES: &[&str] = &[
    "a.json",
    "b-clean.json",
    "b-noisy.json",
    "b-adversarial.json",
    "c.json",
    "d.json",
    "e.json",
    "f.json",
    "g.json",
];

/// Loads and parses one dataset file into a typed model.
pub fn load<T: serde::de::DeserializeOwned>(file: &str) -> T {
    let path = datasets_dir_existing().join(file);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Loads dataset `b-clean`.
pub fn b_clean() -> Vec<Telemetry> {
    load("b-clean.json")
}

/// Loads dataset `b-noisy`.
pub fn b_noisy() -> Vec<Telemetry> {
    load("b-noisy.json")
}

/// Loads dataset `b-adversarial`.
pub fn b_adversarial() -> Vec<Telemetry> {
    load("b-adversarial.json")
}

/// Loads dataset `c`.
pub fn c_users() -> Vec<User> {
    load("c.json")
}

/// Loads dataset `f`.
pub fn f_random() -> Vec<RandomRow> {
    load("f.json")
}

/// Loads dataset `g`.
pub fn g_sparse() -> Vec<SparseRow> {
    load("g.json")
}

//! Deterministic dataset generation (Phase 0).
//!
//! Datasets are generated once, committed, and must never be edited again
//! after Phase 3 starts. Regenerating with the same seeds reproduces them
//! byte for byte, which
//! `cargo run -p slim-bench --bin gen-datasets -- --check` verifies.

use crate::datasets::datasets_dir;

pub mod config;
pub mod json;
pub mod records;
pub mod rng;
pub mod telemetry;
/// Renders every dataset as `(file name, JSON text)`.
///
/// Single source of truth for both `gen-datasets` (writes) and the committed
/// dataset tests (verifies), so the two can never disagree.
pub fn render_all() -> Vec<(String, String)> {
    vec![
        ("a.json".to_owned(), telemetry::a_json()),
        (
            "b-clean.json".to_owned(),
            json::lines_of(&telemetry::b_clean_records()),
        ),
        (
            "b-noisy.json".to_owned(),
            json::lines_of(&telemetry::b_noisy_records()),
        ),
        (
            "b-adversarial.json".to_owned(),
            json::lines_of(&telemetry::b_adversarial_records()),
        ),
        ("c.json".to_owned(), json::lines_of(&records::c_records())),
        ("d.json".to_owned(), json::pretty(&config::d_value())),
        ("e.json".to_owned(), json::lines_of(&config::e_values(200))),
        ("f.json".to_owned(), json::lines_of(&records::f_records())),
        ("g.json".to_owned(), json::lines_of(&records::g_records())),
    ]
}

/// Path of one dataset file.
pub fn dataset_path(name: &str) -> std::path::PathBuf {
    datasets_dir().join(name)
}

/// Writes one dataset file, creating parent directories.
pub fn write_dataset(name: &str, text: &str) -> std::io::Result<()> {
    let path = dataset_path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

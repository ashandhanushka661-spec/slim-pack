//! Integration tests over the **committed** datasets.
//!
//! These are the guard rails for the measurement protocol: after Phase 3 the
//! files in `/datasets` must never change, so `cargo test` fails if a dataset
//! is edited by hand or if the generator's seeds/algorithms drift.

use slim_bench::datasets::{
    self, Config, RandomRow, SparseRow, Telemetry, TelemetrySmall, User, DATASET_FILES,
};
use slim_bench::gen::records::G_ABSENT;
use slim_bench::gen::render_all;
use slim_bench::gen::telemetry::{B_RECORDS, RSRP_MAX_I64, RSRP_MIN_I64};

#[test]
fn every_dataset_file_exists_and_is_non_trivial() {
    for name in DATASET_FILES {
        let bytes = std::fs::read(slim_bench::gen::dataset_path(name))
            .unwrap_or_else(|e| panic!("{name} missing: {e} (run gen-datasets)"));
        assert!(
            bytes.len() > 40,
            "{name} is suspiciously small: {} bytes",
            bytes.len()
        );
    }
}

#[test]
fn committed_datasets_match_the_generator_byte_for_byte() {
    for (name, generated) in render_all() {
        let committed = std::fs::read(slim_bench::gen::dataset_path(&name))
            .unwrap_or_else(|e| panic!("{name} missing: {e}"));
        assert_eq!(
            committed.len(),
            generated.len(),
            "{name}: committed length differs from generated length \
             (a dataset was edited by hand, or the generator changed)"
        );
        assert!(
            committed == generated.as_bytes(),
            "{name}: bytes differ from generated"
        );
    }
}

#[test]
fn dataset_a_is_a_single_telemetry_record() {
    let record: TelemetrySmall = datasets::load("a.json");
    assert!(record.speed.is_finite());
}

#[test]
fn b_variants_all_have_1000_records_in_rsrp_range() {
    for rows in [
        datasets::b_clean(),
        datasets::b_noisy(),
        datasets::b_adversarial(),
    ] {
        assert_eq!(rows.len(), B_RECORDS);
        assert!(rows
            .iter()
            .all(|r| (RSRP_MIN_I64..=RSRP_MAX_I64).contains(&i64::from(r.rsrp))));
        assert!(rows.iter().all(|r| r.speed.is_finite() && r.speed >= 0.0));
        assert!(rows.iter().all(|r| r.timestamp > 0));
    }
}

#[test]
fn b_variants_differ_in_structure_as_documented() {
    let clean: Vec<Telemetry> = datasets::b_clean();
    let noisy = datasets::b_noisy();
    let adversarial: Vec<Telemetry> = datasets::b_adversarial();

    // clean: constant step
    assert!(clean
        .windows(2)
        .all(|p| p[1].timestamp - p[0].timestamp == 1000));
    // noisy: jittered but monotonic
    assert!(noisy.windows(2).all(|p| p[1].timestamp >= p[0].timestamp));
    assert!(noisy
        .windows(2)
        .any(|p| p[1].timestamp - p[0].timestamp != 1000));
    // adversarial: not sorted
    let ascending = adversarial
        .windows(2)
        .filter(|p| p[1].timestamp > p[0].timestamp)
        .count();
    assert!((400..600).contains(&ascending));
}

#[test]
fn c_has_strings_bools_and_missing_optionals() {
    let rows: Vec<User> = datasets::c_users();
    assert_eq!(rows.len(), 1000);
    assert!(rows.iter().any(|u| u.city.is_some()));
    assert!(rows.iter().any(|u| u.city.is_none()));
    assert!(rows.iter().any(|u| u.active));
    assert!(rows.iter().any(|u| !u.active));
    // Absent keys, not nulls.
    let raw = std::fs::read_to_string(slim_bench::gen::dataset_path("c.json")).unwrap();
    assert!(!raw.contains("null"), "dataset c must not contain null");
    let first = &raw.lines().nth(1).unwrap();
    assert!(
        !first.contains("city"),
        "missing optional must be absent, got {first}"
    );
}

#[test]
fn d_round_trips_as_a_nested_config() {
    let cfg: Config = datasets::load("d.json");
    assert!(!cfg.backends.is_empty());
    assert!(cfg.backends.iter().all(|b| !b.hosts.is_empty()));
    let text = std::fs::read_to_string(slim_bench::gen::dataset_path("d.json")).unwrap();
    assert!(text.contains("\"limits\"") && text.contains("\"backends\""));
}

#[test]
fn e_is_mixed_arbitrary_json() {
    let values: Vec<serde_json::Value> = datasets::load("e.json");
    assert_eq!(values.len(), 200);
    assert!(values.iter().any(|v| v.is_object()));
    assert!(values.iter().any(|v| v.is_array()));
    assert!(values.iter().any(|v| v.is_string()));
    assert!(values.iter().any(|v| v.is_boolean()));
    assert!(values.iter().any(|v| v.is_null()));
    assert!(values.iter().any(|v| v.is_number()));
}

#[test]
fn f_is_high_entropy_and_finite() {
    let rows: Vec<RandomRow> = datasets::f_random();
    assert_eq!(rows.len(), 1000);
    assert!(rows
        .iter()
        .all(|r| [r.f1, r.f2, r.f3, r.f4].iter().all(|v| v.is_finite())));
    let raw = std::fs::read_to_string(slim_bench::gen::dataset_path("f.json")).unwrap();
    assert!(!raw.contains("null"), "dataset f must not contain null");
    assert!(
        !raw.contains("NaN") && !raw.contains("inf"),
        "dataset f must be JSON-finite"
    );
}

#[test]
fn g_is_about_70_percent_sparse() {
    let rows: Vec<SparseRow> = datasets::g_sparse();
    assert_eq!(rows.len(), 1000);
    let total = rows.len() * 4;
    let present = rows
        .iter()
        .map(|r| {
            [
                r.value.is_some(),
                r.label.is_some(),
                r.flag.is_some(),
                r.extra.is_some(),
            ]
            .iter()
            .filter(|p| **p)
            .count()
        })
        .sum::<usize>();
    let absent = 1.0 - present as f64 / total as f64;
    assert!(
        (G_ABSENT - 0.05..=G_ABSENT + 0.05).contains(&absent),
        "absent fraction {absent} is not near {G_ABSENT}"
    );
    let raw = std::fs::read_to_string(slim_bench::gen::dataset_path("g.json")).unwrap();
    assert!(
        !raw.contains("null"),
        "sparse fields must be absent, not null"
    );
    let first = raw.lines().nth(1).unwrap();
    assert_eq!(first, "  {\"id\":1,\"ts\":1700000001000,\"kind\":1},");
}

#[test]
fn json_datasets_reparse_into_their_typed_models() {
    // Catches a dataset that is valid JSON but not the documented shape.
    let _: Vec<Telemetry> = datasets::load("b-clean.json");
    let _: Vec<User> = datasets::load("c.json");
    let _: Vec<RandomRow> = datasets::load("f.json");
    let _: Vec<SparseRow> = datasets::load("g.json");
    let _: Config = datasets::load("d.json");
    let _: Vec<serde_json::Value> = datasets::load("e.json");
    let _: TelemetrySmall = datasets::load("a.json");
}

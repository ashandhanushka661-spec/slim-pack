//! Dataset `d`: a nested configuration object.
//!
//! One object with real nesting (object inside object, array of objects,
//! strings, string vectors, bools, integers and floats). Used for the Dynamic
//! mode rows (W4) and as a hand-written fixed-layout comparison.

use crate::datasets::{Backend, Config, Limits};
use crate::gen::rng::{seed, Rng};

const KINDS: &[&str] = &["kafka", "s3", "sqlite", "redis", "https", "grpc"];

const HOST_PREFIX: &[&str] = &["ingest", "edge", "core", "replica", "archive"];

const REGIONS: &[&str] = &["eu-west-1", "eu-central-1", "us-east-1", "ap-south-1"];

/// Dataset `d`: the nested config value.
pub fn d_value() -> Config {
    let mut rng = Rng::new(seed::D);
    let backends = (0..40)
        .map(|i| {
            let host_count = rng.range_i64(1, 4) as usize;
            Backend {
                kind: (*rng.pick(KINDS)).to_owned(),
                weight: rng.quantized_f32(0.0, 1.0, 0.01),
                hosts: (0..host_count)
                    .map(|h| {
                        format!(
                            "{}-{}-{}.{}.example.net",
                            rng.pick(HOST_PREFIX),
                            *rng.pick(REGIONS),
                            i,
                            h
                        )
                    })
                    .collect(),
                tls: rng.chance(4, 5),
            }
        })
        .collect();
    Config {
        name: "ingest-edge".to_owned(),
        version: 3,
        debug: false,
        limits: Limits {
            max_records: 1_000_000,
            max_bytes: 268_435_456,
            timeout_ms: 2_500,
        },
        backends,
    }
}

/// Dataset `e`: mixed arbitrary JSON.
///
/// An array of 200 heterogeneous values -- objects with differing key sets,
/// arrays, strings, integers, floats, booleans and nulls -- because
/// `serde_json::Value` is the only honest model here (no typed struct exists
/// for arbitrary JSON). The benchmark reports this exception in results.json.
pub fn e_values(count: usize) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut rng = Rng::new(seed::E);
    (0..count)
        .map(|i| match i % 12 {
            0 => json!({"id": i, "name": format!("row-{i}"), "ok": true}),
            1 => json!({"id": i, "value": rng.range_i64(-1000, 1000), "tags": ["a", "b"]}),
            2 => json!([1, 2, 3, {"deep": [true, false, null]}]),
            3 => json!(rng.range_i64(-1_000_000, 1_000_000)),
            4 => json!(format!("free-form string {i} with spaces")),
            5 => json!({"nested": {"deeper": {"deepest": [1.5, 2.25, {"k": null}]}}}),
            6 => json!(null),
            7 => json!(rng.quantized_f32(-100.0, 100.0, 0.01)),
            8 => json!({"a": 1, "b": [true, false], "c": "x", "d": 1.25, "e": null}),
            9 => json!({"mixed": [{"k": i}, [], {}, 0, -1.5], "n": i}),
            10 => json!(i % 2 == 0),
            _ => json!({"u": i * 1_000_000, "s": format!("k{i}"), "e": []}),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d_is_nested_and_non_trivial() {
        let cfg = d_value();
        assert_eq!(cfg.backends.len(), 40);
        assert!(cfg.backends.iter().all(|b| !b.hosts.is_empty()));
        assert!(cfg.backends.iter().any(|b| b.hosts.len() > 1));
        assert!(cfg.backends.iter().all(|b| (0.0..=1.0).contains(&b.weight)));
    }

    #[test]
    fn e_covers_every_json_type() {
        let values = e_values(200);
        assert_eq!(values.len(), 200);
        let has = |pred: fn(&serde_json::Value) -> bool| values.iter().any(pred);
        assert!(has(|v| v.is_object()));
        assert!(has(|v| v.is_array()));
        assert!(has(|v| v.is_string()));
        assert!(has(|v| v.is_boolean()));
        assert!(has(|v| v.is_null()));
        assert!(has(|v| v.is_i64() || v.is_u64()));
        assert!(has(|v| v.is_f64()));
        assert!(has(|v| v.get("nested").is_some()));
        // Distinct key sets: shape is not uniform.
        let key_counts: std::collections::BTreeSet<usize> = values
            .iter()
            .filter_map(|v| v.as_object().map(|o| o.len()))
            .collect();
        assert!(key_counts.len() >= 4, "expected varied key sets");
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(d_value(), d_value());
        assert_eq!(e_values(200), e_values(200));
    }
}

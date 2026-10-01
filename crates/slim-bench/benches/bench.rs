use std::fs;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::Value as JsonValue;

fn dataset_bytes(name: &str) -> Vec<u8> {
    fs::read(slim_bench::gen::dataset_path(name)).unwrap_or_else(|e| {
        panic!("read dataset {name}: {e}")
    })
}

fn slimpack_dynamic_roundtrip(bytes: &[u8]) {
    let text = std::str::from_utf8(bytes).expect("dataset is valid UTF-8");
    let limits = slimpack::limits::Limits::default();
    let value = slimpack::dynamic::json::parse_json(text, &limits).expect("parse JSON into Value");
    let mut encoded = Vec::new();
    slimpack::dynamic::encode_body(&[value], &[], &mut encoded).expect("encode Value body");
    let decoded = slimpack::dynamic::decode_body(&encoded, &[], &limits).expect("decode Value body");
    assert_eq!(decoded.len(), 1);
}

fn bench_serde_json_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_roundtrip");
    for name in ["a.json", "b-clean.json", "c.json", "d.json", "f.json", "g.json"] {
        let bytes = dataset_bytes(name);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::new("serde_json", name), &bytes, |b, bytes| {
            b.iter(|| {
                let value: JsonValue = serde_json::from_slice(bytes).expect("read JSON");
                let encoded = serde_json::to_vec(&value).expect("write JSON");
                let decoded: JsonValue = serde_json::from_slice(&encoded).expect("decode JSON");
                std::hint::black_box(decoded);
            });
        });
        group.bench_with_input(BenchmarkId::new("slimpack_dynamic", name), &bytes, |b, bytes| {
            b.iter(|| {
                let limits = slimpack::limits::Limits::default();
                let parsed = slimpack::dynamic::json::parse_json(
                    std::str::from_utf8(bytes).expect("dataset UTF-8"),
                    &limits,
                )
                .expect("parse JSON to Value");
                let mut encoded = Vec::new();
                slimpack::dynamic::encode_body(std::slice::from_ref(&parsed), &[], &mut encoded)
                    .expect("encode JSON value");
                let decoded = slimpack::dynamic::decode_body(&encoded, &[], &limits)
                    .expect("decode JSON value");
                std::hint::black_box(decoded);
            });
        });
    }
    group.finish();
}

fn bench_telemetry_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("telemetry_batch");
    for name in ["a.json", "b-clean.json", "b-adversarial.json"] {
        let bytes = dataset_bytes(name);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::new("serde_json", name), &bytes, |b, bytes| {
            b.iter(|| {
                let value: JsonValue = serde_json::from_slice(bytes).expect("JSON parse");
                let encoded = serde_json::to_vec(&value).expect("JSON write");
                let _ = serde_json::from_slice::<JsonValue>(&encoded).expect("JSON decode");
            });
        });
        group.bench_with_input(BenchmarkId::new("slimpack_dynamic", name), &bytes, |b, bytes| {
            b.iter(|| slimpack_dynamic_roundtrip(bytes));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_serde_json_roundtrip,
    bench_telemetry_batch,
);
criterion_main!(benches);

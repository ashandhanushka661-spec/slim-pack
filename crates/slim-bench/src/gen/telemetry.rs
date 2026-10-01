//! Datasets `a`, `b-clean`, `b-noisy`, `b-adversarial` (telemetry).
//!
//! All three `b` variants hold 1000 records of the same schema
//! `{rsrp, band, speed, timestamp, cellId}` and differ *only* in how regular
//! timestamps and `cellId` are, which is exactly what the columnar encoders
//! are supposed to exploit:
//!
//! * `b-clean` -- timestamps increase by exactly 1000 ms, `cellId` stays in
//!   runs of ~50 records cycling through 12 cells;
//! * `b-noisy` -- timestamps jitter by +/-250 ms around a 1000 ms step and stay
//!   monotonic, `cellId` repeats 90% of the time;
//! * `b-adversarial` -- timestamps are random over a 40-year span in random
//!   order and `cellId` is uniform over the whole `u32` range.

use crate::datasets::{Telemetry, TelemetrySmall};
use crate::gen::rng::{seed, Rng};

/// Number of records in every `b` variant.
pub const B_RECORDS: usize = 1000;

/// First timestamp of the `b` variants (2023-11-14T22:13:20Z).
pub const B_START_MS: i64 = 1_700_000_000_000;

/// RSRP range actually generated.
///
/// DEVIATION (D-0009, see PROGRESS.md): the brief asks for
/// `rsrp: i8 in -140..=-44`, but `-140` is **not representable in `i8`**
/// (`i8::MIN == -128`). We keep the field type `i8` as specified and generate
/// the representable part of the RSRP range, `-128..=-44`. Both ends still need
/// 7 bits under `#[slim(range = ...)]`, so no published size result changes,
/// and SPEC.md uses the same range in its worked example.
///
/// RSRP lower bound actually generated, see DEVIATION D-0009 above.
pub const RSRP_MIN_I64: i64 = -128;

/// RSRP upper bound.
pub const RSRP_MAX_I64: i64 = -44;

/// The 12 cell ids of `b-clean`, in a seeded but stable order.
fn shuffled_cells(rng: &mut Rng) -> Vec<u32> {
    let mut cells: Vec<u32> = (0..12).map(|i| 1000 + i * 7).collect();
    for i in (1..cells.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        cells.swap(i, j);
    }
    cells
}

/// Band indices a real device reports.
const BANDS: &[u8] = &[1, 3, 5, 7, 8, 20, 28, 66];

/// The single record of dataset `a`.
pub fn a_record() -> TelemetrySmall {
    TelemetrySmall {
        rsrp: -103,
        band: 3,
        speed: 27.5,
    }
}

/// Dataset `a` as JSON text.
pub fn a_json() -> String {
    crate::gen::json::pretty(&a_record())
}

/// Records for `b-clean`: regular 1000 ms timestamps, `cellId` in runs.
pub fn b_clean_records() -> Vec<Telemetry> {
    let mut rng = Rng::new(seed::B_CLEAN);
    let mut ts = B_START_MS;
    let cells = shuffled_cells(&mut rng);
    let mut rows = Vec::with_capacity(B_RECORDS);
    for i in 0..B_RECORDS {
        let cell = cells[(i / 50) % cells.len()];
        rows.push(Telemetry {
            rsrp: rng.range_i64(RSRP_MIN_I64, RSRP_MAX_I64) as i8,
            band: *rng.pick(BANDS),
            speed: rng.quantized_f32(0.0, 140.0, 0.5),
            timestamp: ts,
            cell_id: cell,
        });
        ts += 1000;
    }
    rows
}

/// Records for `b-noisy`: jittered monotonic timestamps, mostly repeating
/// `cellId`.
pub fn b_noisy_records() -> Vec<Telemetry> {
    let mut rng = Rng::new(seed::B_NOISY);
    let mut ts = B_START_MS;
    let mut cell = 4242u32;
    let mut rows = Vec::with_capacity(B_RECORDS);
    for _ in 0..B_RECORDS {
        if rng.chance(1, 10) {
            cell = (cell + 1 + rng.below(500) as u32) % 100_000;
        }
        rows.push(Telemetry {
            rsrp: rng.range_i64(RSRP_MIN_I64, RSRP_MAX_I64) as i8,
            band: *rng.pick(BANDS),
            speed: rng.quantized_f32(0.0, 140.0, 0.5),
            timestamp: ts,
            cell_id: cell,
        });
        ts += 1000 + rng.range_i64(-250, 250);
        ts = ts.max(rows[rows.len() - 1].timestamp); // monotonic, ties allowed
    }
    rows
}

/// Records for `b-adversarial`: random timestamps in random order and uniform
/// random `cellId`.
pub fn b_adversarial_records() -> Vec<Telemetry> {
    let mut rng = Rng::new(seed::B_ADVERSARIAL);
    (0..B_RECORDS)
        .map(|_| Telemetry {
            rsrp: rng.range_i64(RSRP_MIN_I64, RSRP_MAX_I64) as i8,
            band: *rng.pick(BANDS),
            speed: rng.quantized_f32(0.0, 140.0, 0.5),
            timestamp: rng.range_i64(0, 1_260_000_000_000),
            cell_id: rng.next_u64() as u32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_b_variant_has_1000_records_in_rsrp_range() {
        for rows in [
            b_clean_records(),
            b_noisy_records(),
            b_adversarial_records(),
        ] {
            assert_eq!(rows.len(), B_RECORDS);
            assert!(
                rows.iter()
                    .all(|r| (RSRP_MIN_I64..=RSRP_MAX_I64).contains(&i64::from(r.rsrp))),
                "rsrp outside {RSRP_MIN_I64}..={RSRP_MAX_I64}"
            );
            assert!(rows.iter().all(|r| BANDS.contains(&r.band)));
            assert!(rows.iter().all(|r| (0.0..=140.0).contains(&r.speed)));
        }
    }

    #[test]
    fn clean_timestamps_are_regular_and_cells_are_in_runs() {
        let rows = b_clean_records();
        for pair in rows.windows(2) {
            assert_eq!(pair[1].timestamp - pair[0].timestamp, 1000);
        }
        let distinct_cells = rows
            .iter()
            .map(|r| r.cell_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(distinct_cells, 12, "b-clean must draw from 12 cells");
    }

    #[test]
    fn noisy_timestamps_jitter_but_stay_monotonic() {
        let rows = b_noisy_records();
        let steps: Vec<i64> = rows
            .windows(2)
            .map(|p| p[1].timestamp - p[0].timestamp)
            .collect();
        assert!(
            steps.iter().all(|s| (750..=1250).contains(s)),
            "steps out of range"
        );
        let repeats = rows
            .windows(2)
            .filter(|p| p[1].cell_id == p[0].cell_id)
            .count();
        assert!(repeats > 800, "cellId should mostly repeat, got {repeats}");
    }

    #[test]
    fn adversarial_timestamps_are_not_ordered() {
        let rows = b_adversarial_records();
        // Random order over a huge span: roughly half the steps go up, and
        // crucially there is no monotone structure a delta-of-delta can use.
        let ascending = rows
            .windows(2)
            .filter(|p| p[1].timestamp > p[0].timestamp)
            .count();
        let fraction = ascending as f64 / (B_RECORDS - 1) as f64;
        assert!((0.4..0.6).contains(&fraction), "{fraction} of steps ascend");
        let distinct_cells = rows
            .iter()
            .map(|r| r.cell_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert!(distinct_cells > 990, "cellId should be near-uniform");
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(b_clean_records(), b_clean_records());
        assert_eq!(b_noisy_records(), b_noisy_records());
        assert_eq!(b_adversarial_records(), b_adversarial_records());
    }

    #[test]
    fn variants_differ_from_each_other() {
        assert_ne!(b_clean_records(), b_noisy_records());
        assert_ne!(b_noisy_records(), b_adversarial_records());
    }
}

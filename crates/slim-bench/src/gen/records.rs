//! Datasets `c` (users), `f` (high-entropy randoms) and `g` (sparse records).
//!
//! Optional fields are *absent keys* in the JSON, not `null`: that is the
//! shape real telemetry has, and it is what the presence bitmap in Batch mode
//! and FlatBuffers' default-value omission are meant to exploit.

use crate::datasets::{RandomRow, SparseRow, User};
use crate::gen::rng::{seed, Rng};

/// Record count for datasets `c`, `f` and `g`.
pub const RECORDS: usize = 1000;

/// First label prefix used for e-mail addresses.
const FIRST: &[&str] = &[
    "ana", "ben", "chi", "dan", "eli", "fay", "gus", "hal", "iris", "jan", "kai", "lena", "mo",
    "nina", "omar", "pia", "quinn", "rita", "sam", "tess", "uma", "vic", "wen", "xu", "yara",
    "zoe",
];

const LAST: &[&str] = &[
    "abbott", "brooks", "chen", "diaz", "engel", "fisher", "gupta", "hansen", "ito", "jensen",
    "kowalski", "li", "moreau", "novak", "okafor", "petrov", "quintana", "rossi", "silva",
    "tanaka", "uddin", "vega", "weber", "xu", "yilmaz", "zhang",
];

const CITIES: &[&str] = &[
    "Amsterdam",
    "Bergen",
    "Cairo",
    "Dublin",
    "Erfurt",
    "Faro",
    "Geneva",
    "Helsinki",
    "Istanbul",
    "Jakarta",
    "Kyoto",
    "Lisbon",
    "Madrid",
    "Naples",
    "Oslo",
    "Porto",
    "Quito",
    "Riga",
    "Sofia",
    "Tunis",
];

const TAGS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu",
];

const LABELS: &[&str] = &["ok", "retry", "timeout", "throttled", "rejected", "queued"];

/// Dataset `c`: 1000 user records with strings, bools and optional fields.
pub fn c_records() -> Vec<User> {
    let mut rng = Rng::new(seed::C);
    (0..RECORDS)
        .map(|i| {
            let first = *rng.pick(FIRST);
            let last = *rng.pick(LAST);
            User {
                id: i as u32 + 1,
                name: format!("{first} {last}"),
                email: format!("{first}.{last}@example.org"),
                active: rng.chance(85, 100),
                age: rng.range_i64(18, 79) as u16,
                city: rng.chance(7, 10).then(|| (*rng.pick(CITIES)).to_owned()),
                tags: rng.chance(7, 10).then(|| {
                    let count = rng.range_i64(1, 3) as usize;
                    (0..count).map(|_| (*rng.pick(TAGS)).to_owned()).collect()
                }),
                score: rng
                    .chance(7, 10)
                    .then(|| rng.quantized_f32(0.0, 100.0, 0.01)),
            }
        })
        .collect()
}

/// A finite `f32` with high entropy: random sign, random exponent in the
/// finite range and random mantissa.
///
/// JSON cannot express NaN or infinity, so a fully random bit pattern would
/// silently become `null` in the dataset files. Entropy is therefore taken
/// from sign + exponent + mantissa, never from the two special exponent
/// values.
pub fn finite_random_f32(rng: &mut Rng) -> f32 {
    let sign = (rng.next_u64() as u32) & 0x8000_0000;
    let exponent = rng.below(254) as u32; // 0 and 255 are the special cases
    let mantissa = ((rng.next_u64() >> 32) as u32) & 0x007F_FFFF;
    f32::from_bits(sign | (exponent << 23) | mantissa)
}

/// Dataset `f`: 1000 rows of high-entropy floats and integers.
pub fn f_records() -> Vec<RandomRow> {
    let mut rng = Rng::new(seed::F);
    (0..RECORDS)
        .map(|_| RandomRow {
            f1: finite_random_f32(&mut rng),
            f2: finite_random_f32(&mut rng),
            f3: finite_random_f32(&mut rng),
            f4: finite_random_f32(&mut rng),
            i1: rng.next_u64() as i64,
            i2: rng.next_u64() as u32 as i32,
            i3: rng.next_u64(),
        })
        .collect()
}

/// Fraction of the four optional fields of `g` that are absent.
pub const G_ABSENT: f64 = 0.70;

/// Dataset `g`: 1000 sparse records, ~70% of optional fields missing.
pub fn g_records() -> Vec<SparseRow> {
    let mut rng = Rng::new(seed::G);
    let mut ts = 1_700_000_000_000i64;
    (0..RECORDS)
        .map(|i| {
            ts += 1000;
            SparseRow {
                id: i as u32 + 1,
                ts,
                kind: rng.below(8) as u8,
                value: rng
                    .chance(3, 10)
                    .then(|| rng.quantized_f32(-40.0, 120.0, 0.25)),
                label: rng.chance(3, 10).then(|| (*rng.pick(LABELS)).to_owned()),
                flag: rng.chance(3, 10).then(|| rng.chance(1, 2)),
                extra: rng.chance(3, 10).then(|| rng.next_u64() as u32),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn c_has_1000_records_with_repeated_strings_and_some_missing_fields() {
        let rows = c_records();
        assert_eq!(rows.len(), RECORDS);
        let names: BTreeSet<&str> = rows.iter().map(|u| u.name.as_str()).collect();
        assert!(names.len() < rows.len(), "names must repeat");
        let missing = rows.iter().filter(|u| u.city.is_none()).count();
        assert!((250..=350).contains(&missing), "{missing} missing cities");
        let missing_tags = rows.iter().filter(|u| u.tags.is_none()).count();
        assert!(
            (250..=350).contains(&missing_tags),
            "{missing_tags} missing tags"
        );
        assert!(rows.iter().all(|u| u.email.contains('@')));
    }

    #[test]
    fn f_values_are_finite_and_high_entropy() {
        let rows = f_records();
        assert_eq!(rows.len(), RECORDS);
        assert!(rows
            .iter()
            .all(|r| [r.f1, r.f2, r.f3, r.f4].iter().all(|v| v.is_finite())));
        let distinct_i3: BTreeSet<u64> = rows.iter().map(|r| r.i3).collect();
        assert_eq!(distinct_i3.len(), RECORDS, "i3 must be unique");
        let distinct_f1: BTreeSet<u32> = rows.iter().map(|r| r.f1.to_bits()).collect();
        assert!(distinct_f1.len() > RECORDS * 9 / 10, "f1 entropy too low");
    }

    #[test]
    fn g_is_about_70_percent_sparse() {
        let rows = g_records();
        assert_eq!(rows.len(), RECORDS);
        let fields = 4.0;
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
            .sum::<usize>() as f64;
        let absent = 1.0 - present / (rows.len() as f64 * fields);
        assert!((0.65..=0.75).contains(&absent), "absent fraction {absent}");
        assert_eq!(G_ABSENT, 0.70);
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(c_records(), c_records());
        assert_eq!(f_records(), f_records());
        assert_eq!(g_records(), g_records());
    }
}

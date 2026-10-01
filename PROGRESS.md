# SlimPack v2 — PROGRESS

Spec-first binary format and toolkit for high-throughput telemetry and
structured records. Rust core, zero runtime dependencies, benchmarks decide
what the README is allowed to claim.

* Spec: [`spec/SPEC.md`](spec/SPEC.md) (draft, written before the encoder)
* Golden vectors: [`spec/vectors/`](spec/vectors) (Phase 1)
* Datasets: [`datasets/`](datasets) (committed, seeded, never edited after
  Phase 3 starts)

---

## Target claim (to be trimmed to what the benchmarks prove)

> On telemetry-style batches SlimPack is smaller than MessagePack, postcard,
> bincode, bitcode, rkyv and FlatBuffers, and faster to encode and decode than
> serde_json, MessagePack and postcard; its fixed-layout zero-copy reads are on
> par with or faster than FlatBuffers and rkyv.

Every clause of that sentence is a **win condition** below (W1–W3). W4–W7 are
reported rows. Anything the benchmarks do not prove is removed from the claim
and the removal is stated in the README.

---

## Phase status

| phase | scope | status |
|------:|-------|--------|
| 0 | foundation: workspace, SPEC draft, datasets, competitor crate compiles | **done** |
| 1 | core: Fixed (aligned + packed) + Dynamic, errors, golden vectors | pending |
| 2 | Batch/columnar: encodings one at a time, effort levels, views, accessors | pending |
| 3 | benchmarks per the measurement protocol → `results.json` (**GATE**) | pending |
| 4 | optimization loop on losing/tied rows (**GATE**) | pending |
| 5 | derive macro + `.slimschema` parsing + layout printing | pending |
| 6 | CLI (`slim`) + integration tests | pending |
| 7 | safety: limits, proptests, fuzzing, schema-evolution tests | pending |
| 8 | docs and release: honest README, LICENSE, CI, `cargo publish --dry-run` | pending |

---

## SCOREBOARD

Empty until Phase 3 produces numbers. One row per claim; **lossless rows only**
for W1–W3. A *win* requires non-overlapping criterion confidence intervals
**and** ≥ 5 % difference; otherwise it is a *tie*. Sizes are deterministic and
need no noise handling. Ties are reported as ties, never as wins.

| id | dataset / row | metric | competitor | slimpack | result |
|----|---------------|--------|-----------|----------|--------|
| — | — | — | — | — | not measured yet |

Planned rows (filled in Phase 3):

* **W1** — `b-clean`, `b-noisy`: size vs MessagePack (array **and** named map),
  postcard, bincode, bitcode, rkyv, FlatBuffers (+ gzip/zstd sizes); Fast-effort
  encode and decode vs serde_json, rmp-serde, postcard. `Max`-effort size on a
  separate row.
* **W2** — `c`, `g`: size vs MessagePack and postcard; speed vs serde_json.
* **W3** — Fixed `aligned` zero-copy field reads vs FlatBuffers and rkyv,
  verified-vs-verified and unverified-vs-unverified; size ≤ both. `packed`
  layout on a separate row.
* **W4** — Dynamic on `a`, `d`, `e`: never larger than JSON; ratios vs
  MessagePack and serde_json.
* **W5** — `decode_batch_into`: zero allocations on reuse, faster than
  `decode_batch` (proved with a counting allocator).
* **W6** — `a`: expected to tie with postcard; report the real result.
* **W7** — `b-adversarial`, `f`: losing is acceptable; report honestly.
* **lossy** — `b-clean` with quantization, labelled `lossy`, never mixed into
  lossless rows.

---

## Pinned dependency versions

All competitors live in `crates/slim-bench` only. Resolved with
`rustc 1.99.0 (b940084d7 2026-09-28)` on 2026-10-01 and pinned with `=`:

| crate | version | note |
|-------|---------|------|
| criterion | 0.8.2 | benches |
| serde | 1.0.229 | `derive` |
| serde_json | 1.0.151 | typed structs, not `Value` (except dataset `e`) |
| rmp-serde | 1.3.1 | MessagePack, array form **and** named map form |
| postcard | 1.1.3 | defaults, `use-std` |
| bincode | **2.0.1** | `bincode 3.0.0` is an upstream placeholder crate (`compile_error!`), so the real 2.x line is pinned |
| bitcode | 0.6.9 | native compact `Encode`/`Decode` derive API |
| rkyv | 0.8.18 | `api::high`, default features (`std`, `bytecheck`) |
| flatbuffers | **2.1.2** | pairs with the installed `flatc version 2.0.8` (see D-0003) |
| flate2 | 1.1.10 | gzip, default level 6 |
| zstd | 0.14.0 | zstd, `DEFAULT_COMPRESSION_LEVEL` |
| proptest | 1.11.0 | Phase 7 |
| syn | 3.0.6 | proc macro only |
| quote | 1.0.47 | proc macro only |
| proc-macro2 | 1.0.107 | proc macro only |

`Cargo.lock` is committed. Environment: AMD EPYC 7763, `nproc` = 2,
`flatc` 2.0.8 installed, no `target-cpu=native` for published numbers.

---

## Decision log

* **D-0001** Core crate is `std`-only with `#![forbid(unsafe_code)]` set in
  `Cargo.toml` (lint), so a stray `unsafe` fails the build, not just review.
* **D-0002** `panic = "abort"` lives in `[profile.release]` only. Cargo forces
  `unwind` for the `test` and `bench` profiles, so all codecs are measured
  under identical codegen settings and the choice cannot bias one codec.
* **D-0003** FlatBuffers competitor = `flatc 2.0.8` **generated** code (not
  hand-written) + the `flatbuffers 2.1.2` runtime. The installed `flatc` emits
  code for a newer runtime than any crate published on crates.io
  (`25.12.19` has `unsafe Follow::follow` / `unsafe Table::get`), so 2.0.8 +
  2.1.2 is the newest pair that compiles. Verified reads come from the
  generated `root_as_*`, unverified from the generated `root_as_*_unchecked`,
  which is exactly what the verified/unverified protocol needs.
* **D-0004** Each competitor gets its best realistic configuration, decided
  **before** any measurement: serde typed structs, MessagePack in both forms,
  postcard defaults, bincode `standard()`, bitcode native compact mode, rkyv
  high API, FlatBuffers via `flatc`.
* **D-0005** Dataset generator uses SplitMix64 with the fixed seeds in
  `crates/slim-bench/src/gen/rng.rs`; `gen-datasets --check` (and the
  `committed_datasets_match_the_generator_byte_for_byte` test) fail if a
  committed dataset ever differs from the seeds.
* **D-0006** Optional fields in datasets `c` and `g` are **absent keys**, not
  `null`: that is the real-world shape and what presence bitmaps exist for.
* **D-0007** JSON rendering for datasets is one compact value per line so the
  committed files stay diffable; correctness comes from `serde_json`.
* **D-0008** Dataset `f` uses random sign + random exponent (0..=253) +
  random mantissa instead of fully random bits, because JSON cannot express
  NaN/infinity and `serde_json` would silently write `null`. Entropy is still
  maximal over the finite float space.
* **D-0009 — DEVIATION.** The brief asks for `rsrp: i8 in -140..=-44`, but
  `-140` is **not representable in `i8`** (`i8::MIN == -128`). Datasets and the
  worked hex example in `spec/SPEC.md` use the representable range
  `-128..=-44`; the field type stays `i8` as specified. Both ends need 7 bits
  under `#[slim(range = ...)]`, so no published size result changes.
* **D-0010** Dynamic mode keeps `Float` as raw `f64` only. An `f32` form is
  *reserved* and rejected in v1, because downcasting `f64 → f32` would break
  the bit-exactness rule.

---

## Optimization log

(Filled in during Phase 4: hypothesis → one change → before/after → keep or
`git revert`.)

---

## Issues and open questions

1. **`nproc` = 2.** Criterion measures single-threaded, so this costs wall-clock
   time, not comparability. Full Phase 3 run will take a while; `sample_size`
   and `measurement_time` are left at criterion defaults so nothing can be
   accused of being tuned after seeing results.
2. **FlatBuffers version pairing** is capped by the installed `flatc`
   (D-0003). A newer `flatc`/crate pair would be a fairer "best current
   FlatBuffers" row; reported as an issue rather than silently ignored.
3. **Scope note for Phase 1:** `spec/SPEC.md` §3.1 writes `column_count` into
   the Batch body so that old readers can ignore unknown trailing columns.
   That costs a varint per batch and must not be dropped in Phase 2 — if it
   looks expensive, report it here instead of changing the rule.
4. **Pending decision (Phase 2):** whether `XOR_PREV` should also be tried for
   the *first* value of a column being the previous batch's last value.
   Default answer: no (columns are independent by construction).
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
| 1 | core: Fixed (aligned + packed) + Dynamic, errors, golden vectors | **done** |
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
* **D-0009 — RESOLVED: `rsrp` is `i16` with `range = -140..=-44`.** The brief
  asks for `rsrp: i8 in -140..=-44`, but `-140` is **not representable in
  `i8`** (`i8::MIN == -128`). Phase 0 initially narrowed the range to the
  representable `-128..=-44`; that was rejected because it silently changes the
  data. The field therefore keeps its full domain and widens to `i16`
  everywhere: SPEC.md §2.5 worked example (recomputed hex below), the
  `.slimschema` example, `Telemetry`/`TelemetrySmall` in `slim-bench`, the
  `flatc` schemas (`rsrp: short`) and all three `b` datasets. Consequences, in
  the direction that matters:
  * `#[slim(range = -140..=-44)]` still needs **7 bits** (97 values), so a
    SlimPack `packed` payload is exactly the same size as before;
  * every competitor that stores a raw `i16` grows by **one byte per record**,
    so the "smaller than X" comparisons run against *larger* competitor bytes.
    This is stated in the README, not hidden.
  * The rejected alternative is recorded here rather than deleted: it would
    have made competitors 1 byte/record smaller and was therefore a
    self-serving choice.

  Worked hex for `{rsrp: i16 range(-140..=-44), band: u8, speed: f32}` =
  `{rsrp: -103, band: 3, speed: 27.5}` (verified by bit arithmetic):

  | layout | bytes | notes |
  |--------|-------|-------|
  | `aligned` | `99 ff 03 00 00 dc 41` | 2 + 1 + 4 = 7 bytes; `range` has no effect |
  | `packed` | `a5 01 00 00 ee 20` | 7 + 8 + 32 = 47 bits, 1 padding bit; `rsrp` stored as `-103 + 140 = 37 = 0b0100101` |
  | `packed`, `band bits = 3` | `a5 01 00 70 07 01` | 7 + 3 + 32 = 42 bits |
* **D-0010** Dynamic mode keeps `Float` as raw `f64` only. An `f32` form is
  *reserved* and rejected in v1, because downcasting `f64 → f32` would break
  the bit-exactness rule.
* **D-0011** A Fixed `f32` field is carried as its raw IEEE-754 bit pattern
  (`Scalar::F32Bits`), not as a narrowed `f64`. `f32 → f64 → f32` quiets a
  signalling NaN, which would silently break the "lossless means bit-exact"
  rule; the vectors `fixed_aligned_extremes` (`7f800001`) and `dynamic_all_kinds`
  pin this.
* **D-0012** Dynamic integer form choice is a pure function of the value, with
  no effort knob in v1: `fixint` for `-128..=127`, else zigzag varint, else
  two's-complement varint for a negative value that would need 9 bytes, else raw
  `i64` when zigzag would need 10 bytes. `i64::MIN`/`MAX` are therefore exact in
  at most 9 bytes and `u64::MAX` in 9. Decoders accept all four forms.
* **D-0013** `range` is defined for integral and bool types only. On a float
  field it is a schema error (`UnsupportedLayout`), because "value − min" is not
  representable for `NaN`; floats that need less precision use the explicitly
  lossy `quantize`.
* **D-0014** A Fixed body stores the **size of the first record** as `stride`
  (never 0). For a fixed-width or `packed` layout that makes the body exactly
  `record_count * stride` bytes; for a layout with `str`/`bytes`/`Vec<T>` the
  records differ in size and a reader walks them in order. A stored `stride`
  below the reader's computed stride is still `MissingField` (R4).
* **D-0015** JSON conversion (§4.2) is hand-written in the core: an integer that
  fits `i64` is `Int`, one above it is `UInt`, an integer outside `u64` is
  `NumberOutOfRange`, and any fraction or exponent is `Float`. Writing appends
  `.0` to a float that would otherwise re-parse as an integer, rejects
  non-finite floats, and keeps duplicate object keys in `Value` (only the
  binary decoder rejects them). `bytes` has no JSON spelling and renders as a
  hex string, for tooling only.

---

### Phase 1 result

`crates/slimpack` implements the header, `Limits`, the canonical varint codec,
the schema model and layout engine, both Fixed layouts, and Dynamic values with
their binary and JSON forms: 135 unit tests plus 5 vector tests, zero runtime
dependencies, `#![forbid(unsafe_code)]`, clippy clean with `-D warnings`.

Nine accepted vectors and fifteen negative vectors are pinned under
`spec/vectors/`. The §2.5 worked example is read straight out of `SPEC.md` by
the test harness, so spec and implementation cannot drift apart silently.

Not in Phase 1, by design: Batch (§3), nested struct inlining, lossy
`quantize`, and any reader that needs a schema resolver — those are Phases 2,
5 and 6.

---

## Optimization log

(Filled in during Phase 4: hypothesis → one change → before/after → keep or
`git revert`.)

---

## Issues and open questions

0. **Dataset change log (only change after Phase 0, before any measurement).**
   `rsrp` widened from `i8`/`-128..=-44` to `i16`/`-140..=-44` (D-0009), so
   `b-clean`, `b-noisy` and `b-adversarial` were regenerated: 78 919 → 79 033,
   79 694 → 79 806 and 83 812 → 83 903 bytes. **No benchmark has run yet**, so
   nothing can have been tuned to these numbers. From here on, datasets are
   frozen (Phase 3 start).
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
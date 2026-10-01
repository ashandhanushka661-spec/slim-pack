# Golden conformance vectors

Phase 0 placeholder. The vectors themselves are **generated from the
implementation in Phase 1** and pinned by
`crates/slimpack/tests/vectors.rs`.

For each case this directory will hold:

* `<name>.slim` — the exact payload bytes (committed, never edited by hand);
* `<name>.json` — the case description: mode, layout, input value(s), expected
  decoded value(s) and, for negative cases, the expected `SlimError` variant.

Rules:

1. Any byte change in a `.slim` file must fail `cargo test` and require a
   format version bump (`spec/SPEC.md` §11).
2. Vectors cover every mode (Fixed aligned, Fixed packed, Batch with every
   encoding, Dynamic), the documented lossless edge cases
   (`-0.0`, NaN payloads, `i64::MIN`/`i64::MAX`, empty batches) and the
   negative cases (truncation, reserved bits, limit violations, unknown
   encoding ids, duplicate keys).
3. Regeneration is explicit and auditable: the generator writes into this
   directory and the diff is reviewed before it is committed.
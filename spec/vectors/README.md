# Golden conformance vectors

Each case is a pair of files:

* `<name>.slim` — the exact payload bytes (committed, never edited by hand);
* `<name>.json` — the case description: mode, layout, input value(s), expected
  decoded value(s) and, for negative cases, the expected `SlimError` variant.

The `.json` file is the source of truth. The `.slim` file is generated from it
by `crates/slimpack/tests/vectors.rs`, which fails if the two disagree. Every
accepted case is also decoded and compared against its description, so a vector
pins bytes *and* meaning.

## Case shapes

```jsonc
// accepted, Fixed mode: schema + values
{
  "case": "fixed",
  "note": "why this case exists",
  "layout": "aligned" | "packed",
  "fields": [
    {"name": "rsrp", "type": "i16", "range": [-140, -44], "value": -103},
    {"name": "speed", "type": "f32", "f32bits": "41dc0000", "value": 27.5},
    {"name": "raw", "type": "bytes", "hex": "00ff10"},
    {"name": "levels", "type": "Vec<u16>", "values": [0, 1, 4095]}
  ]
}

// accepted, Dynamic mode
{ "case": "dynamic", "keys": ["rsrp"], "values": [null, true, -1, 1.5, {"hex": "00ff", "as": "bytes"}] }

// negative: hand-written bytes, exact expected error
{ "case": "reject", "bytes": "53 4c 4d 50 01 01 00", "expect_error": "Truncated" }
```

`f32bits` / `f64bits` carry an exact IEEE-754 bit pattern, which is how the
vectors pin `-0.0` and NaN payloads. `"hex"` is also the description-only
spelling for `bytes` (JSON has no bytes type).

## What is covered

| group | vectors |
|-------|---------|
| SPEC §2.5 worked example | `spec_2_5_aligned`, `spec_2_5_packed`, `spec_2_5_packed_bits3` |
| lossless extremes | `fixed_aligned_extremes`, `fixed_packed_extremes` |
| variable-length aligned | `fixed_aligned_variable` |
| Dynamic | `dynamic_all_kinds`, `dynamic_interned_keys`, `dynamic_bytes_and_depth` |
| negative cases | `reject_*` — truncation, bad magic, unknown mode/version/tag, reserved flag/byte/form, overlong varint, duplicate key, out-of-range packed value, invalid bool byte, invalid UTF-8, limit violation, trailing bytes, missing field |

Batch mode vectors arrive with the Batch codec (Phase 2).

## Rules

1. Any byte change in a `.slim` file must fail `cargo test` and require a
   format version bump (`spec/SPEC.md` §11).
2. Regeneration is explicit and auditable:

   ```
   SLIMPACK_BLESS=1 cargo test -p slimpack --test vectors
   ```

   Review the diff before committing it.
3. The harness reads the hex of the §2.5 worked example out of `SPEC.md`
   itself, so the specification and the implementation cannot drift apart
   without a test failure.
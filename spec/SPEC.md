# SlimPack v2 — format specification

Status: **draft for Phase 1 implementation.** Every byte layout in this
document is pinned by golden vectors in `/spec/vectors`; changing any byte
requires a version bump (see [Versioning](#11-versioning)).

* Version byte in the current draft: `0x01`.
* All multi-byte scalars are **little-endian**.
* All reserved bits and bytes **must** be zero on write and **must** be
  rejected on read.
* "Lossless" always means **bit-exact**: `f32`/`f64` values round-trip with
  identical bit patterns, including NaN payloads, signalling NaNs, `-0.0`
  and both infinities.
* Integer delta arithmetic uses **wrapping** arithmetic and is tested at
  `i64::MIN` / `i64::MAX`.
* Lossy features exist (quantization only) and are always explicit opt-ins.
  A lossy payload sets `flags.LOSSY`, and lossy results are reported on
  separate rows that never appear in lossless claims.

---

## 1. Common header

Every payload starts with exactly 8 bytes:

| offset | size | field | meaning |
|-------:|-----:|-------|---------|
| 0 | 4 | `magic` | ASCII `SLMP` = `53 4C 4D 50` |
| 4 | 1 | `version` | format version, `0x01` in this draft |
| 5 | 1 | `mode` | `1` = Fixed, `2` = Batch, `3` = Dynamic |
| 6 | 1 | `flags` | see below; reserved bits must be 0 |
| 7 | 1 | `reserved` | must be 0 |

### 1.1 Flags

| bit | mask | name | meaning |
|----:|------|------|---------|
| 0 | `0x01` | `LOSSY` | payload contains at least one lossy (quantized) column or field |
| 1 | `0x02` | `EXT` | an extended header section follows the fixed 8-byte header |
| 2–7 | `0xFC` | reserved | must be 0; a reader returns `SlimError::ReservedBitsSet` |

### 1.2 Extended header section (`EXT`)

Present only when `flags.EXT` is set. It carries the *only* names that ever
appear in a SlimPack payload, and only in modes whose schema is not known to
the reader in advance:

| size | field | meaning |
|-----:|-------|---------|
| 1 | `section_kind` | `1` = Dynamic key table, `2` = Fixed layout descriptor, `3` = Batch field-name table |
| varint | `section_len` | byte length of the section payload |
| `section_len` | payload | section contents |

* `section_kind = 1` (Dynamic): `varint count`, then `count` × (`varint len`,
  UTF-8 bytes). These are the interned keys.
* `section_kind = 2` (Fixed): `varint field_count`, then per field
  `u8 name_len`, name bytes, `u8 type_code` (see §2.1). Written only by tools
  that want a self-describing payload (`slim convert`); the Bare encoding used
  by the library omits it.
* `section_kind = 3` (Batch): `varint field_count`, then `field_count` ×
  (`varint len`, UTF-8 bytes), in column order.

---

## 2. Fixed mode

Fixed mode means **the schema is known to both sides**: no field names, no
tags, no type bytes in the payload. Two layouts exist; the layout is chosen
per type (via `layout = aligned|packed` or the default rule in §2.6).

Body (after the 8-byte header):

```
varint  record_count
varint  stride          // bytes per record, >= 1
u8[]    records         // record_count * stride bytes
```

`stride` exists so that a reader which knows only some of the fields can skip
the rest ([schema evolution](#7-schema-evolution)).

### 2.1 Type codes (used only by the §1.2 layout descriptor)

| code | type | aligned width | packed width |
|-----:|------|--------------:|-------------:|
| 1 | `bool` | 1 | 1 |
| 2 | `i8` | 1 | 8 |
| 3 | `u8` | 8 | 8 |
| 4 | `i16` | 2 | 16 |
| 5 | `u16` | 2 | 16 |
| 6 | `i32` | 4 | 32 |
| 7 | `u32` | 4 | 32 |
| 8 | `i64` | 8 | 64 |
| 9 | `u64` | 8 | 64 |
| 10 | `f32` | 4 | 32 |
| 11 | `f64` | 8 | 64 |
| 12 | `str` | varint len + bytes | not allowed |
| 13 | `bytes` | varint len + bytes | not allowed |
| 14 | `Vec<T>` | varint len + `len` × `T` | not allowed |
| 15 | nested struct | §2.3 | not allowed |

### 2.2 `aligned` layout

**Algorithm.** Walk the fields in declaration order. Each field occupies its
natural width in bytes. The offset of field *i* is the sum of the widths of
fields `0..i`. There is **no padding and no alignment requirement**: field
offsets are byte-aligned but not necessarily word-aligned, which is what
makes the layout dense. `stride` = total width.

| type | bytes |
|------|-------|
| `bool` | 1 (`0x00` = false, `0x01` = true; any other value is rejected) |
| `i8`/`u8` | 1 |
| `i16`/`u16` | 2, LE two's complement |
| `i32`/`u32`, `f32` | 4, LE |
| `i64`/`u64`, `f64` | 8, LE |
| `str`, `bytes` | `varint len` + `len` bytes (UTF-8 validated for `str`) |
| `Vec<T>` | `varint len` + `len` × aligned `T` |
| nested struct | inline, recursively |

`#[slim(range = A..=B)]` is **accepted but has no effect** in `aligned` mode:
the full natural width is stored. This is lossless but not compressed; pick
`packed` when the range matters.

### 2.3 Nested structs in `aligned` mode

A nested struct is inlined at its parent offset, with **no length prefix and
no tag**. Recursion depth is bounded by `Limits::max_depth` (default 64);
a payload that nests deeper is rejected.

### 2.4 `packed` layout

**Algorithm.** Every field gets a *bit width*, fields are concatenated in
declaration order without gaps, and the resulting bit string is stored
LSB-first: logical bit *i* is bit `i % 8` (counting from the LSB) of byte
`i / 8`. `stride` = `ceil(total_bits / 8)`; the unused high bits of the last
byte are **zero on write and ignored on read**.

| type | bit width |
|------|----------:|
| `bool` | 1 (value 0 or 1; other values rejected) |
| `i8`/`u8` | 8 |
| `i16`/`u16` | 16 |
| `i32`/`u32`, `f32` | 32 |
| `i64`/`u64`, `f64` | 64 |
| with `range = A..=B` | `ceil(log2(B - A + 1))`, minimum 1 |
| with `bits = N` | `N`, exactly as declared |

`range` is **lossless**: the stored value is `value - A`, so decoding
rejects any stored value `> B - A`. `bits = N` is lossless for `bool` and for
unsigned integers that fit in `N` bits; on encode, a value that does not fit
returns `SlimError::ValueOutOfRange` (never a silent truncation). Signed
integers with `bits = N` are stored as two's complement in `N` bits; decoding
sign-extends, so only `-2^(N-1) ..= 2^(N-1) - 1` round-trips — the range check
is enforced on encode.

`packed` supports scalars only. `str`, `bytes`, `Vec<T>` and nested structs
are rejected with `SlimError::UnsupportedLayout` in this draft.

### 2.5 Worked hex example

Type: `{ rsrp: i16 range(-140..=-44), band: u8, speed: f32 }`, value
`{ rsrp: -103, band: 3, speed: 27.5 }`.

> **DECISION D-0009 (resolved): `rsrp` is `i16`, not `i8`.** The RSRP range
> `-140..=-44` (97 values) is the real telemetry range and is **not
> representable in `i8`** (`i8::MIN == -128`). Rather than narrowing the range
> to what `i8` can hold, the field keeps its full domain and widens to `i16`;
> `#[slim(range = -140..=-44)]` still packs it into **7 bits**, so a SlimPack
> `packed` payload is the same size as an `i8` one, while every competitor
> that stores raw `i16` grows by one byte per record. The comparison is
> therefore run against the *larger* competitor bytes.

`27.5f32` has bit pattern `0x41DC0000`. The `range` attribute covers 97
values, so it needs `ceil(log2(97)) = 7` bits.

**`aligned`** — widths 2 + 1 + 4 = 7 bytes (`range` has no effect here, §2.2):

```
99 ff 03 00 00 dc 41
^^^^ rsrp = -103 = 0xFF9D LE (two's complement)
    ^^ band  = 3
       ^^^^^^ speed = 0x41DC0000 LE
```

Full file for one record (Bare, no `EXT` section):

```
53 4C 4D 50   magic "SLMP"
01            version
01            mode = Fixed
00            flags (neither LOSSY nor EXT)
00            reserved
01            varint record_count = 1
07            varint stride = 7
99 ff 03 00 00 dc 41
```

**`packed`** — widths 7 (`range`) + 8 (`u8`) + 32 (`f32`) = 47 bits → 6 bytes
with 1 zero padding bit in the high position of byte 5. `rsrp` is stored as
`-103 - (-140) = 37 = 0b0100101`:

```
a5 01 00 00 ee 20
^^ byte 0 = bits 0..7  : rsrp bits 0..6 = 37 = 0b0100101, band bit 0 in bit 7
   ^^ byte 1 = bits 8..15: band bit 1 in bit 0, speed bit 0 in bit 7 (= 0)
      ^^ byte 2 = bits 16..23: speed bits 1..8        (all 0 here)
         ^^ byte 3 = bits 24..31: speed bits 9..16    (all 0 here)
            ^^ byte 4 = bits 32..39: speed bits 17..24 = 0xEE
               ^^ byte 5 = bits 40..47: speed bits 25..31 = 0b0100000 in bits 0..6,
                                    bit 7 is the zero padding bit
```

Verification of the interesting bytes: byte 0 is `37 | (3 & 1) << 7 =
0x25 | 0x80 = 0xA5`; `band`'s second bit lands in byte 1 bit 0, giving `0x01`.
`speed = 0x41DC0000` starts at bit 15, so its low 16 bits (`0x0000`) vanish
into bytes 1–3 and its top 16 bits (`0x41DC`) land in bytes 4–5 as `ee 20`
once shifted by the 15-bit offset.

**`packed` with `bits = 3` on `band`** — 7 + 3 + 32 = 42 bits → 6 bytes:

```
a5 01 00 70 07 01
```

### 2.6 Default layout choice

`aligned` is the default. `packed` must be requested explicitly
(`#[slim(layout = "packed")]`, or `layout = packed;` in `.slimschema`).
Rationale: `aligned` needs no shifts/masks per field, which is what W3
measures against FlatBuffers and rkyv.

### 2.7 Zero-copy views

`view::<T>(&[u8]) -> Result<View<T>, SlimError>` reads fields directly out of
the borrowed slice: no allocation, no copying. Two variants are measured
against competitors (§ Phase 3 protocol):

* **VERIFIED** — all bounds checks, reserved-bit checks and (for Batch)
  encoding validation run first; then field access is a pure load.
* **UNVERIFIED** — validation is skipped; only the per-field bounds check
  remains. Only compared against rkyv's `access_unchecked` and FlatBuffers'
  `root_as_*_unchecked`.

---

## 3. Batch mode (columnar)

Batch mode stores a table of `N` records of one schema as one column per
field. Body (after the 8-byte header):

```
varint  record_count
varint  column_count
column  × column_count
```

### 3.1 Column header

```
u8      encoding_id
u8      flags
varint  byte_len          // total bytes of (presence bitmap + payload)
u8[]    presence bitmap   // only if flags.HAS_PRESENCE
u8[]    payload           // the rest of byte_len bytes
```

Skipping a column costs one header read plus one `seek(byte_len)`, so any
column can be read without touching the others.

| bit | mask | name | meaning |
|----:|------|------|---------|
| 0 | `0x01` | `HAS_PRESENCE` | presence bitmap follows the header |
| 1 | `0x02` | `LOSSY` | this column is quantized (lossy) |
| 2–7 | `0xFC` | reserved | must be 0 |

**Presence bitmap**: `ceil(record_count / 8)` bytes, LSB-first, bit *i* = 1
means record *i* has a value, 0 means the field is absent. Trailing bits of
the last byte must be 0.

### 3.2 Encoding ids

Integers:

| id | name | params | payload |
|---:|------|--------|---------|
| `0x10` | `RAW_U8` | — | `n` bytes |
| `0x11` | `RAW_U16` | — | `n` × 2 bytes LE |
| `0x12` | `RAW_U32` | — | `n` × 4 bytes LE |
| `0x13` | `RAW_U64` | — | `n` × 8 bytes LE |
| `0x14` | `RAW_I8` | — | `n` × 1 byte, two's complement |
| `0x15` | `RAW_I16` | — | `n` × 2 bytes LE |
| `0x16` | `RAW_I32` | — | `n` × 4 bytes LE |
| `0x17` | `RAW_I64` | — | `n` × 8 bytes LE |
| `0x20` | `FOR` | `u8 bits` | `varint zigzag(min)` + `ceil(n*bits/8)` bytes of LSB-first bit-packed `v - min` |
| `0x21` | `DELTA_VARINT` | — | `varint zigzag(v0)` + `(n-1)` × `varint zigzag(v[i] - v[i-1])` |
| `0x22` | `DELTA2_VARINT` | — | `varint zigzag(v0)`, `varint zigzag(d0)`, then `(n-2)` × `varint zigzag(dd)` |
| `0x23` | `RLE` | — | `varint runs` + `runs` × (`varint zigzag(run_len)`, `varint zigzag(value)`) |
| `0x24` | `CONSTANT` | — | `varint zigzag(value)` |
| `0x25` | `DICT_PACK` | `u8 bits` | `varint dict_len`, `dict_len` × `varint zigzag(value)`, then bit-packed indices |

Floats (always bit-exact unless the column is explicitly lossy):

| id | name | params | payload |
|---:|------|--------|---------|
| `0x30` | `RAW_F32` | — | `n` × 4 bytes, IEEE-754 bit pattern LE |
| `0x31` | `RAW_F64` | — | `n` × 8 bytes, IEEE-754 bit pattern LE |
| `0x32` | `XOR_PREV` | `u8 width` (4 or 8) | first value raw (`width` bytes), then Gorilla blocks |
| `0x33` | `QUANT` | `f64 scale` | **LOSSY**: `varint zigzag(round(v / scale))` per value |

Strings:

| id | name | params | payload |
|---:|------|--------|---------|
| `0x40` | `BYTES` | — | `n` × (`varint len`, `len` UTF-8 bytes) |
| `0x41` | `DICT` | `u8 bits` | `varint dict_len`, `dict_len` × (`varint len`, bytes), then bit-packed indices |
| `0x42` | `FIXED` | `u8 width` | `n` × `width` bytes, no per-value length |

Bools:

| id | name | params | payload |
|---:|------|--------|---------|
| `0x50` | `BITPACK` | — | `ceil(n/8)` bytes, LSB-first |

`XOR_PREV` detail (Gorilla): after the first raw value, each subsequent value
is XORed with the previous **decoded** value. Per value the writer emits one
control byte `c`:

* `c = 0` → the XORed value is zero; nothing follows;
* `c > 0` → the meaningful window is `((c - 1) >> 4) + 1` bits wide, starting
  at bit position `((c - 1) & 0x0F) + 1`, followed by that many bits of the
  XORed value (LSB-first, bit *i* is bit `i % 8` of byte `i / 8`).

This encoding is **lossless/bit-exact**; it only exploits repeated bit
patterns. The leading/trailing zero runs are derived from the XOR word, and a
window of 64 bits covers the full width.

### 3.3 Effort levels

| effort | algorithm |
|--------|-----------|
| `Fast` | heuristic on a **sample of the first 64 values** (all values if fewer); encode once |
| `Balanced` | fully encode the **top 2 candidates** from the sample, keep the smaller |
| `Max` | try **every applicable** encoding over the whole column, keep the smallest |

Fast heuristic, in order (first match wins):

1. `N == 0` → `CONSTANT` with an empty payload;
2. all sample values equal → `CONSTANT`;
3. `max - min` fits in ≤ 8 bits and `N >= 32` → `FOR`;
4. all first differences fit in `[-2^14, 2^14]` → `DELTA_VARINT`;
5. all second differences fit in `[-2^12, 2^12]` → `DELTA2_VARINT`;
6. repeated adjacent values dominate (≥ 25% of runs) → `RLE`;
7. otherwise → the natural-width `RAW_*` encoding.

Strings: `DICT` if distinct/sample ≤ 0.5, else `BYTES`; `FIXED` is only
chosen by `Balanced`/`Max` when every sample string has the same length.
Floats: `XOR_PREV` if the average leading-zero run of the XORed words is ≥ 16,
else `RAW_F32`/`RAW_F64`.

The winner is chosen on **encoded byte length**, and ties (equal bytes) keep
the lower encoding id, so output is deterministic.

---

## 4. Dynamic mode

Dynamic mode is self-describing: values carry their own type.

Body:

```
varint  value_count
value   × value_count        // an array; a single root value uses count = 1
```

### 4.1 Value tags

Each value starts with one tag byte: `kind << 4 | form`.

| kind | name | forms |
|-----:|------|-------|
| 0 | `Null` | 0 |
| 1 | `Bool` | 0 = false, 1 = true (no payload) |
| 2 | `Int(i64)` | 0 = fixint, 1 = zigzag varint, 2 = two's-complement varint, 3 = raw `i64` |
| 3 | `UInt(u64)` | same four forms |
| 4 | `Float(f64)` | 0 = raw `f64`; **1 is reserved for a future f32 form and is rejected in v1** |
| 5 | `Str` | 0 = short (1 length byte), 1 = varint length, 2 = interned key index |
| 6 | `Bytes` | 0 = short (1 length byte), 1 = varint length |
| 7 | `Array` | 0 = varint count + elements, 1 = empty |
| 8 | `Map` | 0 = varint count + (key, value) pairs, 1 = empty |

* `fixint` (Int form 0): the next byte holds the value as `i8`; only used for
  `-128..=127`.
* varints are unsigned LEB128 (`u64`); `zigzag` is `(n << 1) ^ (n >> 63)` on
  `i64`.
* Int form 3 (`raw i64`) exists so that `i64::MIN`/`i64::MAX` are exactly
  representable in 9 bytes; all delta/delta-of-delta arithmetic is wrapping.
* Map keys are always `Str`; a key that exists in the header key table is
  written with form 2 (interned index), otherwise with form 1.
* **Duplicate keys in a map are an error on decode**
  (`SlimError::DuplicateKey`), because maps preserve insertion order as
  `Vec<(String, Value)>` and duplicates would be ambiguous.

### 4.2 JSON conversion rules

| JSON | SlimPack |
|------|----------|
| integer that fits `i64` | `Int` |
| integer in `i64::MAX+1 ..= u64::MAX` | `UInt` |
| integer outside `u64` | error `SlimError::NumberOutOfRange` |
| number with a fraction or exponent | `Float` |
| `true` / `false` | `Bool` |
| `null` | `Null` |
| string | `Str` |
| array | `Array` |
| object | `Map`, keys in **document order** |

Writing JSON back:

* `Int`/`UInt` are written without a decimal point.
* `Float` is written with Rust's shortest round-trip formatting; if the result
  has no fractional part and no exponent, `.0` is appended so it re-parses as
  a float.
* Non-finite floats (`NaN`, `±inf`) cannot be represented in JSON and are
  rejected with `SlimError::NonFiniteFloat`; SlimPack can still *encode* them
  (raw IEEE-754 bits) and round-trips them bit-exactly.
* Duplicate keys are preserved by `Value` (they are only an error when
  decoding a SlimPack map).

---

## 5. Integers: delta arithmetic

All delta encodings use wrapping arithmetic so that no operation can panic:

```
d1(i)   = v[i].wrapping_sub(v[i-1])
d2(i)   = d1(i).wrapping_sub(d1(i-1))
```

Decoding reverses them with the same wrapping operations, so any `i64`
sequence round-trips, including sequences that contain `i64::MIN`,
`i64::MAX` and both extremes in adjacent positions. This is covered by unit
tests at the extremes, not only by property tests.

---

## 6. Optional fields and presence

Batch mode represents absent values with a presence bitmap (§3.1); the value
bytes for absent records are *not stored* (the column payload is compacted).
Fixed mode v1 has no optional fields (see §7).

---

## 7. Schema evolution

Rules, all of them test-covered:

* **R1 — append only.** New fields may only be appended at the end of a
  struct. Reordering, removing or inserting in the middle is a breaking change.
* **R2 — version byte.** Any breaking change (reorder, remove, type change,
  width change, `aligned` ⇄ `packed`) **must** bump the version byte. A reader
  that does not know the version returns `SlimError::UnsupportedVersion`.
* **R3 — Batch readers are position-independent.** A Batch reader reads
  `column_count` and may decode a prefix of the columns; trailing columns it
  does not know are ignored. New optional fields are appended as new columns
  with a presence bitmap, so old readers keep working.
* **R4 — Fixed readers skip by stride.** Because the body stores `stride`, a
  reader that knows fewer fields still reads its own fields and ignores the
  trailing bytes of each record. If the stored `stride` is **smaller** than
  the reader's known stride, that is
  `SlimError::MissingField` (the writer is older than the reader).
* **R5 — reserved bits and unknown encodings are errors.** A reader never
  guesses: reserved flag bits that are set, an unknown `mode`, an unknown
  `encoding_id` or a malformed params section all return `SlimError`.
* **R6 — Fixed mode v1 has no optional fields.** Optionality needs a
  presence bitmap, and a per-record presence bitmap would break the fixed
  stride. Add optional fields in Batch mode.

---

## 8. `.slimschema`

A tiny text schema. The layout engine lives in the **core crate** and is used
by both the derive macro and `slim schema layout`, so they can never disagree.

```
struct Telemetry {
  rsrp:  i16 range(-140, -44);
  band:  u8;
  speed: f32;
}

struct Batch {
  layout = packed;
  records: Telemetry;
}
```

Grammar:

```
schema      := item*
item        := 'struct' IDENT '{' field* '}' | 'struct' IDENT '{' 'layout' '=' ('aligned'|'packed') ';' 'records' ':' IDENT ';'
field       := IDENT ':' type attr* ';'
attr        := 'range' '(' INT ',' INT ')'
             | 'bits' '(' INT ')'
             | 'quantize' '(' NUMBER ')'          // LOSSY, opt-in
             | 'optional'                         // Batch mode only
type        := 'bool' | 'i8'|'i16'|'i32'|'i64' | 'u8'|'u16'|'u32'|'u64' | 'f32'|'f64'
             | 'str' | 'bytes' | 'Vec' '<' type '>' | IDENT (nested struct)
```

`slim schema layout <file.slimschema>` prints the exact byte/bit layout: for
each field its offset (byte and bit), width in bits, and the encoding
attribute in force; plus the total `stride`.

---

## 9. Limits and safety

No length, count or size in a payload is ever used to allocate before it has
been checked against **both** the remaining input bytes and the configured
limits. `Limits` (all configurable):

| limit | default | meaning |
|-------|--------:|---------|
| `max_bytes` | 64 MiB | total payload size accepted |
| `max_records` | 10_000_000 | `record_count` / `value_count` |
| `max_depth` | 64 | nesting depth (arrays, maps, structs) |
| `max_len` | 16 MiB | string/bytes length, run length, dictionary entry count |
| `max_dictionary` | 1_000_000 | dictionary entries |
| `max_columns` | 4096 | `column_count` |

Errors (never panics, never hangs):

| variant | raised when |
|---------|-------------|
| `Truncated { needed, available }` | input shorter than a declared length |
| `LimitExceeded { limit, value, max }` | a declared count/length exceeds a limit |
| `ReservedBitsSet { mask }` | reserved flag bits are non-zero |
| `UnknownEncoding(u8)` | unknown `encoding_id` or `section_kind` |
| `ValueOutOfRange { value, min, max }` | a value violates `range`/`bits` |
| `DuplicateKey(String)` | duplicate map key on decode |
| `InvalidUtf8` | string payload is not valid UTF-8 |
| `MissingField` | stored stride smaller than the reader's stride |
| `NonFiniteFloat` | writing a non-finite float to JSON |
| `NumberOutOfRange` | JSON integer outside `u64` |
| `UnsupportedVersion(u8)` / `UnsupportedLayout(&'static str)` / `SizeOverflow` | see above |
| `TrailingBytes { extra }` | `decode` requires an exact-length payload |

---

## 10. Conformance vectors

`/spec/vectors/` holds one file per case:

* `<name>.slim` — the exact payload bytes;
* `<name>.json` — the human-readable description: mode, layout, the input
  value(s), and the expected decoded value(s).

Vectors are **generated from the implementation** and pinned by tests
(`crates/slimpack/tests/vectors.rs`). Editing a vector byte without bumping the
format version makes the test suite fail. Every mode is covered, plus
truncation, reserved-bit and limit violations (each expected to yield a
specific `SlimError` variant).

## 11. Versioning

`version` is a single byte. Readers accept the version they were written for
and reject everything else with `SlimError::UnsupportedVersion`. A new version
is required for: any layout change, any new mandatory section, any change to
an existing encoding's payload, and any semantic change to a field attribute.

---

## 12. CLI surface (for reference; implemented in Phase 6)

```
slim inspect <file> [--schema <file.slimschema>]
slim convert <in.json> -o <out.slim> [--effort fast|balanced|max]
slim convert <in.slim> -o <out.json>
slim bench <in.json> [--effort ...]
slim schema layout <file.slimschema>
```
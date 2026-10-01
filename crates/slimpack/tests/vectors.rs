//! Golden vector tests (`spec/vectors/`).
//!
//! Each `spec/vectors/<name>.json` is a human-readable description of one case;
//! the matching `spec/vectors/<name>.slim` holds the exact payload bytes.
//!
//! * `case = "fixed"` and `case = "dynamic"` cases are *generated* from the
//!   description, compared byte for byte with the `.slim` file, then decoded
//!   and compared against the description again.
//! * `case = "reject"` cases carry hand-written bytes and the exact
//!   `SlimError` variant a reader must report.
//!
//! To regenerate the `.slim` files after an intentional format change:
//!
//! ```text
//! SLIMPACK_BLESS=1 cargo test -p slimpack --test vectors
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use slimpack::dynamic::json::parse_json;
use slimpack::dynamic::{decode_body, encode_body, Value};
use slimpack::fixed::{
    decode_packed_record, decode_record, decode_records, encode_packed_record, encode_record,
    encode_records, read_body_header, write_body_header, Scalar,
};
use slimpack::header::{self, flags, mode, section, Header};
use slimpack::limits::Limits;
use slimpack::schema::{Attr, FieldSpec, Layout, LayoutKind, StructSpec, Type};
use slimpack::varint::{read_varint, write_varint};
use slimpack::Result;
use slimpack::SlimError;

fn spec_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec")
        .join(name)
}

fn vectors_dir() -> PathBuf {
    spec_path("vectors")
}

fn bless() -> bool {
    std::env::var_os("SLIMPACK_BLESS").is_some()
}

fn case_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(vectors_dir())
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", vectors_dir().display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no vector descriptions found");
    files
}

/// Reads one case description by name.
fn load(name: &str) -> Value {
    let path = vectors_dir().join(format!("{name}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    parse_json(&text, &Limits::default()).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Reads hex bytes, accepting both `00 ff` and `00ff` spellings.
fn read_hex(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        assert!(
            token.len() % 2 == 0,
            "{token:?} is not a whole number of hex bytes"
        );
        for pair in token.as_bytes().chunks(2) {
            let pair = std::str::from_utf8(pair).expect("ascii hex");
            out.push(
                u8::from_str_radix(pair, 16).unwrap_or_else(|_| panic!("not a hex byte: {pair:?}")),
            );
        }
    }
    out
}

fn hex_of(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

// --- accessors over a description `Value` -----------------------------------

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    match value {
        Value::Map(pairs) => pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, item)| item)
            .unwrap_or_else(|| panic!("missing key {name:?} in {value:?}")),
        _ => panic!("expected a JSON object, got {value:?}"),
    }
}

fn optional<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Map(pairs) => pairs.iter().find(|(key, _)| key == name).map(|(_, v)| v),
        _ => None,
    }
}

fn as_text(value: &Value) -> &str {
    match value {
        Value::Str(text) => text,
        _ => panic!("expected a string, got {value:?}"),
    }
}

fn as_array(value: &Value) -> &[Value] {
    match value {
        Value::Array(items) => items,
        _ => panic!("expected an array, got {value:?}"),
    }
}

fn as_i128(value: &Value) -> i128 {
    match value {
        Value::Int(v) => i128::from(*v),
        Value::UInt(v) => i128::from(*v),
        _ => panic!("expected an integer, got {value:?}"),
    }
}

fn layout_kind(desc: &Value) -> LayoutKind {
    match optional(desc, "layout").map(as_text) {
        Some("packed") => LayoutKind::Packed,
        Some("aligned") | None => LayoutKind::Aligned,
        Some(other) => panic!("unknown layout {other:?}"),
    }
}

// --- description -> schema and values --------------------------------------

fn parse_type(text: &str) -> Type {
    if let Some(inner) = text.strip_prefix("Vec<").and_then(|t| t.strip_suffix('>')) {
        return Type::Vec(Box::new(parse_type(inner)));
    }
    match text {
        "bool" => Type::Bool,
        "i8" => Type::I8,
        "i16" => Type::I16,
        "i32" => Type::I32,
        "i64" => Type::I64,
        "u8" => Type::U8,
        "u16" => Type::U16,
        "u32" => Type::U32,
        "u64" => Type::U64,
        "f32" => Type::F32,
        "f64" => Type::F64,
        "str" => Type::Str,
        "bytes" => Type::Bytes,
        other => panic!("unknown type {other:?}"),
    }
}

fn parse_field(desc: &Value) -> FieldSpec {
    let mut spec = FieldSpec::new(
        as_text(field(desc, "name")),
        parse_type(as_text(field(desc, "type"))),
    );
    if let Some(bounds) = optional(desc, "range") {
        let bounds: Vec<i128> = as_array(bounds).iter().map(as_i128).collect();
        let [min, max] = bounds.as_slice() else {
            panic!("range needs two bounds: {desc:?}");
        };
        spec = spec.with(Attr::Range {
            min: *min,
            max: *max,
        });
    }
    if let Some(bits) = optional(desc, "bits") {
        spec = spec.with(Attr::Bits(as_i128(bits) as u8));
    }
    spec
}

fn layout_of(desc: &Value, fields_key: &str) -> Result<Layout> {
    let specs: Vec<FieldSpec> = as_array(field(desc, fields_key))
        .iter()
        .map(parse_field)
        .collect();
    Layout::compute(&StructSpec::new("Vector", specs), layout_kind(desc))
}

/// Builds the `Scalar` a field description asks for.
///
/// `f32bits` / `f64bits` carry an exact IEEE-754 pattern, which is how the
/// vectors pin NaN payloads and `-0.0`. Values are leaked so that the
/// borrowing `Scalar` can outlive the description.
fn parse_scalar(desc: &Value) -> Scalar<'static> {
    // A bare literal is a Vec element: `[0, 1, 4095]`.
    if !matches!(desc, Value::Map(_)) {
        return match desc {
            Value::Bool(b) => Scalar::Bool(*b),
            Value::Int(v) => Scalar::Int(*v),
            Value::UInt(v) => Scalar::UInt(*v),
            Value::Float(v) => Scalar::Float(*v),
            Value::Str(text) => Scalar::Str(Box::leak(text.clone().into_boxed_str())),
            other => panic!("cannot build a Scalar from {other:?}"),
        };
    }
    if let Some(hex) = optional(desc, "hex") {
        let bytes: &'static [u8] = Box::leak(read_hex(as_text(hex)).into_boxed_slice());
        return Scalar::Bytes(bytes);
    }
    if let Some(bits) = optional(desc, "f32bits") {
        let bytes = read_hex(as_text(bits));
        return Scalar::F32Bits(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
    }
    if let Some(bits) = optional(desc, "f64bits") {
        let bytes = read_hex(as_text(bits));
        let mut array = [0u8; 8];
        array.copy_from_slice(&bytes);
        return Scalar::Float(f64::from_be_bytes(array));
    }
    if let Some(items) = optional(desc, "values") {
        return Scalar::Seq(as_array(items).iter().map(parse_scalar).collect());
    }
    match field(desc, "value") {
        Value::Bool(b) => Scalar::Bool(*b),
        Value::Int(v) => Scalar::Int(*v),
        Value::UInt(v) => Scalar::UInt(*v),
        Value::Float(v) => Scalar::Float(*v),
        Value::Str(text) => Scalar::Str(Box::leak(text.clone().into_boxed_str())),
        other => panic!("cannot build a Scalar from {other:?}"),
    }
}

/// Builds one `Scalar` per field, normalised to what a decoder returns.
fn parse_scalars(desc: &Value) -> Vec<Scalar<'static>> {
    as_array(field(desc, "fields"))
        .iter()
        .map(parse_field)
        .zip(as_array(field(desc, "fields")).iter().map(parse_scalar))
        .map(|(spec, value)| coerce_scalar(&spec.ty, value))
        .collect()
}

/// Normalises a description value to the form a decoder returns for `ty`:
/// unsigned fields decode to `UInt`, an `f32` field to `F32Bits`, and so on.
fn coerce_scalar(ty: &Type, value: Scalar<'static>) -> Scalar<'static> {
    match (ty, value.clone()) {
        (Type::Vec(inner), Scalar::Seq(items)) => Scalar::Seq(
            items
                .into_iter()
                .map(|item| coerce_scalar(inner, item))
                .collect(),
        ),
        (Type::Bool, Scalar::UInt(0)) => Scalar::Bool(false),
        (Type::Bool, Scalar::UInt(1)) => Scalar::Bool(true),
        (Type::U8 | Type::U16 | Type::U32 | Type::U64, Scalar::Int(v)) if v >= 0 => {
            Scalar::UInt(v as u64)
        }
        (Type::I8 | Type::I16 | Type::I32 | Type::I64, Scalar::UInt(v)) if v <= i64::MAX as u64 => {
            Scalar::Int(v as i64)
        }
        (Type::F32, Scalar::Float(v)) => Scalar::F32Bits(
            slimpack::fixed::to_f32_exact(v)
                .unwrap_or_else(|| panic!("{v} is not exactly an f32"))
                .to_bits(),
        ),
        _ => value,
    }
}

// --- description -> payload -------------------------------------------------

fn build_fixed(desc: &Value) -> Vec<u8> {
    let layout = layout_of(desc, "fields").expect("layout");
    let fields = parse_scalars(desc);
    let limits = Limits::default();
    let mut out = Vec::new();
    Header::new(mode::FIXED, 0).expect("header").write(&mut out);
    if layout.kind == LayoutKind::Packed {
        write_body_header(&mut out, 1, layout.stride());
        encode_packed_record(&layout, &fields, &mut out).expect("encode packed record");
    } else {
        encode_records(&layout, &[fields], &mut out, &limits).expect("encode aligned record");
    }
    out
}

fn build_dynamic(desc: &Value) -> Vec<u8> {
    let keys = parse_keys(desc);
    let values: Vec<Value> = as_array(field(desc, "values"))
        .iter()
        .map(parse_dynamic_value)
        .collect();
    let mut out = Vec::new();
    Header::new(mode::DYNAMIC, if keys.is_empty() { 0 } else { flags::EXT })
        .expect("header")
        .write(&mut out);
    if !keys.is_empty() {
        let mut payload = Vec::new();
        write_varint(&mut payload, keys.len() as u64);
        for key in &keys {
            write_varint(&mut payload, key.len() as u64);
            payload.extend_from_slice(key.as_bytes());
        }
        header::write_section(&mut out, section::KEYS, &payload);
    }
    encode_body(&values, &keys, &mut out).expect("encode dynamic body");
    out
}

fn parse_keys(desc: &Value) -> Vec<String> {
    match optional(desc, "keys") {
        Some(Value::Array(items)) => items.iter().map(|v| as_text(v).to_string()).collect(),
        _ => Vec::new(),
    }
}

/// JSON -> `Value`, with one description-only extension: `{"hex": "...",
/// "as": "bytes"}`, because JSON has no bytes type.
fn parse_dynamic_value(desc: &Value) -> Value {
    if let (Some(hex), Some(kind)) = (optional(desc, "hex"), optional(desc, "as").map(as_text)) {
        assert_eq!(kind, "bytes", "only the bytes extension is supported");
        return Value::Bytes(read_hex(as_text(hex)));
    }
    desc.clone()
}

// --- decoding helpers -------------------------------------------------------

fn key_table(body: &[u8]) -> Vec<String> {
    let (found, _) = header::read_section(body).expect("keys section");
    assert_eq!(found.kind, section::KEYS);
    let mut pos = 0;
    let count = read_varint(found.payload, &mut pos).expect("key count");
    (0..count)
        .map(|_| {
            let len = read_varint(found.payload, &mut pos).expect("key length") as usize;
            let text = std::str::from_utf8(&found.payload[pos..pos + len])
                .expect("utf-8 key")
                .to_owned();
            pos += len;
            text
        })
        .collect()
}

/// The `SlimError` variant name used by the reject vector descriptions.
fn error_name(error: &SlimError) -> &'static str {
    match error {
        SlimError::Truncated { .. } => "Truncated",
        SlimError::LimitExceeded { .. } => "LimitExceeded",
        SlimError::ReservedBitsSet { .. } => "ReservedBitsSet",
        SlimError::UnknownEncoding(_) => "UnknownEncoding",
        SlimError::ValueOutOfRange { .. } => "ValueOutOfRange",
        SlimError::DuplicateKey(_) => "DuplicateKey",
        SlimError::InvalidUtf8 => "InvalidUtf8",
        SlimError::MissingField => "MissingField",
        SlimError::NonFiniteFloat => "NonFiniteFloat",
        SlimError::NumberOutOfRange => "NumberOutOfRange",
        SlimError::UnsupportedVersion(_) => "UnsupportedVersion",
        SlimError::UnsupportedMode(_) => "UnsupportedMode",
        SlimError::InvalidMagic(_) => "InvalidMagic",
        SlimError::UnsupportedLayout(_) => "UnsupportedLayout",
        SlimError::SizeOverflow => "SizeOverflow",
        SlimError::TrailingBytes { .. } => "TrailingBytes",
        SlimError::InvalidBool(_) => "InvalidBool",
        SlimError::PaddingBitsSet { .. } => "PaddingBitsSet",
    }
}

/// Decodes one reject case: a fixed body when the description carries a
/// layout, otherwise a dynamic one.
fn decode_vector(desc: &Value, bytes: &[u8], limits: &Limits) -> Result<()> {
    let (head, body) = Header::read(bytes)?;
    match head.mode {
        mode::FIXED => {
            let layout = match optional(desc, "layout_fields") {
                Some(_) => layout_of(desc, "layout_fields")?,
                None => {
                    return Err(SlimError::UnsupportedLayout(
                        "reject vector has no layout_fields",
                    ))
                }
            };
            if layout.kind == LayoutKind::Packed {
                let (count, stride, start) = read_body_header(body, limits)?;
                let records = &body[start..];
                for index in 0..count {
                    let at = index * stride;
                    let record = records
                        .get(at..at + stride)
                        .ok_or_else(|| SlimError::truncated(at + stride, records.len()))?;
                    decode_packed_record(&layout, record)?;
                }
            } else {
                decode_records(&layout, body, limits)?;
            }
            Ok(())
        }
        mode::DYNAMIC => {
            let keys = if head.has_ext() {
                key_table(body)
            } else {
                Vec::new()
            };
            decode_body(body_for_dynamic(body, head.has_ext()), &keys, limits)?;
            Ok(())
        }
        other => Err(SlimError::UnsupportedMode(other)),
    }
}

/// The dynamic body starts after the header (and after the EXT section).
fn body_for_dynamic(body: &[u8], has_ext: bool) -> &[u8] {
    if has_ext {
        let (_, rest) = header::read_section(body).expect("section");
        rest
    } else {
        body
    }
}

// --- the tests --------------------------------------------------------------

#[test]
fn accepted_vectors_are_generated_then_decoded_back() {
    let limits = Limits::default();
    for json_path in case_files() {
        let name = json_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let desc = load(&name);
        let case = as_text(field(&desc, "case"));
        let slim_path = json_path.with_extension("slim");

        match case {
            "fixed" => {
                let generated = build_fixed(&desc);
                let stored = check_vector(stored_vector(&slim_path), &generated, &slim_path);
                let decoded = decode_fixed(&desc, &stored, &limits)
                    .unwrap_or_else(|e| panic!("{name}: decode failed: {e:?}"));
                let expected = parse_scalars(&desc);
                assert!(
                    scalar_eq(&decoded, &expected),
                    "{name}: decoded {decoded:?} differs from {expected:?}"
                );
            }
            "dynamic" => {
                let generated = build_dynamic(&desc);
                let stored = check_vector(stored_vector(&slim_path), &generated, &slim_path);
                let (head, body) = Header::read(&stored).expect("header");
                assert_eq!(head.mode, mode::DYNAMIC, "{name}");
                let keys = if head.has_ext() {
                    key_table(body)
                } else {
                    Vec::new()
                };
                let dynamic = body_for_dynamic(body, head.has_ext());
                let values = decode_body(dynamic, &keys, &limits)
                    .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
                let expected: Vec<Value> = as_array(field(&desc, "values"))
                    .iter()
                    .map(parse_dynamic_value)
                    .collect();
                assert_eq!(values, expected, "{name}: decoded values differ");
                let mut again = Vec::new();
                encode_body(&values, &keys, &mut again).expect("re-encode");
                assert_eq!(again, dynamic, "{name}: re-encoded body differs");
            }
            "reject" => continue,
            other => panic!("{name}: unknown case {other:?}"),
        }
    }
}

/// Loads the committed payload of an accepted vector, regenerating it when
/// `SLIMPACK_BLESS=1` asks for it.
fn stored_vector(path: &Path) -> Vec<u8> {
    match fs::read(path) {
        Ok(stored) => stored,
        Err(_) if bless() => Vec::new(),
        Err(e) => panic!("{}: {e} (run with SLIMPACK_BLESS=1)", path.display()),
    }
}

/// Compares the generated payload with the stored vector, writing the file when
/// `SLIMPACK_BLESS=1` asks for a regeneration.
fn check_vector(stored: Vec<u8>, generated: &[u8], path: &Path) -> Vec<u8> {
    if stored == generated {
        return stored;
    }
    assert!(
        bless(),
        "{} is stale: stored {} but generated {} (run with SLIMPACK_BLESS=1)",
        path.display(),
        hex_of(&stored),
        hex_of(generated)
    );
    fs::write(path, generated).expect("write vector");
    generated.to_vec()
}

/// Bit-exact comparison: `f64` NaN never equals itself with `==`, and lossless
/// means bit-exact.
fn scalar_eq(left: &[Scalar<'_>], right: &[Scalar<'_>]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| match (a, b) {
            (Scalar::Float(x), Scalar::Float(y)) => x.to_bits() == y.to_bits(),
            (Scalar::F32Bits(x), Scalar::F32Bits(y)) => x == y,
            _ => a == b,
        })
}

/// Decodes the single record of a `fixed` vector.
fn decode_fixed(desc: &Value, bytes: &[u8], limits: &Limits) -> Result<Vec<Scalar<'static>>> {
    let bytes: &'static [u8] = Box::leak(bytes.to_vec().into_boxed_slice());
    let layout = layout_of(desc, "fields")?;
    let (head, body) = Header::read(bytes)?;
    debug_assert_eq!(head.mode, mode::FIXED);
    match layout.kind {
        LayoutKind::Aligned => Ok(decode_records(&layout, body, limits)?
            .into_iter()
            .next()
            .unwrap_or_default()),
        LayoutKind::Packed => {
            let (count, stride, start) = read_body_header(body, limits)?;
            let records = &body[start..];
            let mut out = Vec::new();
            for index in 0..count {
                let at = index * stride;
                let record = records
                    .get(at..at + stride)
                    .ok_or_else(|| SlimError::truncated(at + stride, records.len()))?;
                out.extend(decode_packed_record(&layout, record)?);
            }
            Ok(out)
        }
    }
}

#[test]
fn reject_vectors_report_the_exact_error() {
    let limits = Limits::default();
    for json_path in case_files() {
        let name = json_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let desc = load(&name);
        if as_text(field(&desc, "case")) != "reject" {
            continue;
        }
        let bytes = read_hex(as_text(field(&desc, "bytes")));
        let error = decode_vector(&desc, &bytes, &limits)
            .err()
            .unwrap_or_else(|| panic!("{name}: expected an error, decoded cleanly"));
        assert_eq!(
            error_name(&error),
            as_text(field(&desc, "expect_error")),
            "{name}: wrong error: {error:?}"
        );
    }
}

#[test]
fn every_vector_file_has_a_description_and_the_reject_suite_is_populated() {
    let mut reject = 0;
    let mut accepted = 0;
    for path in case_files() {
        let desc = load(&path.file_stem().unwrap().to_string_lossy());
        match as_text(field(&desc, "case")) {
            "reject" => reject += 1,
            "fixed" | "dynamic" => accepted += 1,
            other => panic!("{}: unknown case {other:?}", path.display()),
        }
        assert!(
            optional(&desc, "note").is_some(),
            "{}: every vector needs a note",
            path.display()
        );
    }
    assert!(reject >= 15, "reject suite shrank to {reject}");
    assert!(accepted >= 8, "accepted suite shrank to {accepted}");
}

/// The worked example in `spec/SPEC.md` §2.5 must stay exactly as written.
///
/// The hex comes out of the spec text, so the specification and the
/// implementation cannot drift apart without this test failing.
#[test]
fn spec_2_5_hex_examples_are_exact() {
    let section_text = spec_2_5();
    // Every line made only of hex bytes; the full-file block repeats the
    // aligned record, so distinct lines are the three worked examples.
    let mut hex_lines: Vec<Vec<u8>> = Vec::new();
    for line in section_text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let pure = tokens
            .iter()
            .all(|token| token.len() == 2 && token.chars().all(|c| c.is_ascii_hexdigit()));
        if pure && !hex_lines.contains(&read_hex(line)) {
            hex_lines.push(read_hex(line));
        }
    }
    assert_eq!(
        hex_lines.len(),
        3,
        "expected the aligned, packed and bits=3 example lines in SPEC 2.5"
    );

    let values = vec![
        Scalar::Int(-103),
        Scalar::UInt(3),
        Scalar::F32Bits(0x41DC_0000),
    ];
    let telemetry = |band: Option<Attr>| {
        let band_spec = match band {
            Some(attr) => FieldSpec::new("band", Type::U8).with(attr),
            None => FieldSpec::new("band", Type::U8),
        };
        StructSpec::new(
            "Telemetry",
            vec![
                FieldSpec::new("rsrp", Type::I16).with(Attr::Range {
                    min: -140,
                    max: -44,
                }),
                band_spec,
                FieldSpec::new("speed", Type::F32),
            ],
        )
    };

    // 1. aligned: a range does not shrink an aligned field, so 2 + 1 + 4.
    let aligned = Layout::compute(&telemetry(None), LayoutKind::Aligned).unwrap();
    assert_eq!(aligned.stride(), 7);
    let mut encoded = Vec::new();
    encode_record(&aligned, &values, &mut encoded, &Limits::default()).unwrap();
    assert_eq!(encoded, hex_lines[0], "aligned example in SPEC 2.5");
    assert_eq!(
        decode_record(&aligned, &encoded, &Limits::default()).unwrap(),
        values
    );

    // 2. packed natural widths: 7 + 8 + 32 = 47 bits -> 6 bytes.
    let packed = Layout::compute(&telemetry(None), LayoutKind::Packed).unwrap();
    assert_eq!(packed.stride(), 6);
    let mut encoded = Vec::new();
    encode_packed_record(&packed, &values, &mut encoded).unwrap();
    assert_eq!(encoded, hex_lines[1], "packed example in SPEC 2.5");
    assert_eq!(decode_packed_record(&packed, &encoded).unwrap(), values);

    // 3. packed with bits = 3 on band: 7 + 3 + 32 = 42 bits -> 6 bytes.
    let bits3 = Layout::compute(&telemetry(Some(Attr::Bits(3))), LayoutKind::Packed).unwrap();
    assert_eq!(bits3.stride(), 6);
    let mut encoded = Vec::new();
    encode_packed_record(&bits3, &values, &mut encoded).unwrap();
    assert_eq!(encoded, hex_lines[2], "bits = 3 example in SPEC 2.5");
    assert_eq!(decode_packed_record(&bits3, &encoded).unwrap(), values);
}

/// The full-file block in SPEC 2.5 (header included) is `spec_2_5_aligned.slim`.
#[test]
fn spec_2_5_full_file_is_the_aligned_vector() {
    // The block introduced by "Full file for one record": each line contributes
    // its leading run of hex bytes, then stops at the annotation.
    let text = spec_2_5();
    let start = text
        .lines()
        .position(|line| line.contains("Full file for one record"))
        .expect("full-file block in SPEC 2.5");
    let mut full: Vec<u8> = Vec::new();
    for line in text.lines().skip(start + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("```") {
            continue; // the blank line and fence before the block
        }
        let run = leading_hex(line);
        if run.is_empty() {
            break;
        }
        full.extend(run);
    }
    // 8 header + 1 record_count + 1 stride + 7 record bytes.
    assert_eq!(full.len(), 17, "got {} bytes", hex_of(&full));
    let stored = fs::read(vectors_dir().join("spec_2_5_aligned.slim")).expect("vector");
    assert_eq!(
        stored, full,
        "spec_2_5_aligned.slim must be the spec's file"
    );
}

/// The hex bytes at the start of `line`, stopping at the first non-hex token.
fn leading_hex(line: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for token in line.split_whitespace() {
        if token.len() == 2 && token.chars().all(|c| c.is_ascii_hexdigit()) {
            out.push(u8::from_str_radix(token, 16).expect("hex"));
        } else {
            break;
        }
    }
    out
}

fn spec_2_5() -> String {
    let spec = fs::read_to_string(spec_path("SPEC.md")).expect("SPEC.md");
    let rest = spec.split("## 2.5").nth(1).expect("SPEC 2.5 section");
    rest.split("## 2.6")
        .next()
        .expect("SPEC 2.5 body")
        .to_owned()
}

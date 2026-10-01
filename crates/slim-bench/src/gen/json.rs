//! Deterministic, reviewable JSON rendering for the dataset generator.
//!
//! Correctness comes from `serde_json` (already a dependency of this
//! competitor crate); only the *layout* is ours: one compact JSON value per
//! line inside a top-level array, so the committed dataset files stay
//! diff-friendly and every record is on its own line.
//!
//! Floats are written by `serde_json`/`ryu` in shortest round-trip form, which
//! `floats_round_trip_bit_exactly` pins down.

use serde::Serialize;

/// Escapes a string as a JSON string literal (including the quotes).
pub fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Renders a single value as compact JSON.
pub fn compact<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).expect("dataset JSON value")
}

/// Renders `items` as a JSON array with one compact value per line.
pub fn lines_of<T: Serialize>(items: &[T]) -> String {
    let mut out = String::with_capacity(items.len() * 64 + 2);
    out.push_str("[\n");
    for (i, item) in items.iter().enumerate() {
        out.push_str("  ");
        out.push_str(&compact(item));
        if i + 1 != items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("]\n");
    out
}

/// Renders `items` as a pretty JSON array (used for the small datasets `a`
/// and `d`).
pub fn pretty<T: Serialize>(value: &T) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("dataset JSON value");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn strings_are_escaped_and_parse_back() {
        let text = json_string("a\"b\\c\nd\te\u{1}");
        assert_eq!(text, "\"a\\\"b\\\\c\\nd\\te\\u0001\"");
        let parsed: String = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, "a\"b\\c\nd\te\u{1}");
    }

    #[test]
    fn floats_round_trip_bit_exactly() {
        for bits in [
            0x3dcccccd_u32, // 0.1
            0x42280000,     // 42.0
            0x3f800000,     // 1.0
            0xc0490fdb,     // -3.1415927
            0x00000000,     // 0.0
            0x80000000,     // -0.0
            0x7f7fffff,     // f32::MAX
            0x00000001,     // smallest subnormal
        ] {
            let value = f32::from_bits(bits);
            let text = compact(&value);
            let back: f32 = serde_json::from_str(&text).unwrap();
            assert_eq!(back.to_bits(), bits, "text was {text}");
        }
    }

    #[test]
    fn lines_of_renders_one_value_per_line() {
        let text = lines_of(&[1u32, 2, 3]);
        assert_eq!(text, "[\n  1,\n  2,\n  3\n]\n");
        let parsed: Vec<u32> = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, vec![1, 2, 3]);
        assert_eq!(text.lines().count(), 5);
    }

    #[test]
    fn pretty_ends_with_a_newline() {
        let text = pretty(&serde_json::json!({"a": 1}));
        assert!(text.ends_with("}\n"));
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["a"], 1);
    }
}

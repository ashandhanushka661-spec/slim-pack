//! Thin, uniform wrappers around the serde-shaped competitors.
//!
//! Every wrapper is generic over `T: Serialize` / `DeserializeOwned` so the
//! benchmarks can hand the *same typed struct* to every codec. Each codec
//! gets its best realistic configuration:
//!
//! | wrapper              | configuration                            |
//! |----------------------|------------------------------------------|
//! | `json_*`             | `serde_json` compact output, typed struct |
//! | `msgpack_array_*`    | `rmp-serde` default (structs as arrays)  |
//! | `msgpack_map_*`      | `rmp-serde` `with_struct_map` (named)    |
//! | `postcard_*`         | `postcard` defaults                      |
//! | `bincode_*`          | `bincode` `standard()` configuration     |
//! | `bitcode_*`          | `bitcode` native compact derive API      |
//!
//! FlatBuffers (`flatc`-generated), rkyv and the compression helpers live in
//! `super::flat`, `super::rkyv_codec` and `super::compress`.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Encodes with `serde_json::to_vec`.
pub fn json_to_vec<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("serde_json encode")
}

/// Decodes with `serde_json::from_slice`.
pub fn json_from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(bytes)
}

/// Encodes with `rmp-serde` using its default (compact array) struct form.
pub fn msgpack_array_to_vec<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    rmp_serde::to_vec(value).expect("rmp-serde encode")
}

/// Decodes an `rmp-serde` array-form payload.
pub fn msgpack_array_from_slice<T: DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, rmp_serde::decode::Error> {
    rmp_serde::from_slice(bytes)
}

/// Encodes with `rmp-serde` forcing the named-map (struct as map) form.
pub fn msgpack_map_to_vec<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut ser = rmp_serde::Serializer::new(&mut buf).with_struct_map();
    value.serialize(&mut ser).expect("rmp-serde named encode");
    buf
}

/// Decodes an `rmp-serde` named-map payload.
pub fn msgpack_map_from_slice<T: DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, rmp_serde::decode::Error> {
    rmp_serde::from_slice(bytes)
}

/// Encodes with `postcard::to_allocvec`.
pub fn postcard_to_vec<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).expect("postcard encode")
}

/// Decodes with `postcard::from_bytes`.
pub fn postcard_from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

/// Encodes with `bincode`'s standard configuration.
///
/// bincode 2 has no serde integration, so it gets its own `Encode` derive --
/// the fairest configuration it can have.
pub fn bincode_to_vec<T: bincode::Encode + ?Sized>(value: &T) -> Vec<u8> {
    bincode::encode_to_vec(value, bincode::config::standard()).expect("bincode encode")
}

/// Decodes with `bincode`'s standard configuration.
pub fn bincode_from_slice<T: bincode::Decode<()>>(
    bytes: &[u8],
) -> Result<T, bincode::error::DecodeError> {
    let (value, _read) = bincode::decode_from_slice(bytes, bincode::config::standard())?;
    Ok(value)
}

/// Encodes with `bitcode`'s compact native derive API.
pub fn bitcode_to_vec<T: bitcode::Encode + ?Sized>(value: &T) -> Vec<u8> {
    bitcode::encode(value)
}

/// Decodes with `bitcode`'s compact native derive API.
pub fn bitcode_from_slice<'a, T: bitcode::Decode<'a>>(
    bytes: &'a [u8],
) -> Result<T, bitcode::Error> {
    bitcode::decode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::TelemetrySmall;

    fn sample() -> TelemetrySmall {
        TelemetrySmall {
            rsrp: -97,
            band: 3,
            speed: 42.5,
        }
    }

    #[test]
    fn serde_family_round_trips() {
        assert_eq!(
            json_from_slice::<TelemetrySmall>(&json_to_vec(&sample())).unwrap(),
            sample()
        );
        assert_eq!(
            msgpack_array_from_slice::<TelemetrySmall>(&msgpack_array_to_vec(&sample())).unwrap(),
            sample()
        );
        assert_eq!(
            msgpack_map_from_slice::<TelemetrySmall>(&msgpack_map_to_vec(&sample())).unwrap(),
            sample()
        );
        assert_eq!(
            postcard_from_slice::<TelemetrySmall>(&postcard_to_vec(&sample())).unwrap(),
            sample()
        );
        assert_eq!(
            bincode_from_slice::<TelemetrySmall>(&bincode_to_vec(&sample())).unwrap(),
            sample()
        );
        assert_eq!(
            bitcode_from_slice::<TelemetrySmall>(&bitcode_to_vec(&sample())).unwrap(),
            sample()
        );
    }

    #[test]
    fn msgpack_forms_differ_in_bytes() {
        let array = msgpack_array_to_vec(&sample());
        let map = msgpack_map_to_vec(&sample());
        assert_ne!(array, map);
        // Only the named form carries the field names.
        let as_text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        assert!(as_text(&map).contains("rsrp") && as_text(&map).contains("speed"));
        assert!(!as_text(&array).contains("rsrp"));
        // The named form is strictly larger: it adds three keys.
        assert!(map.len() > array.len());
    }

    #[test]
    fn every_payload_is_non_empty_and_bounded() {
        let sample = sample();
        assert!(!json_to_vec(&sample).is_empty());
        assert!(!postcard_to_vec(&sample).is_empty());
        assert!(!bincode_to_vec(&sample).is_empty());
        assert!(!bitcode_to_vec(&sample).is_empty());
    }
}

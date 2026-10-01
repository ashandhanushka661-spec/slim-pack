//! Fixed mode: `aligned` and `packed` record codecs (`spec/SPEC.md` §2).

mod scalar;

mod aligned;
mod packed;

pub use aligned::{
    decode_record, decode_records, encode_record, encode_records, read_body_header,
    write_body_header,
};
pub use packed::{
    decode_record as decode_packed_record, encode_record as encode_packed_record, read_bits,
    sign_extend, write_bits,
};
pub use scalar::{
    read_bool, read_int, read_uint, signed_domain, to_f32_exact, unsigned_domain, write_bool,
    write_int, write_uint, Scalar,
};

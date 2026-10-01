//! Schema model and layout engine (`spec/SPEC.md` §2, §8).

mod layout;
mod model;

pub use layout::{
    bits_for_values, is_lossy, range_width, signed_bits_domain, FieldLayout, Layout, LayoutKind,
};
pub use model::{Attr, FieldSpec, StructSpec, Type};

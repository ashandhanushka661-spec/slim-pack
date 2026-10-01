//! `#[derive(SlimPack)]` for the SlimPack format.
//!
//! Status: Phase 0 skeleton. Phase 5 implements the macro against the shared
//! layout engine in the core crate, so the derive output and the CLI's
//! `slim schema layout` command can never disagree about a byte layout.

use proc_macro::TokenStream;

/// Derives `SlimPack` for a struct.
///
/// Status: not implemented yet (Phase 5). The accepted attributes are
/// `range`, `bits` and `quantize`.
#[proc_macro_derive(SlimPack, attributes(slim))]
pub fn derive_slim_pack(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}

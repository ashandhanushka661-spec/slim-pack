//! `#[derive(SlimPack)]` for the SlimPack format.
//!
//! The derive generates the shared `StructSpec` + `FieldSpec` tree that the
//! library uses for the layout engine, so the same schema model is shared by the
//! macro, the core crate and the CLI.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, punctuated::Punctuated, Error, DeriveInput, Expr, ExprLit, Lit, Meta,
    Token, Type as SynType,
};

/// Derives `SlimPack` for a struct.
#[proc_macro_derive(SlimPack, attributes(slim))]
pub fn derive_slim_pack(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident.clone();
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => {
                return Error::new_spanned(input, "SlimPack can only be derived for named structs")
                    .to_compile_error()
                    .into();
            }
        },
        _ => {
            return Error::new_spanned(input, "SlimPack can only be derived for named structs")
                .to_compile_error()
                .into();
        }
    };

    let field_specs = fields.iter().map(|field| {
        let name = field.ident.as_ref().unwrap().to_string();
        let ty = rust_type_to_schema_tokens(&field.ty);
        let attrs = field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("slim"))
            .flat_map(|attr| parse_slim_attrs(attr).into_iter())
            .collect::<Vec<_>>();
        quote! {
            ::slimpack::FieldSpec::new(#name, #ty)#(.with(#attrs))*
        }
    });

    let expanded = quote! {
        impl ::slimpack::SlimPack for #ident {
            fn slim_schema() -> ::slimpack::StructSpec {
                ::slimpack::StructSpec::new(stringify!(#ident), vec![#(#field_specs),*])
            }
        }
    };
    expanded.into()
}

#[derive(Default)]
struct SlimArgs {
    items: Vec<proc_macro2::TokenStream>,
}

impl Parse for SlimArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let parsed = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        let mut items = Vec::with_capacity(parsed.len());
        for meta in parsed {
            items.push(parse_attr_meta(&meta)?);
        }
        Ok(Self { items })
    }
}

fn parse_slim_attrs(attr: &syn::Attribute) -> Vec<proc_macro2::TokenStream> {
    let list = match &attr.meta {
        Meta::List(list) => &list.tokens,
        _ => return Vec::new(),
    };
    match syn::parse2::<SlimArgs>(list.clone()) {
        Ok(args) => args.items,
        Err(_) => Vec::new(),
    }
}

fn parse_attr_meta(meta: &Meta) -> syn::Result<proc_macro2::TokenStream> {
    match meta {
        Meta::NameValue(nv) if nv.path.is_ident("range") => {
            let (min, max) = parse_range_expr(&nv.value)?;
            Ok(quote! { ::slimpack::Attr::Range { min: #min, max: #max } })
        }
        Meta::NameValue(nv) if nv.path.is_ident("bits") => {
            let bits = parse_u8_expr(&nv.value)?;
            Ok(quote! { ::slimpack::Attr::Bits(#bits) })
        }
        Meta::NameValue(nv) if nv.path.is_ident("quantize") => {
            let quantize = parse_f64_expr(&nv.value)?;
            Ok(quote! { ::slimpack::Attr::Quantize(#quantize) })
        }
        Meta::Path(path) if path.is_ident("optional") => {
            Ok(quote! { ::slimpack::Attr::Optional })
        }
        other => Err(Error::new_spanned(other, "unsupported slim attribute")),
    }
}

fn parse_range_expr(expr: &Expr) -> syn::Result<(i128, i128)> {
    let range = match expr {
        Expr::Range(range) => range,
        other => return Err(Error::new_spanned(other, "range must use a Rust range expression")),
    };
    let start = range.start.as_ref().ok_or_else(|| Error::new_spanned(range, "range requires a start value"))?;
    let end = range.end.as_ref().ok_or_else(|| Error::new_spanned(range, "range requires an end value"))?;
    let min = expr_to_i128(start)?;
    let max = expr_to_i128(end)?;
    Ok((min, max))
}

fn parse_u8_expr(expr: &Expr) -> syn::Result<u8> {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Int(value), .. }) => Ok(value.base10_parse::<u8>()?),
        other => Err(Error::new_spanned(other, "bits must be an integer literal")),
    }
}

fn parse_f64_expr(expr: &Expr) -> syn::Result<f64> {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Float(value), .. }) => Ok(value.base10_parse::<f64>()?),
        other => Err(Error::new_spanned(other, "quantize must be a float literal")),
    }
}

fn expr_to_i128(expr: &Expr) -> syn::Result<i128> {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Int(value), .. }) => Ok(value.base10_parse::<i128>()?),
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => Ok(-(expr_to_i128(&unary.expr)?)),
        other => Err(Error::new_spanned(other, "range endpoint must be an integer literal")),
    }
}

fn rust_type_to_schema_tokens(ty: &SynType) -> proc_macro2::TokenStream {
    match ty {
        SynType::Path(path) => {
            let ident = path.path.segments.last().unwrap().ident.to_string();
            match ident.as_str() {
                "bool" => quote!(::slimpack::Type::Bool),
                "i8" => quote!(::slimpack::Type::I8),
                "u8" => quote!(::slimpack::Type::U8),
                "i16" => quote!(::slimpack::Type::I16),
                "u16" => quote!(::slimpack::Type::U16),
                "i32" => quote!(::slimpack::Type::I32),
                "u32" => quote!(::slimpack::Type::U32),
                "i64" => quote!(::slimpack::Type::I64),
                "u64" => quote!(::slimpack::Type::U64),
                "f32" => quote!(::slimpack::Type::F32),
                "f64" => quote!(::slimpack::Type::F64),
                "str" => quote!(::slimpack::Type::Str),
                "String" => quote!(::slimpack::Type::Str),
                "Vec" => {
                    let inner = path.path.segments.last().unwrap();
                    if let syn::PathArguments::AngleBracketed(args) = &inner.arguments {
                        let first = args.args.first().unwrap();
                        if let syn::GenericArgument::Type(inner_ty) = first {
                            let inner_tokens = rust_type_to_schema_tokens(inner_ty);
                            return quote!(::slimpack::Type::Vec(Box::new(#inner_tokens)));
                        }
                    }
                    quote!(::slimpack::Type::Bytes)
                }
                _ => quote!(::slimpack::Type::Struct(#ident.to_owned())),
            }
        }
        SynType::Reference(reference) => {
            if matches!(reference.elem.as_ref(), SynType::Slice(_)) {
                quote!(::slimpack::Type::Bytes)
            } else {
                rust_type_to_schema_tokens(reference.elem.as_ref())
            }
        }
        SynType::Slice(slice) => {
            let inner = rust_type_to_schema_tokens(&slice.elem);
            match *slice.elem.clone() {
                SynType::Path(inner_path) if inner_path.path.is_ident("u8") => {
                    quote!(::slimpack::Type::Bytes)
                }
                _ => quote!(::slimpack::Type::Vec(Box::new(#inner))),
            }
        }
        _ => quote!(::slimpack::Type::Struct("unknown".to_owned())),
    }
}

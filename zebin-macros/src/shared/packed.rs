use super::FieldSpec;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Field, Result, Type, spanned::Spanned};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackedElementKind {
    Bool,
    U8,
}

pub fn packed_element_kind(ty: &Type) -> Option<PackedElementKind> {
    match ty {
        Type::Path(path) => {
            let segment = path.path.segments.last()?;
            let ident = segment.ident.to_string();
            if ident != "Vec" {
                return None;
            }
            let inner = match &segment.arguments {
                syn::PathArguments::AngleBracketed(args) => {
                    args.args.iter().find_map(|arg| match arg {
                        syn::GenericArgument::Type(inner) => Some(inner),
                        _ => None,
                    })?
                }
                _ => return None,
            };
            match inner {
                Type::Path(inner_path) if inner_path.path.is_ident("bool") => {
                    Some(PackedElementKind::Bool)
                }
                Type::Path(inner_path) if inner_path.path.is_ident("u8") => {
                    Some(PackedElementKind::U8)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn packed_type_name(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

fn packed_error(field: &Field, detail: &str) -> syn::Error {
    syn::Error::new_spanned(
        &field.ty,
        format!("{detail}；当前字段类型是 `{}`", packed_type_name(&field.ty)),
    )
}

pub fn parse_packed_bits(field: &Field) -> Result<Option<u8>> {
    let mut packed_bits: Option<u8> = None;
    let kind = packed_element_kind(&field.ty);

    for attr in &field.attrs {
        if !attr.path().is_ident("zebin") {
            continue;
        }
        let tokens = attr.meta.require_list()?.tokens.to_string();
        if !tokens.contains("packed") && !tokens.contains("bits") {
            continue;
        }

        let explicit = super::attrs::parse_uint_after_token(&tokens, "packed")
            .or_else(|| super::attrs::parse_uint_after_token(&tokens, "bits"));

        let bits = match explicit {
            Some(bits) => bits,
            None => match kind {
                Some(PackedElementKind::Bool) => 1,
                Some(PackedElementKind::U8) => {
                    return Err(syn::Error::new(
                        field.span(),
                        "u8 packed 字段需要显式提供 bits",
                    ));
                }
                None => {
                    return Err(packed_error(
                        field,
                        "packed 只能用于 `Vec<bool>` 或 `Vec<u8>`",
                    ));
                }
            },
        };

        let bits = u8::try_from(bits)
            .map_err(|_| syn::Error::new(field.span(), "packed bits exceeds u8 range"))?;
        packed_bits = Some(bits);
    }

    if let Some(bits) = packed_bits {
        match kind {
            Some(PackedElementKind::Bool) if bits != 1 => {
                return Err(syn::Error::new(
                    field.span(),
                    "bool packed 字段只能使用 1 bit",
                ));
            }
            Some(PackedElementKind::U8) if bits == 0 || bits > 8 => {
                return Err(syn::Error::new(
                    field.span(),
                    "u8 packed 字段的 bits 必须在 1..=8",
                ));
            }
            Some(_) => {}
            None => {
                return Err(packed_error(
                    field,
                    "packed 只能用于 `Vec<bool>` 或 `Vec<u8>`",
                ));
            }
        }
    }

    Ok(packed_bits)
}

pub fn packed_info(field: &FieldSpec<'_>) -> Option<(PackedElementKind, u8)> {
    let bits = field.packed_bits?;
    let kind = packed_element_kind(field.ty)?;
    Some((kind, bits))
}

pub fn packed_wrapper_type(field: &FieldSpec<'_>) -> Option<TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! { zebin::PackedSlice<'a, bool, 1> },
        PackedElementKind::U8 => quote! { zebin::PackedSlice<'a, u8, #bits> },
    })
}

pub fn packed_wrapper_type_expr(field: &FieldSpec<'_>) -> Option<TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! { zebin::PackedSlice<'_, bool, 1> },
        PackedElementKind::U8 => quote! { zebin::PackedSlice<'_, u8, #bits> },
    })
}

pub fn packed_archived_type(field: &FieldSpec<'_>) -> Option<TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! { zebin::ArchivedPackedBoolSlice },
        PackedElementKind::U8 => quote! { zebin::ArchivedPackedU8Slice<#bits> },
    })
}

pub fn packed_begin_expr(field: &FieldSpec<'_>, value: TokenStream) -> Option<TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! {
            zebin::PackedSequenceState::new_bool(#value.as_ref())
        },
        PackedElementKind::U8 => quote! {
            zebin::PackedSequenceState::new_u8(#value.as_ref(), #bits)
        },
    })
}

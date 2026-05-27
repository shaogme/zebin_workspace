use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, Member};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, field_user_ident, field_view_type, has_schema, input_member,
    parse_item, variant_field_name, view_name,
};

fn view_member(record: &RecordSpec<'_>, index: usize) -> Member {
    match record.style {
        RecordStyle::Named => Member::Named(field_user_ident(record, index)),
        RecordStyle::Unnamed => {
            let active_index = record.fields[..index].iter().filter(|f| !f.skip).count();
            Member::Unnamed(syn::Index::from(active_index))
        }
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

fn deserialize_field_expr(
    record: &RecordSpec<'_>,
    index: usize,
    source: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let field = &record.fields[index];
    if field.skip {
        return quote! { ::core::default::Default::default() };
    }

    if has_schema(record) {
        let member = field_user_ident(record, index);
        let field_ty = field.ty;
        let ty = field_view_type(field);
        if field.default || field.default_value.is_some() {
            let fallback = if let Some(default_value) = &field.default_value {
                quote! { #default_value }
            } else {
                quote! { <#ty as zebin::io::ArchivedDefault>::archived_default() }
            };
            quote! {
                #source.#member.as_ref().unwrap_or(#fallback).deserialize()?
            }
        } else {
            quote! {
                match #source.#member.as_ref() {
                    Some(value) => value.deserialize()?,
                    None => <#ty as zebin::io::Deserialize<#field_ty>>::deserialize_missing()?,
                }
            }
        }
    } else {
        let member = view_member(record, index);
        quote! { #source.#member.deserialize()? }
    }
}

fn record_deserialize_impl(
    name: &Ident,
    view: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let fields = record.fields.iter().enumerate().map(|(index, _)| {
        let source = quote! { self };
        let expr = deserialize_field_expr(record, index, source);
        match record.style {
            RecordStyle::Named => {
                let member = input_member(record, index);
                quote! { #member: #expr }
            }
            RecordStyle::Unnamed => quote! { #expr },
            RecordStyle::Unit => quote! {},
        }
    });
    let constructor = match record.style {
        RecordStyle::Named => quote! { #name { #(#fields,)* } },
        RecordStyle::Unnamed => quote! { #name( #(#fields,)* ) },
        RecordStyle::Unit => quote! { #name },
    };
    quote! {
        impl<'a> zebin::io::Deserialize<#name> for #view<'a> {
            fn deserialize(&self) -> Result<#name, zebin::ZebinError> {
                Ok(#constructor)
            }
        }
    }
}

fn struct_impl(name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let view = view_name(name);
    record_deserialize_impl(name, &view, record)
}

fn enum_impl(
    name: &Ident,
    variants: &[crate::shared::VariantSpec<'_>],
) -> proc_macro2::TokenStream {
    let view = view_name(name);

    let deserialize_arms: Vec<_> = variants.iter().map(|variant| {
        let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
        let original_variant = variant.ident;
        if variant.record.style == RecordStyle::Unit {
            quote! { #view::#view_variant => Ok(#name::#original_variant) }
        } else {
            let payload_ident = variant_field_name(original_variant);
            let fields = variant.record.fields.iter().enumerate().map(|(index, _)| {
                let expr = deserialize_field_expr(&variant.record, index, quote! { #payload_ident });
                match variant.record.style {
                    RecordStyle::Named => {
                        let member = input_member(&variant.record, index);
                        quote! { #member: #expr }
                    }
                    RecordStyle::Unnamed => quote! { #expr },
                    RecordStyle::Unit => quote! {},
                }
            });
            match variant.record.style {
                RecordStyle::Named => quote! {
                    #view::#view_variant(#payload_ident) => Ok(#name::#original_variant { #(#fields,)* })
                },
                RecordStyle::Unnamed => quote! {
                    #view::#view_variant(#payload_ident) => Ok(#name::#original_variant( #(#fields,)* ))
                },
                RecordStyle::Unit => unreachable!(),
            }
        }
    }).collect();

    quote! {
        impl<'a> zebin::io::Deserialize<#name> for #view<'a> {
            fn deserialize(&self) -> Result<#name, zebin::ZebinError> {
                match self {
                    #view::__ZebinMarker(_) => unreachable!("marker variant is never constructed"),
                    #(#deserialize_arms,)*
                }
            }
        }
    }
}

pub fn derive(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let spec = match parse_item(&input) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };
    let name = input.ident.clone();
    let expanded = match spec {
        ItemSpec::Struct(record) => struct_impl(&name, &record),
        ItemSpec::Enum(variants) => enum_impl(&name, &variants),
    };
    TokenStream::from(expanded)
}

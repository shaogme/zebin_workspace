use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Ident, Member};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, archived_name, field_archived_type, field_encoding,
    field_user_ident, field_view_type, has_schema, input_member, parse_item, variant_archived_name,
    variant_field_name, variant_method_name,
};

fn view_name(name: &Ident) -> Ident {
    format_ident!("{}View", name)
}

fn variant_view_name(enum_name: &Ident, variant: &Ident) -> Ident {
    format_ident!("{}{}View", enum_name, variant)
}

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

fn record_view_definition(view: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    match record.style {
        RecordStyle::Unit => {
            quote! { pub struct #view<'a> { _marker: ::core::marker::PhantomData<&'a ()> } }
        }
        RecordStyle::Named => {
            let fields = record.active_fields().map(|(index, field)| {
                let ident = field_user_ident(record, index);
                let ty = field_view_type(field);
                if has_schema(record) {
                    quote! { #ident: ::core::option::Option<#ty> }
                } else {
                    quote! { pub #ident: #ty }
                }
            });
            let schema_fields = if has_schema(record) {
                quote! { pos: usize, stable_schema_key: u32, schema_revision: u32, }
            } else {
                quote! {}
            };
            quote! { pub struct #view<'a> { #schema_fields #(#fields,)* _marker: ::core::marker::PhantomData<&'a ()> } }
        }
        RecordStyle::Unnamed => {
            if has_schema(record) {
                let fields = record.active_fields().map(|(index, field)| {
                    let ident = field_user_ident(record, index);
                    let ty = field_view_type(field);
                    quote! { #ident: ::core::option::Option<#ty> }
                });
                quote! {
                    pub struct #view<'a> {
                        pos: usize,
                        stable_schema_key: u32,
                        schema_revision: u32,
                        #(#fields,)*
                        _marker: ::core::marker::PhantomData<&'a ()>,
                    }
                }
            } else {
                let fields = record.active_fields().map(|(_, field)| {
                    let ty = field_view_type(field);
                    quote! { pub #ty }
                });
                quote! { pub struct #view<'a>(#(#fields,)* pub ::core::marker::PhantomData<&'a ()>); }
            }
        }
    }
}

fn schema_accessors(view: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }

    let accessors = record.active_fields().map(|(index, field)| {
        let method = field_user_ident(record, index);
        let ty = field_view_type(field);
        let field_id = field.field_id.expect("field ids validated");
        if field.default || field.default_value.is_some() {
            let fallback = if let Some(default_value) = &field.default_value {
                quote! { #default_value }
            } else {
                quote! { <#ty as zebin::io::ArchivedDefault>::archived_default() }
            };
            quote! {
                pub fn #method(&self) -> Result<&#ty, zebin::ZebinError> {
                    match self.#method.as_ref() {
                        Some(value) => Ok(value),
                        None => Ok(#fallback),
                    }
                }
            }
        } else {
            quote! {
                pub fn #method(&self) -> Result<&#ty, zebin::ZebinError> {
                    <#ty as zebin::io::ArchivedField>::resolve_field(self.#method.as_ref(), #field_id, self.pos)
                }
            }
        }
    });

    quote! {
        impl<'a> #view<'a> {
            pub fn pos(&self) -> usize { self.pos }
            pub fn stable_schema_key(&self) -> u32 { self.stable_schema_key }
            pub fn schema_revision(&self) -> u32 { self.schema_revision }
            #(#accessors)*
        }

        impl<'a> zebin::io::SchemaAware for #view<'a> {
            fn pos(&self) -> usize { self.pos }
            fn stable_schema_key(&self) -> u32 { self.stable_schema_key }
            fn schema_revision(&self) -> u32 { self.schema_revision }
        }
    }
}

fn decode_known_field(
    record: &RecordSpec<'_>,
    index: usize,
    field_var: &Ident,
) -> proc_macro2::TokenStream {
    let field = &record.fields[index];
    let field_id = field.field_id.expect("field ids validated");
    let field_name = field_user_ident(record, index);
    let archived_ty = field_archived_type(field);
    let expected_encoding = field_encoding(field);
    quote! {
        #field_id => {
            let mut __field_guard = __guard.push_field(stringify!(#field_name));
            __entry.check_decodable(__entry_pos, #expected_encoding, #field_var.is_some(), &mut *__field_guard)?;
            let mut __field_cursor = zebin::io::Cursor::new(__payload, 0);
            let __value = <#archived_ty as zebin::Decode>::decode(&mut __field_cursor, &mut *__field_guard)?;
            __entry.check_payload_len(__entry_pos, __field_cursor.pos(), &mut *__field_guard)?;
            #field_var = ::core::option::Option::Some(__value);
            Ok(())
        }
    }
}

fn validate_known_field(
    record: &RecordSpec<'_>,
    index: usize,
    seen_expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let field = &record.fields[index];
    let field_id = field.field_id.expect("field ids validated");
    let field_name = field_user_ident(record, index);
    let archived_ty = field_archived_type(field);
    let expected_encoding = field_encoding(field);
    quote! {
        #field_id => {
            let mut __field_guard = __guard.push_field(stringify!(#field_name));
            __entry.check_decodable(__entry_pos, #expected_encoding, #seen_expr, &mut *__field_guard)?;
            let mut __field_cursor = zebin::io::Cursor::new(__payload, 0);
            <#archived_ty as zebin::Decode>::validate(&mut __field_cursor, &mut *__field_guard)?;
            __entry.check_payload_len(__entry_pos, __field_cursor.pos(), &mut *__field_guard)?;
            #seen_expr = true;
            Ok(())
        }
    }
}

fn record_layout_impl(marker: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    if has_schema(record) {
        quote! {
            impl zebin::io::ArchivedLayout for #marker {
                const FIELD_ENCODING: zebin::schema::FieldEncoding = zebin::schema::FieldEncoding::SchemaAware;
            }
        }
    } else {
        quote! {
            impl zebin::io::ArchivedLayout for #marker {}
        }
    }
}

fn record_decode_impl(
    marker: &Ident,
    view: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    if has_schema(record) {
        let key = record
            .stable_schema_key
            .expect("schema-bearing records require key");
        let vars: Vec<_> = record
            .active_fields()
            .map(|(index, _)| format_ident!("__field_{}", field_user_ident(record, index)))
            .collect();
        let var_decls = vars
            .iter()
            .map(|var| quote! { let mut #var = ::core::option::Option::None; });
        let field_arms = record
            .active_fields()
            .zip(vars.iter())
            .map(|((index, _), var)| decode_known_field(record, index, var));
        let active_fields_count = record.active_fields().count();
        let seen_var_decls = quote! { let mut __seen = [false; #active_fields_count]; };
        let validate_field_arms = record.active_fields().enumerate().map(|(i, (index, _))| {
            let seen_expr = quote! { __seen[#i] };
            validate_known_field(record, index, seen_expr)
        });
        let missing_checks = record.active_fields().zip(vars.iter()).filter_map(
            |((index, field), var)| {
                if field.default || field.default_value.is_some() {
                    None
                } else {
                    let field_id = field.field_id.expect("field ids validated");
                    let field_name = field_user_ident(record, index);
                    let field_ty = field.ty;
                    Some(quote! {
                        if !<#field_ty as zebin::Archive>::ALLOW_MISSING && #var.is_none() {
                            let mut __field_guard = __guard.push_field(stringify!(#field_name));
                            return Err(__field_guard.error(zebin::error::DecodeError::MissingField {
                                field_id: #field_id,
                                pos: __object_start,
                            }));
                        }
                    })
                }
            },
        );
        let validate_missing_checks = record.active_fields().enumerate().filter_map(
            |(i, (index, field))| {
                if field.default || field.default_value.is_some() {
                    None
                } else {
                    let field_id = field.field_id.expect("field ids validated");
                    let field_name = field_user_ident(record, index);
                    let field_ty = field.ty;
                    let seen_expr = quote! { __seen[#i] };
                    Some(quote! {
                        if !<#field_ty as zebin::Archive>::ALLOW_MISSING && !#seen_expr {
                            let mut __field_guard = __guard.push_field(stringify!(#field_name));
                            return Err(__field_guard.error(zebin::error::DecodeError::MissingField {
                                field_id: #field_id,
                                pos: __object_start,
                            }));
                        }
                    })
                }
            },
        );
        let construct_fields = record
            .active_fields()
            .zip(vars.iter())
            .map(|((index, _), var)| {
                let member = field_user_ident(record, index);
                quote! { #member: #var }
            });
        let layout = record_layout_impl(marker, record);

        quote! {
            #layout

            impl zebin::Decode for #marker {
                type View<'a>
                    = #view<'a>
                where
                    Self: 'a;
                #[cfg(feature = "alloc")]
                type DecodeStrategy = zebin::io::BackwardSequenceStrategy;

                fn decode<'a, C>(cursor: &mut zebin::io::Cursor<'a>, context: &mut C) -> Result<Self::View<'a>, zebin::error::DecodeError>
                where
                    C: zebin::validation::ValidationContext + ?Sized,
                    Self: 'a
                {
                    let mut __guard = context.guard()?;
                    let __object_start = cursor.pos();
                    let __header = zebin::schema::SchemaObjectHeader::decode_and_verify(cursor, &mut *__guard, #key)?;
                    let __stable_schema_key = __header.stable_schema_key;
                    let __schema_revision = __header.schema_revision;
                    let __field_count = __header.field_count as usize;

                    #(#var_decls)*

                    zebin::schema::process_trailing_field_table(cursor, __field_count, &mut *__guard, |__entry, __entry_pos, __payload, __guard| {
                        match __entry.field_id {
                            #(#field_arms,)*
                            _ => Ok(()),
                        }
                    })?;

                    #(#missing_checks)*

                    Ok(#view {
                        pos: __object_start,
                        stable_schema_key: __stable_schema_key,
                        schema_revision: __schema_revision,
                        #(#construct_fields,)*
                        _marker: ::core::marker::PhantomData,
                    })
                }

                fn validate<'a, C>(cursor: &mut zebin::io::Cursor<'a>, context: &mut C) -> Result<(), zebin::error::DecodeError>
                where
                    C: zebin::validation::ValidationContext + ?Sized,
                {
                    let mut __guard = context.guard()?;
                    let __object_start = cursor.pos();
                    let __header = zebin::schema::SchemaObjectHeader::decode_and_verify(cursor, &mut *__guard, #key)?;
                    let _schema_revision = __header.schema_revision;
                    let __field_count = __header.field_count as usize;

                    #seen_var_decls

                    zebin::schema::process_trailing_field_table(cursor, __field_count, &mut *__guard, |__entry, __entry_pos, __payload, __guard| {
                        match __entry.field_id {
                            #(#validate_field_arms,)*
                            _ => Ok(()),
                        }
                    })?;

                    #(#validate_missing_checks)*

                    Ok(())
                }
            }
        }
    } else {
        let decodes = record.active_fields().map(|(index, field)| {
            let archived_ty = field_archived_type(field);
            let local = format_ident!("__field_{index}");
            let name = field_user_ident(record, index);
            quote! {
                let #local = {
                    let mut __field_guard = __guard.push_field(stringify!(#name));
                    <#archived_ty as zebin::Decode>::decode(cursor, &mut *__field_guard)?
                };
            }
        });
        let validates = record.active_fields().map(|(index, field)| {
            let archived_ty = field_archived_type(field);
            let name = field_user_ident(record, index);
            quote! {
                {
                    let mut __field_guard = __guard.push_field(stringify!(#name));
                    <#archived_ty as zebin::Decode>::validate(cursor, &mut *__field_guard)?;
                }
            }
        });
        let construct = match record.style {
            RecordStyle::Unit => quote! { #view { _marker: ::core::marker::PhantomData } },
            RecordStyle::Named => {
                let fields = record.active_fields().map(|(index, _)| {
                    let member = field_user_ident(record, index);
                    let local = format_ident!("__field_{index}");
                    quote! { #member: #local }
                });
                quote! { #view { #(#fields,)* _marker: ::core::marker::PhantomData } }
            }
            RecordStyle::Unnamed => {
                let fields = record.active_fields().map(|(index, _)| {
                    let local = format_ident!("__field_{index}");
                    quote! { #local }
                });
                quote! { #view( #(#fields,)* ::core::marker::PhantomData ) }
            }
        };
        let layout = record_layout_impl(marker, record);
        quote! {
            #layout

            impl zebin::Decode for #marker {
                type View<'a>
                    = #view<'a>
                where
                    Self: 'a;
                #[cfg(feature = "alloc")]
                type DecodeStrategy = zebin::io::ForwardSequenceStrategy;
                fn decode<'a, C>(cursor: &mut zebin::io::Cursor<'a>, context: &mut C) -> Result<Self::View<'a>, zebin::error::DecodeError>
                where
                    C: zebin::validation::ValidationContext + ?Sized,
                    Self: 'a
                {
                    let mut __guard = context.guard()?;
                    #(#decodes)*
                    Ok(#construct)
                }

                fn validate<'a, C>(cursor: &mut zebin::io::Cursor<'a>, context: &mut C) -> Result<(), zebin::error::DecodeError>
                where
                    C: zebin::validation::ValidationContext + ?Sized,
                {
                    let mut __guard = context.guard()?;
                    #(#validates)*
                    Ok(())
                }
            }
        }
    }
}

fn helper_record(
    marker: &Ident,
    view: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let view_def = record_view_definition(view, record);
    let decode = record_decode_impl(marker, view, record);
    let accessors = schema_accessors(view, record);
    quote! {
        pub struct #marker;
        #view_def
        #decode
        #accessors
        impl<'a> zebin::io::ArchivedField<'a> for #view<'a> {}
    }
}

fn restore_field_expr(
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
                #source.#member.as_ref().unwrap_or(#fallback).restore()?
            }
        } else {
            quote! {
                match #source.#member.as_ref() {
                    Some(value) => value.restore()?,
                    None => <#ty as zebin::io::Restore<#field_ty>>::restore_missing()?,
                }
            }
        }
    } else {
        let member = view_member(record, index);
        quote! { #source.#member.restore()? }
    }
}

fn record_restore_impl(
    name: &Ident,
    view: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let fields = record.fields.iter().enumerate().map(|(index, _)| {
        let source = quote! { self };
        let expr = restore_field_expr(record, index, source);
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
        impl<'a> zebin::io::Restore<#name> for #view<'a> {
            fn restore(&self) -> Result<#name, zebin::ZebinError> {
                Ok(#constructor)
            }
        }
    }
}

fn struct_impl(name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let marker = archived_name(name);
    let view = view_name(name);
    let helper = helper_record(&marker, &view, record);
    let restore = record_restore_impl(name, &view, record);
    quote! {
        #helper
        impl zebin::Archive for #name {
            type Archived = #marker;
        }
        #restore
    }
}

fn enum_impl(
    name: &Ident,
    variants: &[crate::shared::VariantSpec<'_>],
) -> proc_macro2::TokenStream {
    let marker = archived_name(name);
    let view = view_name(name);

    let helpers: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let helper_marker = variant_archived_name(name, variant.ident);
            let helper_view = variant_view_name(name, variant.ident);
            helper_record(&helper_marker, &helper_view, &variant.record)
        })
        .collect();

    let view_variants: Vec<_> = variants
        .iter()
        .map(|variant| {
            let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
            if variant.record.style == RecordStyle::Unit {
                quote! { #view_variant }
            } else {
                let helper_view = variant_view_name(name, variant.ident);
                quote! { #view_variant(#helper_view<'a>) }
            }
        })
        .collect();

    let decode_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let tag = index as u32;
            let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
            if variant.record.style == RecordStyle::Unit {
                quote! { #tag => Ok(#view::#view_variant) }
            } else {
                let helper_marker = variant_archived_name(name, variant.ident);
                quote! {
                    #tag => {
                        let mut __variant_guard = __guard.push_variant(stringify!(#view_variant));
                        Ok(#view::#view_variant(<#helper_marker as zebin::Decode>::decode(cursor, &mut *__variant_guard)?))
                    }
                }
            }
        })
        .collect();

    let validate_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let tag = index as u32;
            let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
            if variant.record.style == RecordStyle::Unit {
                quote! { #tag => Ok(()) }
            } else {
                let helper_marker = variant_archived_name(name, variant.ident);
                quote! {
                    #tag => {
                        let mut __variant_guard = __guard.push_variant(stringify!(#view_variant));
                        <#helper_marker as zebin::Decode>::validate(cursor, &mut *__variant_guard)
                    }
                }
            }
        })
        .collect();

    let tag_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
            let tag = index as u32;
            if variant.record.style == RecordStyle::Unit {
                quote! { #view::#view_variant => #tag }
            } else {
                quote! { #view::#view_variant(_) => #tag }
            }
        })
        .collect();

    let is_methods: Vec<_> = variants
        .iter()
        .map(|variant| {
            let method_ident = variant.rename.as_ref().unwrap_or(variant.ident);
            let method = variant_method_name("is", method_ident);
            let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
            if variant.record.style == RecordStyle::Unit {
                quote! { pub fn #method(&self) -> bool { matches!(self, #view::#view_variant) } }
            } else {
                quote! { pub fn #method(&self) -> bool { matches!(self, #view::#view_variant(_)) } }
            }
        })
        .collect();

    let as_methods: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let method_ident = variant.rename.as_ref().unwrap_or(variant.ident);
            let method = variant_method_name("as", method_ident);
            let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
            let helper_view = variant_view_name(name, variant.ident);
            quote! {
                pub fn #method(&self) -> ::core::option::Option<&#helper_view<'a>> {
                    match self {
                        #view::#view_variant(value) => ::core::option::Option::Some(value),
                        _ => ::core::option::Option::None,
                    }
                }
            }
        })
        .collect();

    let restore_arms: Vec<_> = variants.iter().map(|variant| {
        let view_variant = variant.rename.as_ref().unwrap_or(variant.ident);
        let original_variant = variant.ident;
        if variant.record.style == RecordStyle::Unit {
            quote! { #view::#view_variant => Ok(#name::#original_variant) }
        } else {
            let payload_ident = variant_field_name(original_variant);
            let fields = variant.record.fields.iter().enumerate().map(|(index, _)| {
                let expr = restore_field_expr(&variant.record, index, quote! { #payload_ident });
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
        #(#helpers)*

        pub struct #marker;

        pub enum #view<'a> {
            #[doc(hidden)]
            __ZebinMarker(::core::marker::PhantomData<&'a ()>),
            #(#view_variants,)*
        }

        impl<'a> #view<'a> {
            pub fn tag(&self) -> u32 {
                match self {
                    #view::__ZebinMarker(_) => unreachable!("marker variant is never constructed"),
                    #(#tag_arms,)*
                }
            }
            #(#is_methods)*
            #(#as_methods)*
        }

        impl zebin::io::ArchivedLayout for #marker {}

        impl zebin::Decode for #marker {
            type View<'a>
                = #view<'a>
            where
                Self: 'a;
            #[cfg(feature = "alloc")]
            type DecodeStrategy = zebin::io::ForwardSequenceStrategy;
            fn decode<'a, C>(cursor: &mut zebin::io::Cursor<'a>, context: &mut C) -> Result<Self::View<'a>, zebin::error::DecodeError>
            where
                C: zebin::validation::ValidationContext + ?Sized,
                Self: 'a
            {
                let mut __guard = context.guard()?;
                let __tag_pos = cursor.pos();
                let tag = <u32 as zebin::Decode>::decode(cursor, &mut *__guard)?;
                match tag {
                    #(#decode_arms,)*
                    _ => Err(__guard.validation_error("Invalid enum discriminant", __tag_pos)),
                }
            }

            fn validate<'a, C>(cursor: &mut zebin::io::Cursor<'a>, context: &mut C) -> Result<(), zebin::error::DecodeError>
            where
                C: zebin::validation::ValidationContext + ?Sized,
            {
                let mut __guard = context.guard()?;
                let __tag_pos = cursor.pos();
                let tag = <u32 as zebin::Decode>::decode(cursor, &mut *__guard)?;
                match tag {
                    #(#validate_arms,)*
                    _ => Err(__guard.validation_error("Invalid enum discriminant", __tag_pos)),
                }
            }
        }

        impl zebin::Archive for #name {
            type Archived = #marker;
        }

        impl<'a> zebin::io::Restore<#name> for #view<'a> {
            fn restore(&self) -> Result<#name, zebin::ZebinError> {
                match self {
                    #view::__ZebinMarker(_) => unreachable!("marker variant is never constructed"),
                    #(#restore_arms,)*
                }
            }
        }

        impl<'a> zebin::io::ArchivedField<'a> for #view<'a> {}
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

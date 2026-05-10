use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Ident};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, archived_name, field_archived_type, field_user_ident,
    has_schema, input_member, layout_field_entries, packed_wrapper_type_expr, parse_item,
    resolver_name, user_member,
};

mod enums;

// --- Helper Functions for Record Archiving ---

pub fn record_schema_field(record: &RecordSpec<'_>) -> Option<proc_macro2::TokenStream> {
    if !has_schema(record) {
        return None;
    }
    Some(match record.style {
        RecordStyle::Named => quote! {
            pub stable_schema_key: u32,
            pub schema_revision: u32
        },
        RecordStyle::Unnamed => quote! { pub u32, pub u32 },
        RecordStyle::Unit => unreachable!("unit never has schema"),
    })
}

pub fn record_field_decl(record: &RecordSpec<'_>, index: usize) -> proc_macro2::TokenStream {
    let archived_ty = field_archived_type(&record.fields[index]);
    match record.style {
        RecordStyle::Named => {
            let ident = field_user_ident(record, index);
            quote! { pub #ident: #archived_ty }
        }
        RecordStyle::Unnamed => quote! { #archived_ty },
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

pub fn record_field_inits(
    record: &RecordSpec<'_>,
    archived_name: &Ident,
) -> Vec<proc_macro2::TokenStream> {
    record
        .active_fields()
        .map(|(index, field)| {
            let archived_ty = field_archived_type(field);
            let member = user_member(record, index);
            let offset = quote! { zebin::memoffset::offset_of!(#archived_name, #member) };
            quote! {
                {
                    let offset = #offset;
                    let size = ::core::mem::size_of::<#archived_ty>();
                    <#archived_ty as zebin::Layout>::write_archived_bytes(
                        &archived.#member,
                        &mut out[offset..offset + size],
                    );
                }
            }
        })
        .collect()
}

pub fn record_schema_write(record: &RecordSpec<'_>) -> Option<proc_macro2::TokenStream> {
    if !has_schema(record) {
        return None;
    }
    Some(match record.style {
        RecordStyle::Named => quote! {
            <u32 as zebin::Layout>::write_archived_bytes(&archived.stable_schema_key, &mut out[0..4]);
            <u32 as zebin::Layout>::write_archived_bytes(&archived.schema_revision, &mut out[4..8]);
        },
        RecordStyle::Unnamed => quote! {
            <u32 as zebin::Layout>::write_archived_bytes(&archived.0, &mut out[0..4]);
            <u32 as zebin::Layout>::write_archived_bytes(&archived.1, &mut out[4..8]);
        },
        RecordStyle::Unit => unreachable!("unit never has schema"),
    })
}

pub fn record_layout_checks_logic(
    record: &RecordSpec<'_>,
    archived_name: &Ident,
) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }
    let stable_schema_key = record
        .stable_schema_key
        .expect("schema-bearing records require an explicit stable schema key");
    let schema_revision = record.schema_revision;
    let checks: Vec<_> = record.active_fields().map(|(index, field)| {
        let field_id = field.field_id.expect("field ids are validated above");
        let member = user_member(record, index);
        let on_missing = if field.optional || field.default || field.default_value.is_some() {
            quote! {}
        } else {
            quote! { return Err(zebin::ValidateError::MissingLayoutField { field_id: #field_id, pos: ptr as usize }); }
        };
        quote! {
            {
                let layout = guard.resolved_layout(#stable_schema_key, #schema_revision)?;
                if let Some(actual_offset) = layout.field_offset(#field_id) {
                    if layout.schema_revision() == #schema_revision {
                        layout.check_field(#field_id, zebin::memoffset::offset_of!(#archived_name, #member) as u32)?;
                    }
                } else { #on_missing }
            }
        }
    }).collect();
    quote! { #(#checks)* }
}

pub fn record_field_validations(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    let stable_schema_key = record.stable_schema_key;
    let schema_revision = record.schema_revision;
    record.active_fields().map(|(index, field)| {
        let archived_ty = field_archived_type(field);
        let member = user_member(record, index);
        let path_name = field_user_ident(record, index);
        if let Some(field_id) = field.field_id {
            let key = stable_schema_key.expect("schema-aware fields require stable_schema_key");
            quote! {
                {
                    let offset = guard.resolved_layout(#key, #schema_revision)?.field_offset(#field_id);
                    if let Some(offset) = offset {
                        let field_ptr = unsafe { (ptr as *const u8).add(offset as usize) as *const #archived_ty };
                        let mut guard = guard.push_field(stringify!(#path_name));
                        unsafe { <#archived_ty as zebin::Validate>::validate::<H, _>(field_ptr, &mut *guard)?; }
                    }
                }
            }
        } else {
            quote! {
                {
                    let field_ptr = unsafe { core::ptr::addr_of!((*ptr).#member) };
                    let mut guard = guard.push_field(stringify!(#path_name));
                    unsafe { <#archived_ty as zebin::Validate>::validate::<H, _>(field_ptr, &mut *guard)?; }
                }
            }
        }
    }).collect()
}

pub fn helper_accessors(
    archived_name: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }
    let layout_fields = layout_field_entries(record, archived_name);
    let mut raw_methods = Vec::new();
    let mut trait_methods = Vec::new();
    let mut impl_methods = Vec::new();

    for (index, field) in record.active_fields() {
        let field_id = field.field_id.expect("field ids are validated above");
        let method = field_user_ident(record, index);
        let ty = field_archived_type(field);

        let raw_method = if field.optional {
            quote! {
                pub unsafe fn #method<'view, H: zebin::ArchiveHeaderTrait>(&'view self, layout: &zebin::ResolvedLayout<H>) -> Result<Option<&'view #ty>, zebin::ValidateError> {
                    let offset = match layout.field_offset(#field_id) { Some(o) => o, None => return Ok(None) };
                    Ok(Some(&*(((self as *const _ as *const u8).add(offset as usize)) as *const #ty)))
                }
            }
        } else if field.default || field.default_value.is_some() {
            let def = if let Some(e) = &field.default_value {
                quote! { #e }
            } else {
                quote! { <#ty as zebin::ArchivedDefault>::archived_default() }
            };
            quote! {
                pub unsafe fn #method<'view, H: zebin::ArchiveHeaderTrait>(&'view self, layout: &zebin::ResolvedLayout<H>) -> Result<&'view #ty, zebin::ValidateError> {
                    let offset = match layout.field_offset(#field_id) { Some(o) => o, None => return Ok(#def) };
                    Ok(&*(((self as *const _ as *const u8).add(offset as usize)) as *const #ty))
                }
            }
        } else {
            quote! {
                pub unsafe fn #method<'view, H: zebin::ArchiveHeaderTrait>(&'view self, layout: &zebin::ResolvedLayout<H>) -> Result<&'view #ty, zebin::ValidateError> {
                    let offset = layout.field_offset(#field_id).ok_or_else(|| zebin::ValidateError::MissingLayoutField { field_id: #field_id, pos: self as *const _ as usize })?;
                    Ok(&*(((self as *const _ as *const u8).add(offset as usize)) as *const #ty))
                }
            }
        };
        raw_methods.push(raw_method);

        if field.optional {
            trait_methods.push(quote! {
                fn #method(&self) -> Result<Option<&'a #ty>, zebin::ZebinError>;
            });
            impl_methods.push(quote! {
                fn #method(&self) -> Result<Option<&'a #ty>, zebin::ZebinError> {
                    let layout: zebin::ResolvedLayout<'a, H> = *self.layout();
                    unsafe { self.data().#method(&layout).map_err(Into::into) }
                }
            });
        } else {
            trait_methods.push(quote! {
                fn #method(&self) -> Result<&'a #ty, zebin::ZebinError>;
            });
            impl_methods.push(quote! {
                fn #method(&self) -> Result<&'a #ty, zebin::ZebinError> {
                    let layout: zebin::ResolvedLayout<'a, H> = *self.layout();
                    unsafe { self.data().#method(&layout).map_err(Into::into) }
                }
            });
        };
    }

    let view_trait_name = format_ident!("{}ViewAccess", archived_name);

    quote! {
        impl #archived_name {
            pub const LAYOUT_FIELDS: &'static [zebin::LayoutField] = &[ #(#layout_fields),* ];
            #(#raw_methods)*
        }

        pub trait #view_trait_name<'a, H: zebin::ArchiveHeaderTrait> {
            #(#trait_methods)*
        }

        impl<'a, H: zebin::ArchiveHeaderTrait> #view_trait_name<'a, H> for zebin::View<'a, &'a #archived_name, H> {
            #(#impl_methods)*
        }
    }
}

pub fn helper_bytes_impl(
    archived_name: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let encoding = if has_schema(record) {
        quote! { zebin::ObjectEncoding::SchemaAware }
    } else {
        quote! { zebin::ObjectEncoding::Fixed }
    };
    let mut writes = Vec::new();
    if let Some(ws) = record_schema_write(record) {
        writes.push(ws);
    }
    writes.extend(record_field_inits(record, archived_name));
    let layout_checks = record_layout_checks_logic(record, archived_name);
    let field_validations = record_field_validations(record);
    let fixed_range = if has_schema(record) {
        quote! {}
    } else {
        quote! { guard.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?; }
    };
    let span_expr = if has_schema(record) {
        quote! { 8 }
    } else {
        quote! { ::core::mem::size_of::<Self>() }
    };
    let schema_aware_impl = if has_schema(record) {
        let (key_member, rev_member) = match record.style {
            RecordStyle::Named => (quote! { stable_schema_key }, quote! { schema_revision }),
            RecordStyle::Unnamed => (quote! { 0 }, quote! { 1 }),
            RecordStyle::Unit => unreachable!(),
        };
        quote! {
            impl zebin::SchemaAware for #archived_name {
                fn stable_schema_key(&self) -> u32 { self.#key_member }
                fn schema_revision(&self) -> u32 { self.#rev_member }
            }
        }
    } else {
        quote! {}
    };
    quote! {
        impl zebin::Layout for #archived_name {
            const ALIGNMENT: ::core::num::NonZeroUsize = unsafe { ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>()) };
            const ENCODING: zebin::ObjectEncoding = #encoding;
            fn write_archived_bytes(archived: &Self, out: &mut [u8]) { zebin::utils::byteops::fill(out, 0); #(#writes)* }
        }
        impl zebin::Validate for #archived_name {
            unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), zebin::ValidateError> where H: zebin::ArchiveHeaderTrait, C: zebin::ValidationContext<H> + ?Sized {
                let mut guard = context.guard()?;
                guard.check_alignment(ptr as *const u8, <Self as zebin::Layout>::ALIGNMENT)?;
                #fixed_range #layout_checks #(#field_validations)* Ok(())
            }
        }
        impl<'a> zebin::Access<'a> for #archived_name {
            type View = &'a Self;
            unsafe fn access<H, C>(ptr: *const u8, context: &mut C) -> Result<(Self::View, usize), zebin::AccessError> where H: zebin::ArchiveHeaderTrait, C: zebin::ValidationContext<H> + ?Sized {
                let typed_ptr = ptr as *const Self;
                unsafe { <Self as zebin::Validate>::validate::<H, C>(typed_ptr, context)?; }
                Ok((unsafe { &*typed_ptr }, #span_expr))
            }
        }
        #schema_aware_impl
    }
}

pub fn helper_record(archived_name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let mut fields = Vec::new();
    if let Some(s) = record_schema_field(record) {
        fields.push(s);
    }
    for (index, field) in record.fields.iter().enumerate() {
        if !field.skip {
            fields.push(record_field_decl(record, index));
        }
    }
    let bytes_impl = helper_bytes_impl(archived_name, record);
    let accessors = helper_accessors(archived_name, record);
    let definition = match record.style {
        RecordStyle::Named => quote! { #[repr(C)] pub struct #archived_name { #(#fields,)* } },
        RecordStyle::Unnamed => quote! { #[repr(C)] pub struct #archived_name( #(#fields,)* ); },
        RecordStyle::Unit => quote! { #[repr(C)] pub struct #archived_name; },
    };
    quote! { #definition #bytes_impl #accessors }
}

// --- Struct Implementation ---

fn struct_impl(name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let archived_name = archived_name(name);
    let resolver_name = resolver_name(name);
    let helper = helper_record(&archived_name, record);
    let stable_key = if has_schema(record) {
        let key = record
            .stable_schema_key
            .expect("schema-bearing records require an explicit stable schema key");
        Some(quote! { #key })
    } else {
        None
    };
    let resolve_expr = match record.style {
        RecordStyle::Named => {
            let mut fields = Vec::new();
            if let Some(key) = &stable_key {
                let revision = record.schema_revision;
                fields.push(quote! { stable_schema_key: #key });
                fields.push(quote! { schema_revision: #revision });
            }
            for (index, field) in record.fields.iter().enumerate() {
                if field.skip {
                    continue;
                }
                let id = field.ident.expect("named field has ident");
                let r_field = field.rename.as_ref().unwrap_or(id);
                let s_member = input_member(record, index);
                let a_member = user_member(record, index);
                let off = quote! { zebin::memoffset::offset_of!(#archived_name, #a_member) };
                if let Some(w) = packed_wrapper_type_expr(field) {
                    fields.push(quote! { #r_field: <#w as zebin::Archive>::resolve(&<#w>::new(self.#s_member.as_ref()), pos + #off, resolver.#r_field)? });
                } else {
                    fields.push(
                        quote! { #r_field: self.#s_member.resolve(pos + #off, resolver.#r_field)? },
                    );
                }
            }
            quote! { #archived_name { #(#fields),* } }
        }
        RecordStyle::Unnamed => {
            let mut items = Vec::new();
            if let Some(key) = &stable_key {
                let revision = record.schema_revision;
                items.push(quote! { #key });
                items.push(quote! { #revision });
            }
            for (index, field) in record.fields.iter().enumerate() {
                if field.skip {
                    continue;
                }
                let s_member = input_member(record, index);
                let a_member = user_member(record, index);
                let res_idx = record.fields[..index].iter().filter(|f| !f.skip).count();
                let res_member = syn::Index::from(res_idx);
                let off = quote! { zebin::memoffset::offset_of!(#archived_name, #a_member) };
                if let Some(w) = packed_wrapper_type_expr(field) {
                    items.push(quote! { <#w as zebin::Archive>::resolve(&<#w>::new(self.#s_member.as_ref()), pos + #off, resolver.#res_member)? });
                } else {
                    items
                        .push(quote! { self.#s_member.resolve(pos + #off, resolver.#res_member)? });
                }
            }
            quote! { #archived_name( #(#items),* ) }
        }
        RecordStyle::Unit => quote! { #archived_name },
    };
    quote! { #helper impl zebin::Archive for #name { type Archived = #archived_name; type Resolver = #resolver_name; fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, zebin::ArchiveError> { Ok(#resolve_expr) } } }
}

// --- Main Entry Point ---

pub fn derive(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let spec = match parse_item(&input) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };
    let name = input.ident.clone();
    let expanded = match spec {
        ItemSpec::Struct(record) => struct_impl(&name, &record),
        ItemSpec::Enum(variants) => enums::enum_impl(&name, &variants),
    };
    TokenStream::from(expanded)
}

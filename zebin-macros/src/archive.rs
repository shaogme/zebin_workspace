use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Index, Member};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, VariantSpec, archived_name, has_schema, parse_item,
    payload_name, user_member, variant_archived_name, variant_field_name, variant_method_name,
};

// --- Helper Functions for Code Generation ---

fn schema_access_expr(record: &RecordSpec<'_>, ident: &syn::Ident) -> proc_macro2::TokenStream {
    let member = schema_member(record);
    quote! { #ident.#member }
}

fn field_defs(record: &RecordSpec<'_>, include_schema: bool) -> Vec<proc_macro2::TokenStream> {
    let mut out = Vec::new();
    if include_schema {
        match record.style {
            RecordStyle::Named => out.push(quote! { pub schema_id: u32 }),
            RecordStyle::Unnamed => out.push(quote! { pub u32 }),
            RecordStyle::Unit => {}
        }
    }

    for field in &record.fields {
        let ty = field.ty;
        match record.style {
            RecordStyle::Named => {
                let ident = field.ident.expect("named field has ident");
                out.push(quote! { pub #ident: <#ty as zebin::Archive>::Archived });
            }
            RecordStyle::Unnamed => out.push(quote! { <#ty as zebin::Archive>::Archived }),
            RecordStyle::Unit => {}
        }
    }
    out
}

fn helper_accessors(
    archived_name: &syn::Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }

    let schema_expr = schema_access_expr(record, &format_ident!("self"));
    let methods = record.fields.iter().enumerate().map(|(index, field)| {
        let ty = field.ty;
        let field_id = field.field_id.expect("field ids are validated above");
        let method = match record.style {
            RecordStyle::Named => field.ident.expect("named field has ident").clone(),
            RecordStyle::Unnamed => format_ident!("field{}", index),
            RecordStyle::Unit => unreachable!("unit has no fields"),
        };
        quote! {
            pub unsafe fn #method<'a>(
                &'a self,
                buffer: &'a [u8],
            ) -> Result<&'a <#ty as zebin::Archive>::Archived, zebin::ZebinError> {
                let header = zebin::ArchiveHeader::parse(buffer)?;
                let layout_dir = zebin::LayoutDirectory::new(
                    buffer,
                    ::core::num::NonZeroUsize::new(header.layout_offset.get() as usize).ok_or_else(|| zebin::ZebinError::ValidationError {
                        message: "Layout offset cannot be zero".to_string(),
                        pos: 4,
                    })?,
                );
                let layout = layout_dir.lookup(#schema_expr)?;
                let offset = layout.field_offset(#field_id).ok_or_else(|| zebin::ZebinError::ValidationError {
                    message: format!("Field ID {} not found in layout", #field_id),
                    pos: self as *const _ as usize,
                })?;
                Ok(&*(((self as *const _ as *const u8).add(offset as usize)) as *const <#ty as zebin::Archive>::Archived))
            }
        }
    });

    quote! {
        impl #archived_name {
            #(#methods)*
        }
    }
}

fn helper_validate(
    archived_name: &syn::Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let layout_checks = if has_schema(record) {
        let schema_expr = schema_access_expr(record, &format_ident!("archived"));
        let checks = record.fields.iter().enumerate().map(|(index, field)| {
            let field_id = field.field_id.expect("field ids are validated above");
            let member = user_member(record, index);
            quote! {
                layout.check_field(#field_id, zebin::memoffset::offset_of!(#archived_name, #member) as u16)?;
            }
        });

        quote! {
            let archived = unsafe { &*ptr };
            let layout = context.layout(#schema_expr)?;
            #(#checks)*
        }
    } else {
        quote! {}
    };

    let field_validations = record.fields.iter().enumerate().map(|(index, field)| {
        let ty = field.ty;
        let member = user_member(record, index);
        quote! {
            {
                let field_ptr = unsafe { core::ptr::addr_of!((*ptr).#member) };
                unsafe { <<#ty as zebin::Archive>::Archived as zebin::Validate<zebin::Validator<'_>>>::validate(field_ptr, context)?; }
            }
        }
    });

    quote! {
        impl zebin::Validate<zebin::Validator<'_>> for #archived_name {
            const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>())
            };

            unsafe fn validate(
                ptr: *const Self,
                context: &mut zebin::Validator<'_>,
            ) -> Result<(), zebin::ZebinError> {
                let _guard = context.enter()?;
                context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
                context.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?;
                #layout_checks
                #(#field_validations)*
                Ok(())
            }
        }
    }
}

fn helper_bytes_impl(
    archived_name: &syn::Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let write_schema = if has_schema(record) {
        match record.style {
            RecordStyle::Named => quote! {
                <u32 as zebin::Archive>::write_archived_bytes(
                    &archived.schema_id,
                    &mut out[0..::core::mem::size_of::<u32>()],
                );
            },
            RecordStyle::Unnamed => quote! {
                <u32 as zebin::Archive>::write_archived_bytes(
                    &archived.0,
                    &mut out[0..::core::mem::size_of::<u32>()],
                );
            },
            RecordStyle::Unit => quote! {},
        }
    } else {
        quote! {}
    };

    let writes = record.fields.iter().enumerate().map(|(index, field)| {
        let ty = field.ty;
        let member = user_member(record, index);
        quote! {
            {
                let offset = zebin::memoffset::offset_of!(#archived_name, #member);
                let size = ::core::mem::size_of::<<#ty as zebin::Archive>::Archived>();
                <#ty as zebin::Archive>::write_archived_bytes(
                    &archived.#member,
                    &mut out[offset..offset + size],
                );
            }
        }
    });

    quote! {
        impl #archived_name {
            pub fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                out.fill(0);
                #write_schema
                #(#writes)*
            }

            pub fn archived_bytes(archived: &Self) -> ::zebin::alloc::vec::Vec<u8> {
                let mut out = ::zebin::alloc::vec![0u8; ::core::mem::size_of::<Self>()];
                Self::write_archived_bytes(archived, &mut out);
                out
            }
        }
    }
}

// --- Unified Record Implementation ---

fn schema_member(record: &RecordSpec<'_>) -> Member {
    match record.style {
        RecordStyle::Named => Member::Named(format_ident!("schema_id")),
        RecordStyle::Unnamed => Member::Unnamed(Index::from(0)),
        RecordStyle::Unit => unreachable!("unit has no schema field"),
    }
}

fn helper_record(archived_name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = field_defs(record, has_schema(record));
    let bytes_impl = helper_bytes_impl(archived_name, record);
    let validate = helper_validate(archived_name, record);
    let accessors = helper_accessors(archived_name, record);

    let definition = match record.style {
        RecordStyle::Named => quote! {
            #[repr(C)]
            pub struct #archived_name {
                #(#fields,)*
            }
        },
        RecordStyle::Unnamed => quote! {
            #[repr(C)]
            pub struct #archived_name(
                #(#fields,)*
            );
        },
        RecordStyle::Unit => quote! {
            #[repr(C)]
            pub struct #archived_name;
        },
    };

    quote! {
        #definition
        #bytes_impl
        #accessors
        #validate
    }
}

// --- Struct Implementation ---

fn struct_impl(name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let archived_name = archived_name(name);
    let resolver_name = crate::shared::resolver_name(name);
    let include_schema = has_schema(record);
    let align = quote! { ::core::mem::align_of::<#archived_name>() };
    let helper = helper_record(&archived_name, record);

    let resolve_expr = match record.style {
        RecordStyle::Named => {
            let mut fields = Vec::new();
            if include_schema {
                fields.push(quote! { schema_id: resolver.schema_id });
            }
            for (index, field) in record.fields.iter().enumerate() {
                let ident = field.ident.expect("named field has ident");
                let member = user_member(record, index);
                fields.push(quote! {
                    #ident: self.#member.resolve(
                        pos + zebin::memoffset::offset_of!(#archived_name, #member),
                        resolver.#ident
                    )?
                });
            }
            quote! { #archived_name { #(#fields),* } }
        }
        RecordStyle::Unnamed => {
            let mut items = Vec::new();
            if include_schema {
                items.push(quote! { resolver.0 });
            }
            for (index, _field) in record.fields.iter().enumerate() {
                let member = user_member(record, index);
                items.push(quote! {
                    self.#member.resolve(
                        pos + zebin::memoffset::offset_of!(#archived_name, #member),
                        resolver.#member
                    )?
                });
            }
            quote! { #archived_name( #(#items),* ) }
        }
        RecordStyle::Unit => quote! { #archived_name },
    };

    quote! {
        #helper

        impl zebin::Archive for #name {
            type Archived = #archived_name;
            type Resolver = #resolver_name;
            const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                ::core::num::NonZeroUsize::new_unchecked(#align)
            };

            fn resolve(
                &self,
                pos: usize,
                resolver: Self::Resolver,
            ) -> Result<Self::Archived, zebin::ZebinError> {
                Ok(#resolve_expr)
            }

            fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
                #archived_name::write_archived_bytes(archived, out)
            }
        }
    }
}

// --- Enum Implementation ---

fn enum_impl(name: &syn::Ident, variants: &[VariantSpec<'_>]) -> proc_macro2::TokenStream {
    let archived_name = archived_name(name);
    let resolver_name = crate::shared::resolver_name(name);
    let payload_name = payload_name(name);

    let mut variant_defs = Vec::new();
    let mut variant_accessors = Vec::new();
    let mut variant_validate_arms = Vec::new();
    let mut variant_write_arms = Vec::new();
    let mut variant_resolve_arms = Vec::new();
    let mut variant_payload_fields = Vec::new();

    for (idx, variant) in variants.iter().enumerate() {
        let helper_name = variant_archived_name(name, variant.ident);
        variant_defs.push(helper_record(&helper_name, &variant.record));
        let payload_field_ident = variant_field_name(variant.ident);
        variant_payload_fields.push(quote! {
            #payload_field_ident: ::core::mem::ManuallyDrop<#helper_name>
        });

        let idx_lit = idx as u32;
        let accessor_name = if variant.record.fields.is_empty() {
            variant_method_name("is", variant.ident)
        } else {
            variant_method_name("as", variant.ident)
        };
        if variant.record.fields.is_empty() {
            variant_accessors.push(quote! {
                pub fn #accessor_name(&self) -> bool {
                    self.tag == #idx_lit
                }
            });
        } else {
            variant_accessors.push(quote! {
                    pub unsafe fn #accessor_name<'a>(&'a self) -> Option<&'a #helper_name> {
                        if self.tag != #idx_lit {
                            return None;
                        }
                        let ptr = unsafe { &self.payload.#payload_field_ident as *const _ as *const #helper_name };
                        Some(&*ptr)
                    }
                });
        }

        variant_validate_arms.push(quote! {
            #idx_lit => {
                let ptr = unsafe { &archived.payload.#payload_field_ident as *const _ as *const #helper_name };
                unsafe { #helper_name::validate(ptr, context)?; }
            }
        });

        variant_write_arms.push(quote! {
            #idx_lit => {
                let ptr = unsafe { &archived.payload.#payload_field_ident as *const _ as *const #helper_name };
                let bytes = #helper_name::archived_bytes(unsafe { &*ptr });
                out[payload_offset..payload_offset + bytes.len()].copy_from_slice(&bytes);
            }
        });

        let record = &variant.record;
        let include_schema = has_schema(record);
        let variant_ident = variant.ident;
        let payload_offset = quote! { zebin::memoffset::offset_of!(#archived_name, payload) };

        let self_pattern = match record.style {
            RecordStyle::Named => {
                let fields = record.fields.iter().map(|field| {
                    let ident = field.ident.expect("named field has ident");
                    quote! { #ident }
                });
                quote! { Self::#variant_ident { #(#fields),* } }
            }
            RecordStyle::Unnamed => {
                let fields = record
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(field_index, _)| format_ident!("field{}", field_index));
                quote! { Self::#variant_ident( #(#fields),* ) }
            }
            RecordStyle::Unit => quote! { Self::#variant_ident },
        };

        let resolver_pattern = quote! { #resolver_name::#variant_ident(resolver) };

        let mut fields = Vec::new();
        if include_schema {
            match record.style {
                RecordStyle::Named => fields.push(quote! { schema_id: resolver.schema_id }),
                RecordStyle::Unnamed => fields.push(quote! { resolver.0 }),
                RecordStyle::Unit => {}
            }
        }
        for (field_index, field) in record.fields.iter().enumerate() {
            let member = user_member(record, field_index);
            match record.style {
                RecordStyle::Named => {
                    let ident = field.ident.expect("named field has ident");
                    let resolver_member = user_member(record, field_index);
                    fields.push(quote! {
                        #ident: #ident.resolve(
                            pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #member),
                            resolver.#resolver_member
                        )?
                    });
                }
                RecordStyle::Unnamed => {
                    let value_ident = format_ident!("field{}", field_index);
                    let resolver_member = user_member(record, field_index);
                    fields.push(quote! {
                        #value_ident.resolve(
                            pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #member),
                            resolver.#resolver_member
                        )?
                    });
                }
                RecordStyle::Unit => {}
            }
        }
        let constructor = match record.style {
            RecordStyle::Named => quote! { #helper_name { #(#fields),* } },
            RecordStyle::Unnamed => quote! { #helper_name( #(#fields),* ) },
            RecordStyle::Unit => quote! { #helper_name },
        };
        variant_resolve_arms.push(quote! {
            (#self_pattern, #resolver_pattern) => {
                let archived = #constructor;
                Ok(#archived_name {
                    tag: #idx_lit,
                    payload: #payload_name {
                        #payload_field_ident: ::core::mem::ManuallyDrop::new(archived),
                    },
                })
            }
        });
    }

    let payload_struct = quote! {
        #[repr(C)]
        union #payload_name {
            #(#variant_payload_fields,)*
        }
    };

    let root_accessors = quote! {
        impl #archived_name {
            pub fn tag(&self) -> u32 {
                self.tag
            }
            #(#variant_accessors)*
        }
    };

    let root_validate = quote! {
        impl zebin::Validate<zebin::Validator<'_>> for #archived_name {
            const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>())
            };

            unsafe fn validate(
                ptr: *const Self,
                context: &mut zebin::Validator<'_>,
            ) -> Result<(), zebin::ZebinError> {
                let _guard = context.enter()?;
                context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
                context.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?;
                let archived = unsafe { &*ptr };
                match archived.tag {
                    #(#variant_validate_arms)*
                    _ => {
                        return Err(zebin::ZebinError::ValidationError {
                            message: "Invalid enum discriminant".to_string(),
                            pos: ptr as usize,
                        });
                    }
                }
                Ok(())
            }
        }
    };

    let root_archive = if variants.is_empty() {
        quote! {
            impl zebin::Archive for #name {
                type Archived = #archived_name;
                type Resolver = #resolver_name;
                const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                    ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self::Archived>())
                };

                fn resolve(
                    &self,
                    _pos: usize,
                    _resolver: Self::Resolver,
                ) -> Result<Self::Archived, zebin::ZebinError> {
                    match *self {}
                }

                fn write_archived_bytes(_archived: &Self::Archived, _out: &mut [u8]) {}
            }
        }
    } else {
        quote! {
            impl zebin::Archive for #name {
                type Archived = #archived_name;
                type Resolver = #resolver_name;
                const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                    ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self::Archived>())
                };

                fn resolve(
                    &self,
                    pos: usize,
                    resolver: Self::Resolver,
                ) -> Result<Self::Archived, zebin::ZebinError> {
                    match (self, resolver) {
                        #(#variant_resolve_arms),*
                        _ => unreachable!("mismatched enum resolver"),
                    }
                }

                fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
                    out.fill(0);
                    <u32 as zebin::Archive>::write_archived_bytes(
                        &archived.tag,
                        &mut out[0..::core::mem::size_of::<u32>()],
                    );
                    let payload_offset = zebin::memoffset::offset_of!(#archived_name, payload);
                    match archived.tag {
                        #(#variant_write_arms)*
                        _ => {}
                    }
                }
            }
        }
    };

    quote! {
        #(#variant_defs)*

        #[repr(C)]
        pub struct #archived_name {
            tag: u32,
            payload: #payload_name,
        }

        #payload_struct
        #root_accessors
        #root_validate
        #root_archive
    }
}

// --- Main Entry Point ---

pub fn derive(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let spec = match parse_item(&input) {
        Ok(spec) => spec,
        Err(err) => return err.to_compile_error().into(),
    };

    let name = input.ident.clone();
    let expanded = match spec {
        ItemSpec::Struct(record) => struct_impl(&name, &record),
        ItemSpec::Enum(variants) => enum_impl(&name, &variants),
    };

    TokenStream::from(expanded)
}

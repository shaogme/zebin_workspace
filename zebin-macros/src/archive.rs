use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Member};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, VariantSpec, archived_name, has_schema, input_member,
    layout_field_entries, packed_archived_type, packed_wrapper_type_expr, parse_item, payload_name,
    user_member, variant_archived_name, variant_field_name, variant_method_name,
};

// --- Helper Functions for Code Generation ---

fn record_schema_field(record: &RecordSpec<'_>) -> Option<proc_macro2::TokenStream> {
    if !has_schema(record) {
        return None;
    }

    Some(match record.style {
        RecordStyle::Named => quote! { pub stable_schema_key: u32 },
        RecordStyle::Unnamed => quote! { pub u32 },
        RecordStyle::Unit => unreachable!("unit never has schema"),
    })
}

fn record_field_decl(record: &RecordSpec<'_>, index: usize) -> proc_macro2::TokenStream {
    let field = &record.fields[index];
    let archived_ty = if let Some(archived) = packed_archived_type(field) {
        archived
    } else {
        let ty = field.ty;
        quote! { <#ty as zebin::Archive>::Archived }
    };
    match record.style {
        RecordStyle::Named => {
            let ident = field
                .rename
                .as_ref()
                .unwrap_or_else(|| field.ident.expect("named field has ident"));
            quote! { pub #ident: #archived_ty }
        }
        RecordStyle::Unnamed => quote! { #archived_ty },
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

fn record_field_member(record: &RecordSpec<'_>, index: usize) -> Member {
    user_member(record, index)
}

fn record_field_offset_expr(
    archived_name: &syn::Ident,
    record: &RecordSpec<'_>,
    index: usize,
) -> proc_macro2::TokenStream {
    let member = record_field_member(record, index);
    quote! { zebin::memoffset::offset_of!(#archived_name, #member) }
}

fn record_field_method_name(record: &RecordSpec<'_>, index: usize) -> syn::Ident {
    let field = &record.fields[index];
    if let Some(rename) = &field.rename {
        return rename.clone();
    }
    match record.style {
        RecordStyle::Named => field.ident.expect("named field has ident").clone(),
        RecordStyle::Unnamed => format_ident!("field{}", index),
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

fn record_field_inits(
    record: &RecordSpec<'_>,
    archived_name: &syn::Ident,
) -> Vec<proc_macro2::TokenStream> {
    record
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.skip)
        .map(|(index, field)| {
            let ty = if let Some(archived) = packed_archived_type(field) {
                archived
            } else {
                let ty = field.ty;
                quote! { <#ty as zebin::Archive>::Archived }
            };
            let member = record_field_member(record, index);
            let offset = record_field_offset_expr(archived_name, record, index);
            quote! {
                {
                    let offset = #offset;
                    let size = ::core::mem::size_of::<#ty>();
                    <#ty as zebin::Layout>::write_archived_bytes(
                        &archived.#member,
                        &mut out[offset..offset + size],
                    );
                }
            }
        })
        .collect()
}

fn record_schema_write(record: &RecordSpec<'_>) -> Option<proc_macro2::TokenStream> {
    if !has_schema(record) {
        return None;
    }

    Some(match record.style {
        RecordStyle::Named => quote! {
            <u32 as zebin::Layout>::write_archived_bytes(
                &archived.stable_schema_key,
                &mut out[0..::core::mem::size_of::<u32>()],
            );
        },
        RecordStyle::Unnamed => quote! {
            <u32 as zebin::Layout>::write_archived_bytes(
                &archived.0,
                &mut out[0..::core::mem::size_of::<u32>()],
            );
        },
        RecordStyle::Unit => unreachable!("unit never has schema"),
    })
}

fn record_layout_checks_logic(
    record: &RecordSpec<'_>,
    archived_name: &syn::Ident,
) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }

    let stable_schema_key = record
        .stable_schema_key
        .expect("schema-bearing records require an explicit stable schema key");
    let schema_revision = record.schema_revision;
    let checks: Vec<_> = record
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.skip)
        .map(|(index, field)| {
            let field_id = field.field_id.expect("field ids are validated above");
            let member = record_field_member(record, index);
            quote! {
                {
                    let expected = zebin::memoffset::offset_of!(#archived_name, #member) as u32;
                    let actual = match layout.field_offset(#field_id) {
                        Some(actual) => actual,
                        None => {
                            return Err(zebin::ValidateError::MissingLayoutField {
                                field_id: #field_id,
                                pos: ptr as usize,
                                path: Default::default(),
                            });
                        }
                    };
                    if actual != expected {
                        return Err(zebin::ValidateError::LayoutOffsetMismatch {
                            field_id: #field_id,
                            expected,
                            actual,
                            pos: ptr as usize,
                            path: Default::default(),
                        });
                    }
                }
            }
        })
        .collect();

    quote! {
        let layout = guard.resolved_layout(#stable_schema_key, #schema_revision)?;
        let layout = layout.layout();
        #(#checks)*
    }
}

fn record_field_validations(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    record
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.skip)
        .map(|(index, field)| {
            let ty = if let Some(archived) = packed_archived_type(field) {
                archived
            } else {
                let ty = field.ty;
                quote! { <#ty as zebin::Archive>::Archived }
            };
            let member = record_field_member(record, index);
            let path_name = record_field_method_name(record, index);
            quote! {
                {
                    let field_ptr = unsafe { core::ptr::addr_of!((*ptr).#member) };
                    unsafe {
                        <#ty as zebin::Validate>::validate::<H, _>(field_ptr, &mut *guard)
                            .map_err(|e| e.at(zebin::ValidationPathSegment::Field(stringify!(#path_name))))?;
                    }
                }
            }
        })
        .collect()
}

fn helper_accessors(
    archived_name: &syn::Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }

    let layout_fields = layout_field_entries(record, archived_name);
    let methods = record
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.skip)
        .map(|(index, field)| {
            let field_id = field.field_id.expect("field ids are validated above");
            let method = record_field_method_name(record, index);
            let ty = if let Some(archived) = packed_archived_type(field) {
                archived
            } else {
                let ty = field.ty;
                quote! { <#ty as zebin::Archive>::Archived }
            };
            quote! {
            pub unsafe fn #method<'a>(
                &'a self,
                layout: &'a zebin::ResolvedLayout<'a>,
            ) -> Result<&'a #ty, zebin::ValidateError> {
                let offset = layout.field_offset(#field_id).ok_or_else(|| zebin::ValidateError::MissingLayoutField {
                    field_id: #field_id,
                    pos: self as *const _ as usize,
                    path: Default::default(),
                })?;
                let base = self as *const _ as *const u8;
                Ok(&*(((base.add(offset as usize)) as *const #ty)))
            }
        }
        });

    quote! {
        impl #archived_name {
            pub const LAYOUT_FIELDS: &'static [zebin::LayoutField] = &[
                #(#layout_fields),*
            ];
            #(#methods)*
        }
    }
}

fn helper_bytes_impl(
    archived_name: &syn::Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let encoding = if has_schema(record) {
        quote! { zebin::ObjectEncoding::SchemaAware }
    } else {
        quote! { zebin::ObjectEncoding::Fixed }
    };

    let mut writes = Vec::new();
    if let Some(write_schema) = record_schema_write(record) {
        writes.push(write_schema);
    }
    writes.extend(record_field_inits(record, archived_name));

    let layout_checks = record_layout_checks_logic(record, archived_name);
    let field_validations = record_field_validations(record);

    quote! {
        impl zebin::Layout for #archived_name {
            const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>())
            };

            const ENCODING: zebin::ObjectEncoding = #encoding;

            fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                zebin::utils::byteops::fill(out, 0);
                #(#writes)*
            }
        }

        impl zebin::Validate for #archived_name {
            unsafe fn validate<H, C>(
                ptr: *const Self,
                context: &mut C,
            ) -> Result<(), zebin::ValidateError>
            where
                H: zebin::ArchiveHeaderTrait,
                C: zebin::ValidationContext<H> + ?Sized,
            {
                let mut guard = context.guard()?;
                guard.check_alignment(ptr as *const u8, <Self as zebin::Layout>::ALIGNMENT)?;
                guard.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?;
                let archived = unsafe { &*ptr };
                #layout_checks
                #(#field_validations)*
                Ok(())
            }
        }

        impl<'a> zebin::Access<'a> for #archived_name {
            type View = &'a Self;

            unsafe fn access<H, C>(
                ptr: *const u8,
                context: &mut C,
            ) -> Result<(Self::View, usize), zebin::AccessError>
            where
                H: zebin::ArchiveHeaderTrait,
                C: zebin::ValidationContext<H> + ?Sized,
            {
                let typed_ptr = ptr as *const Self;
                unsafe { <Self as zebin::Validate>::validate::<H, C>(typed_ptr, context)?; }
                Ok((unsafe { &*typed_ptr }, ::core::mem::size_of::<Self>()))
            }
        }
    }
}

fn helper_record(archived_name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let mut fields = Vec::new();
    if let Some(schema) = record_schema_field(record) {
        fields.push(schema);
    }
    for (index, field) in record.fields.iter().enumerate() {
        if field.skip {
            continue;
        }
        fields.push(record_field_decl(record, index));
    }
    let bytes_impl = helper_bytes_impl(archived_name, record);
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
    }
}

// --- Struct Implementation ---

fn struct_impl(name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let archived_name = archived_name(name);
    let resolver_name = crate::shared::resolver_name(name);
    let include_schema = has_schema(record);
    let helper = helper_record(&archived_name, record);
    let stable_schema_key = if include_schema {
        let stable_schema_key = record
            .stable_schema_key
            .expect("schema-bearing records require an explicit stable schema key");
        Some(quote! { #stable_schema_key })
    } else {
        None
    };

    let resolve_expr = match record.style {
        RecordStyle::Named => {
            let mut fields = Vec::new();
            if let Some(stable_schema_key) = &stable_schema_key {
                fields.push(quote! { stable_schema_key: #stable_schema_key });
            }
            for (index, field) in record.fields.iter().enumerate() {
                if field.skip {
                    continue;
                }
                let ident = field.ident.expect("named field has ident");
                let resolver_field = field.rename.as_ref().unwrap_or(ident);
                let source_member = input_member(record, index);
                let archived_member = user_member(record, index);
                if let Some(wrapper) = packed_wrapper_type_expr(field) {
                    fields.push(quote! {
                        #resolver_field: <#wrapper as zebin::Archive>::resolve(
                            &<#wrapper>::new(self.#source_member.as_ref()),
                            pos + zebin::memoffset::offset_of!(#archived_name, #archived_member),
                            resolver.#resolver_field
                        )?
                    });
                } else {
                    fields.push(quote! {
                        #resolver_field: self.#source_member.resolve(
                            pos + zebin::memoffset::offset_of!(#archived_name, #archived_member),
                            resolver.#resolver_field
                        )?
                    });
                }
            }
            quote! { #archived_name { #(#fields),* } }
        }
        RecordStyle::Unnamed => {
            let mut items = Vec::new();
            if let Some(stable_schema_key) = &stable_schema_key {
                items.push(quote! { #stable_schema_key });
            }
            for (index, field) in record.fields.iter().enumerate() {
                if field.skip {
                    continue;
                }
                let source_member = input_member(record, index);
                let archived_member = user_member(record, index);
                let resolver_index = record.fields[..index].iter().filter(|f| !f.skip).count();
                let resolver_member = syn::Index::from(resolver_index);
                if let Some(wrapper) = packed_wrapper_type_expr(field) {
                    items.push(quote! {
                        <#wrapper as zebin::Archive>::resolve(
                            &<#wrapper>::new(self.#source_member.as_ref()),
                            pos + zebin::memoffset::offset_of!(#archived_name, #archived_member),
                            resolver.#resolver_member
                        )?
                    });
                } else {
                    items.push(quote! {
                        self.#source_member.resolve(
                            pos + zebin::memoffset::offset_of!(#archived_name, #archived_member),
                            resolver.#resolver_member
                        )?
                    });
                }
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

            fn resolve(
                &self,
                pos: usize,
                resolver: Self::Resolver,
            ) -> Result<Self::Archived, zebin::ArchiveError> {
                Ok(#resolve_expr)
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
        let variant_user_ident = variant.rename.as_ref().unwrap_or(variant.ident);
        let helper_name = variant_archived_name(name, variant_user_ident);
        variant_defs.push(helper_record(&helper_name, &variant.record));
        let payload_field_ident = variant_field_name(variant_user_ident);
        let helper_stable_schema_key = if has_schema(&variant.record) {
            let helper_stable_schema_key = variant
                .record
                .stable_schema_key
                .expect("schema-bearing records require an explicit stable schema key");
            Some(quote! { #helper_stable_schema_key })
        } else {
            None
        };
        variant_payload_fields.push(quote! {
            #payload_field_ident: ::core::mem::ManuallyDrop<#helper_name>
        });

        let idx_lit = idx as u32;
        let variant_ident = variant.ident;
        let variant_user_ident = variant.rename.as_ref().unwrap_or(variant.ident);
        let resolver_pattern = quote! { #resolver_name::#variant_user_ident(resolver) };
        let accessor_name = if variant.record.fields.is_empty() {
            variant_method_name("is", variant_user_ident)
        } else {
            variant_method_name("as", variant_user_ident)
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

        let layout_checks = record_layout_checks_logic(&variant.record, &helper_name);
        let field_validations = record_field_validations(&variant.record);
        variant_validate_arms.push(quote! {
            #idx_lit => {
                let ptr = unsafe { &archived.payload.#payload_field_ident as *const _ as *const #helper_name };
                let result: Result<(), zebin::ValidateError> = (|| -> Result<(), zebin::ValidateError> {
                    #layout_checks
                    #(#field_validations)*
                    Ok(())
                })();
                result.map_err(|e| e.at(zebin::ValidationPathSegment::Variant(stringify!(#variant_ident))))?;
            }
        });

        variant_write_arms.push(quote! {
            #idx_lit => {
                let ptr = unsafe { &archived.payload.#payload_field_ident as *const _ as *const #helper_name };
                let bytes = zebin::archived_bytes(unsafe { &*ptr });
                out[payload_offset..payload_offset + bytes.len()].copy_from_slice(&bytes);
            }
        });

        let record = &variant.record;
        let payload_offset = quote! { zebin::memoffset::offset_of!(#archived_name, payload) };

        let self_pattern = match record.style {
            RecordStyle::Named => {
                let fields = record.fields.iter().map(|field| {
                    let ident = field.ident.expect("named field has ident");
                    if field.skip {
                        quote! { #ident: _ }
                    } else {
                        quote! { #ident }
                    }
                });
                quote! { Self::#variant_ident { #(#fields),* } }
            }
            RecordStyle::Unnamed => {
                let fields = record.fields.iter().enumerate().map(|(field_index, field)| {
                    if field.skip {
                        quote! { _ }
                    } else {
                        let ident = format_ident!("field{}", field_index);
                        quote! { #ident }
                    }
                });
                quote! { Self::#variant_ident( #(#fields),* ) }
            }
            RecordStyle::Unit => quote! { Self::#variant_ident },
        };

        let mut fields = Vec::new();
        if let Some(helper_stable_schema_key) = &helper_stable_schema_key {
            match record.style {
                RecordStyle::Named => {
                    fields.push(quote! { stable_schema_key: #helper_stable_schema_key })
                }
                RecordStyle::Unnamed => fields.push(quote! { #helper_stable_schema_key }),
                RecordStyle::Unit => {}
            }
        }
        for (field_index, field) in record.fields.iter().enumerate() {
            if field.skip {
                continue;
            }
            let archived_member = user_member(record, field_index);
            match record.style {
                RecordStyle::Named => {
                    let ident = field.ident.expect("named field has ident");
                    let resolver_field = field.rename.as_ref().unwrap_or(ident);
                    if let Some(wrapper) = packed_wrapper_type_expr(field) {
                        fields.push(quote! {
                            #resolver_field: <#wrapper as zebin::Archive>::resolve(
                                &<#wrapper>::new(#ident.as_ref()),
                                pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #archived_member),
                                resolver.#resolver_field
                            )?
                        });
                    } else {
                        fields.push(quote! {
                            #resolver_field: #ident.resolve(
                                pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #archived_member),
                                resolver.#resolver_field
                            )?
                        });
                    }
                }
                RecordStyle::Unnamed => {
                    let value_ident = format_ident!("field{}", field_index);
                    let resolver_index = record.fields[..field_index]
                        .iter()
                        .filter(|f| !f.skip)
                        .count();
                    let resolver_member = syn::Index::from(resolver_index);
                    if let Some(wrapper) = packed_wrapper_type_expr(field) {
                        fields.push(quote! {
                            <#wrapper as zebin::Archive>::resolve(
                                &<#wrapper>::new(#value_ident.as_ref()),
                                pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #archived_member),
                                resolver.#resolver_member
                            )?
                        });
                    } else {
                        fields.push(quote! {
                            #value_ident.resolve(
                                pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #archived_member),
                                resolver.#resolver_member
                            )?
                        });
                    }
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

    let root_bytes = if variants.is_empty() {
        quote! {
            impl zebin::Layout for #archived_name {
                const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                    ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>())
                };

                fn write_archived_bytes(_archived: &Self, out: &mut [u8]) {
                    zebin::utils::byteops::fill(out, 0);
                }
            }

            impl zebin::Validate for #archived_name {
                unsafe fn validate<H, C>(
                    ptr: *const Self,
                    context: &mut C,
                ) -> Result<(), zebin::ValidateError>
                where
                    H: zebin::ArchiveHeaderTrait,
                    C: zebin::ValidationContext<H> + ?Sized,
                {
                    let mut guard = context.guard()?;
                    guard.check_alignment(ptr as *const u8, <Self as zebin::Layout>::ALIGNMENT)?;
                    guard.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?;
                    Ok(())
                }
            }
        }
    } else {
        quote! {
            impl zebin::Layout for #archived_name {
                const ALIGNMENT: ::core::num::NonZeroUsize = unsafe {
                    ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>())
                };

                fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                    zebin::utils::byteops::fill(out, 0);
                    <u32 as zebin::Layout>::write_archived_bytes(
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

            impl zebin::Validate for #archived_name {
                unsafe fn validate<H, C>(
                    ptr: *const Self,
                    context: &mut C,
                ) -> Result<(), zebin::ValidateError>
                where
                    H: zebin::ArchiveHeaderTrait,
                    C: zebin::ValidationContext<H> + ?Sized,
                {
                    let mut guard = context.guard()?;
                    guard.check_alignment(ptr as *const u8, <Self as zebin::Layout>::ALIGNMENT)?;
                    guard.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?;
                    let archived = unsafe { &*ptr };
                    match archived.tag {
                        #(#variant_validate_arms)*
                        _ => {
                            return Err(zebin::ValidateError::ValidationError {
                                message: "Invalid enum discriminant",
                                pos: ptr as usize,
                                path: Default::default(),
                            });
                        }
                    }
                    Ok(())
                }
            }
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

    let root_validate = quote! {};

    let root_decode = quote! {
        impl<'a> zebin::Access<'a> for #archived_name {
            type View = &'a Self;

            unsafe fn access<H, C>(
                ptr: *const u8,
                context: &mut C,
            ) -> Result<(Self::View, usize), zebin::AccessError>
            where
                H: zebin::ArchiveHeaderTrait,
                C: zebin::ValidationContext<H> + ?Sized,
            {
                let typed_ptr = ptr as *const Self;
                unsafe { <Self as zebin::Validate>::validate::<H, C>(typed_ptr, context)?; }
                Ok((unsafe { &*typed_ptr }, ::core::mem::size_of::<Self>()))
            }
        }
    };

    let root_archive = if variants.is_empty() {
        quote! {
            impl zebin::Archive for #name {
                type Archived = #archived_name;
                type Resolver = #resolver_name;

                fn resolve(
                    &self,
                    _pos: usize,
                    _resolver: Self::Resolver,
                ) -> Result<Self::Archived, zebin::ArchiveError> {
                    match *self {}
                }
            }
        }
    } else {
        quote! {
            impl zebin::Archive for #name {
                type Archived = #archived_name;
                type Resolver = #resolver_name;

                fn resolve(
                    &self,
                    pos: usize,
                    resolver: Self::Resolver,
                ) -> Result<Self::Archived, zebin::ArchiveError> {
                    match (self, resolver) {
                        #(#variant_resolve_arms),*
                        _ => Err(zebin::ArchiveError::InvalidResolver { pos }),
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
        #root_bytes
        #root_decode
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

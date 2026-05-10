use super::{helper_record, record_field_validations, record_layout_checks_logic};
use crate::shared::{
    RecordStyle, VariantSpec, archived_name, has_schema, packed_wrapper_type_expr, payload_name,
    user_member, variant_archived_name, variant_field_name, variant_method_name,
};
use quote::{format_ident, quote};
use syn::Ident;

pub fn enum_impl(name: &Ident, variants: &[VariantSpec<'_>]) -> proc_macro2::TokenStream {
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
            let key = variant
                .record
                .stable_schema_key
                .expect("schema-bearing records require an explicit stable schema key");
            Some(quote! { #key })
        } else {
            None
        };
        variant_payload_fields.push(quote! {
            #payload_field_ident: ::core::mem::ManuallyDrop<#helper_name>
        });

        let idx_lit = idx as u32;
        let variant_ident = variant.ident;
        let resolver_pattern = quote! { #resolver_name::#variant_user_ident(resolver) };
        let accessor_name = if variant.record.fields.is_empty() {
            variant_method_name("is", variant_user_ident)
        } else {
            variant_method_name("as", variant_user_ident)
        };
        if variant.record.fields.is_empty() {
            variant_accessors.push(quote! {
                pub fn #accessor_name(&self) -> bool { self.tag == #idx_lit }
            });
        } else {
            variant_accessors.push(quote! {
                pub unsafe fn #accessor_name<'a>(&'a self) -> Option<&'a #helper_name> {
                    if self.tag != #idx_lit { return None; }
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
                {
                    let mut guard = guard.push_variant(stringify!(#variant_ident));
                    #layout_checks
                    #(#field_validations)*
                }
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
                let fields = record.fields.iter().enumerate().map(|(fi, field)| {
                    if field.skip {
                        quote! { _ }
                    } else {
                        let ident = format_ident!("field{}", fi);
                        quote! { #ident }
                    }
                });
                quote! { Self::#variant_ident( #(#fields),* ) }
            }
            RecordStyle::Unit => quote! { Self::#variant_ident },
        };

        let mut fields = Vec::new();
        if let Some(key) = &helper_stable_schema_key {
            let revision = record.schema_revision;
            match record.style {
                RecordStyle::Named => {
                    fields.push(quote! { stable_schema_key: #key });
                    fields.push(quote! { schema_revision: #revision });
                }
                RecordStyle::Unnamed => {
                    fields.push(quote! { #key });
                    fields.push(quote! { #revision });
                }
                RecordStyle::Unit => {}
            }
        }
        for (fi, field) in record.fields.iter().enumerate() {
            if field.skip {
                continue;
            }
            let archived_member = user_member(record, fi);
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
                    let val_ident = format_ident!("field{}", fi);
                    let res_idx = record.fields[..fi].iter().filter(|f| !f.skip).count();
                    let res_member = syn::Index::from(res_idx);
                    if let Some(wrapper) = packed_wrapper_type_expr(field) {
                        fields.push(quote! {
                            <#wrapper as zebin::Archive>::resolve(
                                &<#wrapper>::new(#val_ident.as_ref()),
                                pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #archived_member),
                                resolver.#res_member
                            )?
                        });
                    } else {
                        fields.push(quote! {
                            #val_ident.resolve(
                                pos + #payload_offset + zebin::memoffset::offset_of!(#helper_name, #archived_member),
                                resolver.#res_member
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
        union #payload_name { #(#variant_payload_fields,)* }
    };

    let (root_bytes, root_archive) = if variants.is_empty() {
        (
            quote! {
                impl zebin::Layout for #archived_name {
                    const ALIGNMENT: ::core::num::NonZeroUsize = unsafe { ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>()) };
                    fn write_archived_bytes(_archived: &Self, out: &mut [u8]) { zebin::utils::byteops::fill(out, 0); }
                }
                impl zebin::Validate for #archived_name {
                    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), zebin::ValidateError>
                    where H: zebin::ArchiveHeaderTrait, C: zebin::ValidationContext<H> + ?Sized {
                        let mut guard = context.guard()?;
                        guard.check_alignment(ptr as *const u8, <Self as zebin::Layout>::ALIGNMENT)?;
                        guard.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?;
                        Ok(())
                    }
                }
            },
            quote! {
                impl zebin::Archive for #name {
                    type Archived = #archived_name; type Resolver = #resolver_name;
                    fn resolve(&self, _pos: usize, _resolver: Self::Resolver) -> Result<Self::Archived, zebin::ArchiveError> { match *self {} }
                }
            },
        )
    } else {
        (
            quote! {
                impl zebin::Layout for #archived_name {
                    const ALIGNMENT: ::core::num::NonZeroUsize = unsafe { ::core::num::NonZeroUsize::new_unchecked(::core::mem::align_of::<Self>()) };
                    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                        zebin::utils::byteops::fill(out, 0);
                        <u32 as zebin::Layout>::write_archived_bytes(&archived.tag, &mut out[0..::core::mem::size_of::<u32>()]);
                        let payload_offset = zebin::memoffset::offset_of!(#archived_name, payload);
                        match archived.tag { #(#variant_write_arms)* _ => {} }
                    }
                }
                impl zebin::Validate for #archived_name {
                    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), zebin::ValidateError>
                    where H: zebin::ArchiveHeaderTrait, C: zebin::ValidationContext<H> + ?Sized {
                        let mut guard = context.guard()?;
                        guard.check_alignment(ptr as *const u8, <Self as zebin::Layout>::ALIGNMENT)?;
                        guard.check_range(ptr as *const u8, ::core::mem::size_of::<Self>())?;
                        let archived = unsafe { &*ptr };
                        match archived.tag {
                            #(#variant_validate_arms)*
                            _ => return Err(zebin::ValidateError::ValidationError { message: "Invalid enum discriminant", pos: ptr as usize }),
                        }
                        Ok(())
                    }
                }
            },
            quote! {
                impl zebin::Archive for #name {
                    type Archived = #archived_name; type Resolver = #resolver_name;
                    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, zebin::ArchiveError> {
                        match (self, resolver) { #(#variant_resolve_arms),* _ => Err(zebin::ArchiveError::InvalidResolver { pos }), }
                    }
                }
            },
        )
    };

    let view_trait_name = format_ident!("{}ViewAccess", archived_name);
    let mut view_trait_methods = Vec::new();
    let mut view_impl_methods = Vec::new();

    view_trait_methods.push(quote! { fn tag(&self) -> u32; });
    view_impl_methods.push(quote! { fn tag(&self) -> u32 { self.data().tag() } });

    for (idx, variant) in variants.iter().enumerate() {
        let variant_user_ident = variant.rename.as_ref().unwrap_or(variant.ident);
        let helper_name = variant_archived_name(name, variant_user_ident);
        let accessor_name = if variant.record.fields.is_empty() {
            variant_method_name("is", variant_user_ident)
        } else {
            variant_method_name("as", variant_user_ident)
        };
        let idx_lit = idx as u32;

        if variant.record.fields.is_empty() {
            view_trait_methods.push(quote! { fn #accessor_name(&self) -> bool; });
            view_impl_methods.push(
                quote! { fn #accessor_name(&self) -> bool { self.data().tag() == #idx_lit } },
            );
        } else if has_schema(&variant.record) {
            view_trait_methods.push(quote! {
                fn #accessor_name(&self) -> Result<Option<zebin::View<'a, &'a #helper_name, H>>, zebin::ZebinError>;
            });
            view_impl_methods.push(quote! {
                fn #accessor_name(&self) -> Result<Option<zebin::View<'a, &'a #helper_name, H>>, zebin::ZebinError> {
                    let variant = unsafe { self.data().#accessor_name() };
                    match variant {
                        Some(v) => Ok(Some(self.view(v)?)),
                        None => Ok(None),
                    }
                }
            });
        } else {
            view_trait_methods.push(quote! {
                fn #accessor_name(&self) -> Option<&'a #helper_name>;
            });
            view_impl_methods.push(quote! {
                fn #accessor_name(&self) -> Option<&'a #helper_name> {
                    unsafe { self.data().#accessor_name() }
                }
            });
        }
    }

    let root_accessors = quote! {
        impl #archived_name {
            pub fn tag(&self) -> u32 { self.tag }
            #(#variant_accessors)*
        }

        pub trait #view_trait_name<'a, H: zebin::ArchiveHeaderTrait> {
            #(#view_trait_methods)*
        }

        impl<'a, H: zebin::ArchiveHeaderTrait> #view_trait_name<'a, H> for zebin::View<'a, &'a #archived_name, H> {
            #(#view_impl_methods)*
        }
    };
    let has_schema_variants = variants.iter().any(|v| has_schema(&v.record));
    let span_expr = if has_schema_variants {
        quote! { 4 }
    } else {
        quote! { ::core::mem::size_of::<Self>() }
    };

    let root_decode = quote! {
        impl<'a> zebin::Access<'a> for #archived_name {
            type View = &'a Self;
            unsafe fn access<H, C>(ptr: *const u8, context: &mut C) -> Result<(Self::View, usize), zebin::AccessError>
            where H: zebin::ArchiveHeaderTrait, C: zebin::ValidationContext<H> + ?Sized {
                let typed_ptr = ptr as *const Self;
                unsafe { <Self as zebin::Validate>::validate::<H, C>(typed_ptr, context)?; }
                Ok((unsafe { &*typed_ptr }, #span_expr))
            }
        }
    };

    quote! {
        #(#variant_defs)*
        #[repr(C)]
        pub struct #archived_name { tag: u32, payload: #payload_name, }
        #payload_struct
        #root_bytes
        #root_decode
        #root_accessors
        #root_archive
    }
}

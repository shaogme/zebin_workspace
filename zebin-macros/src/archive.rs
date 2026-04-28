use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Field, Result, parse_macro_input, spanned::Spanned};

fn parse_field_id(field: &Field) -> Result<Option<u16>> {
    let mut field_id: Option<u16> = None;
    for attr in &field.attrs {
        if attr.path().is_ident("zebin") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    let value = meta.value()?;
                    field_id = Some(value.parse::<syn::LitInt>()?.base10_parse::<u16>()?);
                }
                Ok(())
            })?;
        }
    }
    Ok(field_id)
}

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let span = input.span();
    let name = input.ident;
    let archived_name = quote::format_ident!("Archived{}", name);
    let resolver_name = quote::format_ident!("{}Resolver", name);

    let fields = match &input.data {
        syn::Data::Struct(syn::DataStruct {
            fields: syn::Fields::Named(syn::FieldsNamed { named, .. }),
            ..
        }) => named,
        _ => {
            return syn::Error::new(span, "ZebinArchive 只支持具名字段的 struct")
                .to_compile_error()
                .into();
        }
    };

    let mut field_ids = Vec::with_capacity(fields.len());
    for field in fields.iter() {
        match parse_field_id(field) {
            Ok(id) => field_ids.push(id),
            Err(err) => return err.to_compile_error().into(),
        }
    }

    let is_evolvable = field_ids.iter().any(|id| id.is_some());
    if is_evolvable {
        for (field, field_id) in fields.iter().zip(field_ids.iter()) {
            if field_id.is_none() {
                return syn::Error::new_spanned(
                    field,
                    "启用 #[zebin(id = ...)] 后，所有字段都必须提供 id",
                )
                .to_compile_error()
                .into();
            }
        }
    }

    let archived_field_defs = fields.iter().map(|f| {
        let name = &f.ident;
        let ty = &f.ty;
        quote! { pub #name: <#ty as zebin::Archive>::Archived }
    });

    let resolver_field_defs = fields.iter().map(|f| {
        let name = &f.ident;
        let ty = &f.ty;
        quote! { pub #name: <#ty as zebin::Archive>::Resolver }
    });

    let resolve_impl_fields = fields.iter().map(|f| {
        let name = &f.ident;
        quote! {
            #name: self.#name.resolve(
                pos + zebin::memoffset::offset_of!(#archived_name, #name),
                resolver.#name
            )?
        }
    });

    let write_archived_fields = fields.iter().map(|f| {
        let name = &f.ident;
        let ty = &f.ty;
        quote! {
            {
                let offset = zebin::memoffset::offset_of!(#archived_name, #name);
                let size = ::std::mem::size_of::<<#ty as zebin::Archive>::Archived>();
                <#ty as zebin::Archive>::write_archived_bytes(
                    &archived.#name,
                    &mut out[offset..offset + size],
                );
            }
        }
    });

    let write_schema_field = if is_evolvable {
        quote! {
            <u32 as zebin::Archive>::write_archived_bytes(
                &archived.schema_id,
                &mut out[0..::std::mem::size_of::<u32>()],
            );
        }
    } else {
        quote! {}
    };

    let accessors = if is_evolvable {
        let methods = fields.iter().zip(field_ids.iter()).map(|(f, id)| {
            let name = &f.ident;
            let ty = &f.ty;
            let id = id.expect("field ids are validated above");

            quote! {
                pub unsafe fn #name<'a>(
                    &'a self,
                    buffer: &'a [u8],
                ) -> Result<&'a <#ty as zebin::Archive>::Archived, zebin::ZebinError> {
                    let header = zebin::ArchiveHeader::parse(buffer)?;
                    let layout_dir = zebin::LayoutDirectory::new(
                        buffer,
                        ::std::num::NonZeroUsize::new(header.layout_offset.get() as usize).ok_or_else(|| zebin::ZebinError::ValidationError {
                            message: "Layout offset cannot be zero".to_string(),
                            pos: 4,
                        })?,
                    );
                    let layout = layout_dir.lookup(self.schema_id)?;
                    let offset = layout.field_offset(#id).ok_or_else(|| zebin::ZebinError::ValidationError {
                        message: format!("Field ID {} not found in layout", #id),
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
    } else {
        quote! {}
    };

    let schema_field = if is_evolvable {
        quote! { pub schema_id: u32, }
    } else {
        quote! {}
    };

    let resolver_schema_field = if is_evolvable {
        quote! { pub schema_id: u32, }
    } else {
        quote! {}
    };

    let resolve_schema_init = if is_evolvable {
        quote! { schema_id: resolver.schema_id, }
    } else {
        quote! {}
    };

    let alignments: Vec<_> = if is_evolvable {
        let mut values = Vec::with_capacity(fields.len() + 1);
        values.push(quote! { <u32 as zebin::Archive>::ALIGNMENT });
        values.extend(fields.iter().map(|f| {
            let ty = &f.ty;
            quote! { <#ty as zebin::Archive>::ALIGNMENT }
        }));
        values
    } else {
        fields
            .iter()
            .map(|f| {
                let ty = &f.ty;
                quote! { <#ty as zebin::Archive>::ALIGNMENT }
            })
            .collect()
    };

    let validate_impl_fields = fields.iter().map(|f| {
        let name = &f.ident;
        let ty = &f.ty;
        quote! {
            {
                let field_ptr = unsafe { std::ptr::addr_of!((*ptr).#name) };
                unsafe { <<#ty as zebin::Archive>::Archived as zebin::Validate<zebin::Validator<'_>>>::validate(field_ptr, context)? };
            }
        }
    });

    let layout_validation = if is_evolvable {
        let checks = fields.iter().zip(field_ids.iter()).map(|(f, id)| {
            let name = &f.ident;
            let id = id.expect("field ids are validated above");

            quote! {
                layout.check_field(#id, zebin::memoffset::offset_of!(#archived_name, #name) as u16)?;
            }
        });

        quote! {
            let archived = unsafe { &*ptr };
            let layout = context.layout(archived.schema_id)?;
            #(#checks)*
        }
    } else {
        quote! {}
    };

    let validate_impl = quote! {
        impl zebin::Validate<zebin::Validator<'_>> for #archived_name {
            const ALIGNMENT: ::std::num::NonZeroUsize = {
                let mut max = 1usize;
                #(
                    let align = #alignments.get();
                    if align > max {
                        max = align;
                    }
                )*
                unsafe { ::std::num::NonZeroUsize::new_unchecked(max) }
            };

            unsafe fn validate(ptr: *const Self, context: &mut zebin::Validator<'_>) -> Result<(), zebin::ZebinError> {
                let _guard = context.enter()?;
                context.check_range(ptr as *const u8, std::mem::size_of::<Self>())?;
                context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
                #layout_validation

                #(#validate_impl_fields)*

                Ok(())
            }
        }
    };

    let expanded = quote! {
        #[repr(C)]
        pub struct #archived_name {
            #schema_field
            #(#archived_field_defs,)*
        }

        #accessors

        #validate_impl

        pub struct #resolver_name {
            #resolver_schema_field
            #(#resolver_field_defs,)*
        }

        impl zebin::Archive for #name {
            type Archived = #archived_name;
            type Resolver = #resolver_name;
            const ALIGNMENT: ::std::num::NonZeroUsize = {
                let mut max = 1usize;
                #(
                    let align = #alignments.get();
                    if align > max {
                        max = align;
                    }
                )*
                unsafe { ::std::num::NonZeroUsize::new_unchecked(max) }
            };

            fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, zebin::ZebinError> {
                Ok(#archived_name {
                    #resolve_schema_init
                    #(#resolve_impl_fields),*
                })
            }

            fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
                out.fill(0);
                #write_schema_field
                #(#write_archived_fields)*
            }
        }
    };

    TokenStream::from(expanded)
}

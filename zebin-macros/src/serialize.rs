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
    let state_name = quote::format_ident!("{}SerializeState", name);

    let fields = match &input.data {
        syn::Data::Struct(syn::DataStruct {
            fields: syn::Fields::Named(syn::FieldsNamed { named, .. }),
            ..
        }) => named,
        _ => {
            return syn::Error::new(span, "ZebinSerialize 只支持具名字段的 struct")
                .to_compile_error()
                .into();
        }
    };

    let mut field_ids = Vec::with_capacity(fields.len());
    let mut field_names = Vec::with_capacity(fields.len());
    let mut field_tys = Vec::with_capacity(fields.len());
    for f in fields.iter() {
        let ident = match f.ident.as_ref() {
            Some(ident) => ident,
            None => unreachable!("ZebinSerialize only supports structs with named fields"),
        };
        field_names.push(ident);
        field_tys.push(&f.ty);

        match parse_field_id(f) {
            Ok(field_id) => field_ids.push(field_id),
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

    let state_field_defs = field_names.iter().zip(field_tys.iter()).map(|(field, ty)| {
        let state_field = quote::format_ident!("{}_state", field);
        quote! {
            pub #state_field: <#ty as zebin::Serialize>::State<'a>,
            pub #field: ::core::option::Option<<#ty as zebin::Archive>::Resolver>,
        }
    });

    let state_inits = field_names.iter().zip(field_tys.iter()).map(|(field, ty)| {
        let state_field = quote::format_ident!("{}_state", field);
        quote! {
            #state_field: <#ty as zebin::Serialize>::begin(&self.#field)?,
            #field: ::core::option::Option::None,
        }
    });

    let layout_fields = if is_evolvable {
        let entries = field_ids.iter().zip(field_names.iter()).map(|(id, field)| {
            let id = id.expect("field ids are validated above");
            quote! {
                zebin::LayoutField {
                    field_id: #id,
                    offset: zebin::memoffset::offset_of!(#archived_name, #field) as u16,
                }
            }
        });

        quote! {
            let layout: &[zebin::LayoutField] = &[
                #(#entries),*
            ];
            if self.schema_id.is_none() {
                self.schema_id = Some(encoder.register_layout(layout)?);
            }
        }
    } else {
        quote! {}
    };

    let poll_steps = field_names.iter().map(|field| {
        let state_field = quote::format_ident!("{}_state", field);
        quote! {
                    if self.#field.is_none() {
                        match self.#state_field.poll(encoder)? {
                            zebin::SerializePoll::Pending => return Ok(zebin::SerializePoll::Pending),
                            zebin::SerializePoll::Error(err) => {
                                return Ok(zebin::SerializePoll::Error(err))
                            }
                            zebin::SerializePoll::Ready(resolver) => {
                                self.#field = ::core::option::Option::Some(resolver);
                            }
                        }
                    }
        }
    });

    let resolver_inits = field_names.iter().map(|field| {
        quote! {
            #field: self.#field.take().expect("field resolver available after polling")
        }
    });

    let schema_field_def = if is_evolvable {
        quote! {
            pub schema_id: ::core::option::Option<u32>,
        }
    } else {
        quote! {}
    };

    let schema_field_init = if is_evolvable {
        quote! { schema_id: ::core::option::Option::None, }
    } else {
        quote! {}
    };

    let schema_resolver_init = if is_evolvable {
        quote! {
            schema_id: self
                .schema_id
                .expect("schema_id registered before resolution"),
        }
    } else {
        quote! {}
    };

    let state_struct = quote! {
        pub struct #state_name<'a> {
            #schema_field_def
            #(#state_field_defs)*
        }
    };

    let expanded = if is_evolvable {
        quote! {
            #state_struct

            impl<'a> zebin::SerializeState for #state_name<'a> {
                type Resolver = #resolver_name;

                fn poll<E: zebin::Encoder + ?Sized>(
                    &mut self,
                    encoder: &mut E,
                ) -> Result<zebin::SerializePoll<Self::Resolver>, E::Error>
                where
                    E::Error: ::core::convert::From<zebin::ZebinError>,
                {
                    #layout_fields
                    #(#poll_steps)*

                    Ok(zebin::SerializePoll::Ready(#resolver_name {
                        #schema_resolver_init
                        #(#resolver_inits),*
                    }))
                }
            }

            impl zebin::Serialize for #name {
                type State<'a> = #state_name<'a> where Self: 'a;

                fn begin(&self) -> Result<Self::State<'_>, zebin::ZebinError> {
                    Ok(#state_name {
                        #schema_field_init
                        #(#state_inits)*
                    })
                }
            }
        }
    } else {
        quote! {
            #state_struct

            impl<'a> zebin::SerializeState for #state_name<'a> {
                type Resolver = #resolver_name;

                fn poll<E: zebin::Encoder + ?Sized>(
                    &mut self,
                    encoder: &mut E,
                ) -> Result<zebin::SerializePoll<Self::Resolver>, E::Error>
                where
                    E::Error: ::core::convert::From<zebin::ZebinError>,
                {
                    #(#poll_steps)*

                    Ok(zebin::SerializePoll::Ready(#resolver_name {
                        #(#resolver_inits),*
                    }))
                }
            }

            impl zebin::Serialize for #name {
                type State<'a> = #state_name<'a> where Self: 'a;

                fn begin(&self) -> Result<Self::State<'_>, zebin::ZebinError> {
                    Ok(#state_name {
                        #(#state_inits)*
                    })
                }
            }
        }
    };

    TokenStream::from(expanded)
}

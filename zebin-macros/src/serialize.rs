use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::DeriveInput;

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, VariantSpec, has_schema, input_member, parse_item,
    resolver_name, state_name, user_member, variant_resolver_name, variant_state_name,
};

// --- Helper Functions for Code Generation ---

fn resolver_def(resolver_name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let include_schema = has_schema(record);
    match record.style {
        RecordStyle::Named => {
            let mut fields = Vec::new();
            if include_schema {
                fields.push(quote! { pub schema_id: u32 });
            }
            for field in &record.fields {
                let ident = field.ident.expect("named field has ident");
                let ty = field.ty;
                fields.push(quote! { pub #ident: <#ty as zebin::Archive>::Resolver });
            }
            quote! { #[allow(non_snake_case)] pub struct #resolver_name { #(#fields,)* } }
        }
        RecordStyle::Unnamed => {
            let mut fields = Vec::new();
            if include_schema {
                fields.push(quote! { pub u32 });
            }
            for field in &record.fields {
                let ty = field.ty;
                fields.push(quote! { <#ty as zebin::Archive>::Resolver });
            }
            quote! { #[allow(non_snake_case)] pub struct #resolver_name( #(#fields,)* ); }
        }
        RecordStyle::Unit => quote! { #[allow(non_snake_case)] pub struct #resolver_name; },
    }
}

fn state_def(state_name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let include_schema = has_schema(record);
    let fields = record.fields.iter().enumerate().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let ty = field.ty;
        let resolver_ident = match record.style {
            RecordStyle::Named => field.ident.expect("named field has ident").clone(),
            RecordStyle::Unnamed => format_ident!("field{}", index),
            RecordStyle::Unit => unreachable!("unit has no fields"),
        };
        quote! {
            pub #state_ident: <#ty as zebin::Serialize>::State<'a>,
            pub #resolver_ident: ::core::option::Option<<#ty as zebin::Archive>::Resolver>,
        }
    });

    let schema_field = if include_schema {
        quote! { pub schema_id: ::core::option::Option<u32>, }
    } else {
        quote! {}
    };

    quote! {
        #[allow(non_snake_case)]
        pub struct #state_name<'a> {
            pub _marker: ::core::marker::PhantomData<&'a ()>,
            #schema_field
            #(#fields)*
        }
    }
}

fn state_init(record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let include_schema = has_schema(record);
    let schema_init = if include_schema {
        quote! { schema_id: ::core::option::Option::None, }
    } else {
        quote! {}
    };

    let fields = record.fields.iter().enumerate().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let ty = field.ty;
        let resolver_ident = match record.style {
            RecordStyle::Named => field.ident.expect("named field has ident").clone(),
            RecordStyle::Unnamed => format_ident!("field{}", index),
            RecordStyle::Unit => unreachable!("unit has no fields"),
        };
        let input_member = input_member(record, index);
        quote! {
            #state_ident: <#ty as zebin::Serialize>::begin(&self.#input_member)?,
            #resolver_ident: ::core::option::Option::None,
        }
    });

    quote! {
        #schema_init
        _marker: ::core::marker::PhantomData,
        #(#fields)*
    }
}

fn layout_fields(record: &RecordSpec<'_>, archived_name: &syn::Ident) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }

    let entries = record.fields.iter().enumerate().map(|(index, field)| {
        let field_id = field.field_id.expect("field ids are validated above");
        let member = user_member(record, index);
        quote! {
            zebin::LayoutField {
                field_id: #field_id,
                offset: zebin::memoffset::offset_of!(#archived_name, #member) as u16,
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
}

fn poll_steps(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    record
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let state_ident = &field.state_ident;
            let resolver_ident = match record.style {
                RecordStyle::Named => field.ident.expect("named field has ident").clone(),
                RecordStyle::Unnamed => format_ident!("field{}", index),
                RecordStyle::Unit => unreachable!("unit has no fields"),
            };
            quote! {
                if self.#resolver_ident.is_none() {
                    match self.#state_ident.poll(encoder)? {
                        zebin::SerializePoll::Pending => return Ok(zebin::SerializePoll::Pending),
                        zebin::SerializePoll::Error(err) => {
                            return Ok(zebin::SerializePoll::Error(err))
                        }
                        zebin::SerializePoll::Ready(resolver) => {
                            self.#resolver_ident = ::core::option::Option::Some(resolver);
                        }
                    }
                }
            }
        })
        .collect()
}

fn resolver_expr(record: &RecordSpec<'_>, resolver_name: &syn::Ident) -> proc_macro2::TokenStream {
    let include_schema = has_schema(record);
    match record.style {
        RecordStyle::Named => {
            let mut fields = Vec::new();
            if include_schema {
                fields.push(quote! { schema_id: self.schema_id.expect("schema_id registered before resolution") });
            }
            for field in &record.fields {
                let ident = field.ident.expect("named field has ident");
                fields.push(quote! {
                    #ident: self.#ident.take().expect("field resolver available after polling")
                });
            }
            quote! { #resolver_name { #(#fields),* } }
        }
        RecordStyle::Unnamed => {
            let mut fields = Vec::new();
            if include_schema {
                fields.push(
                    quote! { self.schema_id.expect("schema_id registered before resolution") },
                );
            }
            for (index, _field) in record.fields.iter().enumerate() {
                let resolver_ident = format_ident!("field{}", index);
                fields.push(quote! {
                    self.#resolver_ident
                        .take()
                        .expect("field resolver available after polling")
                });
            }
            quote! { #resolver_name( #(#fields),* ) }
        }
        RecordStyle::Unit => quote! { #resolver_name },
    }
}

// --- Record State Implementation ---

fn record_state_impl(
    state_name: &syn::Ident,
    resolver_name: &syn::Ident,
    record: &RecordSpec<'_>,
    archived_name: &syn::Ident,
) -> proc_macro2::TokenStream {
    let layout = layout_fields(record, archived_name);
    let polls = poll_steps(record);
    let resolver_expr = resolver_expr(record, resolver_name);

    if has_schema(record) {
        quote! {
            impl<'a> zebin::SerializeState for #state_name<'a> {
                type Resolver = #resolver_name;

                fn poll<E: zebin::Encoder + ?Sized>(
                    &mut self,
                    encoder: &mut E,
                ) -> Result<zebin::SerializePoll<Self::Resolver>, E::Error>
                where
                    E::Error: ::core::convert::From<zebin::ZebinError>,
                {
                    #layout
                    #(#polls)*

                    Ok(zebin::SerializePoll::Ready(#resolver_expr))
                }
            }
        }
    } else {
        quote! {
            impl<'a> zebin::SerializeState for #state_name<'a> {
                type Resolver = #resolver_name;

                fn poll<E: zebin::Encoder + ?Sized>(
                    &mut self,
                    encoder: &mut E,
                ) -> Result<zebin::SerializePoll<Self::Resolver>, E::Error>
                where
                    E::Error: ::core::convert::From<zebin::ZebinError>,
                {
                    #(#polls)*

                    Ok(zebin::SerializePoll::Ready(#resolver_expr))
                }
            }
        }
    }
}

// --- Struct Implementation ---

fn struct_impl(name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let archived_name = crate::shared::archived_name(name);
    let resolver_name = resolver_name(name);
    let state_name = state_name(name);
    let resolver_def = resolver_def(&resolver_name, record);
    let state_def = state_def(&state_name, record);
    let state_impl = record_state_impl(&state_name, &resolver_name, record, &archived_name);
    let init_fields = state_init(record);

    quote! {
        #state_def
        #resolver_def
        #state_impl

        impl zebin::Serialize for #name {
            type State<'a> = #state_name<'a> where Self: 'a;

            fn begin(&self) -> Result<Self::State<'_>, zebin::ZebinError> {
                Ok(#state_name {
                    #init_fields
                })
            }
        }
    }
}

fn variant_state_def(
    enum_name: &syn::Ident,
    variant: &VariantSpec<'_>,
) -> proc_macro2::TokenStream {
    let name = variant_state_name(enum_name, variant.ident);
    state_def(&name, &variant.record)
}

fn variant_state_impl(
    enum_name: &syn::Ident,
    variant: &VariantSpec<'_>,
) -> proc_macro2::TokenStream {
    let state = variant_state_name(enum_name, variant.ident);
    let resolver = variant_resolver_name(enum_name, variant.ident);
    let archived_name = crate::shared::variant_archived_name(enum_name, variant.ident);
    record_state_impl(&state, &resolver, &variant.record, &archived_name)
}

fn variant_resolver_def(
    enum_name: &syn::Ident,
    variant: &VariantSpec<'_>,
) -> proc_macro2::TokenStream {
    let name = variant_resolver_name(enum_name, variant.ident);
    resolver_def(&name, &variant.record)
}

fn variant_begin_arm(
    state_name: &syn::Ident,
    enum_name: &syn::Ident,
    variant: &VariantSpec<'_>,
) -> proc_macro2::TokenStream {
    let state = variant_state_name(enum_name, variant.ident);
    let variant_ident = variant.ident;
    let include_schema = has_schema(&variant.record);
    match variant.record.style {
        RecordStyle::Named => {
            let binders = variant
                .record
                .fields
                .iter()
                .map(|field| field.ident.expect("named field has ident").clone());
            let init_fields = variant.record.fields.iter().map(|field| {
                let ident = field.ident.expect("named field has ident");
                let state_ident = &field.state_ident;
                let ty = field.ty;
                quote! {
                    #state_ident: <#ty as zebin::Serialize>::begin(&#ident)?,
                    #ident: ::core::option::Option::None,
                }
            });
            if include_schema {
                quote! {
                    Self::#variant_ident { #(#binders),* } => {
                        Ok(#state_name::#variant_ident(
                            #state {
                                _marker: ::core::marker::PhantomData,
                                schema_id: ::core::option::Option::None,
                                #(#init_fields)*
                            }
                        ))
                    }
                }
            } else {
                quote! {
                    Self::#variant_ident { #(#binders),* } => {
                        Ok(#state_name::#variant_ident(
                            #state {
                                _marker: ::core::marker::PhantomData,
                                #(#init_fields)*
                            }
                        ))
                    }
                }
            }
        }
        RecordStyle::Unnamed => {
            let binders = variant
                .record
                .fields
                .iter()
                .enumerate()
                .map(|(index, _)| format_ident!("field{}", index));
            let init_fields = variant
                .record
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let binder = format_ident!("field{}", index);
                    let state_ident = &field.state_ident;
                    let ty = field.ty;
                    quote! {
                        #state_ident: <#ty as zebin::Serialize>::begin(&#binder)?,
                        #binder: ::core::option::Option::None,
                    }
                });
            if include_schema {
                quote! {
                    Self::#variant_ident( #(#binders),* ) => {
                        Ok(#state_name::#variant_ident(
                            #state {
                                _marker: ::core::marker::PhantomData,
                                schema_id: ::core::option::Option::None,
                                #(#init_fields)*
                            }
                        ))
                    }
                }
            } else {
                quote! {
                    Self::#variant_ident( #(#binders),* ) => {
                        Ok(#state_name::#variant_ident(
                            #state {
                                _marker: ::core::marker::PhantomData,
                                #(#init_fields)*
                            }
                        ))
                    }
                }
            }
        }
        RecordStyle::Unit => {
            quote! {
                Self::#variant_ident => {
                    Ok(#state_name::#variant_ident(#state {
                        _marker: ::core::marker::PhantomData,
                    }))
                }
            }
        }
    }
}

// --- Enum Implementation ---

fn enum_impl(name: &syn::Ident, variants: &[VariantSpec<'_>]) -> proc_macro2::TokenStream {
    let state_name = state_name(name);
    let resolver_name = resolver_name(name);
    let variant_state_defs = variants
        .iter()
        .map(|variant| variant_state_def(name, variant));
    let variant_state_impls = variants
        .iter()
        .map(|variant| variant_state_impl(name, variant));
    let variant_resolver_defs = variants
        .iter()
        .map(|variant| variant_resolver_def(name, variant));

    let state_enum_variants = variants.iter().map(|variant| {
        let variant_state = variant_state_name(name, variant.ident);
        let variant_ident = variant.ident;
        quote! { #variant_ident(#variant_state<'a>) }
    });

    let resolver_enum_variants = variants.iter().map(|variant| {
        let variant_resolver = variant_resolver_name(name, variant.ident);
        let variant_ident = variant.ident;
        quote! { #variant_ident(#variant_resolver) }
    });

    let begin_arms = variants
        .iter()
        .map(|variant| variant_begin_arm(&state_name, name, variant));

    let poll_arms = variants.iter().map(|variant| {
        let variant_ident = variant.ident;
        quote! {
            #state_name::#variant_ident(state) => match state.poll(encoder)? {
                zebin::SerializePoll::Pending => Ok(zebin::SerializePoll::Pending),
                zebin::SerializePoll::Error(err) => Ok(zebin::SerializePoll::Error(err)),
                zebin::SerializePoll::Ready(resolver) => {
                    Ok(zebin::SerializePoll::Ready(#resolver_name::#variant_ident(resolver)))
                }
            }
        }
    });

    quote! {
        #(#variant_state_defs)*
        #(#variant_state_impls)*
        #(#variant_resolver_defs)*

        pub enum #state_name<'a> {
            #(#state_enum_variants),*
        }

        pub enum #resolver_name {
            #(#resolver_enum_variants),*
        }

        impl<'a> zebin::SerializeState for #state_name<'a> {
            type Resolver = #resolver_name;

            fn poll<E: zebin::Encoder + ?Sized>(
                &mut self,
                encoder: &mut E,
            ) -> Result<zebin::SerializePoll<Self::Resolver>, E::Error>
            where
                E::Error: ::core::convert::From<zebin::ZebinError>,
            {
                match self {
                    #(#poll_arms),*
                }
            }
        }

        impl zebin::Serialize for #name {
            type State<'a> = #state_name<'a> where Self: 'a;

            fn begin(&self) -> Result<Self::State<'_>, zebin::ZebinError> {
                match self {
                    #(#begin_arms),*
                }
            }
        }
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

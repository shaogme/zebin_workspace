use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::DeriveInput;

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, VariantSpec, binder_slot_ident, field_resolver_type,
    has_schema, input_member, layout_field_entries, packed_begin_expr, packed_wrapper_type,
    parse_item, resolver_name, resolver_slot_ident, state_name, state_slot_ident,
    variant_resolver_name, variant_state_name,
};

// --- Helper Functions for Code Generation ---

fn resolver_def(resolver_name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    match record.style {
        RecordStyle::Named => {
            let mut fields = Vec::new();
            for field in &record.fields {
                let ident = field.ident.expect("named field has ident");
                let ty = field_resolver_type(field);
                fields.push(quote! { pub #ident: #ty });
            }
            quote! { pub struct #resolver_name { #(#fields,)* } }
        }
        RecordStyle::Unnamed => {
            let mut fields = Vec::new();
            for field in &record.fields {
                let ty = field_resolver_type(field);
                fields.push(quote! { #ty });
            }
            quote! { pub struct #resolver_name( #(#fields,)* ); }
        }
        RecordStyle::Unit => quote! { pub struct #resolver_name; },
    }
}

fn state_def(state_name: &syn::Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = record.fields.iter().enumerate().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let state_ty = if let Some(wrapper) = packed_wrapper_type(field) {
            quote! { <#wrapper as zebin::ArchiveBuilder>::State<'a> }
        } else {
            let ty = field.ty;
            quote! { <#ty as zebin::ArchiveBuilder>::State<'a> }
        };
        let resolver_ty = if let Some(wrapper) = packed_wrapper_type(field) {
            quote! { <#wrapper as zebin::Archive>::Resolver }
        } else {
            let ty = field.ty;
            quote! { <#ty as zebin::Archive>::Resolver }
        };
        let resolver_ident = resolver_slot_ident(record, index);
        quote! {
            pub #state_ident: #state_ty,
            pub #resolver_ident: ::core::option::Option<#resolver_ty>,
        }
    });

    quote! {
        pub struct #state_name<'a> {
            pub _marker: ::core::marker::PhantomData<&'a ()>,
            #(#fields)*
        }
    }
}

fn state_init(record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = record.fields.iter().enumerate().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let resolver_ident = resolver_slot_ident(record, index);
        let input_member = input_member(record, index);
        let init = if let Some(init) = packed_begin_expr(field, quote! { self.#input_member }) {
            init
        } else {
            let ty = field.ty;
            quote! {
                <#ty as zebin::ArchiveBuilder>::begin(&self.#input_member)?
            }
        };
        quote! {
            #state_ident: #init,
            #resolver_ident: ::core::option::Option::None,
        }
    });

    quote! {
        _marker: ::core::marker::PhantomData,
        #(#fields)*
    }
}

fn layout_fields(
    record: &RecordSpec<'_>,
    archived_name: &syn::Ident,
    stable_schema_key: &proc_macro2::TokenStream,
    schema_revision: u32,
) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }

    let entries = layout_field_entries(record, archived_name);

    quote! {
        let layout: &[zebin::LayoutField] = &[
            #(#entries),*
        ];
        encoder.register_layout(
            #stable_schema_key,
            #schema_revision,
            zebin::ObjectEncoding::Fixed,
            layout,
        )?;
    }
}

fn poll_steps(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    record
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let state_ident = &field.state_ident;
            let resolver_ident = resolver_slot_ident(record, index);
            quote! {
                if self.#resolver_ident.is_none() {
                    match self.#state_ident.poll(encoder)? {
                        ::core::task::Poll::Pending => return Ok(::core::task::Poll::Pending),
                        ::core::task::Poll::Ready(resolver) => {
                            self.#resolver_ident = ::core::option::Option::Some(resolver);
                        }
                    }
                }
            }
        })
        .collect()
}

fn resolver_expr(record: &RecordSpec<'_>, resolver_name: &syn::Ident) -> proc_macro2::TokenStream {
    match record.style {
        RecordStyle::Named => {
            let mut fields = Vec::new();
            for (index, _field) in record.fields.iter().enumerate() {
                let ident = state_slot_ident(record, index);
                fields.push(quote! {
                    #ident: self.#ident.take().expect("field resolver available after polling")
                });
            }
            quote! { #resolver_name { #(#fields),* } }
        }
        RecordStyle::Unnamed => {
            let mut fields = Vec::new();
            for (index, _field) in record.fields.iter().enumerate() {
                let resolver_ident = resolver_slot_ident(record, index);
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
    stable_schema_key: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let layout = layout_fields(
        record,
        archived_name,
        stable_schema_key,
        record.schema_revision,
    );
    let polls = poll_steps(record);
    let resolver_expr = resolver_expr(record, resolver_name);

    if has_schema(record) {
        quote! {
            impl<'a> zebin::ArchiveState for #state_name<'a> {
                type Resolver = #resolver_name;

                fn poll<E: zebin::ByteSink + zebin::LayoutSink + ?Sized>(
                    &mut self,
                    encoder: &mut E,
                ) -> Result<::core::task::Poll<Self::Resolver>, zebin::ZebinError>
                {
                    #layout
                    #(#polls)*

                    Ok(::core::task::Poll::Ready(#resolver_expr))
                }
            }
        }
    } else {
        quote! {
            impl<'a> zebin::ArchiveState for #state_name<'a> {
                type Resolver = #resolver_name;

                fn poll<E: zebin::ByteSink + zebin::LayoutSink + ?Sized>(
                    &mut self,
                    encoder: &mut E,
                ) -> Result<::core::task::Poll<Self::Resolver>, zebin::ZebinError>
                {
                    #(#polls)*

                    Ok(::core::task::Poll::Ready(#resolver_expr))
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
    let stable_schema_key = if has_schema(record) {
        let stable_schema_key = record
            .stable_schema_key
            .expect("schema-bearing records require an explicit stable schema key");
        Some(quote! { #stable_schema_key })
    } else {
        None
    };
    let state_impl = record_state_impl(
        &state_name,
        &resolver_name,
        record,
        &archived_name,
        &stable_schema_key.unwrap_or_else(|| quote! { 0 }),
    );
    let init_fields = state_init(record);

    quote! {
        #state_def
        #resolver_def
        #state_impl

        impl zebin::ArchiveBuilder for #name {
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
    let stable_schema_key = if has_schema(&variant.record) {
        let stable_schema_key = variant
            .record
            .stable_schema_key
            .expect("schema-bearing records require an explicit stable schema key");
        Some(quote! { #stable_schema_key })
    } else {
        None
    };
    record_state_impl(
        &state,
        &resolver,
        &variant.record,
        &archived_name,
        &stable_schema_key.unwrap_or_else(|| quote! { 0 }),
    )
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
    match variant.record.style {
        RecordStyle::Named => {
            let binders = variant
                .record
                .fields
                .iter()
                .enumerate()
                .map(|(index, _)| binder_slot_ident(&variant.record, index));
            let init_fields = variant.record.fields.iter().map(|field| {
                let ident = field.ident.expect("named field has ident");
                let state_ident = &field.state_ident;
                let ty = field.ty;
                let begin = if let Some(begin) = packed_begin_expr(field, quote! { #ident }) {
                    begin
                } else {
                    quote! {
                        <#ty as zebin::ArchiveBuilder>::begin(&#ident)?
                    }
                };
                quote! {
                    #state_ident: #begin,
                    #ident: ::core::option::Option::None,
                }
            });
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
        RecordStyle::Unnamed => {
            let binders = variant
                .record
                .fields
                .iter()
                .enumerate()
                .map(|(index, _)| binder_slot_ident(&variant.record, index));
            let init_fields = variant
                .record
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let binder = format_ident!("field{}", index);
                    let state_ident = &field.state_ident;
                    let ty = field.ty;
                    let begin = if let Some(begin) = packed_begin_expr(field, quote! { #binder }) {
                        begin
                    } else {
                        quote! {
                            <#ty as zebin::ArchiveBuilder>::begin(&#binder)?
                        }
                    };
                    quote! {
                        #state_ident: #begin,
                        #binder: ::core::option::Option::None,
                    }
                });
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
                ::core::task::Poll::Pending => Ok(::core::task::Poll::Pending),
                ::core::task::Poll::Ready(resolver) => {
                    Ok(::core::task::Poll::Ready(#resolver_name::#variant_ident(resolver)))
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

        impl<'a> zebin::ArchiveState for #state_name<'a> {
            type Resolver = #resolver_name;

            fn poll<E: zebin::ByteSink + zebin::LayoutSink + ?Sized>(
                &mut self,
                encoder: &mut E,
            ) -> Result<::core::task::Poll<Self::Resolver>, zebin::ZebinError>
            {
                match self {
                    #(#poll_arms),*
                }
            }
        }

        impl zebin::ArchiveBuilder for #name {
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

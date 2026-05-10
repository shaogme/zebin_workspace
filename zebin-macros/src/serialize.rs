use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, archived_name, field_resolver_type, field_state_type,
    field_user_ident, has_schema, input_member, layout_field_entries, packed_begin_expr,
    parse_item, resolver_name, resolver_slot_ident, state_name,
};

mod enums;

// --- Helper Functions for Record Serialization ---

pub fn resolver_def(resolver_name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    match record.style {
        RecordStyle::Named => {
            let fields = record.active_fields().map(|(index, field)| {
                let ident = field_user_ident(record, index);
                let ty = field_resolver_type(field);
                quote! { pub #ident: #ty }
            });
            quote! { pub struct #resolver_name { #(#fields,)* } }
        }
        RecordStyle::Unnamed => {
            let fields = record.active_fields().map(|(_, field)| {
                let ty = field_resolver_type(field);
                quote! { #ty }
            });
            quote! { pub struct #resolver_name( #(#fields,)* ); }
        }
        RecordStyle::Unit => quote! { pub struct #resolver_name; },
    }
}

pub fn state_def(state_name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = record.active_fields().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let state_ty = field_state_type(field);
        let resolver_ty = field_resolver_type(field);
        let resolver_ident = resolver_slot_ident(record, index);
        quote! { pub #state_ident: #state_ty, pub #resolver_ident: ::core::option::Option<#resolver_ty>, }
    });
    quote! { pub struct #state_name<'a> { pub _marker: ::core::marker::PhantomData<&'a ()>, #(#fields)* } }
}

pub fn state_init(record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = record.active_fields().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let resolver_ident = resolver_slot_ident(record, index);
        let input_member = input_member(record, index);
        let init = if let Some(init) = packed_begin_expr(field, quote! { self.#input_member }) { init } else {
            let ty = field.ty;
            quote! { <#ty as zebin::Serialize>::begin_serialize(&self.#input_member)? }
        };
        quote! { #state_ident: #init, #resolver_ident: ::core::option::Option::None, }
    });
    quote! { _marker: ::core::marker::PhantomData, #(#fields)* }
}

pub fn record_state_poll_logic(
    record: &RecordSpec<'_>,
    archived_name: &Ident,
    stable_schema_key: &proc_macro2::TokenStream,
    prefix: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let layout = if !has_schema(record) { quote! {} } else {
        let entries = layout_field_entries(record, archived_name);
        let schema_revision = record.schema_revision;
        quote! {
            let layout: &[zebin::LayoutField] = &[ #(#entries),* ];
            encoder.register_layout(#stable_schema_key, #schema_revision, zebin::ObjectEncoding::SchemaAware, layout)?;
        }
    };
    let polls = record.active_fields().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let resolver_ident = resolver_slot_ident(record, index);
        quote! {
            if #prefix.#resolver_ident.is_none() {
                match #prefix.#state_ident.poll(encoder)? {
                    ::core::task::Poll::Pending => return Ok(::core::task::Poll::Pending),
                    ::core::task::Poll::Ready(resolver) => { #prefix.#resolver_ident = ::core::option::Option::Some(resolver); }
                }
            }
        }
    });
    quote! { #layout #(#polls)* }
}

pub fn resolver_expr(
    record: &RecordSpec<'_>,
    resolver_name: &Ident,
    prefix: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match record.style {
        RecordStyle::Named => {
            let fields = record.active_fields().map(|(index, _field)| {
                let name = field_user_ident(record, index);
                let slot = resolver_slot_ident(record, index);
                quote! { #name: #prefix.#slot.take().expect("field resolver available after polling") }
            });
            quote! { #resolver_name { #(#fields),* } }
        }
        RecordStyle::Unnamed => {
            let fields = record.active_fields().map(|(index, _field)| {
                let slot = resolver_slot_ident(record, index);
                quote! { #prefix.#slot.take().expect("field resolver available after polling") }
            });
            quote! { #resolver_name( #(#fields),* ) }
        }
        RecordStyle::Unit => quote! { #resolver_name },
    }
}

pub fn record_state_impl(
    state_name: &Ident,
    resolver_name: &Ident,
    record: &RecordSpec<'_>,
    archived_name: &Ident,
    stable_schema_key: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let poll_logic = record_state_poll_logic(record, archived_name, stable_schema_key, quote! { self });
    let res_expr = resolver_expr(record, resolver_name, quote! { self });
    quote! {
        impl<'a> zebin::SerializeState<'a> for #state_name<'a> {
            type Resolver = #resolver_name;
            fn poll<E: zebin::ByteSink + zebin::LayoutSink<'a> + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<Self::Resolver>, zebin::ZebinError> {
                #poll_logic
                Ok(::core::task::Poll::Ready(#res_expr))
            }
        }
    }
}

// --- Struct Implementation ---

fn struct_impl(name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let archived_name = archived_name(name);
    let resolver_name = resolver_name(name);
    let state_name = state_name(name);
    let res_def = resolver_def(&resolver_name, record);
    let s_def = state_def(&state_name, record);
    let key = if has_schema(record) {
        let k = record.stable_schema_key.expect("schema-bearing records require key");
        quote! { #k }
    } else { quote! { 0 } };
    let s_impl = record_state_impl(&state_name, &resolver_name, record, &archived_name, &key);
    let init = state_init(record);
    quote! { #s_def #res_def #s_impl impl zebin::Serialize for #name { type State<'a> = #state_name<'a> where Self: 'a; fn begin_serialize(&self) -> Result<Self::State<'_>, zebin::ZebinError> { Ok(#state_name { #init }) } } }
}

// --- Main Entry Point ---

pub fn derive(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let spec = match parse_item(&input) { Ok(s) => s, Err(e) => return e.to_compile_error().into() };
    let name = input.ident.clone();
    let expanded = match spec {
        ItemSpec::Struct(record) => struct_impl(&name, &record),
        ItemSpec::Enum(variants) => enums::enum_impl(&name, &variants),
    };
    TokenStream::from(expanded)
}

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Ident};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, active_fields_by_id, field_encoding, field_len_ident,
    field_state_type, has_schema, input_member, packed_begin_expr, parse_item, state_name,
    variant_method_name, variant_state_name,
};

fn state_field_decls(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    let mut fields = Vec::new();
    if has_schema(record) {
        fields.push(quote! { pub __zebin_header_cursor: usize });
    }
    for (index, field) in record.active_fields() {
        let state_ident = &field.state_ident;
        let state_ty = field_state_type(field);
        fields.push(quote! { pub #state_ident: #state_ty });
        if has_schema(record) {
            let len_ident = field_len_ident(record, index);
            fields.push(quote! { pub #len_ident: u32 });
        }
    }
    fields
}

fn field_value_expr(record: &RecordSpec<'_>, index: usize) -> proc_macro2::TokenStream {
    let member = input_member(record, index);
    quote! { &self.#member }
}

fn field_measure_expr_for_value(
    field: &crate::shared::FieldSpec<'_>,
    value: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if let Some(wrapper) = crate::shared::packed_wrapper_type_expr(field) {
        quote! {
            {
                let __zebin_wrapped = <#wrapper>::new(#value.as_ref());
                zebin::measure_serialized_len(&__zebin_wrapped)?
            }
        }
    } else {
        quote! { zebin::measure_serialized_len(#value)? }
    }
}

fn field_state_init_for_value(
    field: &crate::shared::FieldSpec<'_>,
    value: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if let Some(wrapper) = crate::shared::packed_wrapper_type_expr(field) {
        let _ = wrapper;
        if let Some(init) = packed_begin_expr(field, quote! { #value }) {
            quote! { #init? }
        } else {
            unreachable!("packed wrapper implies packed begin expression")
        }
    } else {
        let ty = field.ty;
        quote! { <#ty as zebin::Encode>::begin_encode(#value)? }
    }
}

fn state_init_from_values(
    record: &RecordSpec<'_>,
    values: &[(usize, proc_macro2::TokenStream)],
) -> Vec<proc_macro2::TokenStream> {
    let mut inits = Vec::new();
    if has_schema(record) {
        inits.push(quote! { __zebin_header_cursor: 0 });
    }
    for (index, value) in values {
        let state_ident = &record.fields[*index].state_ident;
        let state_init = field_state_init_for_value(&record.fields[*index], value.clone());
        inits.push(quote! { #state_ident: #state_init });
        if has_schema(record) {
            let len_ident = field_len_ident(record, *index);
            let measure = field_measure_expr_for_value(&record.fields[*index], value.clone());
            inits.push(quote! {
                #len_ident: {
                    let __zebin_len = #measure;
                    u32::try_from(__zebin_len).map_err(|_| zebin::ZebinError::SerializationError {
                        pos: 0,
                        message: "field payload length exceeds u32 range",
                    })?
                }
            });
        }
    }
    inits
}

fn state_init(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    let values: Vec<_> = record
        .active_fields()
        .map(|(index, _)| (index, field_value_expr(record, index)))
        .collect();
    state_init_from_values(record, &values)
}

fn schema_header_poll(record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    if !has_schema(record) {
        return quote! {};
    }

    let stable_schema_key = record
        .stable_schema_key
        .expect("schema-bearing records require key");
    let schema_revision = record.schema_revision;
    let fields = active_fields_by_id(record);
    let field_count = fields.len();
    let header_len = quote! { 12 + #field_count * zebin::FieldEntry::SIZE };
    let entries = fields
        .iter()
        .enumerate()
        .map(|(entry_index, (index, field))| {
            let field_id = field.field_id.expect("field ids validated");
            let encoding = field_encoding(field);
            let len_ident = field_len_ident(record, *index);
            let start = quote! { 12 + #entry_index * zebin::FieldEntry::SIZE };
            quote! {
                __zebin_header[#start..#start + zebin::FieldEntry::SIZE].copy_from_slice(
                    &zebin::FieldEntry {
                        field_id: #field_id as u16,
                        encoding: #encoding,
                        payload_len: self.#len_ident,
                    }
                    .to_bytes()
                );
            }
        });

    quote! {
        if self.__zebin_header_cursor < #header_len {
            let mut __zebin_header = [0u8; #header_len];
            __zebin_header[0..4].copy_from_slice(&(#stable_schema_key as u32).to_le_bytes());
            __zebin_header[4..8].copy_from_slice(&(#schema_revision as u32).to_le_bytes());
            __zebin_header[8..10].copy_from_slice(&(#field_count as u16).to_le_bytes());
            __zebin_header[10..12].copy_from_slice(&0u16.to_le_bytes());
            #(#entries)*
            let __zebin_remaining = #header_len - self.__zebin_header_cursor;
            if encoder.write(&__zebin_header[self.__zebin_header_cursor..])?
                .advance_cursor(&mut self.__zebin_header_cursor, __zebin_remaining)
                .is_pending()
            {
                return Ok(::core::task::Poll::Pending);
            }
        }
    }
}

fn record_poll_logic(record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let header = schema_header_poll(record);
    let fields: Vec<_> = if has_schema(record) {
        active_fields_by_id(record)
    } else {
        record.active_fields().collect()
    };
    let polls = fields.iter().map(|(index, _)| {
        let state_ident = &record.fields[*index].state_ident;
        quote! {
            match self.#state_ident.poll_pending(encoder)? {
                ::core::task::Poll::Pending => return Ok(::core::task::Poll::Pending),
                ::core::task::Poll::Ready(()) => {}
            }
        }
    });
    quote! {
        #header
        #(#polls)*
        Ok(::core::task::Poll::Ready(()))
    }
}

fn record_state_def(state_name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = state_field_decls(record);
    quote! { pub struct #state_name<'a> { pub _marker: ::core::marker::PhantomData<&'a ()>, #(#fields,)* } }
}

fn record_state_impl(state_name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let logic = record_poll_logic(record);
    let finishes = record.active_fields().map(|(index, _)| {
        let state_ident = &record.fields[index].state_ident;
        quote! {
            let _ = self.#state_ident.finish(sink)?;
        }
    });
    quote! {
        impl<'a> zebin::Encoder<'a> for #state_name<'a> {
            type Input = ();
            fn input<S: zebin::ByteSink + ?Sized>(&mut self, _item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                self.poll_pending(sink)
            }
            fn poll_pending<E: zebin::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                #logic
            }
            fn finish<S: zebin::ByteSink + ?Sized>(self, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                #(#finishes)*
                Ok(::core::task::Poll::Ready(()))
            }
        }
    }
}

fn struct_impl(name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let s_name = state_name(name);
    let state_def = record_state_def(&s_name, record);
    let state_impl = record_state_impl(&s_name, record);
    let inits = state_init(record);
    quote! {
        #state_def
        #state_impl
        impl zebin::Encode for #name {
            type Encoder<'a> = #s_name<'a> where Self: 'a;
            fn begin_encode(&self) -> Result<Self::Encoder<'_>, zebin::ZebinError> {
                Ok(#s_name { _marker: ::core::marker::PhantomData, #(#inits,)* })
            }
        }
    }
}

fn variant_state_constructor(
    enum_name: &Ident,
    payload_state: &Ident,
    variant: &crate::shared::VariantSpec<'_>,
    variant_index: usize,
) -> proc_macro2::TokenStream {
    let variant_ident = variant.ident;
    let state_ident = variant_state_name(enum_name, variant_ident);
    let tag = variant_index as u32;
    match variant.record.style {
        RecordStyle::Unit => quote! {
            #enum_name::#variant_ident => Ok(Self::Encoder::new_unit(#tag))
        },
        RecordStyle::Named => {
            let binders: Vec<_> = variant
                .record
                .fields
                .iter()
                .map(|f| {
                    let ident = f.ident.expect("named field");
                    if f.skip {
                        quote! { #ident: _ }
                    } else {
                        quote! { #ident }
                    }
                })
                .collect();
            let values: Vec<_> = variant
                .record
                .active_fields()
                .map(|(index, field)| {
                    let ident = field.ident.expect("named field");
                    (index, quote! { #ident })
                })
                .collect();
            let inits = state_init_from_values(&variant.record, &values);
            quote! {
                #enum_name::#variant_ident { #(#binders),* } => {
                    let __zebin_payload_state = #state_ident { _marker: ::core::marker::PhantomData, #(#inits,)* };
                    Ok(Self::Encoder::new_payload(#tag, #payload_state::#variant_ident(__zebin_payload_state)))
                }
            }
        }
        RecordStyle::Unnamed => {
            let binders: Vec<_> = variant
                .record
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    if field.skip {
                        quote! { _ }
                    } else {
                        let ident = format_ident!("field{index}");
                        quote! { #ident }
                    }
                })
                .collect();
            let values: Vec<_> = variant
                .record
                .active_fields()
                .map(|(index, _field)| {
                    let ident = format_ident!("field{index}");
                    (index, quote! { #ident })
                })
                .collect();
            let inits = state_init_from_values(&variant.record, &values);
            quote! {
                #enum_name::#variant_ident( #(#binders),* ) => {
                    let __zebin_payload_state = #state_ident { _marker: ::core::marker::PhantomData, #(#inits,)* };
                    Ok(Self::Encoder::new_payload(#tag, #payload_state::#variant_ident(__zebin_payload_state)))
                }
            }
        }
    }
}

fn enum_impl(
    name: &Ident,
    variants: &[crate::shared::VariantSpec<'_>],
) -> proc_macro2::TokenStream {
    let enum_state = state_name(name);
    let payload_state = format_ident!("{}ArchivePayloadState", name);
    let variant_state_defs: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let state_ident = variant_state_name(name, variant.ident);
            record_state_def(&state_ident, &variant.record)
        })
        .collect();
    let variant_state_impls: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let state_ident = variant_state_name(name, variant.ident);
            record_state_impl(&state_ident, &variant.record)
        })
        .collect();
    let payload_variants: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let ident = variant.ident;
            let state_ident = variant_state_name(name, ident);
            quote! { #ident(#state_ident<'a>) }
        })
        .collect();
    let payload_polls: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let ident = variant.ident;
            quote! { #payload_state::#ident(state) => state.poll_pending(encoder) }
        })
        .collect();
    let payload_finishes: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let ident = variant.ident;
            quote! { #payload_state::#ident(state) => state.finish(sink) }
        })
        .collect();
    let begin_matches: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| variant_state_constructor(name, &payload_state, variant, index))
        .collect();

    let _method_names: Vec<_> = variants
        .iter()
        .map(|variant| {
            let method_ident = variant.rename.as_ref().unwrap_or(variant.ident);
            variant_method_name("as", method_ident)
        })
        .collect();

    quote! {
        #(#variant_state_defs)*
        #(#variant_state_impls)*

        pub enum #payload_state<'a> {
            __Never(::core::marker::PhantomData<&'a ()>),
            #(#payload_variants,)*
        }

        impl<'a> zebin::Encoder<'a> for #payload_state<'a> {
            type Input = ();
            fn input<S: zebin::ByteSink + ?Sized>(&mut self, _item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                self.poll_pending(sink)
            }
            fn poll_pending<E: zebin::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                match self {
                    #payload_state::__Never(_) => Ok(::core::task::Poll::Ready(())),
                    #(#payload_polls,)*
                }
            }
            fn finish<S: zebin::ByteSink + ?Sized>(self, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                match self {
                    #payload_state::__Never(_) => Ok(::core::task::Poll::Ready(())),
                    #(#payload_finishes,)*
                }
            }
        }

        pub struct #enum_state<'a> {
            tag: [u8; 4],
            tag_cursor: usize,
            payload: Option<#payload_state<'a>>,
        }

        impl<'a> #enum_state<'a> {
            fn new_unit(tag: u32) -> Self {
                Self { tag: tag.to_le_bytes(), tag_cursor: 0, payload: None }
            }

            fn new_payload(tag: u32, payload: #payload_state<'a>) -> Self {
                Self { tag: tag.to_le_bytes(), tag_cursor: 0, payload: Some(payload) }
            }
        }

        impl<'a> zebin::Encoder<'a> for #enum_state<'a> {
            type Input = ();
            fn input<S: zebin::ByteSink + ?Sized>(&mut self, _item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                self.poll_pending(sink)
            }
            fn poll_pending<E: zebin::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                if self.tag_cursor < self.tag.len() {
                    let __zebin_remaining = self.tag.len() - self.tag_cursor;
                    if encoder.write(&self.tag[self.tag_cursor..])?
                        .advance_cursor(&mut self.tag_cursor, __zebin_remaining)
                        .is_pending()
                    {
                        return Ok(::core::task::Poll::Pending);
                    }
                }
                if let Some(payload) = &mut self.payload {
                    match payload.poll_pending(encoder)? {
                        ::core::task::Poll::Pending => Ok(::core::task::Poll::Pending),
                        ::core::task::Poll::Ready(()) => {
                            Ok(::core::task::Poll::Ready(()))
                        }
                    }
                } else {
                    Ok(::core::task::Poll::Ready(()))
                }
            }
            fn finish<S: zebin::ByteSink + ?Sized>(self, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                if let Some(payload) = self.payload {
                    payload.finish(sink)
                } else {
                    Ok(::core::task::Poll::Ready(()))
                }
            }
        }

        impl zebin::Encode for #name {
            type Encoder<'a> = #enum_state<'a> where Self: 'a;
            fn begin_encode(&self) -> Result<Self::Encoder<'_>, zebin::ZebinError> {
                match self {
                    #(#begin_matches,)*
                }
            }
        }
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

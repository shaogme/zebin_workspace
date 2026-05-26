use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Ident};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, active_fields_by_id, field_encoding, field_len_ident,
    field_state_type, has_schema, input_member, parse_item, state_name, variant_state_name,
};

fn field_started_ident(field: &crate::shared::FieldSpec<'_>) -> Ident {
    format_ident!("__started_{}", field.state_ident)
}

fn state_field_decls(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    let mut fields = Vec::new();
    if has_schema(record) {
        fields.push(quote! { pub __zebin_header_cursor: usize });
        fields.push(quote! { pub __zebin_object_start: usize });
        fields.push(quote! { pub __zebin_table_start: usize });
        fields.push(quote! { pub __zebin_table_offset_cursor: usize });
        fields.push(quote! { pub __zebin_object_len_cursor: usize });
    }
    for (index, field) in record.active_fields() {
        let state_ident = &field.state_ident;
        let state_ty = field_state_type(field);
        fields.push(quote! { pub #state_ident: #state_ty });
        let started_ident = field_started_ident(field);
        fields.push(quote! { pub #started_ident: bool });
        if has_schema(record) {
            let len_ident = field_len_ident(record, index);
            fields.push(quote! { pub #len_ident: u32 });
            let entry_cursor_ident = format_ident!("__field_entry_cursor_{}", index);
            fields.push(quote! { pub #entry_cursor_ident: usize });
        }
    }
    fields
}

fn field_user_ident(record: &RecordSpec<'_>, index: usize) -> Ident {
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

fn state_init(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    let mut inits = Vec::new();
    if has_schema(record) {
        inits.push(quote! { __zebin_header_cursor: 0 });
        inits.push(quote! { __zebin_object_start: 0 });
        inits.push(quote! { __zebin_table_start: 0 });
        inits.push(quote! { __zebin_table_offset_cursor: 0 });
        inits.push(quote! { __zebin_object_len_cursor: 0 });
    }
    for (index, field) in record.active_fields() {
        let state_ident = &field.state_ident;
        let started_ident = field_started_ident(field);
        inits.push(quote! { #started_ident: false });
        if let Some(_wrapper) = crate::shared::packed_wrapper_type_expr(field) {
            inits.push(quote! { #state_ident: zebin::archive::PackedSequenceEncoder::new_empty() });
        } else {
            let ty = field.ty;
            inits.push(quote! { #state_ident: <#ty as zebin::Encode>::encoder() });
        }
        if has_schema(record) {
            let len_ident = field_len_ident(record, index);
            inits.push(quote! { #len_ident: 0 });
            let entry_cursor_ident = format_ident!("__field_entry_cursor_{}", index);
            inits.push(quote! { #entry_cursor_ident: 0 });
        }
    }
    inits
}

fn record_poll_logic(
    record: &RecordSpec<'_>,
    input_ty_bare: &Ident,
    is_variant: bool,
    variant_ident: Option<&Ident>,
) -> proc_macro2::TokenStream {
    let fields = active_fields_by_id(record);
    let field_count = fields.len();

    let header_write = if has_schema(record) {
        let stable_schema_key = record
            .stable_schema_key
            .expect("schema-bearing records require key");
        let schema_revision = record.schema_revision;
        quote! {
            if self.__zebin_header_cursor == 0 {
                self.__zebin_object_start = encoder.pos();
            }
            if self.__zebin_header_cursor < 12 {
                let mut __zebin_header = [0u8; 12];
                __zebin_header[0..4].copy_from_slice(&(#stable_schema_key as u32).to_le_bytes());
                __zebin_header[4..8].copy_from_slice(&(#schema_revision as u32).to_le_bytes());
                __zebin_header[8..10].copy_from_slice(&(#field_count as u16).to_le_bytes());
                __zebin_header[10..12].copy_from_slice(&0u16.to_le_bytes());
                let __zebin_remaining = 12 - self.__zebin_header_cursor;
                if encoder.write(&__zebin_header[self.__zebin_header_cursor..])?
                    .advance_cursor(&mut self.__zebin_header_cursor, __zebin_remaining)
                    .is_pending()
                {
                    return Ok(::core::task::Poll::Pending);
                }
            }
        }
    } else {
        quote! {}
    };

    let payload_polls = fields.iter().map(|(index, field)| {
        let state_ident = &field.state_ident;
        let started_ident = field_started_ident(field);
        let len_ident = field_len_ident(record, *index);

        let val_expr = if is_variant {
            let var_ident = field_user_ident(record, *index);
            if let Some(wrapper) = crate::shared::packed_wrapper_type_expr(field) {
                quote! { <#wrapper>::new(#var_ident.as_ref()) }
            } else {
                quote! { #var_ident }
            }
        } else {
            let member = input_member(record, *index);
            if let Some(wrapper) = crate::shared::packed_wrapper_type_expr(field) {
                quote! { <#wrapper>::new(__item.#member.as_ref()) }
            } else {
                quote! { &__item.#member }
            }
        };

        let measure_expr = if is_variant {
            let var_ident = field_user_ident(record, *index);
            field_measure_expr_for_value(field, quote! { #var_ident })
        } else {
            let member = input_member(record, *index);
            field_measure_expr_for_value(field, quote! { &__item.#member })
        };

        let len_measure_stmt = if has_schema(record) {
            quote! {
                self.#len_ident = {
                    let __zebin_len = #measure_expr;
                    u32::try_from(__zebin_len).map_err(|_| zebin::ZebinError::SerializationError {
                        pos: encoder.pos(),
                        message: "field payload length exceeds u32 range",
                    })?
                };
            }
        } else {
            quote! {}
        };

        quote! {
            if !self.#started_ident {
                #len_measure_stmt
                let __val = #val_expr;
                match self.#state_ident.input(__val, encoder)? {
                    ::core::task::Poll::Pending => {
                        self.#started_ident = true;
                        return Ok(::core::task::Poll::Pending);
                    }
                    ::core::task::Poll::Ready(()) => {
                        self.#started_ident = true;
                    }
                }
            } else {
                match self.#state_ident.poll_pending(encoder)? {
                    ::core::task::Poll::Pending => return Ok(::core::task::Poll::Pending),
                    ::core::task::Poll::Ready(()) => {}
                }
            }
        }
    });

    let table_write_and_len = if has_schema(record) {
        let table_polls = fields.iter().map(|(index, field)| {
            let len_ident = field_len_ident(record, *index);
            let entry_cursor_ident = format_ident!("__field_entry_cursor_{}", *index);
            let field_id = field.field_id.expect("field ids validated");
            let encoding = field_encoding(field);

            quote! {
                if self.#entry_cursor_ident < zebin::schema::FieldEntry::SIZE {
                    let __field_entry_bytes = zebin::schema::FieldEntry {
                        field_id: #field_id as u16,
                        encoding: #encoding,
                        payload_len: self.#len_ident,
                    }
                    .to_bytes();
                    let __zebin_remaining = zebin::schema::FieldEntry::SIZE - self.#entry_cursor_ident;
                    if encoder.write(&__field_entry_bytes[self.#entry_cursor_ident..])?
                        .advance_cursor(&mut self.#entry_cursor_ident, __zebin_remaining)
                        .is_pending()
                    {
                        return Ok(::core::task::Poll::Pending);
                    }
                }
            }
        });

        quote! {
            if self.__zebin_table_start == 0 {
                self.__zebin_table_start = encoder.pos();
            }
            #(#table_polls)*
            if self.__zebin_table_offset_cursor < 4 {
                let __offset_val = (self.__zebin_table_start - self.__zebin_object_start) as u32;
                let __offset_bytes = __offset_val.to_le_bytes();
                let __zebin_remaining = 4 - self.__zebin_table_offset_cursor;
                if encoder.write(&__offset_bytes[self.__zebin_table_offset_cursor..])?
                    .advance_cursor(&mut self.__zebin_table_offset_cursor, __zebin_remaining)
                    .is_pending()
                {
                    return Ok(::core::task::Poll::Pending);
                }
            }
            if self.__zebin_object_len_cursor < 4 {
                let __total_len = (encoder.pos() - self.__zebin_object_start + 4 - self.__zebin_object_len_cursor) as u32;
                let __len_bytes = __total_len.to_le_bytes();
                let __zebin_remaining = 4 - self.__zebin_object_len_cursor;
                if encoder.write(&__len_bytes[self.__zebin_object_len_cursor..])?
                    .advance_cursor(&mut self.__zebin_object_len_cursor, __zebin_remaining)
                    .is_pending()
                {
                    return Ok(::core::task::Poll::Pending);
                }
            }
        }
    } else {
        quote! {}
    };

    if is_variant {
        let variant_ident = variant_ident.expect("variant_ident present if is_variant");
        let binders: Vec<_> = record
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let ident = field_user_ident(record, index);
                match record.style {
                    RecordStyle::Named => {
                        let field_ident = field.ident.expect("named field has ident");
                        if field.skip {
                            quote! { #field_ident: _ }
                        } else if *field_ident == ident {
                            quote! { #ident }
                        } else {
                            quote! { #field_ident: #ident }
                        }
                    }
                    RecordStyle::Unnamed => {
                        if field.skip {
                            quote! { _ }
                        } else {
                            quote! { #ident }
                        }
                    }
                    RecordStyle::Unit => unreachable!(),
                }
            })
            .collect();

        let pattern = match record.style {
            RecordStyle::Named => quote! { #input_ty_bare::#variant_ident { #(#binders),* } },
            RecordStyle::Unnamed => quote! { #input_ty_bare::#variant_ident(#(#binders),*) },
            RecordStyle::Unit => quote! { #input_ty_bare::#variant_ident },
        };

        quote! {
            match __item {
                #pattern => {
                    #header_write
                    #(#payload_polls)*
                    #table_write_and_len
                    Ok(::core::task::Poll::Ready(()))
                }
                _ => unsafe { ::core::hint::unreachable_unchecked() }
            }
        }
    } else {
        quote! {
            #header_write
            #(#payload_polls)*
            #table_write_and_len
            Ok(::core::task::Poll::Ready(()))
        }
    }
}

fn record_state_def(
    vis: &syn::Visibility,
    state_name: &Ident,
    input_ty_bare: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let fields = state_field_decls(record);
    quote! {
        #vis struct #state_name<'a> {
            pub _marker: ::core::marker::PhantomData<&'a ()>,
            pub __item: Option<&'a #input_ty_bare>,
            #(#fields,)*
        }
    }
}

fn record_state_impl(
    state_name: &Ident,
    input_ty_bare: &Ident,
    record: &RecordSpec<'_>,
    is_variant: bool,
    variant_ident: Option<&Ident>,
) -> proc_macro2::TokenStream {
    let logic = record_poll_logic(record, input_ty_bare, is_variant, variant_ident);
    let finishes = record.active_fields().map(|(index, _)| {
        let state_ident = &record.fields[index].state_ident;
        quote! {
            let _ = self.#state_ident.finish(sink)?;
        }
    });
    quote! {
        impl<'a> zebin::io::Encoder<'a> for #state_name<'a> {
            type Input = &'a #input_ty_bare;
            fn input<S: zebin::io::ByteSink + ?Sized>(&mut self, item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                self.__item = Some(item);
                self.poll_pending(sink)
            }
            fn poll_pending<E: zebin::io::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                let __item = self.__item.ok_or(zebin::ZebinError::SerializationError {
                    pos: encoder.pos(),
                    message: "encoder polled before input",
                })?;
                #logic
            }
            fn finish<S: zebin::io::ByteSink + ?Sized>(self, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                #(#finishes)*
                Ok(::core::task::Poll::Ready(()))
            }
        }
    }
}

fn struct_impl(
    vis: &syn::Visibility,
    name: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let s_name = state_name(name);
    let state_def = record_state_def(vis, &s_name, name, record);
    let state_impl = record_state_impl(&s_name, name, record, false, None);
    let inits = state_init(record);
    quote! {
        #state_def
        #state_impl
        impl zebin::Encode for #name {
            type Encoder<'a> = #s_name<'a> where Self: 'a;
            fn encoder<'a>() -> Self::Encoder<'a> where Self: 'a {
                #s_name {
                    _marker: ::core::marker::PhantomData,
                    __item: None,
                    #(#inits,)*
                }
            }
        }
    }
}

fn enum_impl(
    vis: &syn::Visibility,
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
            record_state_def(vis, &state_ident, name, &variant.record)
        })
        .collect();
    let variant_state_impls: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let state_ident = variant_state_name(name, variant.ident);
            record_state_impl(
                &state_ident,
                name,
                &variant.record,
                true,
                Some(variant.ident),
            )
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

    let payload_input_arms: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let ident = variant.ident;
            quote! { #payload_state::#ident(state) => state.input(item, sink) }
        })
        .collect();

    let payload_assign_arms: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let ident = variant.ident;
            quote! { #payload_state::#ident(state) => { state.__item = ::core::option::Option::Some(item); } }
        })
        .collect();

    let begin_matches: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let variant_ident = variant.ident;
            let tag = index as u32;
            let state_ident = variant_state_name(name, variant_ident);
            let inits = state_init(&variant.record);
            match variant.record.style {
                RecordStyle::Unit => quote! {
                    #name::#variant_ident => (#tag, None)
                },
                RecordStyle::Named => quote! {
                    #name::#variant_ident { .. } => {
                        let __variant_state = #state_ident {
                            _marker: ::core::marker::PhantomData,
                            __item: None,
                            #(#inits,)*
                        };
                        (#tag, Some(#payload_state::#variant_ident(__variant_state)))
                    }
                },
                RecordStyle::Unnamed => quote! {
                    #name::#variant_ident( .. ) => {
                        let __variant_state = #state_ident {
                            _marker: ::core::marker::PhantomData,
                            __item: None,
                            #(#inits,)*
                        };
                        (#tag, Some(#payload_state::#variant_ident(__variant_state)))
                    }
                },
            }
        })
        .collect();

    quote! {
        #(#variant_state_defs)*
        #(#variant_state_impls)*

        #vis enum #payload_state<'a> {
            __Never(::core::marker::PhantomData<&'a ()>),
            #(#payload_variants,)*
        }

        impl<'a> zebin::io::Encoder<'a> for #payload_state<'a> {
            type Input = &'a #name;
            fn input<S: zebin::io::ByteSink + ?Sized>(&mut self, item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                match self {
                    #payload_state::__Never(_) => Ok(::core::task::Poll::Ready(())),
                    #(#payload_input_arms,)*
                }
            }
            fn poll_pending<E: zebin::io::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                match self {
                    #payload_state::__Never(_) => Ok(::core::task::Poll::Ready(())),
                    #(#payload_polls,)*
                }
            }
            fn finish<S: zebin::io::ByteSink + ?Sized>(self, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                match self {
                    #payload_state::__Never(_) => Ok(::core::task::Poll::Ready(())),
                    #(#payload_finishes,)*
                }
            }
        }

        #vis struct #enum_state<'a> {
            tag: [u8; 4],
            tag_cursor: usize,
            payload: Option<#payload_state<'a>>,
            __item: Option<&'a #name>,
        }

        impl<'a> zebin::io::Encoder<'a> for #enum_state<'a> {
            type Input = &'a #name;
            fn input<S: zebin::io::ByteSink + ?Sized>(&mut self, item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                self.__item = Some(item);
                let (tag_val, payload_val) = match item {
                    #(#begin_matches,)*
                };
                self.tag = tag_val.to_le_bytes();
                self.payload = payload_val;
                if let Some(payload) = &mut self.payload {
                    match payload {
                        #payload_state::__Never(_) => {},
                        #(#payload_assign_arms,)*
                    }
                }
                self.poll_pending(sink)
            }
            fn poll_pending<E: zebin::io::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
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
            fn finish<S: zebin::io::ByteSink + ?Sized>(self, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                if let Some(payload) = self.payload {
                    payload.finish(sink)
                } else {
                    Ok(::core::task::Poll::Ready(()))
                }
            }
        }

        impl zebin::Encode for #name {
            type Encoder<'a> = #enum_state<'a> where Self: 'a;
            fn encoder<'a>() -> Self::Encoder<'a> where Self: 'a {
                #enum_state {
                    tag: [0; 4],
                    tag_cursor: 0,
                    payload: None,
                    __item: None,
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
        ItemSpec::Struct(record) => struct_impl(&input.vis, &name, &record),
        ItemSpec::Enum(variants) => enum_impl(&input.vis, &name, &variants),
    };
    TokenStream::from(expanded)
}

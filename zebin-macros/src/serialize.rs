use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Ident};

use crate::shared::{
    ItemSpec, RecordSpec, RecordStyle, active_fields_by_id, field_encoding, field_len_ident,
    field_state_type, has_schema, parse_item, state_name, variant_state_name,
};

fn field_started_ident(field: &crate::shared::FieldSpec<'_>) -> Ident {
    format_ident!("__started_{}", field.state_ident)
}

/// Identifier for the per-field `Option<FieldTy>` slot that holds destructured input.
fn field_slot_ident(field: &crate::shared::FieldSpec<'_>) -> Ident {
    format_ident!("__slot_{}", field.state_ident)
}

fn state_field_decls(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    let mut fields = Vec::new();
    if has_schema(record) {
        fields.push(quote! { pub __schema_encoder: zebin::io::SchemaObjectEncoder });
    }
    for (index, field) in record.active_fields() {
        let state_ident = &field.state_ident;
        let state_ty = field_state_type(field);
        fields.push(quote! { pub #state_ident: #state_ty });
        let started_ident = field_started_ident(field);
        fields.push(quote! { pub #started_ident: bool });
        let slot_ident = field_slot_ident(field);
        let ty = field.ty;
        // Always store the destructured field as Option<FieldTy>.
        // For packed wrappers, field is consumed via .as_ref() into a wrapper
        // each poll iteration so we keep the Option populated for the duration.
        fields.push(quote! { pub #slot_ident: ::core::option::Option<#ty> });
        if has_schema(record) {
            let len_ident = field_len_ident(record, index);
            fields.push(quote! { pub #len_ident: u32 });
            let entry_encoder_ident = format_ident!("__field_entry_encoder_{}", index);
            fields.push(quote! { pub #entry_encoder_ident: zebin::io::FieldEntryEncoder });
        }
    }
    fields
}

/// Build expression that produces `usize` for the encoded length of a field.
/// Walks via `MeasureBody` (no encoder, no consumption).
fn field_measure_expr(
    field: &crate::shared::FieldSpec<'_>,
    value: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if let Some((kind, bits)) = crate::shared::packed::packed_info_pub(field) {
        match kind {
            crate::shared::packed::PackedElementKind::Bool => quote! {
                {
                    let __zebin_packed_len = (#value).len();
                    4usize + __zebin_packed_len.div_ceil(8)
                }
            },
            crate::shared::packed::PackedElementKind::U8 => {
                let bits_lit = bits as usize;
                quote! {
                    {
                        let __zebin_packed_len = (#value).len();
                        4usize + (__zebin_packed_len * #bits_lit).div_ceil(8)
                    }
                }
            }
        }
    } else {
        quote! { zebin::MeasureBody::measure_body(#value)? }
    }
}

fn state_init(record: &RecordSpec<'_>) -> Vec<proc_macro2::TokenStream> {
    let mut inits = Vec::new();
    if has_schema(record) {
        inits.push(quote! { __schema_encoder: zebin::io::SchemaObjectEncoder::new() });
    }
    for (index, field) in record.active_fields() {
        let state_ident = &field.state_ident;
        let started_ident = field_started_ident(field);
        let slot_ident = field_slot_ident(field);
        inits.push(quote! { #started_ident: false });
        inits.push(quote! { #slot_ident: ::core::option::Option::None });
        if let Some(_wrapper) = crate::shared::packed_wrapper_type_expr(field) {
            inits.push(quote! { #state_ident: ::core::default::Default::default() });
        } else {
            let ty = field.ty;
            inits.push(quote! { #state_ident: <#ty as zebin::Encode>::encoder() });
        }
        if has_schema(record) {
            let len_ident = field_len_ident(record, index);
            inits.push(quote! { #len_ident: 0 });
            let entry_encoder_ident = format_ident!("__field_entry_encoder_{}", index);
            inits.push(quote! { #entry_encoder_ident: zebin::io::FieldEntryEncoder::new() });
        }
    }
    inits
}

/// Generate `let pat = item;` destructuring for struct or variant.
fn destructure_pattern(
    record: &RecordSpec<'_>,
    type_path: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let binders: Vec<_> = record
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let user_id = field_user_ident_for(record, index, field);
            match record.style {
                RecordStyle::Named => {
                    let field_ident = field.ident.expect("named field has ident");
                    if field.skip {
                        quote! { #field_ident: _ }
                    } else if *field_ident == user_id {
                        quote! { #user_id }
                    } else {
                        quote! { #field_ident: #user_id }
                    }
                }
                RecordStyle::Unnamed => {
                    if field.skip {
                        quote! { _ }
                    } else {
                        quote! { #user_id }
                    }
                }
                RecordStyle::Unit => unreachable!(),
            }
        })
        .collect();

    match record.style {
        RecordStyle::Named => quote! { #type_path { #(#binders),* } },
        RecordStyle::Unnamed => quote! { #type_path(#(#binders),*) },
        RecordStyle::Unit => quote! { #type_path },
    }
}

fn field_user_ident_for(
    record: &RecordSpec<'_>,
    index: usize,
    field: &crate::shared::FieldSpec<'_>,
) -> Ident {
    if let Some(rename) = &field.rename {
        return rename.clone();
    }
    match record.style {
        RecordStyle::Named => field.ident.expect("named field has ident").clone(),
        RecordStyle::Unnamed => format_ident!("field{}", index),
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

/// Builds the body of `poll_pending` after the input has been destructured into
/// per-field slots. Each field block:
/// - if not started: `take()` from slot, call `state.input(val, encoder)`;
/// - if started: call `state.poll_pending(encoder)`.
fn record_poll_logic(record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = active_fields_by_id(record);
    let field_count = fields.len();

    let header_write = if has_schema(record) {
        let stable_schema_key = record
            .stable_schema_key
            .expect("schema-bearing records require key");
        let schema_revision = record.schema_revision;
        quote! {
            if self.__schema_encoder.poll_write_header(encoder, #stable_schema_key, #schema_revision, #field_count as u16)?.is_pending() {
                return Ok(::core::task::Poll::Pending);
            }
        }
    } else {
        quote! {}
    };

    let payload_polls = fields.iter().map(|(_, field)| {
        let state_ident = &field.state_ident;
        let started_ident = field_started_ident(field);
        let slot_ident = field_slot_ident(field);

        // Build the value expression we hand to state.input(...). For packed
        // wrappers we wrap the slot's owned contents into the owned wrapper
        // (PackedBoolVec / PackedU8Vec) which is moved by value.
        let input_val_expr = if let Some(wrapper) = crate::shared::packed_wrapper_type_expr(field) {
            quote! {
                <#wrapper>::new(self.#slot_ident.take().expect("packed field already consumed"))
            }
        } else {
            quote! { self.#slot_ident.take().expect("field already consumed") }
        };

        quote! {
            if !self.#started_ident {
                match self.#state_ident.input(#input_val_expr, encoder)? {
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
            let entry_encoder_ident = format_ident!("__field_entry_encoder_{}", *index);
            let field_id = field.field_id.expect("field ids validated");
            let encoding = field_encoding(field);

            quote! {
                if self.#entry_encoder_ident.poll_write(
                    encoder,
                    #field_id as u16,
                    #encoding,
                    self.#len_ident,
                )?.is_pending() {
                    return Ok(::core::task::Poll::Pending);
                }
            }
        });

        quote! {
            self.__schema_encoder.mark_table_start(encoder);
            #(#table_polls)*
            if self.__schema_encoder.poll_write_footer(encoder)?.is_pending() {
                return Ok(::core::task::Poll::Pending);
            }
        }
    } else {
        quote! {}
    };

    quote! {
        #header_write
        #(#payload_polls)*
        #table_write_and_len
        Ok(::core::task::Poll::Ready(()))
    }
}

fn record_state_def(
    vis: &syn::Visibility,
    state_name: &Ident,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let fields = state_field_decls(record);
    quote! {
        #vis struct #state_name<'a> {
            pub _marker: ::core::marker::PhantomData<&'a ()>,
            #(#fields,)*
        }
    }
}

/// Generates the `Encoder` impl for a record state struct.
///
/// On `input(item)`:
/// 1. destructure `item` into per-field locals;
/// 2. for each schema field, call `MeasureBody::measure_body` on the local
///    (or via packed wrapper) and store into `#len_ident`;
/// 3. move each local into its slot;
/// 4. delegate to `poll_pending`.
fn record_state_input_impl(
    state_name: &Ident,
    input_ty: proc_macro2::TokenStream,
    destructure_target: proc_macro2::TokenStream,
    record: &RecordSpec<'_>,
) -> proc_macro2::TokenStream {
    let logic = record_poll_logic(record);
    let pattern = destructure_pattern(record, destructure_target);

    // Build measure-and-store steps.
    let mut measure_and_store: Vec<proc_macro2::TokenStream> = Vec::new();
    for (index, field) in record.active_fields() {
        let user_id = field_user_ident_for(record, index, field);
        let slot_ident = field_slot_ident(field);

        if has_schema(record) {
            let len_ident = field_len_ident(record, index);
            let measure_expr = field_measure_expr(field, quote! { &#user_id });
            measure_and_store.push(quote! {
                {
                    let __zebin_len = #measure_expr;
                    self.#len_ident = u32::try_from(__zebin_len).map_err(|_| zebin::ZebinError::SerializationError {
                        pos: 0,
                        message: "field payload length exceeds u32 range",
                    })?;
                }
            });
        }
        measure_and_store.push(quote! {
            self.#slot_ident = ::core::option::Option::Some(#user_id);
        });
    }

    let finishes = record.active_fields().map(|(_, field)| {
        let state_ident = &field.state_ident;
        quote! {
            let _ = self.#state_ident.finish(sink)?;
        }
    });
    quote! {
        impl<'a> zebin::io::Encoder for #state_name<'a> {
            type Input = #input_ty;
            fn input<S: zebin::io::ByteSink + ?Sized>(&mut self, item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                #[allow(irrefutable_let_patterns)]
                let #pattern = item else {
                    unsafe { ::core::hint::unreachable_unchecked() }
                };
                #(#measure_and_store)*
                self.poll_pending(sink)
            }
            fn poll_pending<E: zebin::io::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
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
    let state_def = record_state_def(vis, &s_name, record);
    let state_impl = record_state_input_impl(&s_name, quote! { #name }, quote! { #name }, record);
    let inits = state_init(record);

    let measure_body_impl = struct_measure_body_impl(name, record);

    quote! {
        #state_def
        #state_impl
        impl zebin::Encode for #name {
            type Input<'a> = #name where Self: 'a;
            type Encoder<'a> = #s_name<'a> where Self: 'a;
            fn encoder<'a>() -> Self::Encoder<'a> where Self: 'a {
                #s_name {
                    _marker: ::core::marker::PhantomData,
                    #(#inits,)*
                }
            }
        }
        #measure_body_impl
    }
}

fn struct_measure_body_impl(name: &Ident, record: &RecordSpec<'_>) -> proc_macro2::TokenStream {
    let fields = active_fields_by_id(record);
    let mut sums: Vec<proc_macro2::TokenStream> = Vec::new();
    let schema = has_schema(record);

    if schema {
        // 12 bytes header + N * FieldEntry::SIZE table + 4 bytes table_offset + 4 bytes total_len.
        sums.push(quote! { 12usize });
        let n = fields.len();
        sums.push(quote! { (#n) * zebin::schema::FieldEntry::SIZE });
        sums.push(quote! { 4usize });
        sums.push(quote! { 4usize });
    }

    for (index, field) in &fields {
        let member = match record.style {
            RecordStyle::Named => {
                let ident = field.ident.expect("named field has ident");
                quote! { self.#ident }
            }
            RecordStyle::Unnamed => {
                let idx = syn::Index::from(*index);
                quote! { self.#idx }
            }
            RecordStyle::Unit => continue,
        };
        let measure = field_measure_expr(field, quote! { &#member });
        sums.push(measure);
    }

    if sums.is_empty() {
        return quote! {
            impl zebin::MeasureBody for #name {
                fn measure_body(&self) -> Result<usize, zebin::ZebinError> {
                    Ok(0)
                }
            }
        };
    }

    quote! {
        impl zebin::MeasureBody for #name {
            fn measure_body(&self) -> Result<usize, zebin::ZebinError> {
                let mut __total: usize = 0;
                #(
                    __total = __total.checked_add(#sums)
                        .ok_or(zebin::ZebinError::ArithmeticOverflow { pos: 0 })?;
                )*
                Ok(__total)
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

    // Emit per-variant state structs (only for variants with fields).
    let variant_state_defs: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let state_ident = variant_state_name(name, variant.ident);
            record_state_def(vis, &state_ident, &variant.record)
        })
        .collect();

    let variant_state_impls: Vec<_> = variants
        .iter()
        .filter(|variant| variant.record.style != RecordStyle::Unit)
        .map(|variant| {
            let state_ident = variant_state_name(name, variant.ident);
            let variant_ident = variant.ident;
            record_state_input_impl(
                &state_ident,
                quote! { #name },
                quote! { #name::#variant_ident },
                &variant.record,
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

    // For each variant, when we see this variant in `input`, set up its tag
    // and a fresh payload state. Note: payload_assign_arms used to populate
    // `__item: Option<&T>` — that field no longer exists, so we drop those.
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
                            #(#inits,)*
                        };
                        (#tag, Some(#payload_state::#variant_ident(__variant_state)))
                    }
                },
                RecordStyle::Unnamed => quote! {
                    #name::#variant_ident( .. ) => {
                        let __variant_state = #state_ident {
                            _marker: ::core::marker::PhantomData,
                            #(#inits,)*
                        };
                        (#tag, Some(#payload_state::#variant_ident(__variant_state)))
                    }
                },
            }
        })
        .collect();

    let measure_body_impl = enum_measure_body_impl(name, variants);

    quote! {
        #(#variant_state_defs)*
        #(#variant_state_impls)*

        #vis enum #payload_state<'a> {
            __Never(::core::marker::PhantomData<&'a ()>),
            #(#payload_variants,)*
        }

        impl<'a> zebin::io::Encoder for #payload_state<'a> {
            type Input = #name;
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
            tag_encoder: zebin::io::TagEncoder,
            payload: Option<#payload_state<'a>>,
            pending_item: Option<#name>,
        }

        impl<'a> zebin::io::Encoder for #enum_state<'a> {
            type Input = #name;
            fn input<S: zebin::io::ByteSink + ?Sized>(&mut self, item: Self::Input, sink: &mut S) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                let (tag_val, payload_val) = match &item {
                    #(#begin_matches,)*
                };
                self.tag_encoder.input(tag_val);
                self.payload = payload_val;
                self.pending_item = Some(item);
                self.poll_pending(sink)
            }
            fn poll_pending<E: zebin::io::ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<::core::task::Poll<()>, zebin::ZebinError> {
                if self.tag_encoder.poll_write(encoder)?.is_pending() {
                    return Ok(::core::task::Poll::Pending);
                }
                if let Some(payload) = &mut self.payload {
                    if let Some(item) = self.pending_item.take() {
                        match payload.input(item, encoder)? {
                            ::core::task::Poll::Pending => return Ok(::core::task::Poll::Pending),
                            ::core::task::Poll::Ready(()) => return Ok(::core::task::Poll::Ready(())),
                        }
                    }
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
            type Input<'a> = #name where Self: 'a;
            type Encoder<'a> = #enum_state<'a> where Self: 'a;
            fn encoder<'a>() -> Self::Encoder<'a> where Self: 'a {
                #enum_state {
                    tag_encoder: zebin::io::TagEncoder::new(),
                    payload: None,
                    pending_item: None,
                }
            }
        }

        #measure_body_impl
    }
}

fn enum_measure_body_impl(
    name: &Ident,
    variants: &[crate::shared::VariantSpec<'_>],
) -> proc_macro2::TokenStream {
    // Each variant: 4 bytes tag + sum of field measures (with schema overhead if applicable).
    let arms = variants.iter().map(|variant| {
        let variant_ident = variant.ident;
        let record = &variant.record;

        let mut sums: Vec<proc_macro2::TokenStream> = Vec::new();
        if has_schema(record) {
            let n = record.active_fields().count();
            sums.push(quote! { 12usize });
            sums.push(quote! { (#n) * zebin::schema::FieldEntry::SIZE });
            sums.push(quote! { 4usize });
            sums.push(quote! { 4usize });
        }

        let bindings = record
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let user_id = field_user_ident_for(record, index, field);
                match record.style {
                    RecordStyle::Named => {
                        let field_ident = field.ident.expect("named field has ident");
                        if field.skip {
                            quote! { #field_ident: _ }
                        } else if *field_ident == user_id {
                            quote! { #user_id }
                        } else {
                            quote! { #field_ident: #user_id }
                        }
                    }
                    RecordStyle::Unnamed => {
                        if field.skip {
                            quote! { _ }
                        } else {
                            quote! { #user_id }
                        }
                    }
                    RecordStyle::Unit => quote! {},
                }
            })
            .collect::<Vec<_>>();

        for (index, field) in record.active_fields() {
            let user_id = field_user_ident_for(record, index, field);
            let measure = field_measure_expr(field, quote! { #user_id });
            sums.push(measure);
        }

        let pattern = match record.style {
            RecordStyle::Unit => quote! { #name::#variant_ident },
            RecordStyle::Named => quote! { #name::#variant_ident { #(#bindings),* } },
            RecordStyle::Unnamed => quote! { #name::#variant_ident( #(#bindings),* ) },
        };

        quote! {
            #pattern => {
                let mut __total: usize = 4;
                #(
                    __total = __total.checked_add(#sums)
                        .ok_or(zebin::ZebinError::ArithmeticOverflow { pos: 0 })?;
                )*
                Ok(__total)
            }
        }
    });

    quote! {
        impl zebin::MeasureBody for #name {
            fn measure_body(&self) -> Result<usize, zebin::ZebinError> {
                match self {
                    #(#arms,)*
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

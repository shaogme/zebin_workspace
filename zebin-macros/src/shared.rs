use core::convert::TryFrom;
use proc_macro2::Span;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Data, DataEnum, DataStruct, DeriveInput, Field, Fields, FieldsNamed, FieldsUnnamed, Ident,
    Index, Member, Result, Type, spanned::Spanned,
};

/// Represents the style of a struct or enum variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordStyle {
    Named,
    Unnamed,
    Unit,
}

/// Specification of a single field.
pub struct FieldSpec<'a> {
    pub ident: Option<&'a Ident>,
    pub state_ident: Ident,
    pub ty: &'a Type,
    pub field_id: Option<u16>,
    pub packed_bits: Option<u8>,
    pub skip: bool,
    pub rename: Option<Ident>,
}

/// Specification of a struct or enum variant.
pub struct RecordSpec<'a> {
    pub style: RecordStyle,
    pub fields: Vec<FieldSpec<'a>>,
    pub stable_schema_key: Option<u32>,
    pub schema_revision: u32,
}

/// Specification of an enum variant.
pub struct VariantSpec<'a> {
    pub ident: &'a Ident,
    pub rename: Option<Ident>,
    pub record: RecordSpec<'a>,
}

/// Specification of the top-level item (struct or enum).
pub enum ItemSpec<'a> {
    Struct(RecordSpec<'a>),
    Enum(Vec<VariantSpec<'a>>),
}

/// Generates the name of the archived type.
pub fn archived_name(name: &Ident) -> Ident {
    format_ident!("Archived{}", name)
}

/// Generates the name of the resolver type.
pub fn resolver_name(name: &Ident) -> Ident {
    format_ident!("{}Resolver", name)
}

pub fn state_name(name: &Ident) -> Ident {
    format_ident!("{}ArchiveState", name)
}

pub fn payload_name(name: &Ident) -> Ident {
    format_ident!("Archived{}Payload", name)
}

pub fn variant_archived_name(enum_name: &Ident, variant: &Ident) -> Ident {
    format_ident!("Archived{}{}", enum_name, variant)
}

pub fn variant_resolver_name(enum_name: &Ident, variant: &Ident) -> Ident {
    format_ident!("{}{}Resolver", enum_name, variant)
}

pub fn variant_state_name(enum_name: &Ident, variant: &Ident) -> Ident {
    format_ident!("{}{}ArchiveState", enum_name, variant)
}

pub fn variant_method_name(prefix: &str, variant: &Ident) -> Ident {
    format_ident!("{}_{}", prefix, variant_snake_case(variant))
}

pub fn variant_field_name(variant: &Ident) -> Ident {
    format_ident!("{}", variant_snake_case(variant))
}

fn variant_snake_case(variant: &Ident) -> String {
    let name = variant.to_string();
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_is_underscore = false;
    let chars = name.chars().peekable();

    for ch in chars {
        if ch.is_uppercase() {
            if !out.is_empty() && !prev_is_underscore {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            prev_is_underscore = false;
        } else {
            prev_is_underscore = ch == '_';
            out.push(ch);
        }
    }

    out
}

fn parse_name_value_str(tokens: proc_macro2::TokenStream, target: &str) -> Result<Option<String>> {
    let text = tokens.to_string();
    for part in text.split(',') {
        let mut pieces = part.split('=');
        let Some(name) = pieces.next() else {
            continue;
        };
        if name.trim() != target {
            continue;
        }
        let Some(value) = pieces.next() else {
            continue;
        };
        return Ok(Some(value.trim().trim_matches('"').to_string()));
    }
    Ok(None)
}

fn parse_name_value_u32(tokens: proc_macro2::TokenStream, target: &str) -> Result<Option<u32>> {
    let text = tokens.to_string();
    for part in text.split(',') {
        let mut pieces = part.split('=');
        let Some(name) = pieces.next() else {
            continue;
        };
        if name.trim() != target {
            continue;
        }
        let Some(value) = pieces.next() else {
            continue;
        };
        let value = value.trim().replace('_', "");
        if let Some(rest) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            return u32::from_str_radix(rest, 16)
                .map(Some)
                .map_err(|err| syn::Error::new(Span::call_site(), err));
        }
        return value
            .parse::<u32>()
            .map(Some)
            .map_err(|err| syn::Error::new(Span::call_site(), err));
    }
    Ok(None)
}

struct FieldAttrs {
    field_id: Option<u16>,
    packed_bits: Option<u8>,
    skip: bool,
    rename: Option<Ident>,
}

fn parse_field_attrs(
    field: &Field,
) -> Result<FieldAttrs> {
    let mut field_id = None;
    let mut skip = false;
    let mut rename = None;

    for attr in &field.attrs {
        if attr.path().is_ident("zebin") {
            let tokens = attr.meta.require_list()?.tokens.clone();
            if let Some(value) = parse_name_value_u32(tokens.clone(), "id")? {
                field_id = Some(u16::try_from(value).map_err(|_| {
                    syn::Error::new(field.span(), "field id exceeds u16 range")
                })?);
            }
            if let Some(name) = parse_name_value_str(tokens.clone(), "rename")? {
                rename = Some(Ident::new(&name, field.span()));
            }
            let text = tokens.to_string();
            for part in text.split(',') {
                let part = part.trim();
                if part == "skip" || part == "skip_serializing" {
                    skip = true;
                }
            }
        }
    }

    let packed_bits = parse_packed_bits(field)?;
    Ok(FieldAttrs {
        field_id,
        packed_bits,
        skip,
        rename,
    })
}

fn parse_uint_after_token(text: &str, token: &str) -> Option<u32> {
    let start = text.find(token)?;
    let text = &text[start + token.len()..];
    let eq = text.find('=')?;
    let text = text[eq + 1..].trim_start();
    let mut end = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_hexdigit() || ch == 'x' || ch == 'X' || ch == '_' {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let value = text[..end].replace('_', "");
    if let Some(rest) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(rest, 16).ok()
    } else {
        value.parse::<u32>().ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackedElementKind {
    Bool,
    U8,
}

pub fn packed_element_kind(ty: &Type) -> Option<PackedElementKind> {
    match ty {
        Type::Path(path) => {
            let segment = path.path.segments.last()?;
            let ident = segment.ident.to_string();
            if ident != "Vec" {
                return None;
            }
            let inner = match &segment.arguments {
                syn::PathArguments::AngleBracketed(args) => {
                    args.args.iter().find_map(|arg| match arg {
                        syn::GenericArgument::Type(inner) => Some(inner),
                        _ => None,
                    })?
                }
                _ => return None,
            };
            match inner {
                Type::Path(inner_path) if inner_path.path.is_ident("bool") => {
                    Some(PackedElementKind::Bool)
                }
                Type::Path(inner_path) if inner_path.path.is_ident("u8") => {
                    Some(PackedElementKind::U8)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn packed_type_name(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

fn packed_error(field: &Field, detail: &str) -> syn::Error {
    syn::Error::new_spanned(
        &field.ty,
        format!("{detail}；当前字段类型是 `{}`", packed_type_name(&field.ty)),
    )
}

fn parse_packed_bits(field: &Field) -> Result<Option<u8>> {
    let mut packed_bits: Option<u8> = None;
    let kind = packed_element_kind(&field.ty);

    for attr in &field.attrs {
        if !attr.path().is_ident("zebin") {
            continue;
        }
        let tokens = attr.meta.require_list()?.tokens.to_string();
        if !tokens.contains("packed") && !tokens.contains("bits") {
            continue;
        }

        let explicit = parse_uint_after_token(&tokens, "packed")
            .or_else(|| parse_uint_after_token(&tokens, "bits"));

        let bits = match explicit {
            Some(bits) => bits,
            None => match kind {
                Some(PackedElementKind::Bool) => 1,
                Some(PackedElementKind::U8) => {
                    return Err(syn::Error::new(
                        field.span(),
                        "u8 packed 字段需要显式提供 bits",
                    ));
                }
                None => {
                    return Err(packed_error(
                        field,
                        "packed 只能用于 `Vec<bool>` 或 `Vec<u8>`",
                    ));
                }
            },
        };

        let bits = u8::try_from(bits)
            .map_err(|_| syn::Error::new(field.span(), "packed bits exceeds u8 range"))?;
        packed_bits = Some(bits);
    }

    if let Some(bits) = packed_bits {
        match kind {
            Some(PackedElementKind::Bool) if bits != 1 => {
                return Err(syn::Error::new(
                    field.span(),
                    "bool packed 字段只能使用 1 bit",
                ));
            }
            Some(PackedElementKind::U8) if bits == 0 || bits > 8 => {
                return Err(syn::Error::new(
                    field.span(),
                    "u8 packed 字段的 bits 必须在 1..=8",
                ));
            }
            Some(_) => {}
            None => {
                return Err(packed_error(
                    field,
                    "packed 只能用于 `Vec<bool>` 或 `Vec<u8>`",
                ));
            }
        }
    }

    Ok(packed_bits)
}

pub fn packed_info(field: &FieldSpec<'_>) -> Option<(PackedElementKind, u8)> {
    let bits = field.packed_bits?;
    let kind = packed_element_kind(field.ty)?;
    Some((kind, bits))
}

pub fn packed_wrapper_type(field: &FieldSpec<'_>) -> Option<proc_macro2::TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! { zebin::PackedSlice<'a, bool, 1> },
        PackedElementKind::U8 => quote! { zebin::PackedSlice<'a, u8, #bits> },
    })
}

pub fn packed_wrapper_type_expr(field: &FieldSpec<'_>) -> Option<proc_macro2::TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! { zebin::PackedSlice<'_, bool, 1> },
        PackedElementKind::U8 => quote! { zebin::PackedSlice<'_, u8, #bits> },
    })
}

pub fn packed_archived_type(field: &FieldSpec<'_>) -> Option<proc_macro2::TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! { zebin::ArchivedPackedBoolSlice },
        PackedElementKind::U8 => quote! { zebin::ArchivedPackedU8Slice<#bits> },
    })
}

fn is_rel_ptr_like(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return false;
            };
            matches!(
                segment.ident.to_string().as_str(),
                "String" | "Vec" | "VecDeque" | "Box" | "Rc" | "Arc" | "Cow"
            )
        }
        _ => false,
    }
}

fn is_varint_like(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "VarInt"),
        _ => false,
    }
}

pub fn field_encoding(field: &FieldSpec<'_>) -> proc_macro2::TokenStream {
    if field.packed_bits.is_some() {
        return quote! { zebin::FieldEncoding::PackedBits };
    }

    if is_varint_like(field.ty) {
        return quote! { zebin::FieldEncoding::VarInt };
    }

    if is_rel_ptr_like(field.ty) {
        return quote! { zebin::FieldEncoding::RelPtr };
    }

    quote! { zebin::FieldEncoding::Fixed }
}

pub fn packed_begin_expr(
    field: &FieldSpec<'_>,
    value: proc_macro2::TokenStream,
) -> Option<proc_macro2::TokenStream> {
    let (kind, bits) = packed_info(field)?;
    Some(match kind {
        PackedElementKind::Bool => quote! {
            zebin::PackedSequenceState::new_bool(#value.as_ref())
        },
        PackedElementKind::U8 => quote! {
            zebin::PackedSequenceState::new_u8(#value.as_ref(), #bits)
        },
    })
}

pub fn field_resolver_type(field: &FieldSpec<'_>) -> proc_macro2::TokenStream {
    if field.packed_bits.is_some() {
        quote! { usize }
    } else {
        let ty = field.ty;
        quote! { <#ty as zebin::Archive>::Resolver }
    }
}

fn parse_schema_key(attrs: &[syn::Attribute]) -> Result<Option<u32>> {
    let mut stable_schema_key: Option<u32> = None;
    for attr in attrs {
        if attr.path().is_ident("zebin") {
            let tokens = attr.meta.require_list()?.tokens.clone();
            if let Some(value) = parse_name_value_u32(tokens, "schema_key")? {
                stable_schema_key = Some(value);
            }
        }
    }
    Ok(stable_schema_key)
}

fn field_state_ident(field_name: Option<&Ident>, index: usize) -> Ident {
    match field_name {
        Some(name) => Ident::new(&format!("{}_state", name), name.span()),
        None => Ident::new(&format!("field{index}_state"), Span::call_site()),
    }
}

fn parse_fields_named(fields: &FieldsNamed) -> Result<RecordSpec<'_>> {
    let mut out = Vec::with_capacity(fields.named.len());
    for (index, field) in fields.named.iter().enumerate() {
        let ident = field.ident.as_ref();
        let attrs = parse_field_attrs(field)?;
        out.push(FieldSpec {
            ident,
            state_ident: field_state_ident(ident, index),
            ty: &field.ty,
            field_id: attrs.field_id,
            packed_bits: attrs.packed_bits,
            skip: attrs.skip,
            rename: attrs.rename,
        });
    }
    Ok(RecordSpec {
        style: RecordStyle::Named,
        fields: out,
        stable_schema_key: None,
        schema_revision: 0,
    })
}

fn parse_fields_unnamed(fields: &FieldsUnnamed) -> Result<RecordSpec<'_>> {
    let mut out = Vec::with_capacity(fields.unnamed.len());
    for (index, field) in fields.unnamed.iter().enumerate() {
        let attrs = parse_field_attrs(field)?;
        out.push(FieldSpec {
            ident: None,
            state_ident: field_state_ident(None, index),
            ty: &field.ty,
            field_id: attrs.field_id,
            packed_bits: attrs.packed_bits,
            skip: attrs.skip,
            rename: attrs.rename,
        });
    }
    Ok(RecordSpec {
        style: RecordStyle::Unnamed,
        fields: out,
        stable_schema_key: None,
        schema_revision: 0,
    })
}

fn parse_fields(fields: &Fields) -> Result<RecordSpec<'_>> {
    match fields {
        Fields::Named(named) => parse_fields_named(named),
        Fields::Unnamed(unnamed) => parse_fields_unnamed(unnamed),
        Fields::Unit => Ok(RecordSpec {
            style: RecordStyle::Unit,
            fields: Vec::new(),
            stable_schema_key: None,
            schema_revision: 0,
        }),
    }
}

fn parse_schema_revision(attrs: &[syn::Attribute]) -> Result<u32> {
    let mut schema_revision = 0u32;
    for attr in attrs {
        if attr.path().is_ident("zebin") {
            let tokens = attr.meta.require_list()?.tokens.clone();
            if let Some(value) = parse_name_value_u32(tokens, "revision")? {
                schema_revision = value;
            }
        }
    }
    Ok(schema_revision)
}

fn validate_field_ids(record: &RecordSpec<'_>, span: Span) -> Result<()> {
    let has_any_id = record
        .fields
        .iter()
        .any(|field| !field.skip && field.field_id.is_some());
    if has_any_id {
        for field in &record.fields {
            if !field.skip && field.field_id.is_none() {
                return Err(syn::Error::new(
                    span,
                    "启用 #[zebin(id = ...)] 后，所有非 skip 字段都必须提供 id",
                ));
            }
        }
    }
    Ok(())
}

fn finalize_record<'a>(
    mut record: RecordSpec<'a>,
    attrs: &[syn::Attribute],
    span: Span,
) -> Result<RecordSpec<'a>> {
    record.stable_schema_key = parse_schema_key(attrs)?;
    if !has_schema(&record) && record.stable_schema_key.is_some() {
        return Err(syn::Error::new(
            span,
            "未启用 schema 字段时，不能使用 #[zebin(schema_key = ...)]",
        ));
    }
    if has_schema(&record) && record.stable_schema_key.is_none() {
        return Err(syn::Error::new(
            span,
            "启用 schema 字段后，必须同时提供 #[zebin(schema_key = ...)]",
        ));
    }
    Ok(record)
}

/// Parses a DeriveInput into an ItemSpec.
pub fn parse_item(input: &DeriveInput) -> Result<ItemSpec<'_>> {
    let schema_revision = parse_schema_revision(&input.attrs)?;
    match &input.data {
        Data::Struct(DataStruct { fields, .. }) => {
            let mut record = parse_fields(fields)?;
            record = finalize_record(record, &input.attrs, input.span())?;
            record.schema_revision = schema_revision;
            validate_field_ids(&record, input.span())?;
            Ok(ItemSpec::Struct(record))
        }
        Data::Enum(DataEnum { variants, .. }) => {
            let mut parsed = Vec::with_capacity(variants.len());
            for variant in variants {
                let mut record = parse_fields(&variant.fields)?;
                record = finalize_record(record, &variant.attrs, variant.span())?;
                record.schema_revision = schema_revision;
                validate_field_ids(&record, variant.span())?;

                let mut rename = None;
                for attr in &variant.attrs {
                    if attr.path().is_ident("zebin") {
                        let tokens = attr.meta.require_list()?.tokens.clone();
                        if let Some(name) = parse_name_value_str(tokens, "rename")? {
                            rename = Some(Ident::new(&name, variant.span()));
                        }
                    }
                }

                parsed.push(VariantSpec {
                    ident: &variant.ident,
                    rename,
                    record,
                });
            }
            Ok(ItemSpec::Enum(parsed))
        }
        _ => Err(syn::Error::new(
            input.span(),
            "ZebinArchive 和 ZebinSerialize 只支持 struct 与 enum",
        )),
    }
}

pub fn has_schema(record: &RecordSpec<'_>) -> bool {
    record
        .fields
        .iter()
        .any(|field| !field.skip && field.field_id.is_some())
}

pub fn user_member(record: &RecordSpec<'_>, index: usize) -> Member {
    let field = &record.fields[index];
    match record.style {
        RecordStyle::Named => {
            let ident = field
                .rename
                .as_ref()
                .unwrap_or_else(|| field.ident.expect("named field has ident"));
            Member::Named(ident.clone())
        }
        RecordStyle::Unnamed => {
            let active_index = record.fields[..index].iter().filter(|f| !f.skip).count();
            Member::Unnamed(Index::from(active_index + usize::from(has_schema(record))))
        }
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

pub fn layout_field_entries(
    record: &RecordSpec<'_>,
    archived_name: &Ident,
) -> Vec<proc_macro2::TokenStream> {
    let mut fields: Vec<_> = record
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.skip)
        .collect();

    // Sort fields by field_id to ensure LayoutDescriptor::new doesn't fail with LayoutError
    fields.sort_by_key(|(_, field)| field.field_id.expect("field ids are validated above"));

    fields
        .into_iter()
        .map(|(index, field)| {
            let field_id = field.field_id.expect("field ids are validated above");
            let member = user_member(record, index);
            let encoding = field_encoding(field);
            quote::quote! {
                zebin::LayoutField {
                    field_id: #field_id,
                    offset: zebin::memoffset::offset_of!(#archived_name, #member) as u32,
                    encoding: #encoding,
                }
            }
        })
        .collect()
}

pub fn input_member(record: &RecordSpec<'_>, index: usize) -> Member {
    match record.style {
        RecordStyle::Named => Member::Named(
            record.fields[index]
                .ident
                .expect("named field has ident")
                .clone(),
        ),
        RecordStyle::Unnamed => Member::Unnamed(Index::from(index)),
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

fn unnamed_slot_ident(prefix: &str, index: usize) -> Ident {
    Ident::new(&format!("{prefix}{index}"), Span::call_site())
}

/// Identifier used for a generated state field at a particular slot.
pub fn state_slot_ident(record: &RecordSpec<'_>, index: usize) -> Ident {
    match record.style {
        RecordStyle::Named => record.fields[index]
            .ident
            .expect("named field has ident")
            .clone(),
        RecordStyle::Unnamed => unnamed_slot_ident("field", index),
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
}

/// Identifier used for a generated resolver field at a particular slot.
pub fn resolver_slot_ident(record: &RecordSpec<'_>, index: usize) -> Ident {
    state_slot_ident(record, index)
}

/// Identifier used for a generated input binder at a particular slot.
pub fn binder_slot_ident(record: &RecordSpec<'_>, index: usize) -> Ident {
    state_slot_ident(record, index)
}

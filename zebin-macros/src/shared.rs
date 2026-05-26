use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
    Data, DataEnum, DataStruct, DeriveInput, Fields, Ident, Index, Member, Result, Type,
    spanned::Spanned,
};

mod attrs;
pub mod packed;

pub use packed::packed_wrapper_type_expr;

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
    pub default: bool,
    pub default_value: Option<syn::Expr>,
}

/// Specification of a struct or enum variant.
pub struct RecordSpec<'a> {
    pub style: RecordStyle,
    pub fields: Vec<FieldSpec<'a>>,
    pub stable_schema_key: Option<u32>,
    pub schema_revision: u32,
}

impl<'a> RecordSpec<'a> {
    pub fn active_fields(&self) -> impl Iterator<Item = (usize, &FieldSpec<'a>)> {
        self.fields.iter().enumerate().filter(|(_, f)| !f.skip)
    }

    pub fn has_schema(&self) -> bool {
        self.fields
            .iter()
            .any(|field| !field.skip && field.field_id.is_some())
    }
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

// --- Naming Utilities ---

pub fn archived_name(name: &Ident) -> Ident {
    format_ident!("Archived{}", name)
}

pub fn state_name(name: &Ident) -> Ident {
    format_ident!("{}ArchiveState", name)
}

pub fn variant_archived_name(enum_name: &Ident, variant: &Ident) -> Ident {
    format_ident!("Archived{}{}", enum_name, variant)
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
    for ch in name.chars() {
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

// --- Logic functions ---

pub fn field_encoding(field: &FieldSpec<'_>) -> TokenStream {
    if field.packed_bits.is_some() {
        return quote! { zebin::schema::FieldEncoding::PackedBits };
    }
    let archived_ty = field_archived_type(field);
    quote! { <#archived_ty as zebin::io::ArchivedLayout>::FIELD_ENCODING }
}

pub fn field_archived_type(field: &FieldSpec<'_>) -> TokenStream {
    if let Some(archived) = packed::packed_archived_type(field) {
        archived
    } else {
        let ty = field.ty;
        quote! { <#ty as zebin::Archive>::Archived }
    }
}

pub fn field_view_type(field: &FieldSpec<'_>) -> TokenStream {
    let archived = field_archived_type(field);
    quote! { <#archived as zebin::Decode<'a>>::View }
}

pub fn field_len_ident(record: &RecordSpec<'_>, index: usize) -> Ident {
    let base = field_user_ident(record, index);
    Ident::new(&format!("{}_len", base), base.span())
}

pub fn active_fields_by_id<'a>(record: &'a RecordSpec<'a>) -> Vec<(usize, &'a FieldSpec<'a>)> {
    let mut fields: Vec<_> = record.active_fields().collect();
    fields.sort_by_key(|(_, field)| field.field_id.unwrap_or(u16::MAX));
    fields
}

pub fn field_state_type(field: &FieldSpec<'_>) -> TokenStream {
    if let Some((kind, bits)) = packed::packed_info(field) {
        match kind {
            packed::PackedElementKind::Bool => {
                quote! { zebin::archive::PackedBoolVecEncoder }
            }
            packed::PackedElementKind::U8 => {
                quote! { zebin::archive::PackedU8VecEncoder<#bits> }
            }
        }
    } else {
        let ty = field.ty;
        quote! { <#ty as zebin::Encode>::Encoder<'a> }
    }
}

pub fn field_user_ident(record: &RecordSpec<'_>, index: usize) -> Ident {
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

pub fn has_schema(record: &RecordSpec<'_>) -> bool {
    record.has_schema()
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

pub fn parse_item(input: &DeriveInput) -> Result<ItemSpec<'_>> {
    let schema_revision = attrs::parse_schema_revision(&input.attrs)?;
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
                        if let Some(name) = attrs::parse_name_value_str(tokens, "rename")? {
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
            "ZebinArchive 和 ZebinEncode 只支持 struct 与 enum",
        )),
    }
}

fn parse_fields(fields: &Fields) -> Result<RecordSpec<'_>> {
    match fields {
        Fields::Named(named) => {
            let mut out = Vec::with_capacity(named.named.len());
            for (index, field) in named.named.iter().enumerate() {
                let ident = field.ident.as_ref();
                let attrs = attrs::parse_field_attrs(field)?;
                out.push(FieldSpec {
                    ident,
                    state_ident: field_state_ident(ident, index),
                    ty: &field.ty,
                    field_id: attrs.field_id,
                    packed_bits: attrs.packed_bits,
                    skip: attrs.skip,
                    rename: attrs.rename,
                    default: attrs.default,
                    default_value: attrs.default_value,
                });
            }
            Ok(RecordSpec {
                style: RecordStyle::Named,
                fields: out,
                stable_schema_key: None,
                schema_revision: 0,
            })
        }
        Fields::Unnamed(unnamed) => {
            let mut out = Vec::with_capacity(unnamed.unnamed.len());
            for (index, field) in unnamed.unnamed.iter().enumerate() {
                let attrs = attrs::parse_field_attrs(field)?;
                out.push(FieldSpec {
                    ident: None,
                    state_ident: field_state_ident(None, index),
                    ty: &field.ty,
                    field_id: attrs.field_id,
                    packed_bits: attrs.packed_bits,
                    skip: attrs.skip,
                    rename: attrs.rename,
                    default: attrs.default,
                    default_value: attrs.default_value,
                });
            }
            Ok(RecordSpec {
                style: RecordStyle::Unnamed,
                fields: out,
                stable_schema_key: None,
                schema_revision: 0,
            })
        }
        Fields::Unit => Ok(RecordSpec {
            style: RecordStyle::Unit,
            fields: Vec::new(),
            stable_schema_key: None,
            schema_revision: 0,
        }),
    }
}

fn field_state_ident(field_name: Option<&Ident>, index: usize) -> Ident {
    match field_name {
        Some(name) => Ident::new(&format!("{}_state", name), name.span()),
        None => Ident::new(&format!("field{index}_state"), Span::call_site()),
    }
}

fn finalize_record<'a>(
    mut record: RecordSpec<'a>,
    attrs: &[syn::Attribute],
    span: Span,
) -> Result<RecordSpec<'a>> {
    record.stable_schema_key = attrs::parse_schema_key(attrs)?;
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

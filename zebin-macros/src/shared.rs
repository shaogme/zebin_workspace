use proc_macro2::Span;
use quote::format_ident;
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

fn parse_field_id(field: &Field) -> Result<Option<u16>> {
    let mut field_id: Option<u16> = None;
    for attr in &field.attrs {
        if attr.path().is_ident("zebin") {
            let tokens = attr.meta.require_list()?.tokens.clone();
            if let Some(value) = parse_name_value_u32(tokens, "id")? {
                field_id =
                    Some(u16::try_from(value).map_err(|_| {
                        syn::Error::new(field.span(), "field id exceeds u16 range")
                    })?);
            }
        }
    }
    Ok(field_id)
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
        out.push(FieldSpec {
            ident,
            state_ident: field_state_ident(ident, index),
            ty: &field.ty,
            field_id: parse_field_id(field)?,
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
        out.push(FieldSpec {
            ident: None,
            state_ident: field_state_ident(None, index),
            ty: &field.ty,
            field_id: parse_field_id(field)?,
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
    let has_any_id = record.fields.iter().any(|field| field.field_id.is_some());
    if has_any_id {
        for field in &record.fields {
            if field.field_id.is_none() {
                return Err(syn::Error::new(
                    span,
                    "启用 #[zebin(id = ...)] 后，所有字段都必须提供 id",
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
                parsed.push(VariantSpec {
                    ident: &variant.ident,
                    record,
                });
            }
            Ok(ItemSpec::Enum(parsed))
        }
        _ => Err(syn::Error::new(
            input.span(),
            "ZebinArchive 和 ZebinArchiveBuilder 只支持 struct 与 enum",
        )),
    }
}

pub fn has_schema(record: &RecordSpec<'_>) -> bool {
    record.fields.iter().any(|field| field.field_id.is_some())
}

pub fn user_member(record: &RecordSpec<'_>, index: usize) -> Member {
    match record.style {
        RecordStyle::Named => Member::Named(
            record.fields[index]
                .ident
                .expect("named field has ident")
                .clone(),
        ),
        RecordStyle::Unnamed => {
            Member::Unnamed(Index::from(index + usize::from(has_schema(record))))
        }
        RecordStyle::Unit => unreachable!("unit has no fields"),
    }
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

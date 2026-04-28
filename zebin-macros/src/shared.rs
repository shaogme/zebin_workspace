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
    format_ident!("{}SerializeState", name)
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
    format_ident!("{}{}SerializeState", enum_name, variant)
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
    })
}

fn parse_fields(fields: &Fields) -> Result<RecordSpec<'_>> {
    match fields {
        Fields::Named(named) => parse_fields_named(named),
        Fields::Unnamed(unnamed) => parse_fields_unnamed(unnamed),
        Fields::Unit => Ok(RecordSpec {
            style: RecordStyle::Unit,
            fields: Vec::new(),
        }),
    }
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

/// Parses a DeriveInput into an ItemSpec.
pub fn parse_item(input: &DeriveInput) -> Result<ItemSpec<'_>> {
    match &input.data {
        Data::Struct(DataStruct { fields, .. }) => {
            let record = parse_fields(fields)?;
            validate_field_ids(&record, input.span())?;
            Ok(ItemSpec::Struct(record))
        }
        Data::Enum(DataEnum { variants, .. }) => {
            let mut parsed = Vec::with_capacity(variants.len());
            for variant in variants {
                let record = parse_fields(&variant.fields)?;
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
            "ZebinArchive 和 ZebinSerialize 只支持 struct 与 enum",
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

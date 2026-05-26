use core::convert::TryFrom;
use proc_macro2::{Span, TokenStream};
use syn::spanned::Spanned;
use syn::{Error, Field, Ident, Result};

pub struct FieldAttrs {
    pub field_id: Option<u16>,
    pub packed_bits: Option<u8>,
    pub skip: bool,
    pub rename: Option<Ident>,
    pub default: bool,
    pub default_value: Option<syn::Expr>,
}

pub fn parse_field_attrs(field: &Field) -> Result<FieldAttrs> {
    let mut field_id = None;
    let mut skip = false;
    let mut rename = None;
    let mut default = false;
    let mut default_value = None;

    for attr in &field.attrs {
        if attr.path().is_ident("zebin") {
            let tokens = attr.meta.require_list()?.tokens.clone();
            if let Some(value) = parse_name_value_u32(tokens.clone(), "id")? {
                field_id = Some(
                    u16::try_from(value)
                        .map_err(|_| Error::new(field.span(), "field id exceeds u16 range"))?,
                );
            }
            if let Some(name) = parse_name_value_str(tokens.clone(), "rename")? {
                rename = Some(Ident::new(&name, field.span()));
            }
            if let Some(expr_str) = parse_name_value_str(tokens.clone(), "default_value")? {
                default_value = Some(syn::parse_str(&expr_str)?);
            }
            let text = tokens.to_string();
            for part in text.split(',') {
                let part = part.trim();
                if part == "skip" || part == "skip_serializing" {
                    skip = true;
                }
                if part == "default" {
                    default = true;
                }
            }
        }
    }

    let packed_bits = super::packed::parse_packed_bits(field)?;
    Ok(FieldAttrs {
        field_id,
        packed_bits,
        skip,
        rename,
        default,
        default_value,
    })
}

pub fn parse_name_value_str(tokens: TokenStream, target: &str) -> Result<Option<String>> {
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

pub fn parse_name_value_u32(tokens: TokenStream, target: &str) -> Result<Option<u32>> {
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
                .map_err(|err| Error::new(Span::call_site(), err));
        }
        return value
            .parse::<u32>()
            .map(Some)
            .map_err(|err| Error::new(Span::call_site(), err));
    }
    Ok(None)
}

pub fn parse_uint_after_token(text: &str, token: &str) -> Option<u32> {
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

pub fn parse_schema_key(attrs: &[syn::Attribute]) -> Result<Option<u32>> {
    let mut stable_schema_key = None;
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

pub fn parse_schema_revision(attrs: &[syn::Attribute]) -> Result<u32> {
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
